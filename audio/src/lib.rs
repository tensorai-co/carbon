//! A machine's sound: the mixer every program's sound goes into, whichever kernel's API it played through, and where
//! the mix goes. `audio` is to sound what `display` is to the screen: the kernels open streams on a [`Mixer`] and write
//! into them as their programs do; an output pulls the mix and sends it on (the host's speakers, a WAV file, nothing).
//!
//! - **Streams** ([`Stream`]) are a program's sound in its own format ([`Format`]): any of the sample types, up to 32
//!   channels, any rate. Each has a buffer of a size its kernel chose, a volume, and can be paused; a write takes what
//!   fits, and the stream's waker says when there's room again, so a kernel can block or wake its program as its own
//!   OS would. A stream can wait to start until enough is buffered (PulseAudio's prebuffering).
//! - **The mix** is 48 kHz stereo floats ([`RATE`]): each stream's frames folded to stereo, resampled (linearly, its
//!   position kept between pulls so nothing clicks), scaled by its volume, and summed. A stream that runs dry plays
//!   silence and counts an underrun.
//! - **The clock is the output's.** Whatever plays the mix pulls it at its own pace ([`Mixer::mix`]): the host's sound
//!   device from its own callback, so there's one clock and nothing drifts; without a device, a thread on the host's
//!   clock ([`output::Timed`]), a period ([`PERIOD`]) at a time. Every stream's position is that clock's, so a kernel
//!   that tells its program how much has played tells it what was heard.
//!
//! It depends on nothing of Carbon's. Recording, loopbacks and the host outputs come as their own pieces: the host
//! outputs in a folder per host (`audio/macos`).

mod format;
pub mod output;
#[cfg(test)]
mod tests;

pub use format::{Format, MAX_CHANNELS, MAX_RATE, Sample};

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

/// The mix's rate, in frames a second, and its channels.
pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// The frames a timed output pulls at a time: 10 ms.
pub const PERIOD: usize = (RATE / 100) as usize;

/// What's called when a stream has room again, or has played what it was asked to drain: from the output's thread,
/// with no lock held, so it may call back into the stream.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// A machine's mixer: shared by every kernel thread that plays and the output that pulls.
#[derive(Clone, Default)]
pub struct Mixer(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    streams: BTreeMap<u64, StreamState>,
    next: u64,
    /// Frames mixed so far: the clock.
    frames: u64,
}

struct StreamState {
    format: Format,
    /// Bytes written and not yet played.
    queue: VecDeque<u8>,
    capacity: usize,
    /// Bytes to have before it starts (or starts again after running dry); 0 starts at once.
    prebuffer: usize,
    started: bool,
    paused: bool,
    volume: [f32; 2],
    /// The resampler: the frame at or before the next output frame, the one after it (taken only when the output
    /// frame falls between them), and how far between them it is (0 to 1).
    prev: [f32; 2],
    next: Option<[f32; 2]>,
    primed: bool,
    at: f64,
    /// Frames of its own taken from the queue so far, and bytes written.
    played: u64,
    written: u64,
    underruns: u64,
    dry: bool,
    waker: Option<Waker>,
}

impl StreamState {
    /// The next frame from the queue, as stereo, if a whole one's there.
    fn pop(&mut self) -> Option<[f32; 2]> {
        let n = self.format.frame_bytes();
        if self.queue.len() < n {
            return None;
        }
        let mut frame = [0u8; 4 * MAX_CHANNELS as usize];
        for (i, b) in self.queue.drain(..n).enumerate() {
            frame[i] = b;
        }
        self.played += 1;
        Some(self.format.stereo(&frame[..n]))
    }

    /// Adds this stream's next `out.len() / 2` frames of the mix to `out`: false if it ran dry on the way.
    fn mix_into(&mut self, out: &mut [f32]) -> bool {
        if self.paused {
            return true;
        }
        if !self.started {
            if self.queue.len() < self.prebuffer.max(1) {
                return true;
            }
            self.started = true;
        }
        let step = f64::from(self.format.rate) / f64::from(RATE);
        for o in out.as_chunks_mut::<CHANNELS>().0 {
            if !self.primed {
                let Some(f) = self.pop() else { return self.ran_dry() };
                self.prev = f;
                self.primed = true;
                self.at = 0.0;
            }
            while self.at >= 1.0 {
                let Some(f) = self.next.take().or_else(|| self.pop()) else { return self.ran_dry() };
                self.prev = f;
                self.at -= 1.0;
            }
            let value = if self.at == 0.0 {
                self.prev
            } else {
                if self.next.is_none() {
                    let Some(f) = self.pop() else { return self.ran_dry() };
                    self.next = Some(f);
                }
                let (a, b, t) = (self.prev, self.next.unwrap_or(self.prev), self.at as f32);
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
            };
            for c in 0..CHANNELS {
                o[c] += value[c] * self.volume[c];
            }
            self.at += step;
        }
        self.dry = false;
        true
    }

    /// It ran out partway: silence for the rest, an underrun counted once for each time it runs dry, and with
    /// prebuffering it waits to fill again before it goes on.
    fn ran_dry(&mut self) -> bool {
        if !self.dry {
            self.underruns += 1;
            self.dry = true;
        }
        if self.prebuffer > 0 {
            self.started = false;
        }
        false
    }
}

