//! The mix on a Mac's speakers: Core Audio's default output unit (an `AudioUnit` of the `DefaultOutput` kind, which
//! follows whatever output the user picks), fed 48 kHz stereo floats by a render callback that pulls them from the
//! mixer. Core Audio calls it on its own real-time thread as the device needs frames, so the device's clock is the
//! mixer's: nothing drifts, and nothing else paces the mix. Core Audio converts to the device's own rate and channels.

#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::ptr;

use audio::{CHANNELS, Mixer, RATE};

#[repr(C)]
struct ComponentDescription {
    kind: u32,
    sub_kind: u32,
    manufacturer: u32,
    flags: u32,
    flags_mask: u32,
}

#[repr(C)]
struct StreamDescription {
    sample_rate: f64,
    format: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}

#[repr(C)]
struct Buffer {
    channels: u32,
    bytes: u32,
    data: *mut c_void,
}

#[repr(C)]
struct BufferList {
    count: u32,
    buffers: [Buffer; 1],
}

type Render = unsafe extern "C" fn(refcon: *mut c_void, flags: *mut u32, time: *const c_void, bus: u32, frames: u32, data: *mut BufferList) -> i32;

#[repr(C)]
struct RenderCallback {
    proc_: Render,
    refcon: *mut c_void,
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioComponentFindNext(after: *mut c_void, desc: *const ComponentDescription) -> *mut c_void;
    fn AudioComponentInstanceNew(component: *mut c_void, out: *mut *mut c_void) -> i32;
    fn AudioComponentInstanceDispose(unit: *mut c_void) -> i32;
    fn AudioUnitSetProperty(unit: *mut c_void, id: u32, scope: u32, element: u32, data: *const c_void, size: u32) -> i32;
    fn AudioUnitInitialize(unit: *mut c_void) -> i32;
    fn AudioUnitUninitialize(unit: *mut c_void) -> i32;
    fn AudioOutputUnitStart(unit: *mut c_void) -> i32;
    fn AudioOutputUnitStop(unit: *mut c_void) -> i32;
}

const fn four(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

const STREAM_FORMAT: u32 = 8;
const SET_RENDER_CALLBACK: u32 = 23;
const SCOPE_INPUT: u32 = 1;
const SCOPE_GLOBAL: u32 = 0;
/// `kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked`: interleaved floats.
const FLOAT_PACKED: u32 = 1 | 8;

/// The mix playing on the Mac's default output until this is dropped.
pub struct Speakers {
    unit: *mut c_void,
    /// The callback's own handle on the mixer, for as long as the unit may call it.
    mixer: *mut Mixer,
}

// SAFETY: the unit is Core Audio's, driven from its own thread; this only starts, stops and frees it.
unsafe impl Send for Speakers {}

impl Speakers {
    /// Starts the default output pulling `mixer`'s mix.
    pub fn start(mixer: Mixer) -> Result<Speakers, String> {
        let desc = ComponentDescription { kind: four(b"auou"), sub_kind: four(b"def "), manufacturer: four(b"appl"), flags: 0, flags_mask: 0 };
        // SAFETY: a description Core Audio reads, and a handle it writes; checked as each call returns.
        unsafe {
            let component = AudioComponentFindNext(ptr::null_mut(), &desc);
            if component.is_null() {
                return Err("the Mac has no default sound output".into());
            }
            let mut unit = ptr::null_mut();
            check(AudioComponentInstanceNew(component, &mut unit), "making the sound output")?;
            let mixer = Box::into_raw(Box::new(mixer));
            let speakers = Speakers { unit, mixer };
            let bytes = (CHANNELS * 4) as u32;
            let format = StreamDescription {
                sample_rate: f64::from(RATE),
                format: four(b"lpcm"),
                format_flags: FLOAT_PACKED,
                bytes_per_packet: bytes,
                frames_per_packet: 1,
                bytes_per_frame: bytes,
                channels: CHANNELS as u32,
                bits: 32,
                reserved: 0,
            };
            let size = |n: usize| n as u32;
            check(
                AudioUnitSetProperty(unit, STREAM_FORMAT, SCOPE_INPUT, 0, (&format as *const StreamDescription).cast(), size(size_of::<StreamDescription>())),
                "setting the sound output's format",
            )?;
            let callback = RenderCallback { proc_: render, refcon: speakers.mixer.cast() };
            check(
                AudioUnitSetProperty(unit, SET_RENDER_CALLBACK, SCOPE_GLOBAL, 0, (&callback as *const RenderCallback).cast(), size(size_of::<RenderCallback>())),
                "giving the sound output its callback",
            )?;
            check(AudioUnitInitialize(unit), "starting the sound output")?;
            check(AudioOutputUnitStart(unit), "starting the sound output")?;
            Ok(speakers)
        }
    }
}

impl Drop for Speakers {
    fn drop(&mut self) {
        // SAFETY: the unit is stopped and freed before the mixer handle its callback uses is.
        unsafe {
            AudioOutputUnitStop(self.unit);
            AudioUnitUninitialize(self.unit);
            AudioComponentInstanceDispose(self.unit);
            drop(Box::from_raw(self.mixer));
        }
    }
}

fn check(status: i32, what: &str) -> Result<(), String> {
    if status == 0 { Ok(()) } else { Err(format!("{what}: Core Audio's error {status}")) }
}

/// Core Audio wants `frames` frames: the mix's next ones, in the one interleaved buffer it gave.
unsafe extern "C" fn render(refcon: *mut c_void, _flags: *mut u32, _time: *const c_void, _bus: u32, frames: u32, data: *mut BufferList) -> i32 {
    // SAFETY: `refcon` is the mixer handle `Speakers` keeps until the unit's stopped; `data` is Core Audio's buffer
    // list for this call, of one interleaved buffer in the format set.
    unsafe {
        let mixer = &*(refcon as *const Mixer);
        let list = &mut *data;
        if list.count == 0 || list.buffers[0].data.is_null() {
            return 0;
        }
        let b = &mut list.buffers[0];
        let n = (frames as usize * CHANNELS).min(b.bytes as usize / 4);
        let out = std::slice::from_raw_parts_mut(b.data.cast::<f32>(), n);
        mixer.mix(out);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio::{Format, Sample};

    /// The default output pulls the mix at the device's pace: a second's quiet stream is taken as fast as it plays.
    #[test]
    fn the_default_output_pulls_the_mix_in_real_time() {
        let mixer = Mixer::new();
        let stream = mixer.open(Format::new(Sample::S16Le, 2, RATE), RATE as usize * 4, 0).unwrap();
        stream.write(&vec![0; RATE as usize * 4]);
        let speakers = match Speakers::start(mixer.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("no sound output to test on: {e}");
                return;
            }
        };
        std::thread::sleep(std::time::Duration::from_millis(300));
        drop(speakers);
        let played = stream.played();
        // About 14,400 frames in 300 ms; the device starts late, and pulls ahead by a buffer.
        assert!((4_000..=24_000).contains(&played), "{played} frames in 300 ms");
        assert!(mixer.frames() >= played);
    }
}
