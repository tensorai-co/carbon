# audio

**Playback is built**: the mixer, the WAV and none outputs, the Mac's speakers (`audio/macos`), and lambda's
PulseAudio server, so Linux programs play through their own libpulse (or OpenAL, SDL and the rest over it). Recording,
atlas's sound APIs, and the Linux and Windows hosts are to do.

`audio` is to sound what [`display`](../display/README.md) is to the screen. Each kernel speaks its programs' sound
APIs, and plays every stream into one mixer, which `audio` sends wherever the machine's sound goes. Neither kernel
knows where that is.

```
 lambda: os/pulse (a PulseAudio server)        atlas: winmm, WASAPI (mmdevapi), DirectSound
                      │                                       │
                      ▼                                       ▼
 audio: the mixer   every playback stream in its own format, rate and volume,
                    converted and mixed to 48 kHz stereo, pulled by the output at its own pace;
                    record streams from the microphone, or from what's playing (to do)
                      │
     ┌────────────────┼───────────────────┬──────────────────┐
     ▼                ▼                   ▼                  ▼
 the host's speakers  remote's viewers    a WAV file         none
 and microphone       (Opus, as datagrams) (for tests)
 macos/ linux/ windows/
```

## What it does

- **Streams.** A kernel opens a playback stream with its format (sample type, channels, rate), writes into it as its
  program does, and sets its volume; the mixer pulls from every stream at its own pace and tells the kernel when one
  has room again, so a program's `write` blocks or wakes as it would on its own OS.
- **The mixer** converts each stream to 48 kHz stereo floats (folded to stereo, resampled linearly with its position
  kept between pulls), applies its volume, and sums them. A stream that underruns plays silence and counts it; one
  that's full takes only what fits.
- **One clock: the output's.** The mix isn't timed by a thread of its own: whatever plays it pulls it as it needs it
  (Core Audio's callback on the Mac), so the device's clock is the only clock and nothing drifts against it. Every
  stream's position is on that clock, so the latency a program works out is what's heard. With no device (`none`, a
  WAV file) a thread pulls a 10 ms period at a time on the host's clock. A slow kernel thread never makes the sound
  stutter: the device takes whatever's buffered.
- **Recording.** A record stream takes the microphone (from the host, or from a remote viewer), or what's playing
  (a loopback), converted to the format it asked for.
- **Outputs**, one or more at once: the host's speakers, each `remote` viewer, a WAV file, or nothing (`--audio none`,
  the default when there's no host to play on). Each output pulls mixed periods from the mixer; one that falls
  behind drops periods rather than delaying the rest.

## Each host

Like `display`, `net` and `host`: the shared part on top, a folder per host, and every host-specific line inside it.

| Host | Playback and recording |
| --- | --- |
| macOS (`audio/macos`) | Core Audio (an `AudioUnit` on the default devices) |
| Linux (`audio/linux`) | ALSA, or PipeWire where it runs |
| Windows (`audio/windows`) | WASAPI, shared mode |

## Layout

```
audio/
  src/          the shared part: streams, formats and conversion, the mixer's thread, record streams, the WAV output
  macos/ linux/ windows/        each host's speakers and microphone
```

It depends on nothing of Carbon's. The kernels use it, `machine` attaches an output to each machine, and `remote`
reads the mixer for its viewers.

## On the command line

```sh
carbon run --audio auto  …          # the host's speakers when the machine's on the host's screen, else none (the default)
carbon run --audio host  …          # the host's speakers, or fail
carbon run --audio none  …          # no sound: streams play into nothing, at the pace they would play
carbon run --audio out.wav …        # everything played, recorded to a file (16-bit stereo, 48 kHz)
```

## Testing

The mixer against known sums: streams of known samples in different formats, rates and volumes, mixed, and the
result checked sample by sample; underruns and overruns; record streams from a loopback. The kernels' sound APIs
through a WAV output, checked the same way.

The mixer's tests (`cargo test -p audio`) mix streams of known samples in every format, at other rates, volumes
and channel counts, and check the sum sample by sample, with underruns, prebuffering, full buffers and wakers;
`audio/macos`'s test checks the Mac's default output pulls the mix in real time. lambda's
`tests/programs/pulse/tone.c` plays a second of 440 Hz through the image's libpulse-simple at 44.1 kHz; run with
`--audio FILE.wav`, the file holds exactly that second, at 440 Hz and half scale, with no gaps.

## To do

- [x] The shared part: streams, conversion, the mixer, the WAV and none outputs, `--audio`.
- [ ] Record streams and loopbacks.
- [x] `audio/macos` (Core Audio's default output); then `audio/linux` and `audio/windows`.
- [x] lambda's PulseAudio server (`kernels/lambda/src/os/pulse.rs`, its native protocol, version 35): one sink,
      playback streams with PulseAudio's buffer attributes, requests, prebuffering, corking, draining, latency and
      volumes. Left: record streams and the monitor source, subscription events, sample uploads.
- [ ] lambda's ALSA (`/dev/snd`'s control and PCM devices), for programs that use only libasound.
- [ ] atlas's sound: winmm (`waveOut*`, `waveIn*`, then `midi*`), WASAPI (`mmdevapi`, `audioses`: programs ask for
      the device enumerator first), DirectSound, then XAudio2.
- [ ] Sound and the microphone over `remote` (Opus, 20 ms datagrams).
