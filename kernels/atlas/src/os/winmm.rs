//! winmm.dll: multimedia timer (`timeGetTime`, `timeBeginPeriod`/`timeEndPeriod`) and `waveOut*` audio playback
//! backed by the machine's `audio::Mixer`. Wave capture and MIDI remain unimplemented stubs.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audio::{Format, Sample, Stream};
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::{Waiter, Woke};

use super::{Reply, dll};
use crate::objects::{Kind, Object};
use crate::process::Task;

dll! {
    "WINMM.dll";
    host PlaySoundW(3) => play_sound;
    host mixerClose(1) => bad_handle;
    host mixerGetControlDetailsA(3) => bad_handle;
    host mixerGetDevCapsA(3) => bad_device;
    host mixerGetID(3) => bad_handle;
    host mixerGetLineControlsA(3) => bad_handle;
    host mixerGetLineInfoA(3) => bad_handle;
    host mixerOpen(5) => open_unsupported;
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
    host waveInOpen(6) => open_unsupported;
    host waveInPrepareHeader(3) => bad_handle;
    host waveInReset(1) => bad_handle;
    host waveInStart(1) => bad_handle;
    host waveInStop(1) => bad_handle;
    host waveInUnprepareHeader(3) => bad_handle;
    host waveOutClose(1) => wave_out_close;
    host waveOutGetDevCapsA(3) => wave_out_get_dev_caps_a;
    host waveOutGetDevCapsW(3) => wave_out_get_dev_caps_w;
    host waveOutGetErrorTextW(3) => wave_out_get_error_text_w;
    host waveOutGetID(2) => wave_out_get_id;
    host waveOutGetNumDevs(0) => wave_out_get_num_devs;
    host waveOutGetPosition(3) => wave_out_get_position;
    host waveOutGetVolume(2) => wave_out_get_volume;
    host waveOutMessage(4) => wave_out_message;
    host waveOutOpen(6) => wave_out_open;
    host waveOutPause(1) => wave_out_pause;
    host waveOutPrepareHeader(3) => wave_out_prepare_header;
    host waveOutReset(1) => wave_out_reset;
    host waveOutRestart(1) => wave_out_restart;
    host waveOutSetVolume(2) => wave_out_set_volume;
    host waveOutUnprepareHeader(3) => wave_out_unprepare_header;
    host waveOutWrite(3) => wave_out_write;
    // Internal helper thread entry for CALLBACK_FUNCTION waveOut notifications.
    host AtlasWinmmPump(1) => atlas_winmm_pump;
    // Not written yet: what larger programs import.
    missing midiInAddBuffer;
    missing midiInClose;
    missing midiInGetDevCapsW;
    host midiInGetNumDevs(0) => no_devices;
    missing midiInOpen;
    missing midiInPrepareHeader;
    missing midiInReset;
    missing midiInStart;
    missing midiInUnprepareHeader;
    missing midiOutClose;
    missing midiOutGetDevCapsW;
    host midiOutGetNumDevs(0) => no_devices;
    missing midiOutLongMsg;
    missing midiOutOpen;
    missing midiOutPrepareHeader;
    missing midiOutReset;
    missing midiOutShortMsg;
    missing midiOutUnprepareHeader;
}

const MMSYSERR_NOERROR: u64 = 0;
const MMSYSERR_BADDEVICEID: u64 = 2;
const MMSYSERR_INVALHANDLE: u64 = 5;
const MMSYSERR_NODRIVER: u64 = 6;
const MMSYSERR_NOMEM: u64 = 7;
const MMSYSERR_NOTSUPPORTED: u64 = 8;
const MMSYSERR_INVALPARAM: u64 = 11;
const WAVERR_BADFORMAT: u64 = 32;
const WAVERR_STILLPLAYING: u64 = 33;
const WAVERR_UNPREPARED: u64 = 34;

