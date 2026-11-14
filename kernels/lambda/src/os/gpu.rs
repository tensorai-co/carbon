//! The machine's GPU, as Linux programs reach one: the render node `/dev/dri/renderD128` (character device 226:128),
//! which lambda's Vulkan driver opens. It answers DRM's version query, as every DRM driver does (`drmGetVersion`
//! says `carbon`), and the driver's own commands from DRM's driver range (`gpu_wire`): the device's description, and
//! the doorbell, which hands the records the driver collected to its Vulkan context (`gpu_api::vulkan`), one for each
//! open of the node. The device itself is `gpu_api`'s, on a thread of its own; this is only the way in, and the
//! guest's memory the context reads and writes is the ringing process's.
//!
//! The node is put in `/dev/dri` at boot when the machine has a GPU (`--gpu` other than `none`), with its sysfs entry
//! (`/sys/class/drm/renderD128`), and opening it on a machine without one is ENXIO, as on Linux with no driver.

use std::sync::Arc;

use kernels_core::thread::Cx;
use kernels_core::user;

use crate::abi::*;
use crate::process::Task;

pub(crate) const MAJOR: u64 = 226;
pub(crate) const MINOR: u64 = 128;
/// Its device number, as the disk keeps it: the major above the minor's byte.
pub(crate) const RDEV: u64 = MAJOR << 8 | MINOR;
pub(crate) const PATH: &str = "/dev/dri/renderD128";

/// The DRM version this driver says it is.
const VERSION: (i32, i32, i32) = (1, 0, 0);
/// The most records one doorbell may hand over.
const MOST_RECORDS: u64 = 1 << 30;

/// An open of the render node: the machine's GPU, and the Vulkan objects the driver that opened it made.
pub(crate) struct Drm {
    gpu: gpu_api::Gpu,
    vulkan: gpu_api::vulkan::Context,
}

impl Drm {
    pub(crate) fn new(gpu: gpu_api::Gpu) -> Arc<Drm> {
        Arc::new(Drm { vulkan: gpu_api::vulkan::Context::new(gpu.clone()), gpu })
    }
}

/// The ringing process's memory, as the Vulkan context reads and writes it.
struct Ringing<'t, 'c, 'x> {
    task: &'t mut Task,
    cx: &'c mut Cx<'x>,
}

impl gpu_api::vulkan::Guest for Ringing<'_, '_, '_> {
    fn read(&mut self, addr: u64, out: &mut [u8]) -> Result<(), String> {
        self.task.mem(self.cx, false, |t| user::read(&t.space(), addr, out)).map_err(|_| format!("the driver's memory at {addr:#x} can't be read"))
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), String> {
        self.task.mem(self.cx, true, |t| user::write(&mut t.space(), addr, data)).map_err(|_| format!("the driver's memory at {addr:#x} can't be written"))
    }
}

impl Task {
    /// An ioctl on the render node.
    pub(crate) fn drm_ioctl(&mut self, cx: &mut Cx<'_>, drm: &Drm, request: u64, arg: u64) -> Result<u64, Errno> {
        match request {
            gpu_wire::DRM_IOCTL_VERSION => {
                // struct drm_version: three ints and a pad, then (length, pointer) for the name, date and description.
                // Each text is copied as far as the caller's length allows, and its whole length given back.
                let header = self.mem(cx, false, |t| user::read_vec(&t.space(), arg, gpu_wire::DRM_VERSION_SIZE))?;
                let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap_or_default());
                let mut out = header.clone();
                out[0..4].copy_from_slice(&VERSION.0.to_le_bytes());
                out[4..8].copy_from_slice(&VERSION.1.to_le_bytes());
                out[8..12].copy_from_slice(&VERSION.2.to_le_bytes());
                for (at, text) in [(16, gpu_wire::DRIVER_NAME), (32, gpu_wire::DRIVER_DATE), (48, gpu_wire::DRIVER_DESCRIPTION)] {
                    let (room, buffer) = (word(at), word(at + 8));
                    let n = (room as usize).min(text.len());
                    if n > 0 && buffer != 0 {
                        self.mem(cx, true, |t| user::write(&mut t.space(), buffer, &text.as_bytes()[..n]))?;
                    }
                    out[at..at + 8].copy_from_slice(&(text.len() as u64).to_le_bytes());
                }
                self.mem(cx, true, |t| user::write(&mut t.space(), arg, &out))?;
                Ok(0)
            }
            gpu_wire::INFO => {
                let bytes = drm.gpu.wire_info().to_bytes();
                self.mem(cx, true, |t| user::write(&mut t.space(), arg, &bytes))?;
                Ok(0)
            }
            gpu_wire::SUBMIT_IOCTL => {
                let words = self.mem(cx, false, |t| user::read_vec(&t.space(), arg, gpu_wire::SUBMIT_SIZE))?;
                let word = |i: usize| u64::from_le_bytes(words[i * 8..i * 8 + 8].try_into().unwrap_or_default());
                let (at, len, reply, room) = (word(0), word(1), word(2), word(3));
                if len > MOST_RECORDS {
                    return Err(E2BIG);
                }
                let records = self.mem(cx, false, |t| user::read_vec(&t.space(), at, len as usize))?;
                match drm.vulkan.execute(&records, &mut Ringing { task: self, cx }) {
                    Ok(()) => Ok(0),
                    // What failed, for the driver to say: its length is the ioctl's answer.
                    Err(message) => {
                        let message = message.as_bytes();
                        let n = message.len().min(room as usize);
                        if n > 0 && reply != 0 {
                            self.mem(cx, true, |t| user::write(&mut t.space(), reply, &message[..n]))?;
                        }
                        Ok(message.len().max(1) as u64)
                    }
                }
            }
            // DRM's other ioctls (capabilities, authentication, GEM, KMS) and other drivers': this one has none of them.
            _ if (request >> 8) & 0xff == u64::from(gpu_wire::DRM_TYPE) => Err(EINVAL),
            _ => Err(ENOTTY),
        }
    }
}
