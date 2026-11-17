//! lambda's sound server: PulseAudio's native protocol on `$XDG_RUNTIME_DIR/pulse/native`, answered by the kernel
//! itself, as its Wayland and X servers are, playing every stream into the machine's mixer (`audio::Mixer`). Programs
//! play through the image's own libpulse (directly, or through OpenAL, SDL, GStreamer or PipeWire's Pulse layer), as
//! they would against a PulseAudio daemon.
//!
//! What it is to a client: one sink, `carbon`, 48 kHz stereo floats at the mixer's rate, with no sources yet. A
//! playback stream is a mixer stream in the client's own format; its buffer attributes are PulseAudio's (a target
//! length the server keeps it filled to by asking for what's missing, a prebuffer it waits for, a largest size), and
//! its position is the mixer's clock, so the latency a client works out is what it hears. Audio comes inline on the
//! socket: the server offers no shared memory, so libpulse sends its memblocks in the stream itself.
//!
//! The protocol: frames of a 20-byte header (length, channel, a 64-bit offset, flags, big-endian) and a payload: a
//! packet (channel all ones) of tagged values (a command, a tag replies echo, its arguments), or a stream's audio.
//! Protocol version 35 (PulseAudio 17); clients from version 13 on.
//!
//! Not yet: record streams and the monitor source, sample uploads, subscription events, moving streams.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use audio::{Format, Mixer, Sample};
use kernels_core::lock;
use layers::New;
use net::{Addr, Domain, Kind, UnixAddr, UnixName};

use crate::abi::S_IFSOCK;
use crate::{Kernel, Meta};

/// The server's directory and socket: `$XDG_RUNTIME_DIR/pulse/native`, where libpulse looks first.
pub(crate) const DIR: &str = "/run/user/0/pulse";
pub(crate) const SOCKET: &str = "/run/user/0/pulse/native";

/// The protocol version this server speaks, and the oldest it answers.
const VERSION: u32 = 35;
const OLDEST: u32 = 13;

/// A packet's channel.
const PACKET: u32 = u32::MAX;
/// The largest frame a client may send.
const MOST: usize = 16 << 20;
/// "None": an index nothing has.
const INVALID: u32 = u32::MAX;

// Commands this server answers, and those it sends.
const ERROR: u32 = 0;
const REPLY: u32 = 2;
const CREATE_PLAYBACK_STREAM: u32 = 3;
const DELETE_PLAYBACK_STREAM: u32 = 4;
const CREATE_RECORD_STREAM: u32 = 5;
const EXIT: u32 = 7;
const AUTH: u32 = 8;
const SET_CLIENT_NAME: u32 = 9;
const LOOKUP_SINK: u32 = 10;
const LOOKUP_SOURCE: u32 = 11;
const DRAIN_PLAYBACK_STREAM: u32 = 12;
const STAT: u32 = 13;
const GET_PLAYBACK_LATENCY: u32 = 14;
const GET_SERVER_INFO: u32 = 20;
const GET_SINK_INFO: u32 = 21;
const GET_SINK_INFO_LIST: u32 = 22;
const GET_SOURCE_INFO: u32 = 23;
const GET_SOURCE_INFO_LIST: u32 = 24;
const GET_MODULE_INFO_LIST: u32 = 26;
const GET_CLIENT_INFO: u32 = 27;
const GET_CLIENT_INFO_LIST: u32 = 28;
const GET_SINK_INPUT_INFO: u32 = 29;
const GET_SINK_INPUT_INFO_LIST: u32 = 30;
const GET_SOURCE_OUTPUT_INFO_LIST: u32 = 32;
const GET_SAMPLE_INFO_LIST: u32 = 34;
const SUBSCRIBE: u32 = 35;
const SET_SINK_VOLUME: u32 = 36;
const SET_SINK_INPUT_VOLUME: u32 = 37;
const SET_SINK_MUTE: u32 = 39;
const CORK_PLAYBACK_STREAM: u32 = 41;
const FLUSH_PLAYBACK_STREAM: u32 = 42;
const TRIGGER_PLAYBACK_STREAM: u32 = 43;
const SET_DEFAULT_SINK: u32 = 44;
const SET_PLAYBACK_STREAM_NAME: u32 = 46;
const PREBUF_PLAYBACK_STREAM: u32 = 60;
const REQUEST: u32 = 61;
const OVERFLOW: u32 = 62;
const UNDERFLOW: u32 = 63;
const SET_SINK_INPUT_MUTE: u32 = 69;
const SET_PLAYBACK_STREAM_BUFFER_ATTR: u32 = 72;
const UPDATE_PLAYBACK_STREAM_PROPLIST: u32 = 81;
const UPDATE_CLIENT_PROPLIST: u32 = 82;
const REMOVE_PLAYBACK_STREAM_PROPLIST: u32 = 84;
const REMOVE_CLIENT_PROPLIST: u32 = 85;
const STARTED: u32 = 86;
const EXTENSION: u32 = 87;
const GET_CARD_INFO_LIST: u32 = 89;