fn play_sound(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

const WAVE_MAPPER: u32 = u32::MAX;
const WAVE_FORMAT_QUERY: u32 = 0x0001;

const CALLBACK_TYPEMASK: u32 = 0x0007_0000;
const CALLBACK_NULL: u32 = 0x0000_0000;
const CALLBACK_WINDOW: u32 = 0x0001_0000;
const CALLBACK_THREAD: u32 = 0x0002_0000;
const CALLBACK_FUNCTION: u32 = 0x0003_0000;
const CALLBACK_EVENT: u32 = 0x0005_0000;

const WOM_OPEN: u32 = 0x3bb;
const WOM_CLOSE: u32 = 0x3bc;
const WOM_DONE: u32 = 0x3bd;

const WHDR_DONE: u32 = 0x0000_0001;
const WHDR_PREPARED: u32 = 0x0000_0002;
const WHDR_INQUEUE: u32 = 0x0000_0010;

/// Per-process open `waveOut` devices.
#[derive(Default)]
pub(crate) struct WaveOutTable {
    next: u64,
    devices: BTreeMap<u64, Arc<Mutex<WaveOutDev>>>,
}

enum WaveCallback {
    None,
    Window(u64),
    Thread(u64),
    Function { func: u64, instance: u64 },
    Event(Arc<Object>),
}

struct PendingHdr {
    hdr_ptr: u64,
    data: Vec<u8>,
    offset: usize,
    end_frame: u64,
}

struct WaveOutDev {
    handle: u64,
    stream: Option<Stream>,
    format: Format,
    callback: WaveCallback,
    waiter: Arc<Waiter>,
    submitted_frames: u64,
    pending: VecDeque<PendingHdr>,
    completed_for_fn: VecDeque<u64>,
    volume: u32,
    closed: bool,
}

impl WaveOutDev {
    /// Pushes any unwritten bytes from pending buffers into the mixer stream and returns headers that finished playing.
    fn advance(&mut self) -> Vec<u64> {
        let mut done = Vec::new();
        let Some(stream) = &self.stream else { return done };
        for p in &mut self.pending {
            if p.offset < p.data.len() {
                let wrote = stream.write(&p.data[p.offset..]);
                p.offset += wrote;
                if p.offset < p.data.len() {
                    break;
                }
            }
        }
        let played = stream.played();
        while let Some(front) = self.pending.front() {
            if front.offset >= front.data.len() && played >= front.end_frame {
                if let Some(finished) = self.pending.pop_front() {
                    done.push(finished.hdr_ptr);
                }
            } else {
                break;
            }
        }
        done
    }
}

/// `timeGetTime()`: milliseconds since the machine started, wrapping at 2^32.
fn time_get_time(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from((task.k.host.monotonic() / 1_000_000) as u32))
}

/// `timeBeginPeriod(ms)` and `timeEndPeriod`: TIMERR_NOERROR; the host's timers are as fine as they get anyway.
fn time_period(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if a[0] as u32 == 0 { 97 } else { 0 })
}

// ---- Unsupported capture / mixer stubs ----

fn no_devices(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn bad_device(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if a[0] as u32 == WAVE_MAPPER { MMSYSERR_NODRIVER } else { MMSYSERR_BADDEVICEID })
}

fn open_unsupported(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] != 0 {
        let _ = task.write_ptr(cx, a[0], 0);
    }
    Reply::Value(if a[1] as u32 == WAVE_MAPPER { MMSYSERR_NODRIVER } else { MMSYSERR_BADDEVICEID })
}

fn bad_handle(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(MMSYSERR_INVALHANDLE)
}

// ---- WAVEHDR guest memory helpers ----

/// Offset of `dwFlags` in `WAVEHDR`: 24 on 64-bit, 16 on 32-bit.
fn hdr_flags_offset(width: u64) -> u64 {
    if width == 8 { 24 } else { 16 }
}

/// Marks a `WAVEHDR` complete (`WHDR_DONE` set, `WHDR_INQUEUE` cleared) in guest memory.
pub(crate) fn mark_hdr_done(task: &mut Task, cx: &mut Cx<'_>, hdr_ptr: u64) {
    if hdr_ptr == 0 {
        return;
    }
    let off = hdr_flags_offset(task.abi().width);
    if let Ok(flags) = task.mem(cx, false, |t| user::read_u32(&t.space(), hdr_ptr + off)) {
        let next = (flags | WHDR_DONE) & !WHDR_INQUEUE;
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), hdr_ptr + off, next));
    }
}

