//! Each API's guest calls lowered onto a machine's [`gpu::Device`], host-side.
//!
//! A machine's device lives on a thread of its own, the service, which opens it and is the only one to touch it:
//! a device is the backend's, and Metal's objects aren't shared between threads here. [`Gpu`] is the handle the
//! kernels get (cloned for each that needs it); what they ask of the device runs on the service ([`Gpu::run`]) and
//! comes back.
//!
//! Built so far: the device's description, for lambda's render node, and [`vulkan`]: a Vulkan driver's records
//! lowered onto the device (memory, compute pipelines, dispatches, copies and fills).

pub mod vulkan;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

pub use gpu::{Choice, Info, Kind};

/// Work for the service: something to do with the device.
type Job = Box<dyn FnOnce(&mut gpu::Device) + Send>;

/// A machine's GPU: its device's description, and the service that owns the device.
#[derive(Clone)]
pub struct Gpu {
    inner: Arc<Inner>,
}

struct Inner {
    info: Info,
    /// Jobs for the service; it ends when the last handle goes.
    jobs: Mutex<mpsc::Sender<Job>>,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gpu").field("info", &self.inner.info).finish()
    }
}

impl Gpu {
    /// The device a machine gets, opened on its service's thread; `None` for [`Choice::None`].
    pub fn open(choice: Choice) -> Result<Option<Gpu>, gpu::Error> {
        let (opened, open) = mpsc::channel();
        let (jobs, incoming) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("gpu".into())
            .spawn(move || {
                let mut device = match gpu::Device::open(choice) {
                    Ok(Some(d)) => d,
                    other => {
                        let _ = opened.send(other.map(|_| None));
                        return;
                    }
                };
                let _ = opened.send(Ok(Some(device.info().clone())));
                // Until every handle is gone: the device is dropped on this thread, where it was made.
                for job in incoming {
                    job(&mut device);
                }
                drop(device);
            })
            .map_err(|e| gpu::Error { message: format!("starting the gpu service: {e}") })?;
        let info = open.recv().map_err(|_| gpu::Error { message: "the gpu service ended as it started".into() })??;
        Ok(info.map(|info| Gpu { inner: Arc::new(Inner { info, jobs: Mutex::new(jobs) }) }))
    }

    pub fn info(&self) -> &Info {
        &self.inner.info
    }

    /// Runs `f` with the device, on the service's thread, and gives back what it returns.
    pub fn run<R: Send + 'static>(&self, f: impl FnOnce(&mut gpu::Device) -> R + Send + 'static) -> Result<R, gpu::Error> {
        let (done, result) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |device| {
            let _ = done.send(f(device));
        });
        let ended = || gpu::Error { message: "the gpu service has ended".into() };
        self.inner.jobs.lock().unwrap_or_else(|p| p.into_inner()).send(job).map_err(|_| ended())?;
        result.recv().map_err(|_| ended())
    }

    /// The description the render node gives the guest's driver.
    pub fn wire_info(&self) -> gpu_wire::Info {
        let i = &self.inner.info;
        gpu_wire::Info {
            version: gpu_wire::VERSION,
            kind: if i.kind == Kind::Host { gpu_wire::KIND_HOST } else { gpu_wire::KIND_SOFTWARE },
            subgroup_size: i.subgroup_size,
            max_workgroup_invocations: i.max_workgroup_invocations,
            name: gpu_wire::padded(&i.name),
            backend: gpu_wire::padded(i.backend),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cpu_device_through_its_service() {
        let gpu = Gpu::open(Choice::Software).unwrap().unwrap();
        assert_eq!(gpu.info().kind, Kind::Software);
        let info = gpu.wire_info();
        assert_eq!(info.kind, gpu_wire::KIND_SOFTWARE);
        assert_eq!(&info.name[..8], b"software");
        let memory = gpu.run(|d| d.alloc(64)).unwrap().unwrap();
        gpu.run(move |d| d.write(memory, 0, &[7; 64])).unwrap().unwrap();
        let back = gpu.run(move |d| {
            let mut out = [0u8; 4];
            d.read(memory, 60, &mut out).map(|()| out)
        });
        assert_eq!(back.unwrap().unwrap(), [7; 4]);
        assert!(Gpu::open(Choice::None).unwrap().is_none());
    }
}
