//! The mixer against known sums: streams of known samples in different formats, rates and volumes, mixed and checked
//! sample by sample; underruns, prebuffering, buffers that fill, wakers; the WAV output's bytes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;
use crate::output::{Timed, Wav};

fn s16(v: &[i16]) -> Vec<u8> {
    v.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn f32le(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn mix(m: &Mixer, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames * CHANNELS];
    m.mix(&mut out);
    out
}

fn close(a: &[f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert!((x - y).abs() < 1e-4, "sample {i}: {x} and {y}\n{a:?}\n{b:?}");
    }
}

#[test]
fn every_sample_format_reads_to_the_same_value() {
    let half = [
        (Sample::U8, vec![192u8]),
        (Sample::S16Le, 16384i16.to_le_bytes().to_vec()),
        (Sample::S16Be, 16384i16.to_be_bytes().to_vec()),
        (Sample::S24Le, vec![0, 0, 0x40]),
        (Sample::S24Be, vec![0x40, 0, 0]),
        (Sample::S24In32Le, vec![0, 0, 0x40, 0]),
        (Sample::S24In32Be, vec![0, 0x40, 0, 0]),
        (Sample::S32Le, (1i32 << 30).to_le_bytes().to_vec()),
        (Sample::S32Be, (1i32 << 30).to_be_bytes().to_vec()),
        (Sample::F32Le, 0.5f32.to_le_bytes().to_vec()),
        (Sample::F32Be, 0.5f32.to_be_bytes().to_vec()),
    ];
    for (sample, bytes) in half {
        assert_eq!(sample.bytes(), bytes.len(), "{sample:?}");
        assert!((sample.read(&bytes) - 0.5).abs() < 1e-6, "{sample:?} reads {}", sample.read(&bytes));
    }
    // Negative full scale, and G.711's extremes and zero.
    assert_eq!(Sample::S16Le.read(&i16::MIN.to_le_bytes()), -1.0);
    assert_eq!(Sample::S24Le.read(&[0, 0, 0x80]), -1.0);
    assert!((Sample::Ulaw.read(&[0x80]) - 32124.0 / 32768.0).abs() < 1e-6);
    assert!((Sample::Ulaw.read(&[0x00]) + 32124.0 / 32768.0).abs() < 1e-6);
    assert_eq!(Sample::Ulaw.read(&[0xff]), 0.0);
    assert!((Sample::Alaw.read(&[0xaa]) - 32256.0 / 32768.0).abs() < 1e-6);
    assert!((Sample::Alaw.read(&[0xd5]) - 8.0 / 32768.0).abs() < 1e-6);
}

#[test]
fn streams_in_different_formats_and_volumes_sum() {
    let m = Mixer::new();
    // Mono 16-bit, to both sides; stereo floats at half volume on the left.
    let a = m.open(Format::new(Sample::S16Le, 1, RATE), 1 << 16, 0).unwrap();
    let b = m.open(Format::new(Sample::F32Le, 2, RATE), 1 << 16, 0).unwrap();
    assert_eq!(a.write(&s16(&[8192, -8192, 16384, 0])), 8);
    b.write(&f32le(&[0.5, 0.1, 0.5, 0.2, 0.5, 0.3, 0.5, 0.4]));
    b.set_volume([0.5, 1.0]);
    let out = mix(&m, 4);
    close(&out, &[0.5, 0.35, 0.0, -0.05, 0.75, 0.8, 0.25, 0.4]);
    assert_eq!((a.played(), b.played(), m.frames()), (4, 4, 4));
}

#[test]
fn a_stream_at_another_rate_is_resampled_linearly() {
    let m = Mixer::new();
    // 24 kHz: each of its frames is two of the mix's, the second halfway to the next.
    let s = m.open(Format::new(Sample::F32Le, 1, RATE / 2), 1 << 16, 0).unwrap();
    s.write(&f32le(&[0.0, 0.4, 0.8, 0.4, 0.0]));
    let out = mix(&m, 6);
    close(&out, &[0.0, 0.0, 0.2, 0.2, 0.4, 0.4, 0.6, 0.6, 0.8, 0.8, 0.6, 0.6]);
    // 96 kHz: every other frame of its own.
    let m = Mixer::new();
    let s = m.open(Format::new(Sample::F32Le, 1, RATE * 2), 1 << 16, 0).unwrap();
    s.write(&f32le(&[0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7]));
    let out = mix(&m, 3);
    close(&out, &[0.0, 0.0, 0.2, 0.2, 0.4, 0.4]);
}

#[test]
fn six_channels_fold_to_stereo() {
    let f = Format::new(Sample::F32Le, 6, RATE);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    // Front left and right, centre, LFE (left out), rear left and right.
    let frame = f32le(&[0.1, 0.2, 0.3, 0.9, 0.4, 0.5]);
    let [l, r] = f.stereo(&frame);
    assert!((l - (0.1 + 0.3 * h + 0.4 * h)).abs() < 1e-6 && (r - (0.2 + 0.3 * h + 0.5 * h)).abs() < 1e-6, "{l} {r}");
}

#[test]
fn running_dry_plays_silence_and_counts_once() {
    let m = Mixer::new();
    let s = m.open(Format::new(Sample::S16Le, 2, RATE), 1 << 16, 0).unwrap();
    s.write(&s16(&[16384, 16384, 16384, 16384]));
    let out = mix(&m, 4);
    close(&out, &[0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
    mix(&m, 4);
    assert_eq!(s.underruns(), 1);
    // Fed again, it goes on, and running dry later counts again.
    s.write(&s16(&[8192, 8192, 8192, 8192]));
    close(&mix(&m, 2), &[0.25, 0.25, 0.25, 0.25]);
    mix(&m, 1);
    assert_eq!(s.underruns(), 2);
}

#[test]
fn prebuffering_waits_and_a_trigger_starts_it() {
    let m = Mixer::new();
    let f = Format::new(Sample::S16Le, 1, RATE);
    let s = m.open(f, 64, 16).unwrap();
    s.write(&s16(&[16384; 4]));
    close(&mix(&m, 2), &[0.0; 4]);
    assert!(!s.is_playing());
    s.write(&s16(&[16384; 4]));
    close(&mix(&m, 2), &[0.5; 4]);
    assert!(s.is_playing());
    // Paused, it keeps what it has; flushed, it waits to prebuffer again until it's triggered.
    s.set_paused(true);
    close(&mix(&m, 2), &[0.0; 4]);
    assert_eq!(s.queued(), 12);
    s.set_paused(false);
    s.flush();
    s.write(&s16(&[16384; 2]));
    close(&mix(&m, 1), &[0.0; 2]);
    s.trigger();
    close(&mix(&m, 1), &[0.5; 2]);
}

#[test]
fn a_full_buffer_takes_what_fits_and_the_waker_says_when_theres_room() {
    let m = Mixer::new();
    let s = m.open(Format::new(Sample::S16Le, 2, RATE), 10, 0).unwrap();
    // 10 bytes is two and a half frames: the buffer is two.
    assert_eq!((s.capacity(), s.write(&[0; 12])), (8, 8));
    assert_eq!(s.room(), 0);
    let woken = Arc::new(AtomicUsize::new(0));
    let w = woken.clone();
    s.set_waker(Some(Arc::new(move || {
        w.fetch_add(1, Ordering::Relaxed);
    })));
    mix(&m, 1);
    assert_eq!(woken.load(Ordering::Relaxed), 1);
    assert_eq!(s.room(), 4);
    mix(&m, 1);
    assert_eq!((woken.load(Ordering::Relaxed), s.room()), (2, 8));
    // Nothing left to take, no wake.
    mix(&m, 1);
    assert_eq!(woken.load(Ordering::Relaxed), 2);
    drop(s);
    assert_eq!(m.streams(), 0);
}

#[test]
fn the_wav_output_writes_its_header_and_samples() {
    let dir = std::env::temp_dir().join(format!("carbon-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.wav");
    {
        let mut w = Wav::create(&path).unwrap();
        w.write(&[0.5, -0.5, 2.0, -2.0]).unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 8);
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), RATE);
    assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
    let samples: Vec<i16> = bytes[44..].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    assert_eq!(samples, [16384, -16384, 32767, -32767]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_timed_output_pulls_a_period_at_a_time_in_real_time() {
    let m = Mixer::new();
    let s = m.open(Format::new(Sample::S16Le, 2, RATE), 1 << 20, 0).unwrap();
    s.write(&vec![0; 1 << 20]);
    let pulls = Arc::new(AtomicUsize::new(0));
    let p = pulls.clone();
    let timed = Timed::start(m.clone(), move |frames| {
        assert_eq!(frames.len(), PERIOD * CHANNELS);
        p.fetch_add(1, Ordering::Relaxed);
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    drop(timed);
    let n = pulls.load(Ordering::Relaxed);
    // About ten in 100 ms: never far more, as it keeps to the clock.
    assert!((3..=14).contains(&n), "{n} periods in 100 ms");
    assert_eq!(s.played(), (n * PERIOD) as u64);
}