fn read_wavehdr(task: &mut Task, cx: &mut Cx<'_>, hdr_ptr: u64) -> Option<(u64, u32, u32)> {
    let abi = task.abi();
    let len = if abi.width == 8 { 32 } else { 24 };
    let b = task.mem(cx, false, |t| user::read_vec(&t.space(), hdr_ptr, len)).ok()?;
    let lp_data = abi.ptr_at(&b, 0);
    let buf_len = if abi.width == 8 {
        u32::from_le_bytes(b[8..12].try_into().ok()?)
    } else {
        u32::from_le_bytes(b[4..8].try_into().ok()?)
    };
    let flags_off = hdr_flags_offset(abi.width) as usize;
    let flags = u32::from_le_bytes(b[flags_off..flags_off + 4].try_into().ok()?);
    Some((lp_data, buf_len, flags))
}

fn parse_waveformat(task: &mut Task, cx: &mut Cx<'_>, ptr: u64) -> Option<Format> {
    if ptr == 0 {
        return None;
    }
    let base = task.mem(cx, false, |t| user::read_vec(&t.space(), ptr, 18)).ok()?;
    let tag = u16::from_le_bytes([base[0], base[1]]);
    let channels = u16::from_le_bytes([base[2], base[3]]);
    let rate = u32::from_le_bytes(base[4..8].try_into().ok()?);
    let bits = u16::from_le_bytes([base[14], base[15]]);
    let cb_size = u16::from_le_bytes([base[16], base[17]]);

    let (is_float, valid_bits) = match tag {
        1 => (false, bits), // WAVE_FORMAT_PCM
        3 => (true, bits),  // WAVE_FORMAT_IEEE_FLOAT
        0xfffe if cb_size >= 22 => {
            // WAVEFORMATEXTENSIBLE: wValidBitsPerSample (18..20), dwChannelMask (20..24), SubFormat GUID (24..40).
            let ext = task.mem(cx, false, |t| user::read_vec(&t.space(), ptr + 18, 22)).ok()?;
            let valid = u16::from_le_bytes([ext[0], ext[1]]);
            let sub_data1 = u32::from_le_bytes(ext[6..10].try_into().ok()?);
            match sub_data1 {
                1 => (false, if valid > 0 { valid } else { bits }),
                3 => (true, if valid > 0 { valid } else { bits }),
                _ => return None,
            }
        }
        _ => return None,
    };

    let sample = match (is_float, bits, valid_bits) {
        (false, 8, _) => Sample::U8,
        (false, 16, _) => Sample::S16Le,
        (false, 24, _) => Sample::S24Le,
        (false, 32, 24) => Sample::S24In32Le,
        (false, 32, _) => Sample::S32Le,
        (true, 32, _) => Sample::F32Le,
        _ => return None,
    };
    let channels = u8::try_from(channels).ok()?;
    let fmt = Format::new(sample, channels, rate);
    fmt.is_valid().then_some(fmt)
}

fn get_dev(task: &Task, handle: u64) -> Option<Arc<Mutex<WaveOutDev>>> {
    lock(&task.process.state).wave_out.devices.get(&handle).cloned()
}

// ---- waveOut exports ----

fn wave_out_get_num_devs(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from(task.k.options.audio.is_some()))
}