// Errors (`pa_error_code`).
const ERR_ACCESS: u32 = 1;
const ERR_INVALID: u32 = 3;
const ERR_NOENTITY: u32 = 5;
const ERR_PROTOCOL: u32 = 7;
const ERR_VERSION: u32 = 17;
const ERR_NOTSUPPORTED: u32 = 19;
const ERR_NOEXTENSION: u32 = 21;

// Tags.
const TAG_STRING: u8 = b't';
const TAG_STRING_NULL: u8 = b'N';
const TAG_U32: u8 = b'L';
const TAG_U8: u8 = b'B';
const TAG_U64: u8 = b'R';
const TAG_S64: u8 = b'r';
const TAG_SAMPLE_SPEC: u8 = b'a';
const TAG_ARBITRARY: u8 = b'x';
const TAG_TRUE: u8 = b'1';
const TAG_FALSE: u8 = b'0';
const TAG_TIMEVAL: u8 = b'T';
const TAG_USEC: u8 = b'U';
const TAG_CHANNEL_MAP: u8 = b'm';
const TAG_CVOLUME: u8 = b'v';
const TAG_PROPLIST: u8 = b'P';
const TAG_VOLUME: u8 = b'V';
const TAG_FORMAT_INFO: u8 = b'f';

/// The sink's name and index, and the latency it reports beyond what's queued (the output's own buffer).
const SINK: &str = "carbon";
const SINK_INDEX: u32 = 0;
const SINK_LATENCY_US: u64 = 20_000;
/// `PA_VOLUME_NORM`: a volume of 1.
const NORM: u32 = 0x10000;
/// `float32le`, the sink's sample format; front left and right, its channels.
const FLOAT32LE: u8 = 5;
const STEREO: [u8; 2] = [1, 2];
/// `PA_ENCODING_PCM`.
const PCM: u8 = 1;

