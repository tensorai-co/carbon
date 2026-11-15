//! Outputs with no sound device of their own: a thread pulls the mix a period at a time on the host's clock, and
//! hands each to a sink: nothing (`--audio none`, so programs' sound is taken at the pace it would play), or a WAV
//! file (`--audio out.wav`, and the tests).

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{CHANNELS, Mixer, PERIOD, RATE};

/// A thread pulling the mix in real time, a [`PERIOD`] at a time, until it's dropped.
pub struct Timed {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Timed {
    /// Pulls `mixer`'s mix every period, on time by the host's clock however long each pull took, and hands it to
    /// `sink`.
    pub fn start(mixer: Mixer, mut sink: impl FnMut(&[f32]) + Send + 'static) -> Timed {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                let period = Duration::from_secs_f64(PERIOD as f64 / f64::from(RATE));
                let mut buffer = vec![0.0f32; PERIOD * CHANNELS];
                let mut next = Instant::now();
                while !stopped.load(Ordering::Acquire) {
                    mixer.mix(&mut buffer);
                    sink(&buffer);
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else if now - next > period * 10 {
                        // Far behind (the host was asleep): start again from now rather than rushing to catch up.
                        next = now;
                    }
                }
            })
            .expect("starting the audio thread");
        Timed { stop, thread: Some(thread) }
    }

    /// The mix taken and thrown away: `--audio none`.
    pub fn none(mixer: Mixer) -> Timed {
        Timed::start(mixer, |_| {})
    }

    /// The mix written to a WAV file, 16-bit stereo at [`RATE`], finished when this is dropped.
    pub fn wav(mixer: Mixer, path: &Path) -> Result<Timed, String> {
        let mut wav = Wav::create(path)?;
        Ok(Timed::start(mixer, move |frames| {
            let _ = wav.write(frames);
        }))
    }
}

impl Drop for Timed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A WAV file being written: 16-bit stereo PCM at [`RATE`]. Its header's sizes are written as it's dropped.
pub struct Wav {
    file: BufWriter<File>,
    bytes: u32,
}

const HEADER: usize = 44;

impl Wav {
    pub fn create(path: &Path) -> Result<Wav, String> {
        let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut w = Wav { file: BufWriter::new(file), bytes: 0 };
        w.file.write_all(&header(0)).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(w)
    }

    /// Interleaved stereo frames, clipped to `[-1, 1]`.
    pub fn write(&mut self, frames: &[f32]) -> std::io::Result<()> {
        let mut out = Vec::with_capacity(frames.len() * 2);
        for &v in frames {
            out.extend(to_i16(v).to_le_bytes());
        }
        self.bytes = self.bytes.saturating_add(out.len() as u32);
        self.file.write_all(&out)
    }

    fn finish(&mut self) -> std::io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&header(self.bytes))?;
        self.file.flush()
    }
}

impl Drop for Wav {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// A sample in `[-1, 1]` as a 16-bit one, rounded, clipped.
pub fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// The 44-byte header of a 16-bit stereo PCM WAV file holding `bytes` of samples.
fn header(bytes: u32) -> [u8; HEADER] {
    let mut h = [0u8; HEADER];
    let block = (CHANNELS * 2) as u16;
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + bytes).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&(CHANNELS as u16).to_le_bytes());
    h[24..28].copy_from_slice(&RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(RATE * u32::from(block)).to_le_bytes());
    h[32..34].copy_from_slice(&block.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&bytes.to_le_bytes());
    h
}