fn wave_out_get_dev_caps(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if task.k.options.audio.is_none() {
        return bad_device(task, cx, a);
    }
    let dev_id = a[0] as u32;
    if dev_id != 0 && dev_id != WAVE_MAPPER {
        return Reply::Value(MMSYSERR_BADDEVICEID);
    }
    let (out, cb) = (a[1], a[2] as u32 as usize);
    if out == 0 || cb == 0 {
        return Reply::Value(MMSYSERR_INVALPARAM);
    }
    let mut caps = Vec::with_capacity(if wide { 84 } else { 52 });
    caps.extend(1u16.to_le_bytes()); // wMid
    caps.extend(1u16.to_le_bytes()); // wPid
    caps.extend(0x0100u32.to_le_bytes()); // vDriverVersion
    let name = "Carbon Audio";
    if wide {
        let mut units = [0u16; 32];
        for (dst, u) in units.iter_mut().zip(name.encode_utf16()) {
            *dst = u;
        }
        for u in units {
            caps.extend(u.to_le_bytes());
        }
    } else {
        let mut bytes = [0u8; 32];
        for (dst, b) in bytes.iter_mut().zip(name.bytes()) {
            *dst = b;
        }
        caps.extend(bytes);
    }
    caps.extend(0x000f_ffffu32.to_le_bytes()); // dwFormats: standard PCM rates/widths
    caps.extend(2u16.to_le_bytes()); // wChannels
    caps.extend(0u16.to_le_bytes()); // wReserved1
    caps.extend(0x000cu32.to_le_bytes()); // dwSupport: WAVECAPS_VOLUME | WAVECAPS_LRVOLUME
    let n = cb.min(caps.len());
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &caps[..n]));
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_get_dev_caps_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    wave_out_get_dev_caps(task, cx, a, false)
}

fn wave_out_get_dev_caps_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    wave_out_get_dev_caps(task, cx, a, true)
}