/// Starts the server: its socket made on the disk and listening, and its thread serving every connection until the
/// machine ends.
pub(crate) fn start(k: &Arc<Kernel>, mixer: Mixer) -> Result<(), String> {
    let ino = {
        let mut disk = lock(&k.disk);
        let _ = disk.put("/run/user", New::Dir, &[], Meta::dir(0o755), Meta::dir(0o755));
        let _ = disk.put(super::wayland::RUNTIME_DIR, New::Dir, &[], Meta::dir(0o700), Meta::dir(0o755));
        let _ = disk.put(DIR, New::Dir, &[], Meta::dir(0o700), Meta::dir(0o700));
        disk.put(SOCKET, New::File, &[], Meta { mode: S_IFSOCK | 0o777, ..Meta::default() }, Meta::dir(0o700)).map_err(|e| format!("{SOCKET}: {e:?}"))?
    };
    let listener = k.net.socket(Domain::Unix, Kind::Stream).map_err(|e| format!("{SOCKET}: {e:?}"))?;
    listener.bind(&Addr::Unix(UnixAddr { name: UnixName::Path(SOCKET.as_bytes().to_vec()), file: Some(ino) })).map_err(|e| format!("{SOCKET}: {e:?}"))?;
    listener.listen(16).map_err(|e| format!("{SOCKET}: {e:?}"))?;
    let k = k.clone();
    std::thread::Builder::new().name("lambda pulse".into()).spawn(move || Server::new(k, mixer, listener).run()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Wakes the server's thread: a client wrote or connected, or the mixer took from a stream.
#[derive(Default)]
struct Signal {
    woken: Mutex<bool>,
    ready: Condvar,
}

impl Signal {
    fn wake(&self) {
        *lock(&self.woken) = true;
        self.ready.notify_one();
    }

    fn wait(&self, most: Duration) {
        let mut woken = lock(&self.woken);
        if !*woken {
            woken = self.ready.wait_timeout(woken, most).map(|(g, _)| g).unwrap_or_else(|p| p.into_inner().0);
        }
        *woken = false;
    }
}

struct Server {
    k: Arc<Kernel>,
    mixer: Mixer,
    listener: net::Socket,
    signal: Arc<Signal>,
    _watch: net::Watch,
    clients: BTreeMap<u64, Client>,
    next_client: u64,
    /// Sink inputs (playback streams) are numbered across clients, as PulseAudio numbers them.
    next_input: u32,
}

struct Client {
    socket: net::Socket,
    _watch: net::Watch,
    input: Vec<u8>,
    out: Vec<u8>,
    /// The protocol version agreed, once it's authenticated.
    version: u32,
    authed: bool,
    index: u32,
    /// Its playback streams, by the channel their audio comes on (which is also their sink input's index).
    streams: BTreeMap<u32, Playback>,
    gone: bool,
}

/// A playback stream: the mixer's, and PulseAudio's bookkeeping around it.
struct Playback {
    stream: audio::Stream,
    frame: usize,
    /// Buffer attributes, in bytes.
    maxlength: usize,
    tlength: usize,
    prebuf: usize,
    minreq: usize,
    /// Bytes asked for (`REQUEST`, or the create reply's `missing`) and not yet come.
    requested: usize,
    /// Drains waiting for it to play out: their tags.
    drains: Vec<u32>,
    /// Underruns told of so far, and whether it's been told it started since the last.
    underruns: u64,
    told_started: bool,
}

impl Playback {
    /// Where it's been read to, in bytes of its own: what libpulse calls the read index.
    fn read_index(&self) -> u64 {
        self.stream.played() * self.frame as u64
    }
}

impl Server {
    fn new(k: Arc<Kernel>, mixer: Mixer, listener: net::Socket) -> Server {
        let signal = Arc::new(Signal::default());
        let s = signal.clone();
        k.when_done(Box::new(move || s.wake()));
        let s = signal.clone();
        let watch = listener.watch(Arc::new(move || s.wake()));
        Server { k, mixer, listener, signal, _watch: watch, clients: BTreeMap::new(), next_client: 1, next_input: 0 }
    }

    fn run(mut self) {
        while !self.k.done.load(std::sync::atomic::Ordering::Acquire) {
            // While streams play, their requests and drains are looked at often, whatever wakes it.
            let playing = self.clients.values().any(|c| !c.streams.is_empty());
            self.signal.wait(Duration::from_millis(if playing { 10 } else { 200 }));
            self.accept();
            let ids: Vec<u64> = self.clients.keys().copied().collect();
            for id in ids {
                self.read(id);
            }
            self.streams();
            self.flush();
        }
    }

    fn accept(&mut self) {
        while let Ok((socket, _)) = self.listener.accept() {
            let s = self.signal.clone();
            let watch = socket.watch(Arc::new(move || s.wake()));
            let id = self.next_client;
            self.next_client += 1;
            self.clients.insert(id, Client { socket, _watch: watch, input: Vec::new(), out: Vec::new(), version: 0, authed: false, index: id as u32, streams: BTreeMap::new(), gone: false });
        }
    }

    /// Everything a client's sent, read and done.
    fn read(&mut self, id: u64) {
        let Some(mut c) = self.clients.remove(&id) else { return };
        loop {
            match c.socket.recv(1 << 16, false) {
                Ok(got) if got.data.is_empty() && got.extra.is_none() => {
                    c.gone = true;
                    break;
                }
                // Credentials and descriptors that come with a packet aren't needed: the socket's the machine's own.
                Ok(got) => c.input.extend(got.data),
                Err(net::Error::WouldBlock) => break,
                Err(_) => {
                    c.gone = true;
                    break;
                }
            }
        }
        while c.input.len() >= 20 && !c.gone {
            let word = |i: usize| u32::from_be_bytes([c.input[i * 4], c.input[i * 4 + 1], c.input[i * 4 + 2], c.input[i * 4 + 3]]);
            let (length, channel, flags) = (word(0) as usize, word(1), word(4));
            let offset = (u64::from(word(2)) << 32 | u64::from(word(3))) as i64;
            if length > MOST {
                c.gone = true;
                break;
            }
            if c.input.len() < 20 + length {
                break;
            }
            let payload: Vec<u8> = c.input[20..20 + length].to_vec();
            c.input.drain(..20 + length);
            if channel == PACKET {
                self.packet(&mut c, &payload);
            } else {
                memblock(&mut c, channel, offset, flags, &payload);
            }
        }
        if !c.gone {
            self.clients.insert(id, c);
        }
    }

    /// A packet: a command, its tag, and its arguments.
    fn packet(&mut self, c: &mut Client, payload: &[u8]) {
        let mut t = Tags::new(payload);
        let (Some(command), Some(tag)) = (t.u32(), t.u32()) else {
            c.gone = true;
            return;
        };
        if !c.authed && command != AUTH {
            return error(c, tag, ERR_ACCESS);
        }
        match command {
            AUTH => {
                let Some(asked) = t.u32() else { return error(c, tag, ERR_PROTOCOL) };
                // The top bits say whether it can use shared memory and memfds: neither is offered, so audio comes
                // inline. The cookie isn't checked: the socket's only reachable inside the machine.
                let version = asked & 0xffff;
                if version < OLDEST {
                    return error(c, tag, ERR_VERSION);
                }
                c.version = version.min(VERSION);
                c.authed = true;
                let mut r = reply(tag);
                put_u32(&mut r, VERSION);
                send(c, &r);
            }
            SET_CLIENT_NAME => {
                let mut r = reply(tag);
                put_u32(&mut r, c.index);
                send(c, &r);
            }
            GET_SERVER_INFO => {
                let mut r = reply(tag);
                put_string(&mut r, Some("pulseaudio"));
                put_string(&mut r, Some("17.0"));
                put_string(&mut r, Some("root"));
                put_string(&mut r, Some(&self.k.options.hostname));
                put_sample_spec(&mut r, FLOAT32LE, 2, audio::RATE);
                put_string(&mut r, Some(SINK));
                put_string(&mut r, None);
                put_u32(&mut r, 0x6361_7262);
                if c.version >= 15 {
                    put_channel_map(&mut r, &STEREO);
                }
                send(c, &r);
            }
            GET_SINK_INFO => {
                let (index, name) = (t.u32().unwrap_or(INVALID), t.string());
                if !(index == SINK_INDEX || name.as_deref() == Some(SINK) || name.as_deref() == Some("@DEFAULT_SINK@")) {
                    return error(c, tag, ERR_NOENTITY);
                }
                let mut r = reply(tag);
                sink_info(&mut r, c.version);
                send(c, &r);
            }
            GET_SINK_INFO_LIST => {
                let mut r = reply(tag);
                sink_info(&mut r, c.version);
                send(c, &r);
            }
            LOOKUP_SINK => {
                let name = t.string();
                if !matches!(name.as_deref(), Some(SINK) | Some("@DEFAULT_SINK@")) {
                    return error(c, tag, ERR_NOENTITY);
                }
                let mut r = reply(tag);
                put_u32(&mut r, SINK_INDEX);
                send(c, &r);
            }
            // No sources, clients, modules, cards, samples or other streams to tell of: empty lists.
            GET_SOURCE_INFO_LIST | GET_MODULE_INFO_LIST | GET_CLIENT_INFO_LIST | GET_SINK_INPUT_INFO_LIST | GET_SOURCE_OUTPUT_INFO_LIST | GET_SAMPLE_INFO_LIST | GET_CARD_INFO_LIST => send(c, &reply(tag)),
            GET_SOURCE_INFO | LOOKUP_SOURCE | GET_CLIENT_INFO | GET_SINK_INPUT_INFO => error(c, tag, ERR_NOENTITY),
            STAT => {
                let mut r = reply(tag);
                for _ in 0..5 {
                    put_u32(&mut r, 0);
                }
                send(c, &r);
            }
            CREATE_PLAYBACK_STREAM => self.create_playback(c, tag, &mut t),
            CREATE_RECORD_STREAM => error(c, tag, ERR_NOTSUPPORTED),
            DELETE_PLAYBACK_STREAM => {
                let index = t.u32().unwrap_or(INVALID);
                match c.streams.remove(&index) {
                    Some(_) => ack(c, tag),
                    None => error(c, tag, ERR_NOENTITY),
                }
            }
            CORK_PLAYBACK_STREAM | FLUSH_PLAYBACK_STREAM | TRIGGER_PLAYBACK_STREAM | PREBUF_PLAYBACK_STREAM | DRAIN_PLAYBACK_STREAM => {
                let index = t.u32().unwrap_or(INVALID);
                let corked = t.boolean().unwrap_or(false);
                let Some(p) = c.streams.get_mut(&index) else { return error(c, tag, ERR_NOENTITY) };
                match command {
                    CORK_PLAYBACK_STREAM => p.stream.set_paused(corked),
                    FLUSH_PLAYBACK_STREAM => {
                        p.stream.flush();
                        p.requested = 0;
                    }
                    TRIGGER_PLAYBACK_STREAM => p.stream.trigger(),
                    PREBUF_PLAYBACK_STREAM => p.stream.flush(),
                    _ => {
                        // Answered once it's played out (or at once, if it has).
                        if p.stream.queued() > 0 {
                            p.stream.trigger();
                            p.drains.push(tag);
                            return;
                        }
                    }
                }
                ack(c, tag);
            }
            GET_PLAYBACK_LATENCY => {
                let index = t.u32().unwrap_or(INVALID);
                let (sec, usec) = t.timeval().unwrap_or((0, 0));
                let Some(p) = c.streams.get(&index) else { return error(c, tag, ERR_NOENTITY) };
                let read = p.read_index();
                let mut r = reply(tag);
                put_usec(&mut r, SINK_LATENCY_US);
                put_usec(&mut r, 0);
                put_bool(&mut r, p.stream.is_playing());
                put_timeval(&mut r, sec, usec);
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                put_timeval(&mut r, now.as_secs() as u32, now.subsec_micros());
                put_s64(&mut r, p.stream.written() as i64);
                put_s64(&mut r, read as i64);
                put_u64(&mut r, 0);
                put_u64(&mut r, read);
                send(c, &r);
            }
            SET_PLAYBACK_STREAM_BUFFER_ATTR => {
                let index = t.u32().unwrap_or(INVALID);
                let asked = [t.u32(), t.u32(), t.u32(), t.u32()].map(|v| v.unwrap_or(u32::MAX));
                let version = c.version;
                let Some(p) = c.streams.get_mut(&index) else { return error(c, tag, ERR_NOENTITY) };
                let format = p.stream.format();
                (p.maxlength, p.tlength, p.prebuf, p.minreq) = attrs(&format, asked);
                p.stream.resize(p.maxlength, p.prebuf);
                let mut r = reply(tag);
                for v in [p.maxlength, p.tlength, p.prebuf, p.minreq] {
                    put_u32(&mut r, v as u32);
                }
                if version >= 13 {
                    put_usec(&mut r, SINK_LATENCY_US);
                }
                send(c, &r);
            }
            SET_SINK_INPUT_VOLUME | SET_SINK_INPUT_MUTE => {
                let index = t.u32().unwrap_or(INVALID);
                let volume = if command == SET_SINK_INPUT_VOLUME { t.cvolume() } else { t.boolean().map(|m| vec![if m { 0 } else { NORM }]) };
                let Some(p) = c.streams.get(&index) else { return error(c, tag, ERR_NOENTITY) };
                if let Some(v) = volume {
                    p.stream.set_volume(linear(&v));
                }
                ack(c, tag);
            }
            // What changes nothing here: names, properties, the one sink's own volume and default, subscriptions
            // (no events are sent yet).
            SUBSCRIBE | SET_SINK_VOLUME | SET_SINK_MUTE | SET_DEFAULT_SINK | SET_PLAYBACK_STREAM_NAME | UPDATE_PLAYBACK_STREAM_PROPLIST | UPDATE_CLIENT_PROPLIST | REMOVE_PLAYBACK_STREAM_PROPLIST | REMOVE_CLIENT_PROPLIST => ack(c, tag),
            EXTENSION => error(c, tag, ERR_NOEXTENSION),
            EXIT => error(c, tag, ERR_ACCESS),
            _ => error(c, tag, ERR_NOTSUPPORTED),
        }
    }

    /// `CREATE_PLAYBACK_STREAM`: a mixer stream in the client's format, with its buffer attributes and volume.
    fn create_playback(&mut self, c: &mut Client, tag: u32, t: &mut Tags) {
        let v = c.version;
        let Some((format, channels, rate)) = t.sample_spec() else { return error(c, tag, ERR_PROTOCOL) };
        let _map = t.channel_map();
        let (_sink, _sink_name) = (t.u32(), t.string());
        let maxlength = t.u32().unwrap_or(u32::MAX);
        let corked = t.boolean().unwrap_or(false);
        let (tlength, prebuf, minreq, _sync) = (t.u32().unwrap_or(u32::MAX), t.u32().unwrap_or(u32::MAX), t.u32().unwrap_or(u32::MAX), t.u32());
        let volume = t.cvolume();
        // Remap, remix, fixed format, rate and channels, don't move, variable rate.
        for _ in 0..7 {
            t.boolean();
        }
        let muted = t.boolean().unwrap_or(false);
        let _adjust_latency = t.boolean();
        let _props = t.proplist();
        let volume_set = if v >= 14 {
            let set = t.boolean().unwrap_or(false);
            let _early_requests = t.boolean();
            set
        } else {
            true
        };
        // From version 15 a client says whether it set the mute at all.
        let muted_set = if v >= 15 {
            let set = t.boolean().unwrap_or(false);
            let (_inhibit, _fail_on_suspend) = (t.boolean(), t.boolean());
            set
        } else {
            true
        };
        let muted = muted && muted_set;
        let Some(sample) = sample_of(format) else { return error(c, tag, ERR_NOTSUPPORTED) };
        let format = Format::new(sample, channels, rate);
        if !format.is_valid() {
            return error(c, tag, ERR_INVALID);
        }
        let (maxlength, tlength, prebuf, minreq) = attrs(&format, [maxlength, tlength, prebuf, minreq]);
        let stream = match self.mixer.open(format, maxlength, prebuf) {
            Ok(s) => s,
            Err(_) => return error(c, tag, ERR_INVALID),
        };
        if corked {
            stream.set_paused(true);
        }
        if muted {
            stream.set_volume([0.0; 2]);
        } else if let Some(v) = volume.filter(|_| volume_set) {
            stream.set_volume(linear(&v));
        }
        let s = self.signal.clone();
        stream.set_waker(Some(Arc::new(move || s.wake())));
        let index = self.next_input;
        self.next_input += 1;
        let missing = tlength;
        c.streams.insert(index, Playback { stream, frame: format.frame_bytes(), maxlength, tlength, prebuf, minreq, requested: missing, drains: Vec::new(), underruns: 0, told_started: false });
        let mut r = reply(tag);
        put_u32(&mut r, index);
        put_u32(&mut r, index);
        put_u32(&mut r, missing as u32);
        for x in [maxlength, tlength, prebuf, minreq] {
            put_u32(&mut r, x as u32);
        }
        put_sample_spec(&mut r, pa_format(sample), channels, rate);
        let map: Vec<u8> = if channels == 1 { vec![0] } else { (1..=channels).collect() };
        put_channel_map(&mut r, &map);
        put_u32(&mut r, SINK_INDEX);
        put_string(&mut r, Some(SINK));
        put_bool(&mut r, false);
        put_usec(&mut r, SINK_LATENCY_US);
        if v >= 21 {
            put_format_info(&mut r, PCM, &[]);
        }
        send(c, &r);
    }

    /// Each playing stream looked at: what it's missing asked for, drains that have played out answered, and its
    /// starts and underruns told.
    fn streams(&mut self) {
        for c in self.clients.values_mut() {
            let version = c.version;
            let mut out = Vec::new();
            for (&index, p) in &mut c.streams {
                let queued = p.stream.queued();
                let missing = p.tlength.saturating_sub(queued + p.requested);
                if missing >= p.minreq.max(p.frame) || (missing > 0 && queued + p.requested == 0) {
                    p.requested += missing;
                    out.push(command(REQUEST, &[index, missing as u32]));
                }
                if queued == 0 {
                    for tag in p.drains.drain(..) {
                        out.push(reply(tag));
                    }
                }
                let underruns = p.stream.underruns();
                if underruns > p.underruns {
                    p.underruns = underruns;
                    p.told_started = false;
                    let mut m = command(UNDERFLOW, &[index]);
                    if version >= 23 {
                        put_s64(&mut m, p.read_index() as i64);
                    }
                    out.push(m);
                } else if !p.told_started && p.stream.is_playing() && p.stream.played() > 0 {
                    p.told_started = true;
                    out.push(command(STARTED, &[index]));
                }
            }
            for m in out {
                send(c, &m);
            }
        }
    }

    /// What's been sent to each client, written; one that's gone, let go (its streams with it).
    fn flush(&mut self) {
        let ids: Vec<u64> = self.clients.keys().copied().collect();
        for id in ids {
            let Some(c) = self.clients.get_mut(&id) else { continue };
            while !c.out.is_empty() {
                let mut extra = None;
                match c.socket.send(&c.out, None, &mut extra) {
                    Ok(n) => {
                        c.out.drain(..n);
                    }
                    Err(net::Error::WouldBlock) => break,
                    Err(_) => {
                        c.gone = true;
                        break;
                    }
                }
            }
            if c.gone {
                self.clients.remove(&id);
            }
        }
    }
}

/// A stream's audio: appended to it (seeks other than to where it's written to aren't made), anything that doesn't
/// fit an overflow.
fn memblock(c: &mut Client, channel: u32, _offset: i64, _flags: u32, data: &[u8]) {
    let Some(p) = c.streams.get_mut(&channel) else { return };
    let took = p.stream.write(data);
    p.requested = p.requested.saturating_sub(data.len());
    if took < data.len() {
        let m = command(OVERFLOW, &[channel]);
        send(c, &m);
    }
}

/// Buffer attributes as a client asked for them (all ones: the server's choice), made whole frames and consistent:
/// a target length of 250 ms by default, a request of at least 10 ms, a prebuffer of the target length, and room for
/// twice the target length.
fn attrs(format: &Format, asked: [u32; 4]) -> (usize, usize, usize, usize) {
    let frame = format.frame_bytes();
    let per_ms = (frame * format.rate as usize / 1000).max(frame);
    let pick = |v: u32, default: usize| if v == u32::MAX || v == 0 { default } else { v as usize };
    let whole = |v: usize| (v / frame).max(1) * frame;
    let tlength = whole(pick(asked[1], 250 * per_ms).clamp(10 * per_ms, 2000 * per_ms));
    let minreq = whole(pick(asked[3], 10 * per_ms).min(tlength / 2).max(frame));
    let maxlength = whole(pick(asked[0], 2 * tlength).max(tlength).min(4 << 20));
    let prebuf = if asked[2] == u32::MAX { tlength } else { whole(asked[2] as usize).min(tlength) };
    (maxlength, tlength, if asked[2] == 0 { 0 } else { prebuf }, minreq)
}

/// PulseAudio's volumes (cubic, `NORM` is 1) as linear gains, for the left and right of the mix.
fn linear(v: &[u32]) -> [f32; 2] {
    let g = |x: u32| (x as f32 / NORM as f32).powi(3);
    match v {
        [] => [1.0; 2],
        [m] => [g(*m); 2],
        [l, r, ..] => [g(*l), g(*r)],
    }
}

fn sample_of(format: u8) -> Option<Sample> {
    Some(match format {
        0 => Sample::U8,
        1 => Sample::Alaw,
        2 => Sample::Ulaw,
        3 => Sample::S16Le,
        4 => Sample::S16Be,
        5 => Sample::F32Le,
        6 => Sample::F32Be,
        7 => Sample::S32Le,
        8 => Sample::S32Be,
        9 => Sample::S24Le,
        10 => Sample::S24Be,
        11 => Sample::S24In32Le,
        12 => Sample::S24In32Be,
        _ => return None,
    })
}

fn pa_format(sample: Sample) -> u8 {
    match sample {
        Sample::U8 => 0,
        Sample::Alaw => 1,
        Sample::Ulaw => 2,
        Sample::S16Le => 3,
        Sample::S16Be => 4,
        Sample::F32Le => 5,
        Sample::F32Be => 6,
        Sample::S32Le => 7,
        Sample::S32Be => 8,
        Sample::S24Le => 9,
        Sample::S24Be => 10,
        Sample::S24In32Le => 11,
        Sample::S24In32Be => 12,
    }
}

/// The one sink, as `GET_SINK_INFO` tells of it at a protocol version.
fn sink_info(r: &mut Vec<u8>, version: u32) {
    put_u32(r, SINK_INDEX);
    put_string(r, Some(SINK));
    put_string(r, Some("Carbon"));
    put_sample_spec(r, FLOAT32LE, 2, audio::RATE);
    put_channel_map(r, &STEREO);
    put_u32(r, INVALID);
    put_cvolume(r, &[NORM, NORM]);
    put_bool(r, false);
    put_u32(r, INVALID);
    put_string(r, None);
    put_usec(r, SINK_LATENCY_US);
    put_string(r, Some("carbon"));
    // PA_SINK_LATENCY | PA_SINK_HARDWARE | PA_SINK_DECIBEL_VOLUME.
    put_u32(r, 0x2 | 0x4 | 0x20);
    if version >= 13 {
        put_proplist(r, &[("device.description", "Carbon"), ("device.class", "sound"), ("device.api", "carbon")]);
        put_usec(r, 0);
    }
    if version >= 15 {
        put_volume(r, NORM);
        // PA_SINK_RUNNING; volume steps; no card.
        put_u32(r, 0);
        put_u32(r, NORM + 1);
        put_u32(r, INVALID);
    }
    if version >= 16 {
        put_u32(r, 0);
        put_string(r, None);
    }
    if version >= 21 {
        put_u8(r, 1);
        put_format_info(r, PCM, &[]);
    }
}

// ---- Frames ----

/// A packet, framed, onto a client's output.
fn send(c: &mut Client, packet: &[u8]) {
    for w in [packet.len() as u32, PACKET, 0, 0, 0] {
        c.out.extend(w.to_be_bytes());
    }
    c.out.extend_from_slice(packet);
}

fn reply(tag: u32) -> Vec<u8> {
    let mut r = Vec::new();
    put_u32(&mut r, REPLY);
    put_u32(&mut r, tag);
    r
}

fn ack(c: &mut Client, tag: u32) {
    send(c, &reply(tag));
}

fn error(c: &mut Client, tag: u32, code: u32) {
    let mut r = Vec::new();
    put_u32(&mut r, ERROR);
    put_u32(&mut r, tag);
    put_u32(&mut r, code);
    send(c, &r);
}

/// A command the server sends unasked: its words after the command and the tag (all ones).
fn command(kind: u32, words: &[u32]) -> Vec<u8> {
    let mut m = Vec::new();
    put_u32(&mut m, kind);
    put_u32(&mut m, u32::MAX);
    for &w in words {
        put_u32(&mut m, w);
    }
    m
}

// ---- Tagged values ----

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.push(TAG_U32);
    out.extend(v.to_be_bytes());
}

fn put_u8(out: &mut Vec<u8>, v: u8) {
    out.extend([TAG_U8, v]);
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.push(TAG_U64);
    out.extend(v.to_be_bytes());
}

fn put_s64(out: &mut Vec<u8>, v: i64) {
    out.push(TAG_S64);
    out.extend(v.to_be_bytes());
}

fn put_usec(out: &mut Vec<u8>, v: u64) {
    out.push(TAG_USEC);
    out.extend(v.to_be_bytes());
}

fn put_bool(out: &mut Vec<u8>, v: bool) {
    out.push(if v { TAG_TRUE } else { TAG_FALSE });
}

fn put_string(out: &mut Vec<u8>, s: Option<&str>) {
    match s {
        Some(s) => {
            out.push(TAG_STRING);
            out.extend(s.as_bytes());
            out.push(0);
        }
        None => out.push(TAG_STRING_NULL),
    }
}

fn put_timeval(out: &mut Vec<u8>, sec: u32, usec: u32) {
    out.push(TAG_TIMEVAL);
    out.extend(sec.to_be_bytes());
    out.extend(usec.to_be_bytes());
}

fn put_sample_spec(out: &mut Vec<u8>, format: u8, channels: u8, rate: u32) {
    out.extend([TAG_SAMPLE_SPEC, format, channels]);
    out.extend(rate.to_be_bytes());
}

fn put_channel_map(out: &mut Vec<u8>, map: &[u8]) {
    out.extend([TAG_CHANNEL_MAP, map.len() as u8]);
    out.extend(map);
}

fn put_cvolume(out: &mut Vec<u8>, volumes: &[u32]) {
    out.extend([TAG_CVOLUME, volumes.len() as u8]);
    for v in volumes {
        out.extend(v.to_be_bytes());
    }
}

fn put_volume(out: &mut Vec<u8>, v: u32) {
    out.push(TAG_VOLUME);
    out.extend(v.to_be_bytes());
}

fn put_proplist(out: &mut Vec<u8>, props: &[(&str, &str)]) {
    out.push(TAG_PROPLIST);
    for (key, value) in props {
        put_string(out, Some(key));
        // A value is its bytes and a NUL, its length said twice, as PulseAudio writes it.
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        put_u32(out, bytes.len() as u32);
        out.push(TAG_ARBITRARY);
        out.extend((bytes.len() as u32).to_be_bytes());
        out.extend(bytes);
    }
    out.push(TAG_STRING_NULL);
}

fn put_format_info(out: &mut Vec<u8>, encoding: u8, props: &[(&str, &str)]) {
    out.push(TAG_FORMAT_INFO);
    put_u8(out, encoding);
    put_proplist(out, props);
}

/// A packet's tagged values, read in order; each read None once they don't read.
struct Tags<'a> {
    b: &'a [u8],
}

