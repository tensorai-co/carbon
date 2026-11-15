// Sound through lambda's PulseAudio server, as a program using the image's libpulse plays it: libpulse-simple,
// loaded by name (no headers needed), opens a playback stream, writes a second of a 440 Hz sine at half scale,
// 16-bit stereo at 44.1 kHz (so the server resamples it), asks for the latency, drains, and closes. Run with
// `--audio FILE.wav`, the file holds the tone: about a second of it, at 440 Hz, at half scale.
//
//     cc -O2 -o tone tone.c -ldl -lm && ./tone

#include <dlfcn.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct { int format; uint32_t rate; uint8_t channels; } sample_spec;
enum { S16LE = 3, PLAYBACK = 1 };

static int failures;

static void check(int ok, const char *what) {
    printf("%s: %s\n", ok ? "ok" : "FAIL", what);
    if (!ok) failures++;
}

int main(void) {
    void *lib = dlopen("libpulse-simple.so.0", RTLD_NOW);
    check(lib != NULL, "libpulse-simple loads");
    if (!lib) return 1;
    void *(*simple_new)(const char *, const char *, int, const char *, const char *, const sample_spec *, const void *, const void *, int *) = dlsym(lib, "pa_simple_new");
    int (*simple_write)(void *, const void *, size_t, int *) = dlsym(lib, "pa_simple_write");
    int (*simple_drain)(void *, int *) = dlsym(lib, "pa_simple_drain");
    uint64_t (*simple_latency)(void *, int *) = dlsym(lib, "pa_simple_get_latency");
    void (*simple_free)(void *) = dlsym(lib, "pa_simple_free");
    const char *(*strerror_)(int) = dlsym(lib, "pa_strerror");

    sample_spec ss = { S16LE, 44100, 2 };
    int error = 0;
    void *s = simple_new(NULL, "tone", PLAYBACK, NULL, "a tone", &ss, NULL, NULL, &error);
    if (!s) printf("pa_simple_new: %s\n", strerror_(error));
    check(s != NULL, "a playback stream on the server");
    if (!s) return 1;

    enum { RATE = 44100, FRAMES = RATE };
    int16_t *buf = malloc(FRAMES * 2 * sizeof(int16_t));
    for (int i = 0; i < FRAMES; i++) {
        int16_t v = (int16_t) lrint(16384.0 * sin(2.0 * M_PI * 440.0 * i / RATE));
        buf[2 * i] = v;
        buf[2 * i + 1] = v;
    }
    // In tenths of a second, as a program writes as it makes its sound.
    int ok = 1;
    for (int i = 0; i < 10 && ok; i++) ok = simple_write(s, buf + i * (FRAMES / 10) * 2, (FRAMES / 10) * 4, &error) == 0;
    if (!ok) printf("pa_simple_write: %s\n", strerror_(error));
    check(ok, "a second written");
    uint64_t latency = simple_latency(s, &error);
    printf("latency: %llu us\n", (unsigned long long) latency);
    check(latency != (uint64_t) -1 && latency < 2000000, "the latency, under two seconds");
    check(simple_drain(s, &error) == 0, "drained");
    simple_free(s);
    printf("%d checks, %d failures\n", 5, failures);
    return failures != 0;
}