fn wave_out_open(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(mixer) = task.k.options.audio.clone() else {
        return open_unsupported(task, cx, a);
    };
    let (phwo, dev_id, pwfx, cb_arg, cb_inst, flags) = (a[0], a[1] as u32, a[2], a[3], a[4], a[5] as u32);
    if dev_id != 0 && dev_id != WAVE_MAPPER {
        return Reply::Value(MMSYSERR_BADDEVICEID);
    }
    let Some(format) = parse_waveformat(task, cx, pwfx) else {
        return Reply::Value(WAVERR_BADFORMAT);
    };
    if flags & WAVE_FORMAT_QUERY != 0 {
        return Reply::Value(MMSYSERR_NOERROR);
    }
    if phwo == 0 {
        return Reply::Value(MMSYSERR_INVALPARAM);
    }
    let callback = match flags & CALLBACK_TYPEMASK {
        CALLBACK_NULL => WaveCallback::None,
        CALLBACK_WINDOW => WaveCallback::Window(cb_arg),
        CALLBACK_THREAD => WaveCallback::Thread(cb_arg & 0xffff_ffff),
        CALLBACK_FUNCTION => {
            if cb_arg == 0 {
                WaveCallback::None
            } else {
                WaveCallback::Function { func: cb_arg, instance: cb_inst }
            }
        }
        CALLBACK_EVENT => match task.object(cb_arg).filter(|o| matches!(o.kind, Kind::Event { .. })) {
            Some(ev) => WaveCallback::Event(ev),
            None => return Reply::Value(MMSYSERR_INVALPARAM),
        },
        _ => return Reply::Value(MMSYSERR_NOTSUPPORTED),
    };

    // Hold up to ~1 second of queued audio in the mixer stream so multiple buffers queue without blocking.
    let capacity = (format.rate as usize).saturating_mul(format.frame_bytes()).max(65536);
    let Ok(stream) = mixer.open(format, capacity, 0) else {
        return Reply::Value(WAVERR_BADFORMAT);
    };

    let waiter = Waiter::new();
    let is_fn_cb = matches!(callback, WaveCallback::Function { .. });
    let fn_cb_pair = match callback {
        WaveCallback::Function { func, instance } => Some((func, instance)),
        _ => None,
    };

    let (handle, dev) = {
        let mut ps = lock(&task.process.state);
        ps.wave_out.next += 1;
        let handle = 0x574f_0000 + ps.wave_out.next;
        let dev = Arc::new(Mutex::new(WaveOutDev {
            handle,
            stream: Some(stream),
            format,
            callback,
            waiter: waiter.clone(),
            submitted_frames: 0,
            pending: VecDeque::new(),
            completed_for_fn: VecDeque::new(),
            volume: 0xffff_ffff,
            closed: false,
        }));
        ps.wave_out.devices.insert(handle, dev.clone());
        (handle, dev)
    };

    // Install mixer waker to advance queued buffers and notify the program as audio plays.
    let weak_k = Arc::downgrade(&task.k);
    let weak_dev = Arc::downgrade(&dev);
    let waker: audio::Waker = Arc::new(move || {
        let (Some(k), Some(dev_arc)) = (weak_k.upgrade(), weak_dev.upgrade()) else { return };
        let (done, notify_kind) = {
            let mut d = lock(&dev_arc);
            if d.closed {
                return;
            }
            let done = d.advance();
            if done.is_empty() {
                return;
            }
            let kind = match &d.callback {
                WaveCallback::None => 0u8,
                WaveCallback::Window(hwnd) => {
                    let h = d.handle;
                    let now = (k.host.monotonic() / 1_000_000) as u32;
                    let mut w = lock(&k.win32k);
                    for hdr in &done {
                        w.post_window_msg(*hwnd, WOM_DONE, h, *hdr, now);
                    }
                    1
                }
                WaveCallback::Thread(tid) => {
                    let h = d.handle;
                    let now = (k.host.monotonic() / 1_000_000) as u32;
                    let mut w = lock(&k.win32k);
                    for hdr in &done {
                        w.post_thread_msg(*tid, WOM_DONE, h, *hdr, now);
                    }
                    2
                }
                WaveCallback::Function { .. } => {
                    d.completed_for_fn.extend(done.iter().copied());
                    d.waiter.wake();
                    3
                }
                WaveCallback::Event(ev) => {
                    k.objects.signal(ev, |s| s.set = true);
                    4
                }
            };
            (done, kind)
        };
        let _ = (done, notify_kind);
    });
    if let Some(s) = &lock(&dev).stream {
        s.set_waker(Some(waker));
    }

    if task.write_ptr(cx, phwo, handle).is_err() {
        lock(&task.process.state).wave_out.devices.remove(&handle);
        return Reply::Value(MMSYSERR_INVALPARAM);
    }

    // Spawn dedicated guest callback thread when CALLBACK_FUNCTION is used.
    if is_fn_cb {
        if let Some(pump_addr) = task.system_export("WINMM.dll", "AtlasWinmmPump") {
            if task.create_thread(cx, 0, pump_addr, handle, 0).is_err() {
                lock(&task.process.state).wave_out.devices.remove(&handle);
                return Reply::Value(MMSYSERR_NOMEM);
            }
        }
    }

    // Fire WOM_OPEN immediately, matching Windows waveOutOpen semantics.
    match fn_cb_pair {
        Some((func, instance)) => {
            if let Err(reply) = task.upcall(cx, func, &[handle, u64::from(WOM_OPEN), instance, 0, 0]) {
                return reply;
            }
        }
        None => {
            let now = (task.k.host.monotonic() / 1_000_000) as u32;
            let d = lock(&dev);
            match &d.callback {
                WaveCallback::Window(hwnd) => {
                    lock(&task.k.win32k).post_window_msg(*hwnd, WOM_OPEN, handle, 0, now);
                }
                WaveCallback::Thread(tid) => {
                    lock(&task.k.win32k).post_thread_msg(*tid, WOM_OPEN, handle, 0, now);
                }
                WaveCallback::Event(ev) => {
                    task.k.objects.signal(ev, |s| s.set = true);
                }
                WaveCallback::None | WaveCallback::Function { .. } => {}
            }
        }
    }

    Reply::Value(MMSYSERR_NOERROR)
}

