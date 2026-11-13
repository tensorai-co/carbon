// The C runtime (ucrtbase.dll) on atlas. Built two ways by tests/programs/build.sh: with clang for ARM64 and x86-64,
// calling ucrtbase's exports itself (printf through __stdio_common_vsprintf), and with MinGW-w64 for x86-64 as an
// ordinary C program, its own startup and headers, linked to the UCRT.
//
// It checks: printf's conversions, flags, widths, precisions and sizes, floats, strings narrow and wide, snprintf
// cut short; strings and memory; strtol, strtoul, strtod and atoi; qsort and bsearch; character classes; malloc,
// calloc and realloc; getenv; the arguments; files written and read in text and binary mode, with fgets, fseek,
// ftell and ungetc; rand; errno; and atexit's functions run last first as the program exits.

#include <stdarg.h>
#include <stddef.h>

#ifdef __MINGW32__
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <wchar.h>
#define FMT snprintf
#else
typedef struct FILE FILE;
#define I __declspec(dllimport)
I int __stdio_common_vsprintf(unsigned long long, char *, size_t, const char *, void *, va_list);
I int __stdio_common_vfprintf(unsigned long long, FILE *, const char *, void *, va_list);
I FILE *__acrt_iob_func(unsigned);
I void *malloc(size_t);
I void *calloc(size_t, size_t);
I void *realloc(void *, size_t);
I void free(void *);
I size_t strlen(const char *);
I int strcmp(const char *, const char *);
I int strncmp(const char *, const char *, size_t);
I int _stricmp(const char *, const char *);
I char *strcpy(char *, const char *);
I char *strcat(char *, const char *);
I char *strchr(const char *, int);
I char *strrchr(const char *, int);
I char *strstr(const char *, const char *);
I char *strtok(char *, const char *);
I char *_strdup(const char *);
I int memcmp(const void *, const void *, size_t);
I void *memmove(void *, const void *, size_t);
I void *memset(void *, int, size_t);
I long strtol(const char *, char **, int);
I unsigned long strtoul(const char *, char **, int);
I double strtod(const char *, char **);
I int atoi(const char *);
I void qsort(void *, size_t, size_t, int (*)(const void *, const void *));
I void *bsearch(const void *, const void *, size_t, size_t, int (*)(const void *, const void *));
I int isalpha(int), isdigit(int), isspace(int), toupper(int);
I char *getenv(const char *);
I int *_errno(void);
I void srand(unsigned);
I int rand(void);
I FILE *fopen(const char *, const char *);
I int fclose(FILE *);
I int fputs(const char *, FILE *);
I char *fgets(char *, int, FILE *);
I size_t fread(void *, size_t, size_t, FILE *);
I int fseek(FILE *, long, int);
I long ftell(FILE *);
I int fgetc(FILE *);
I int ungetc(int, FILE *);
I int feof(FILE *);
I int atexit(void (*)(void));
I void exit(int);
I int _configure_narrow_argv(int);
I int *__p___argc(void);
I char ***__p___argv(void);
#define errno (*_errno())
#define ERANGE 34
#define EOF (-1)
#define stdout __acrt_iob_func(1)
#if defined(__x86_64__)
// MSVC's linker wants it wherever floating point is used; a program's C runtime would define it.
int _fltused = 0;
#endif
static int FMT(char *b, size_t n, const char *f, ...) {
    va_list ap;
    va_start(ap, f);
    int r = __stdio_common_vsprintf(2, b, n, f, 0, ap);
    va_end(ap);
    return r;
}
static int fprintf(FILE *s, const char *f, ...) {
    va_list ap;
    va_start(ap, f);
    int r = __stdio_common_vfprintf(0, s, f, 0, ap);
    va_end(ap);
    return r;
}
#endif

__declspec(dllimport) int __stdcall DeleteFileA(const char *);