impl<'a> Tags<'a> {
    fn new(b: &'a [u8]) -> Tags<'a> {
        Tags { b }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.b.len() < n {
            self.b = &[];
            return None;
        }
        let (head, rest) = self.b.split_at(n);
        self.b = rest;
        Some(head)
    }

    fn tag(&mut self, want: u8) -> Option<()> {
        (self.b.first() == Some(&want)).then(|| self.b = &self.b[1..])
    }

    fn be32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u32(&mut self) -> Option<u32> {
        self.tag(TAG_U32)?;
        self.be32()
    }

    fn boolean(&mut self) -> Option<bool> {
        match self.b.first() {
            Some(&TAG_TRUE) => {
                self.b = &self.b[1..];
                Some(true)
            }
            Some(&TAG_FALSE) => {
                self.b = &self.b[1..];
                Some(false)
            }
            _ => None,
        }
    }

    fn string(&mut self) -> Option<String> {
        match *self.b.first()? {
            TAG_STRING_NULL => {
                self.b = &self.b[1..];
                None
            }
            TAG_STRING => {
                let end = self.b.iter().position(|&x| x == 0)?;
                let s = String::from_utf8_lossy(&self.b[1..end]).into_owned();
                self.b = &self.b[end + 1..];
                Some(s)
            }
            _ => None,
        }
    }

    fn timeval(&mut self) -> Option<(u32, u32)> {
        self.tag(TAG_TIMEVAL)?;
        Some((self.be32()?, self.be32()?))
    }

    fn sample_spec(&mut self) -> Option<(u8, u8, u32)> {
        self.tag(TAG_SAMPLE_SPEC)?;
        let b = self.take(2)?;
        let (format, channels) = (b[0], b[1]);
        Some((format, channels, self.be32()?))
    }

    fn channel_map(&mut self) -> Option<Vec<u8>> {
        self.tag(TAG_CHANNEL_MAP)?;
        let n = self.take(1)?[0] as usize;
        self.take(n).map(<[u8]>::to_vec)
    }

    fn cvolume(&mut self) -> Option<Vec<u32>> {
        self.tag(TAG_CVOLUME)?;
        let n = self.take(1)?[0] as usize;
        (0..n).map(|_| self.be32()).collect()
    }

    fn proplist(&mut self) -> Option<Vec<(String, Vec<u8>)>> {
        self.tag(TAG_PROPLIST)?;
        let mut props = Vec::new();
        loop {
            if self.b.first() == Some(&TAG_STRING_NULL) {
                self.b = &self.b[1..];
                return Some(props);
            }
            let key = self.string()?;
            let n = self.u32()? as usize;
            self.tag(TAG_ARBITRARY)?;
            let m = self.be32()? as usize;
            if m != n {
                return None;
            }
            props.push((key, self.take(n)?.to_vec()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_values_read_back() {
        let mut b = Vec::new();
        put_u32(&mut b, 7);
        put_string(&mut b, Some("carbon"));
        put_string(&mut b, None);
        put_bool(&mut b, true);
        put_sample_spec(&mut b, 3, 2, 44100);
        put_channel_map(&mut b, &[1, 2]);
        put_cvolume(&mut b, &[NORM, NORM / 2]);
        put_proplist(&mut b, &[("media.name", "music")]);
        put_timeval(&mut b, 5, 6);
        let mut t = Tags::new(&b);
        assert_eq!(t.u32(), Some(7));
        assert_eq!(t.string().as_deref(), Some("carbon"));
        assert_eq!(t.string(), None);
        assert_eq!(t.boolean(), Some(true));
        assert_eq!(t.sample_spec(), Some((3, 2, 44100)));
        assert_eq!(t.channel_map(), Some(vec![1, 2]));
        assert_eq!(t.cvolume(), Some(vec![NORM, NORM / 2]));
        assert_eq!(t.proplist(), Some(vec![("media.name".to_string(), b"music\0".to_vec())]));
        assert_eq!(t.timeval(), Some((5, 6)));
        assert!(t.b.is_empty());
        // A u8 isn't a u32.
        let mut b = Vec::new();
        put_u8(&mut b, 1);
        assert_eq!(Tags::new(&b).u32(), None);
    }

    #[test]
    fn buffer_attributes_are_whole_frames_and_consistent() {
        let f = Format::new(Sample::S16Le, 2, 48_000);
        // The server's choices: 250 ms, 10 ms requests, prebuffered to the target, twice the target to hold.
        assert_eq!(attrs(&f, [u32::MAX; 4]), (96_000, 48_000, 48_000, 1_920));
        // Odd sizes rounded to frames, a request no more than half the target, a prebuffer no more than it.
        let (max, t, pre, min) = attrs(&f, [10_001, 9_999, 20_000, 9_000]);
        assert_eq!((t % 4, min % 4, pre, max % 4), (0, 0, t, 0));
        assert!(min <= t / 2 && max >= t);
        // A prebuffer of 0 starts at once.
        assert_eq!(attrs(&f, [u32::MAX, u32::MAX, 0, u32::MAX]).2, 0);
    }

    #[test]
    fn volumes_are_cubic() {
        assert_eq!(linear(&[NORM, NORM / 2]), [1.0, 0.125]);
        assert_eq!(linear(&[NORM / 2]), [0.125, 0.125]);
    }
}