/// Guest callback thread loop for `CALLBACK_FUNCTION` waveOut devices.
fn atlas_winmm_pump(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let handle = a[0];
    let Some(dev) = get_dev(task, handle) else { return Reply::Value(0) };
    let (func, instance, waiter) = {
        let mut d = lock(&dev);
        // Share this thread's waiter so process exit wakes the pump cleanly too.
        d.waiter = task.waiter.clone();
        match d.callback {
            WaveCallback::Function { func, instance } => (func, instance, d.waiter.clone()),
            _ => return Reply::Value(0),
        }
    };
    loop {
        if task.ending() {
            break;
        }
        waiter.reset();
        let (hdrs, closed) = {
            let mut d = lock(&dev);
            let extra = d.advance();
            d.completed_for_fn.extend(extra);
            let hdrs: Vec<u64> = d.completed_for_fn.drain(..).collect();
            (hdrs, d.closed)
        };
        for hdr in hdrs {
            mark_hdr_done(task, cx, hdr);
            if let Err(reply) = task.upcall(cx, func, &[handle, u64::from(WOM_DONE), instance, hdr, 0]) {
                return reply;
            }
        }
        if closed || task.ending() {
            break;
        }
        let deadline = Some(std::time::Instant::now() + Duration::from_millis(25));
        if waiter.wait(deadline) == Woke::Interrupted && task.ending() {
            break;
        }
    }
    Reply::Value(0)
}