static int failures;
static void check(int ok, const char *what) {
    fputs(ok ? "ok      " : "FAILED  ", stdout);
    fputs(what, stdout);
    fputs("\n", stdout);
    if (!ok) failures = failures + 1;
}
static char b[256];
static int fmt_is(const char *want, const char *f, ...) {
    va_list ap;
    va_start(ap, f);
#ifdef __MINGW32__
    vsnprintf(b, sizeof b, f, ap);
#else
    __stdio_common_vsprintf(2, b, sizeof b, f, 0, ap);
#endif
    va_end(ap);
    int same = strcmp(b, want) == 0;
    if (!same) {
        fputs("   got [", stdout);
        fputs(b, stdout);
        fputs("]\n", stdout);
    }
    return same;
}
static int by_int(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }

static char trail[8];
static void second(void) { strcat(trail, "2"); }
static void first(void) {
    strcat(trail, "1");
    check(strcmp(trail, "21") == 0, "atexit: the functions run last registered first, as the program exits");
}

int run(int argc, char **argv) {
#if defined(__aarch64__)
    fputs("the C runtime, arm64\n", stdout);
#elif defined(__MINGW32__)
    fputs("the C runtime, x86-64, built by MinGW-w64\n", stdout);
#else
    fputs("the C runtime, x86-64\n", stdout);
#endif
    check(fmt_is("   42|42   |00042|+42| 42", "%5d|%-5d|%05d|%+d|% d", 42, 42, 42, 42, 42), "printf: widths and flags");
    check(fmt_is("ff FF 0xff 377 0377 -7 4294967289", "%x %X %#x %o %#o %d %u", 255, 255, 255, 255, 255, -7, -7), "  hex, octal, signed and unsigned");
    check(fmt_is("-1234567890123 18446744073709551615 9", "%lld %llu %I64d", -1234567890123LL, 18446744073709551615ULL, 9LL), "  64-bit sizes");
    check(fmt_is("3.142     2.50 2.50    | 1.234568e+04 1.230000E-04", "%.3f %8.2f %-8.2f| %e %E", 3.14159, 2.5, 2.5, 12345.678, 0.000123), "  %f and %e");
    check(fmt_is("100000 1e+06 1e-05 0.0001 0.5 2.50000", "%g %g %g %g %g %#g", 100000.0, 1000000.0, 0.00001, 0.0001, 0.5, 2.5), "  %g");
    check(fmt_is("hello|hel|     hello|hello     |", "%s|%.3s|%10s|%-10s|", "hello", "hello", "hello", "hello"), "  strings, cut and padded");
    check(fmt_is("ab% 0000000000001234 wide (null)", "%c%c%% %p %ls %s", 'a', 'b', (void *)0x1234, L"wide", (char *)0), "  %c, %%, %p, a wide string and a null one");
    check(fmt_is("     7|1.00|-0003", "%*d|%.*f|%05d", 6, 7, 2, 1.0, -3), "  * for width and precision; zeros after the sign");
    check(fmt_is("inf -inf nan", "%f %f %f", 1.0 / 0.0, -1.0 / 0.0, 0.0 / 0.0), "  inf and nan");
    check(FMT(b, 5, "%s", "abcdefgh") == 8 && strcmp(b, "abcd") == 0, "snprintf: cut to fit, terminated, the whole length given");

    char s[64];
    strcpy(s, "hello");
    strcat(s, ", world");
    check(strlen(s) == 12 && strcmp(s, "hello, world") == 0 && strcmp("a", "b") < 0 && strncmp("abcd", "abce", 3) == 0 && _stricmp("HeLLo", "hello") == 0,
          "strcpy, strcat, strlen, strcmp, strncmp, _stricmp");
    check(strchr(s, 'o') == s + 4 && strrchr(s, 'o') == s + 8 && strstr(s, "wor") == s + 7 && !strstr(s, "xyz"), "strchr, strrchr, strstr");
    char t[] = "a,b;;c";
    char *one = strtok(t, ",;"), *two = strtok(0, ",;"), *three = strtok(0, ",;"), *none = strtok(0, ",;");
    check(one && two && three && !none && strcmp(one, "a") == 0 && strcmp(two, "b") == 0 && strcmp(three, "c") == 0, "strtok");
    char *dup = _strdup(s);
    check(dup && strcmp(dup, s) == 0 && dup != s, "_strdup");
    free(dup);
    char m[] = "123456789";
    memmove(m + 2, m, 5);
    check(memcmp(m, "121234589", 9) == 0, "memmove, overlapping");

    char *end = 0;
    check(strtol("  -0x1fz", &end, 0) == -31 && *end == 'z', "strtol, base 0, with a prefix");
    errno = 0;
    check(strtoul("4294967296", 0, 10) == 0xffffffffUL && errno == ERANGE, "strtoul: too big, ERANGE");
    check(strtod("3.25e2xyz", &end) == 325.0 && strcmp(end, "xyz") == 0 && strtod("-.5", 0) == -0.5, "strtod");
    check(atoi("123abc") == 123 && atoi("  -9") == -9, "atoi");

    int numbers[] = {5, 3, 9, 1, 7, 2, 8};
    qsort(numbers, 7, sizeof(int), by_int);
    int key = 7;
    int *found = bsearch(&key, numbers, 7, sizeof(int), by_int);
    check(numbers[0] == 1 && numbers[6] == 9 && numbers[3] == 5 && found == numbers + 4, "qsort and bsearch");
    check(isalpha('q') && !isalpha('1') && isdigit('7') && isspace('\t') && toupper('x') == 'X', "character classes");

    char *p = malloc(16);
    strcpy(p, "kept");
    p = realloc(p, 4096);
    int *zeros = calloc(100, sizeof(int));
    int all_zero = 1;
    for (int i = 0; i < 100; i++) all_zero &= zeros[i] == 0;
    check(p && strcmp(p, "kept") == 0 && zeros && all_zero, "malloc, realloc keeping what was there, calloc zeroed");
    free(p);
    free(zeros);
    check(getenv("PATH") != 0 && getenv("path") != 0 && getenv("NO_SUCH_VARIABLE") == 0, "getenv, folding case");
    check(argc == 3 && strcmp(argv[1], "one") == 0 && strcmp(argv[2], "two words") == 0, "the arguments, one with a space");

    FILE *f = fopen("C:\\crt.txt", "w");
    check(f != 0, "fopen to write");
    fputs("first line\nsecond\n", f);
    fprintf(f, "%d %s\n", 3, "third");
    fclose(f);
    f = fopen("C:\\crt.txt", "rb");
    char raw[64];
    size_t n = fread(raw, 1, sizeof raw, f);
    fclose(f);
    check(n == 29 && raw[10] == '\r' && raw[11] == '\n', "text mode: each LF written as CR LF, as binary reading sees");
    f = fopen("C:\\crt.txt", "r");
    char line[64];
    check(fgets(line, sizeof line, f) && strcmp(line, "first line\n") == 0, "fgets: a line, CR LF read as LF");
    long at = ftell(f);
    fgets(line, sizeof line, f);
    check(strcmp(line, "second\n") == 0 && at == 12, "  the next, and ftell where it starts");
    fseek(f, at, 0);
    int c = fgetc(f);
    ungetc(c, f);
    check(c == 's' && fgets(line, sizeof line, f) && strcmp(line, "second\n") == 0, "fseek back, fgetc and ungetc");
    fgets(line, sizeof line, f);
    check(strcmp(line, "3 third\n") == 0 && fgetc(f) == EOF && feof(f), "  the last, then EOF");
    fclose(f);
    check(fopen("C:\\no\\such.txt", "r") == 0 && errno == 2, "fopen of what isn't there: null, ENOENT");
    DeleteFileA("C:\\crt.txt");

    srand(1);
    check(rand() == 41 && rand() == 18467, "rand after srand(1): the sequence Windows gives");

    atexit(first);
    atexit(second);
    return failures;
}

#ifdef __MINGW32__
int main(int argc, char **argv) { return run(argc, argv); }
#else
void mainCRTStartup(void) {
    _configure_narrow_argv(1);
    exit(run(*__p___argc(), *__p___argv()));
}
#endif