impl std::fmt::Debug for Mixer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Mixer({} streams)", self.streams())
    }
}

impl Mixer {
    pub fn new() -> Mixer {
        Mixer::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A playback stream in `format`, holding at most `capacity` bytes, waiting for `prebuffer` bytes before it starts.
    pub fn open(&self, format: Format, capacity: usize, prebuffer: usize) -> Result<Stream, String> {
        if !format.is_valid() {
            return Err(format!("a stream of {} channels at {} Hz", format.channels, format.rate));
        }
        let n = format.frame_bytes();
        let capacity = capacity.max(n) / n * n;
        let mut s = self.lock();
        s.next += 1;
        let id = s.next;
        s.streams.insert(
            id,
            StreamState {
                format,
                queue: VecDeque::with_capacity(capacity),
                capacity,
                prebuffer: prebuffer.min(capacity),
                started: false,
                paused: false,
                volume: [1.0; 2],
                prev: [0.0; 2],
                next: None,
                primed: false,
                at: 0.0,
                played: 0,
                written: 0,
                underruns: 0,
                dry: false,
                waker: None,
            },
        );
        Ok(Stream { id, mixer: self.clone() })
    }

    /// The next `out.len() / 2` frames of the mix, interleaved stereo, written over `out`: what an output calls at its
    /// own pace. Streams that took frames have their wakers called after, with no lock held.
    pub fn mix(&self, out: &mut [f32]) {
        out.fill(0.0);
        let mut wake = Vec::new();
        {
            let mut s = self.lock();
            s.frames += (out.len() / CHANNELS) as u64;
            for st in s.streams.values_mut() {
                let before = st.queue.len();
                st.mix_into(out);
                if st.queue.len() < before
                    && let Some(w) = &st.waker
                {
                    wake.push(w.clone());
                }
            }
        }
        for w in wake {
            w();
        }
    }

    /// Frames mixed so far, at [`RATE`]: the clock every stream's position is on.
    pub fn frames(&self) -> u64 {
        self.lock().frames
    }

    /// How many streams are open.
    pub fn streams(&self) -> usize {
        self.lock().streams.len()
    }
}

/// A playback stream: closed when it's dropped, whatever it still held let go.
pub struct Stream {
    id: u64,
    mixer: Mixer,
}

impl Stream {
    fn with<T>(&self, f: impl FnOnce(&mut StreamState) -> T) -> T {
        let mut s = self.mixer.lock();
        f(s.streams.get_mut(&self.id).expect("an open stream is the mixer's"))
    }

    pub fn format(&self) -> Format {
        self.with(|s| s.format)
    }

    /// Takes as many of `bytes` as fit: how many it took.
    pub fn write(&self, bytes: &[u8]) -> usize {
        self.with(|s| {
            let n = bytes.len().min(s.capacity - s.queue.len());
            s.queue.extend(&bytes[..n]);
            s.written += n as u64;
            n
        })
    }

    /// The bytes it has room for now.
    pub fn room(&self) -> usize {
        self.with(|s| s.capacity - s.queue.len())
    }

    /// The bytes written and not played yet.
    pub fn queued(&self) -> usize {
        self.with(|s| s.queue.len())
    }

    /// The buffer's size, in bytes.
    pub fn capacity(&self) -> usize {
        self.with(|s| s.capacity)
    }

    /// The buffer made another size (and the prebuffering with it), keeping what fits.
    pub fn resize(&self, capacity: usize, prebuffer: usize) {
        self.with(|s| {
            let n = s.format.frame_bytes();
            s.capacity = capacity.max(n) / n * n;
            s.prebuffer = prebuffer.min(s.capacity);
            let excess = s.queue.len().saturating_sub(s.capacity);
            s.queue.truncate(s.queue.len() - excess);
        })
    }

    /// Frames of its own played (taken by the mix) so far.
    pub fn played(&self) -> u64 {
        self.with(|s| s.played)
    }

    /// Bytes written so far, all of it, played or not.
    pub fn written(&self) -> u64 {
        self.with(|s| s.written)
    }

    pub fn underruns(&self) -> u64 {
        self.with(|s| s.underruns)
    }

    /// Whether it's playing: started (prebuffered, or triggered), not paused.
    pub fn is_playing(&self) -> bool {
        self.with(|s| s.started && !s.paused)
    }

    /// Each side's volume, linear (1 as it is).
    pub fn set_volume(&self, volume: [f32; 2]) {
        self.with(|s| s.volume = volume);
    }

    /// Paused streams keep what they hold, and play nothing (PulseAudio's cork).
    pub fn set_paused(&self, paused: bool) {
        self.with(|s| s.paused = paused);
    }

    /// Starts it now, however little it holds (PulseAudio's trigger).
    pub fn trigger(&self) {
        self.with(|s| s.started = true);
    }

    /// Lets go of what it holds, unplayed; it waits to prebuffer again.
    pub fn flush(&self) {
        self.with(|s| {
            s.queue.clear();
            s.primed = false;
            s.next = None;
            if s.prebuffer > 0 {
                s.started = false;
            }
        });
    }

    /// What's called when the mix takes from it, so there's room again.
    pub fn set_waker(&self, waker: Option<Waker>) {
        self.with(|s| s.waker = waker);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.mixer.lock().streams.remove(&self.id);
    }
}
