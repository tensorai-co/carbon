//! winmm.dll: the multimedia timer's time and resolution, written; its sound (waveIn, waveOut, the mixer) is declared
//! so programs that import it load, and says it isn't written yet when called.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "WINMM.dll";
    missing PlaySoundW;
    missing mixerClose;
    missing mixerGetControlDetailsA;
    missing mixerGetDevCapsA;
    missing mixerGetID;
    missing mixerGetLineControlsA;
    missing mixerGetLineInfoA;
    missing mixerOpen;
    missing mixerSetControlDetails;
    host timeBeginPeriod(1) => time_period;
    host timeEndPeriod(1) => time_period;
    host timeGetTime(0) => time_get_time;
    missing timeKillEvent;
    missing timeSetEvent;
    missing waveInAddBuffer;
    missing waveInClose;
    missing waveInGetDevCapsW;
    missing waveInGetNumDevs;
    missing waveInMessage;
    missing waveInOpen;
    missing waveInPrepareHeader;
    missing waveInReset;
    missing waveInStart;
    missing waveInUnprepareHeader;
    missing waveOutClose;
    missing waveOutGetDevCapsA;
    missing waveOutGetDevCapsW;
    missing waveOutGetErrorTextW;
    missing waveOutGetID;
    missing waveOutGetNumDevs;
    missing waveOutMessage;
    missing waveOutOpen;
    missing waveOutPrepareHeader;
    missing waveOutReset;
    missing waveOutUnprepareHeader;
    missing waveOutWrite;
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
