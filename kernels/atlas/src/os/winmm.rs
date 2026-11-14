//! winmm.dll: the multimedia timer's time and resolution, written; its sound as a machine with no sound devices has
//! it, until `audio` gives it some: no wave input or output devices and no mixers, so a device's number is
//! MMSYSERR_BADDEVICEID (the default device, WAVE_MAPPER, MMSYSERR_NODRIVER) and a handle, which none can be,
//! MMSYSERR_INVALHANDLE. MIDI is declared so programs that import it load, and says it isn't written yet.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "WINMM.dll";
    missing PlaySoundW;
    host mixerClose(1) => bad_handle;
    host mixerGetControlDetailsA(3) => bad_handle;
    host mixerGetDevCapsA(3) => bad_device;
    host mixerGetID(3) => bad_handle;
    host mixerGetLineControlsA(3) => bad_handle;
    host mixerGetLineInfoA(3) => bad_handle;
    host mixerOpen(5) => open_device;
    host mixerSetControlDetails(3) => bad_handle;
    host timeBeginPeriod(1) => time_period;
    host timeEndPeriod(1) => time_period;
    host timeGetTime(0) => time_get_time;
    missing timeKillEvent;
    missing timeSetEvent;
    host waveInAddBuffer(3) => bad_handle;
    host waveInClose(1) => bad_handle;
    host waveInGetDevCapsW(3) => bad_device;
    host waveInGetNumDevs(0) => no_devices;
    host waveInMessage(4) => bad_device;
    host waveInOpen(6) => open_device;
    host waveInPrepareHeader(3) => bad_handle;
    host waveInReset(1) => bad_handle;
    host waveInStart(1) => bad_handle;
    host waveInUnprepareHeader(3) => bad_handle;
    host waveOutClose(1) => bad_handle;
    host waveOutGetDevCapsA(3) => bad_device;
    host waveOutGetDevCapsW(3) => bad_device;
    host waveOutGetErrorTextW(3) => wave_out_get_error_text_w;
    host waveOutGetID(2) => bad_handle;
    host waveOutGetNumDevs(0) => no_devices;
    host waveOutMessage(4) => bad_device;
    host waveOutOpen(6) => open_device;
    host waveOutPrepareHeader(3) => bad_handle;
    host waveOutReset(1) => bad_handle;
    host waveOutUnprepareHeader(3) => bad_handle;
    host waveOutWrite(3) => bad_handle;
    // Not written yet: what larger programs import.
    missing midiInAddBuffer;
    missing midiInClose;
    missing midiInGetDevCapsW;
    missing midiInGetNumDevs;
    missing midiInOpen;
    missing midiInPrepareHeader;
    missing midiInReset;
    missing midiInStart;
    missing midiInUnprepareHeader;
    missing midiOutClose;
    missing midiOutGetDevCapsW;
    missing midiOutGetNumDevs;
    missing midiOutLongMsg;
    missing midiOutOpen;
    missing midiOutPrepareHeader;
    missing midiOutReset;
    missing midiOutShortMsg;
    missing midiOutUnprepareHeader;
    missing waveOutPause;
    missing waveOutRestart;
}

/// `timeGetTime()`: milliseconds since the machine started, wrapping at 2^32.
fn time_get_time(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from((task.k.host.monotonic() / 1_000_000) as u32))
}

/// `timeBeginPeriod(ms)` and `timeEndPeriod`: TIMERR_NOERROR; the host's timers are as fine as they get anyway.
fn time_period(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if a[0] as u32 == 0 { 97 } else { 0 })
}

// ---- Sound devices: none yet ----

const MMSYSERR_BADDEVICEID: u64 = 2;
const MMSYSERR_INVALHANDLE: u64 = 5;
const MMSYSERR_NODRIVER: u64 = 6;
/// WAVE_MAPPER: the default device, whichever that is.
const WAVE_MAPPER: u32 = u32::MAX;

/// `waveInGetNumDevs()` and `waveOutGetNumDevs()`: none.
fn no_devices(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// A device by number (`waveOutGetDevCaps`, `waveInMessage`, `mixerGetDevCaps`): there is none; the default device
/// has no driver behind it.
fn bad_device(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if a[0] as u32 == WAVE_MAPPER { MMSYSERR_NODRIVER } else { MMSYSERR_BADDEVICEID })
}

/// `waveOutOpen(&handle, device, ...)`, `waveInOpen` and `mixerOpen(&handle, device, ...)`: no device to open, and
/// no handle.
fn open_device(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] != 0 {
        let _ = task.write_ptr(cx, a[0], 0);
    }
    Reply::Value(if a[1] as u32 == WAVE_MAPPER { MMSYSERR_NODRIVER } else { MMSYSERR_BADDEVICEID })
}

/// Anything given a device's handle: there can't be one.
fn bad_handle(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(MMSYSERR_INVALHANDLE)
}

/// `waveOutGetErrorTextW(error, text, room)`: the error's description, as winmm words it.
fn wave_out_get_error_text_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let text = match a[0] as u32 {
        0 => "The command was carried out.",
        2 => "The specified device identifier is out of range.",
        5 => "The specified device handle is invalid.",
        6 => "There is no driver installed on your system.",
        _ => "An unknown error occurred.",
    };
    let room = a[2] as u32 as usize;
    if a[1] == 0 || room == 0 {
        return Reply::Value(11); // MMSYSERR_INVALPARAM
    }
    let units: Vec<u16> = text.encode_utf16().take(room - 1).chain([0]).collect();
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    let _ = task.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), a[1], &bytes));
    Reply::Value(0)
}
