# audio

**Not started.** A machine's sound: the mixer every program's sound goes into, and the microphone it records from.
This is its design, and what's to do.

`audio` is to sound what [`display`](../display/README.md) is to the screen. Each kernel speaks its programs' sound
APIs, and plays every stream into one mixer, which `audio` sends wherever the machine's sound goes. Neither kernel
knows where that is.

```
 lambda: os/pulse (a PulseAudio server)        atlas: winmm, WASAPI (mmdevapi), DirectSound
                      │                                       │
                      ▼                                       ▼
 audio: the mixer   every playback stream in its own format, rate and volume,
                    converted and mixed to 48 kHz stereo in real time, on a thread of its own;
                    record streams from the microphone, or from what's playing
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
- **The mixer** converts each stream to 48 kHz stereo floats, applies its volume, and sums them, on a thread of its
  own with a fixed period (10 ms), so a slow kernel thread never makes the sound stutter. A stream that underruns
  plays silence; one that overruns waits.
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
carbon run --audio host  …          # the host's speakers and microphone (the default where there are some)
carbon run --audio none  …          # no sound: streams play into nothing, recording hears silence
carbon run --audio out.wav …        # everything played, recorded to a file
```

`carbon run` takes only `--audio none` today.

## Testing

The mixer against known sums: streams of known samples in different formats, rates and volumes, mixed, and the
result checked sample by sample; underruns and overruns; record streams from a loopback. The kernels' sound APIs
through a WAV output, checked the same way.

## To do

- [ ] The shared part: streams, conversion, the mixer's thread, record streams, the WAV output, `--audio`.
- [ ] `audio/macos` (Core Audio), then `audio/linux` and `audio/windows`.
- [ ] lambda's PulseAudio server (`kernels/lambda/src/os/pulse`, its native protocol), so programs that use
      libpulse or PipeWire's Pulse layer play.
- [ ] atlas's sound: winmm (`waveOut*`, `waveIn*`, then `midi*`), WASAPI (`mmdevapi`, `audioses`: programs ask for
      the device enumerator first), DirectSound, then XAudio2.
- [ ] Sound and the microphone over `remote` (Opus, 20 ms datagrams).