fn wave_out_prepare_header(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if get_dev(task, a[0]).is_none() {
        return Reply::Value(MMSYSERR_INVALHANDLE);
    }
    let hdr_ptr = a[1];
    if hdr_ptr == 0 {
        return Reply::Value(MMSYSERR_INVALPARAM);
    }
    let off = hdr_flags_offset(task.abi().width);
    let Ok(flags) = task.mem(cx, false, |t| user::read_u32(&t.space(), hdr_ptr + off)) else {
        return Reply::Value(MMSYSERR_INVALPARAM);
    };
    let next = (flags | WHDR_PREPARED) & !(WHDR_DONE | WHDR_INQUEUE);
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), hdr_ptr + off, next));
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_unprepare_header(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(dev) = get_dev(task, a[0]) else { return Reply::Value(MMSYSERR_INVALHANDLE) };
    let hdr_ptr = a[1];
    if hdr_ptr == 0 {
        return Reply::Value(MMSYSERR_INVALPARAM);
    }
    {
        let mut d = lock(&dev);
        let _ = d.advance();
        if d.pending.iter().any(|p| p.hdr_ptr == hdr_ptr) {
            return Reply::Value(WAVERR_STILLPLAYING);
        }
    }
    let off = hdr_flags_offset(task.abi().width);
    if let Ok(flags) = task.mem(cx, false, |t| user::read_u32(&t.space(), hdr_ptr + off)) {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), hdr_ptr + off, flags & !WHDR_PREPARED));
    }
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_write(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(dev) = get_dev(task, a[0]) else { return Reply::Value(MMSYSERR_INVALHANDLE) };
    let hdr_ptr = a[1];
    let Some((lp_data, buf_len, flags)) = read_wavehdr(task, cx, hdr_ptr) else {
        return Reply::Value(MMSYSERR_INVALPARAM);
    };
    if flags & WHDR_PREPARED == 0 {
        return Reply::Value(WAVERR_UNPREPARED);
    }
    let data = if buf_len == 0 || lp_data == 0 {
        Vec::new()
    } else {
        let Ok(bytes) = task.mem(cx, false, |t| user::read_vec(&t.space(), lp_data, buf_len as usize)) else {
            return Reply::Value(MMSYSERR_INVALPARAM);
        };
        bytes
    };

    // Mark header as in-queue before submitting to the mixer.
    let off = hdr_flags_offset(task.abi().width);
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), hdr_ptr + off, (flags | WHDR_INQUEUE) & !WHDR_DONE));

    let empty_done = {
        let mut d = lock(&dev);
        let frame_bytes = d.format.frame_bytes().max(1);
        let frames = (data.len() / frame_bytes) as u64;
        if frames == 0 {
            true
        } else {
            d.submitted_frames += frames;
            let end_frame = d.submitted_frames;
            let can_write_now = d.pending.iter().all(|p| p.offset >= p.data.len());
            let wrote = if can_write_now {
                d.stream.as_ref().map_or(0, |s| s.write(&data))
            } else {
                0
            };
            d.pending.push_back(PendingHdr { hdr_ptr, data, offset: wrote, end_frame });
            false
        }
    };

    if empty_done {
        mark_hdr_done(task, cx, hdr_ptr);
        let fn_cb = {
            let d = lock(&dev);
            match &d.callback {
                WaveCallback::Function { func, instance } => Some((*func, *instance)),
                WaveCallback::Window(hwnd) => {
                    let now = (task.k.host.monotonic() / 1_000_000) as u32;
                    lock(&task.k.win32k).post_window_msg(*hwnd, WOM_DONE, a[0], hdr_ptr, now);
                    None
                }
                WaveCallback::Thread(tid) => {
                    let now = (task.k.host.monotonic() / 1_000_000) as u32;
                    lock(&task.k.win32k).post_thread_msg(*tid, WOM_DONE, a[0], hdr_ptr, now);
                    None
                }
                WaveCallback::Event(ev) => {
                    task.k.objects.signal(ev, |s| s.set = true);
                    None
                }
                WaveCallback::None => None,
            }
        };
        if let Some((func, instance)) = fn_cb {
            if let Err(reply) = task.upcall(cx, func, &[a[0], u64::from(WOM_DONE), instance, hdr_ptr, 0]) {
                return reply;
            }
        }
    }

    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_reset(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(dev) = get_dev(task, a[0]) else { return Reply::Value(MMSYSERR_INVALHANDLE) };
    let (drained, fn_cb) = {
        let mut d = lock(&dev);
        if let Some(s) = &d.stream {
            s.flush();
            d.submitted_frames = s.played();
        }
        let mut drained: Vec<u64> = d.completed_for_fn.drain(..).collect();
        drained.extend(d.pending.drain(..).map(|p| p.hdr_ptr));
        let fn_cb = match &d.callback {
            WaveCallback::Function { func, instance } => Some((*func, *instance)),
            WaveCallback::Window(hwnd) => {
                let now = (task.k.host.monotonic() / 1_000_000) as u32;
                let mut w = lock(&task.k.win32k);
                for &hdr in &drained {
                    w.post_window_msg(*hwnd, WOM_DONE, a[0], hdr, now);
                }
                None
            }
            WaveCallback::Thread(tid) => {
                let now = (task.k.host.monotonic() / 1_000_000) as u32;
                let mut w = lock(&task.k.win32k);
                for &hdr in &drained {
                    w.post_thread_msg(*tid, WOM_DONE, a[0], hdr, now);
                }
                None
            }
            WaveCallback::Event(ev) => {
                if !drained.is_empty() {
                    task.k.objects.signal(ev, |s| s.set = true);
                }
                None
            }
            WaveCallback::None => None,
        };
        (drained, fn_cb)
    };
    for hdr in drained {
        mark_hdr_done(task, cx, hdr);
        if let Some((func, instance)) = fn_cb {
            if let Err(reply) = task.upcall(cx, func, &[a[0], u64::from(WOM_DONE), instance, hdr, 0]) {
                return reply;
            }
        }
    }
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_pause(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(dev) = get_dev(task, a[0]) else { return Reply::Value(MMSYSERR_INVALHANDLE) };
    if let Some(s) = &lock(&dev).stream {
        s.set_paused(true);
    }
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_restart(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(dev) = get_dev(task, a[0]) else { return Reply::Value(MMSYSERR_INVALHANDLE) };
    if let Some(s) = &lock(&dev).stream {
        s.set_paused(false);
    }
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_close(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(dev) = lock(&task.process.state).wave_out.devices.remove(&a[0]) else {
        return Reply::Value(MMSYSERR_INVALHANDLE);
    };
    let fn_cb = {
        let mut d = lock(&dev);
        d.closed = true;
        if let Some(s) = d.stream.take() {
            s.set_waker(None);
            drop(s);
        }
        d.waiter.wake();
        match &d.callback {
            WaveCallback::Function { func, instance } => Some((*func, *instance)),
            WaveCallback::Window(hwnd) => {
                let now = (task.k.host.monotonic() / 1_000_000) as u32;
                lock(&task.k.win32k).post_window_msg(*hwnd, WOM_CLOSE, a[0], 0, now);
                None
            }
            WaveCallback::Thread(tid) => {
                let now = (task.k.host.monotonic() / 1_000_000) as u32;
                lock(&task.k.win32k).post_thread_msg(*tid, WOM_CLOSE, a[0], 0, now);
                None
            }
            WaveCallback::Event(ev) => {
                task.k.objects.signal(ev, |s| s.set = true);
                None
            }
            WaveCallback::None => None,
        }
    };
    if let Some((func, instance)) = fn_cb {
        if let Err(reply) = task.upcall(cx, func, &[a[0], u64::from(WOM_CLOSE), instance, 0, 0]) {
            return reply;
        }
    }
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_get_id(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if get_dev(task, a[0]).is_none() {
        return Reply::Value(MMSYSERR_INVALHANDLE);
    }
    if a[1] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], 0));
    }
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_get_position(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(dev) = get_dev(task, a[0]) else { return Reply::Value(MMSYSERR_INVALHANDLE) };
    let (out, cb) = (a[1], a[2] as u32 as usize);
    if out == 0 || cb < 12 {
        return Reply::Value(MMSYSERR_INVALPARAM);
    }
    let req_type = task.mem(cx, false, |t| user::read_u32(&t.space(), out)).unwrap_or(0x0004);
    let (played, fmt) = {
        let d = lock(&dev);
        (d.stream.as_ref().map_or(0, Stream::played), d.format)
    };
    let (out_type, value): (u32, u32) = match req_type {
        0x0001 => (0x0001, (played * 1000 / u64::from(fmt.rate.max(1))) as u32), // TIME_MS
        0x0002 => (0x0002, played as u32),                                       // TIME_SAMPLES
        _ => (0x0004, (played * fmt.frame_bytes() as u64) as u32),               // TIME_BYTES
    };
    let mut mmtime = [0u8; 12];
    mmtime[0..4].copy_from_slice(&out_type.to_le_bytes());
    mmtime[4..8].copy_from_slice(&value.to_le_bytes());
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &mmtime));
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_get_volume(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let vol = get_dev(task, a[0]).map_or(0xffff_ffff, |d| lock(&d).volume);
    if a[1] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], vol));
    }
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_set_volume(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let vol = a[1] as u32;
    let l = f32::from((vol & 0xffff) as u16) / 65535.0;
    let r = f32::from(((vol >> 16) & 0xffff) as u16) / 65535.0;
    if let Some(dev) = get_dev(task, a[0]) {
        let mut d = lock(&dev);
        d.volume = vol;
        if let Some(s) = &d.stream {
            s.set_volume([l, r]);
        }
    }
    Reply::Value(MMSYSERR_NOERROR)
}

fn wave_out_message(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    if task.k.options.audio.is_none() {
        return Reply::Value(MMSYSERR_BADDEVICEID);
    }
    let _ = a;
    Reply::Value(MMSYSERR_NOTSUPPORTED)
}

/// `waveOutGetErrorTextW(error, text, room)`: the error's description, as winmm words it.
fn wave_out_get_error_text_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let text = match a[0] as u32 {
        0 => "The command was carried out.",
        2 => "The specified device identifier is out of range.",
        5 => "The specified device handle is invalid.",
        6 => "There is no driver installed on your system.",
        32 => "The specified format is not supported or cannot be translated.",
        33 => "The buffer is still playing.",
        34 => "The wave header was not prepared.",
        _ => "An unknown error occurred.",
    };
    let room = a[2] as u32 as usize;
    if a[1] == 0 || room == 0 {
        return Reply::Value(MMSYSERR_INVALPARAM);
    }
    let units: Vec<u16> = text.encode_utf16().take(room - 1).chain([0]).collect();
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &bytes));
    Reply::Value(MMSYSERR_NOERROR)
}
