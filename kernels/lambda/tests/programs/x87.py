#!/usr/bin/env python3
"""Writes x87.c: x87 instructions run on known inputs, their results checked byte for byte.

Each case is raw machine code (so assembler spellings, like AT&T's swapped fsub and fsubr, can't get in the way), run
with rdi pointing at its inputs and rsi at 64 bytes of output. The answers come from Python's own floating point and
exact fractions: what an x87 stores, rounded to a double where the case stores a double. The cases keep to results an
x87 at its full 64-bit precision gives too, so they hold on real hardware as well as through exec's translator, which
computes at double precision.

    python3 x87.py > x87.c
"""

import math
import struct
from fractions import Fraction

RDI, RSI = 7, 6


def d(x):
    return struct.pack('<d', x)


def f(x):
    return struct.pack('<f', x)


def i16(x):
    return struct.pack('<h', x)


def i32(x):
    return struct.pack('<i', x)


def i64(x):
    return struct.pack('<q', x)


def u16(x):
    return struct.pack('<H', x)


def ext(x):
    """A double, or an exact fraction rounded to nearest at 64 bits, in the 80-bit format."""
    if isinstance(x, float):
        if math.isinf(x):
            return struct.pack('<QH', 1 << 63, 0x7FFF | (0x8000 if x < 0 else 0))
        if x == 0:
            return struct.pack('<QH', 0, 0x8000 if math.copysign(1, x) < 0 else 0)
        x = Fraction(x)
    sign = 0x8000 if x < 0 else 0
    x = abs(x)
    e = 0
    while x >= 2:
        x /= 2
        e += 1
    while x < 1:
        x *= 2
        e -= 1
    m = x * 2**63
    n = int(m)
    r = m - n
    if r > Fraction(1, 2) or (r == Fraction(1, 2) and n % 2):
        n += 1
    if n == 1 << 64:
        n >>= 1
        e += 1
    return struct.pack('<QH', n, sign | (e + 16383))


def raw(significand, top):
    return struct.pack('<QH', significand, top)


# ---- Encodings ----

def mem(op, ext_, base, disp):
    return [op, 0x40 | ext_ << 3 | base, disp & 0xFF]


def reg(op, ext_, i):
    return [op, 0xC0 | ext_ << 3 | i]


def fld64(at): return mem(0xDD, 0, RDI, at)
def fld32(at): return mem(0xD9, 0, RDI, at)
def fld80(at): return mem(0xDB, 5, RDI, at)
def fstp64(at): return mem(0xDD, 3, RSI, at)
def fst64(at): return mem(0xDD, 2, RSI, at)
def fstp32(at): return mem(0xD9, 3, RSI, at)
def fstp80(at): return mem(0xDB, 7, RSI, at)
def fldcw(at): return mem(0xD9, 5, RDI, at)
def fnstcw(at): return mem(0xD9, 7, RSI, at)


POP = reg(0xDD, 3, 0)  # fstp st(0)
FNSTSW_AX = [0xDF, 0xE0]
# and ax, 0x4700 (C0 to C3); mov [rsi+at], ax
def status(at): return FNSTSW_AX + [0x66, 0x25, 0x00, 0x47, 0x66, 0x89, 0x46, at]
def setcc(cc, at): return [0x0F, 0x90 | cc, 0x46, at]


# ---- Cases: (name, inputs, code, expected output with None for bytes not checked) ----

cases = []


def case(name, inputs, code, want, reference=True):
    """`reference`: whether Rosetta 2's x87 runs it too (it hasn't the undocumented `ffreep`)."""
    cases.append((name, bytes(inputs), code, list(want), reference))


def careless(n):
    return [None] * n


# Before anything else: a new program's control word.
case("fnstcw at the start: the default", b'', fnstcw(0), u16(0x37F))

# Loads and stores.
case("fld m64, fstp m64", d(3.25), fld64(0) + fstp64(0), d(3.25))
case("fld m32, fstp m64", f(1.5), fld32(0) + fstp64(0), d(1.5))
case("fld m64, fstp m32: rounded", d(0.1), fld64(0) + fstp32(0), f(0.1))
case("fld m80 (a third), fstp m64: rounded", ext(Fraction(1, 3)), fld80(0) + fstp64(0), d(1 / 3))
case("fld m64, fstp m80: exact", d(0.1), fld64(0) + fstp80(0), ext(0.1))
case("fstp m80 of a double's denormal", d(5e-324), fld64(0) + fstp80(0), ext(5e-324))
case("fld m80 infinity", ext(-math.inf), fld80(0) + fstp64(0), d(-math.inf))
case("fld m80 -0", ext(-0.0), fld80(0) + fstp64(0), d(-0.0))
case("fld m80 below a double's range", raw(1 << 63, 1), fld80(0) + fstp64(0), d(0.0))
case("fld m80 past a double's range", raw(1 << 63, 0x7FFE), fld80(0) + fstp64(0), d(math.inf))
case("fld m80 and fstp m80, through faddp", ext(1.5) + ext(2.5),
     fld80(0) + fld80(10) + reg(0xDE, 0, 1) + fstp80(0), ext(4.0))
case("fst m64 keeps the value", d(7.5), fld64(0) + fst64(0) + fstp64(8), d(7.5) + d(7.5))

# Integers in.
case("fild m16, m32, m64", i16(-300) + i16(0) + i32(-7) + i64(123456789012),
     mem(0xDF, 0, RDI, 0) + fstp64(0) + mem(0xDB, 0, RDI, 4) + fstp64(8) + mem(0xDF, 5, RDI, 8) + fstp64(16),
     d(-300) + d(-7) + d(123456789012))
case("fild m64 rounded to a double on the way out", i64(2**62 + 1), mem(0xDF, 5, RDI, 0) + fstp64(0), d(2.0**62))

# Arithmetic with memory.
for name, ext_, a, b, r in [("fadd", 0, 1.5, 2.25, 3.75), ("fmul", 1, 3, 0.125, 0.375), ("fsub", 4, 10, 0.5, 9.5),
                            ("fsubr", 5, 10, 2, -8), ("fdiv", 6, 1, 4, 0.25), ("fdivr", 7, 8, 1, 0.125)]:
    case(f"{name} m64", d(a) + d(b), fld64(0) + mem(0xDC, ext_, RDI, 8) + fstp64(0), d(r))
case("fadd m32", d(1.25) + f(0.5), fld64(0) + mem(0xD8, 0, RDI, 8) + fstp64(0), d(1.75))
case("fiadd m32, fimul m16, fisubr m32", d(1.5) + i32(3) + i16(-2) + i16(0) + i32(10),
     fld64(0) + mem(0xDA, 0, RDI, 8) + mem(0xDE, 1, RDI, 12) + mem(0xDA, 5, RDI, 16) + fstp64(0), d(10 - (1.5 + 3) * -2))

# Arithmetic on the stack: ST(1) = 10, ST(0) = 2 (or 8 and 2 for division). Intel's names throughout.
for name, code, out in [
    ("D8 E1 fsub st0, st1", reg(0xD8, 4, 1), [-8, 10]),
    ("D8 E9 fsubr st0, st1", reg(0xD8, 5, 1), [8, 10]),
    ("DC E1 fsubr st1, st0", reg(0xDC, 4, 1), [2, -8]),
    ("DC E9 fsub st1, st0", reg(0xDC, 5, 1), [2, 8]),
    ("DE E9 fsubp st1, st0", reg(0xDE, 5, 1), [8]),
    ("DE E1 fsubrp st1, st0", reg(0xDE, 4, 1), [-8]),
    ("DE C1 faddp", reg(0xDE, 0, 1), [12]),
    ("DE C9 fmulp", reg(0xDE, 1, 1), [20]),
    ("D8 C1 fadd st0, st1", reg(0xD8, 0, 1), [12, 10]),
    ("DC C9 fmul st1, st0", reg(0xDC, 1, 1), [2, 20]),
]:
    pops = b''.join(bytes(fstp64(8 * k)) for k in range(len(out)))
    case(name, d(10) + d(2), fld64(0) + fld64(8) + code + list(pops), b''.join(d(x) for x in out))
for name, code, out in [
    ("D8 F1 fdiv st0, st1", reg(0xD8, 6, 1), [0.25, 8]),
    ("D8 F9 fdivr st0, st1", reg(0xD8, 7, 1), [4, 8]),
    ("DC F1 fdivr st1, st0", reg(0xDC, 6, 1), [2, 0.25]),
    ("DC F9 fdiv st1, st0", reg(0xDC, 7, 1), [2, 4]),
    ("DE F9 fdivp st1, st0", reg(0xDE, 7, 1), [4]),
    ("DE F1 fdivrp st1, st0", reg(0xDE, 6, 1), [0.25]),
]:
    pops = b''.join(bytes(fstp64(8 * k)) for k in range(len(out)))
    case(name, d(8) + d(2), fld64(0) + fld64(8) + code + list(pops), b''.join(d(x) for x in out))

# The stack itself.
case("fxch", d(1) + d(2), fld64(0) + fld64(8) + reg(0xD9, 1, 1) + fstp64(0) + fstp64(8), d(1) + d(2))
case("fld st(1)", d(1) + d(2), fld64(0) + fld64(8) + reg(0xD9, 0, 1) + fstp64(0) + fstp64(8) + fstp64(16),
     d(1) + d(2) + d(1))
case("fstp st(1)", d(1) + d(2), fld64(0) + fld64(8) + reg(0xDD, 3, 1) + fstp64(0), d(2))
case("fst st(2)", d(1) + d(2) + d(3), fld64(0) + fld64(8) + fld64(16) + reg(0xDD, 2, 2)
     + fstp64(0) + fstp64(8) + fstp64(16), d(3) + d(2) + d(3))
case("a full stack of eight", b''.join(d(k) for k in range(1, 9)),
     sum((fld64(8 * k) for k in range(8)), []) + reg(0xDE, 0, 7) + reg(0xDE, 0, 6) + reg(0xDE, 0, 5)
     + reg(0xDE, 0, 4) + reg(0xDE, 0, 3) + reg(0xDE, 0, 2) + reg(0xDE, 0, 1) + fstp64(0), d(36))
case("ffreep st(0)", d(1) + d(2), fld64(0) + fld64(8) + [0xDF, 0xC0] + fstp64(0), d(1), reference=False)

# One-operand instructions and constants.
case("fchs, fabs", d(-3), fld64(0) + [0xD9, 0xE0] + fst64(0) + [0xD9, 0xE1] + fstp64(8), d(3) + d(3))
case("fchs of 0", d(0.0), fld64(0) + [0xD9, 0xE0] + fstp64(0), d(-0.0))
case("fsqrt", d(2.25), fld64(0) + [0xD9, 0xFA] + fstp64(0), d(1.5))
constants = [(0xE8, 1.0), (0xEE, 0.0), (0xEB, math.pi), (0xEA, 1 / math.log(2)), (0xE9, math.log2(10)),
             (0xEC, math.log10(2)), (0xED, math.log(2))]
case("fld1, fldz, fldpi, fldl2e, fldl2t, fldlg2, fldln2", b'',
     sum(([0xD9, op] + fstp64(8 * k) for k, (op, _) in enumerate(constants)), []),
     b''.join(d(v) for _, v in constants))
case("fscale", d(4) + d(3), fld64(0) + fld64(8) + [0xD9, 0xFD] + fstp64(0) + fstp64(8), d(48) + d(4))
case("fscale by -2.7: toward zero", d(-2.7) + d(1.5), fld64(0) + fld64(8) + [0xD9, 0xFD] + fstp64(0) + POP, d(0.375))
case("fscale to 2^-1074", d(-1074) + d(1), fld64(0) + fld64(8) + [0xD9, 0xFD] + fstp64(0) + POP, d(5e-324))

# Integers out, with the control word's rounding.
for rc, name, want in [(0, "nearest", (2, -2, 4)), (1, "down", (2, -3, 3)), (2, "up", (3, -2, 4)),
                       (3, "toward zero", (2, -2, 3))]:
    cw = 0x37F | rc << 10
    case(f"fistp m32 rounding {name}", d(2.5) + d(-2.5) + d(3.5) + u16(cw) + u16(0x37F),
         fldcw(24) + fld64(0) + mem(0xDB, 3, RSI, 0) + fld64(8) + mem(0xDB, 3, RSI, 4) + fld64(16)
         + mem(0xDB, 3, RSI, 8) + fldcw(26),
         i32(want[0]) + i32(want[1]) + i32(want[2]))
case("frndint rounding up", d(2.1) + u16(0xB7F) + u16(0x37F),
     fldcw(8) + fld64(0) + [0xD9, 0xFC] + fldcw(10) + fstp64(0), d(3))
case("frndint to nearest even", d(2.5), fld64(0) + [0xD9, 0xFC] + fstp64(0), d(2))
case("fisttp m32, m64, m16: toward zero", d(-2.7) + d(-2.7) + d(2.7),
     fld64(0) + mem(0xDB, 1, RSI, 0) + fld64(8) + mem(0xDD, 1, RSI, 4) + fld64(16) + mem(0xDF, 1, RSI, 12),
     i32(-2) + i64(-2) + i16(2))
case("fist m16 keeps it, fistp m64", d(7), fld64(0) + mem(0xDF, 2, RSI, 0) + mem(0xDF, 7, RSI, 2), i16(7) + i64(7))
case("fistp out of range: indefinite", d(3e9) + d(40000) + d(math.nan) + d(-1e19),
     fld64(0) + mem(0xDB, 3, RSI, 0) + fld64(8) + mem(0xDF, 3, RSI, 4) + fld64(16) + mem(0xDF, 7, RSI, 6)
     + fld64(24) + mem(0xDF, 7, RSI, 14),
     i32(-2**31) + i16(-2**15) + i64(-2**63) + i64(-2**63))
case("fistp m64 of -123456789012", d(-123456789012), fld64(0) + mem(0xDF, 7, RSI, 0), i64(-123456789012))

# Compares into C0, C2, C3, through fnstsw ax.
C0, C1, C2, C3 = 0x100, 0x200, 0x400, 0x4000
for name, a, b, want in [("greater", 3, 2, 0), ("less", 1, 2, C0), ("equal", 2, 2, C3),
                         ("unordered", math.nan, 2, C0 | C2 | C3)]:
    case(f"fcom st1, {name}", d(b) + d(a), fld64(0) + fld64(8) + reg(0xD8, 2, 1) + status(0) + POP + POP, u16(want))
case("fcomp m64, less", d(1) + d(2), fld64(0) + mem(0xDC, 3, RDI, 8) + status(0) + fld64(8) + fstp64(2),
     u16(C0) + d(2))
case("fucompp, equal: both popped", d(5) + d(5) + d(9),
     fld64(16) + fld64(0) + fld64(8) + [0xDA, 0xE9] + status(0) + fstp64(2), u16(C3) + d(9))
case("fcompp, greater", d(1) + d(2), fld64(0) + fld64(8) + [0xDE, 0xD9] + status(0), u16(0))
case("ftst of -1", d(-1), fld64(0) + [0xD9, 0xE4] + status(0) + POP, u16(C0))
case("ficom m32, equal", d(6) + i32(6), fld64(0) + mem(0xDA, 2, RDI, 8) + status(0) + POP, u16(C3))
case("fucom st1, then fucomp", d(2) + d(1),
     fld64(0) + fld64(8) + reg(0xDD, 4, 1) + status(0) + reg(0xDD, 5, 1) + status(2) + fstp64(4),
     u16(C0) + u16(C0) + d(2))

# Compares into ZF, PF, CF.
for name, a, b, want in [("greater", 3, 2, [0, 0, 0]), ("less", 1, 2, [0, 1, 0]), ("equal", 2, 2, [1, 0, 0]),
                         ("unordered", math.nan, 2, [1, 1, 1])]:
    case(f"fcomi st1, {name}", d(b) + d(a),
         fld64(0) + fld64(8) + reg(0xDB, 6, 1) + setcc(4, 0) + setcc(2, 1) + setcc(10, 2) + POP + POP, want)
case("fucomip, less: popped", d(2) + d(1),
     fld64(0) + fld64(8) + reg(0xDF, 5, 1) + setcc(2, 0) + fstp64(1), [1] + list(d(2)))

# fxam: C3, C2, C0 for the class, C1 for the sign.
for name, x, want in [("zero", 0.0, C3), ("-1.5", -1.5, C2 | C1), ("infinity", math.inf, C2 | C0),
                      ("NaN", math.nan, C0), ("-0", -0.0, C3 | C1)]:
    case(f"fxam {name}", d(x), fld64(0) + [0xD9, 0xE5] + status(0) + POP, u16(want))

# fprem, and its quotient's low bits: Q2 in C0, Q1 in C3, Q0 in C1.
case("fprem 5.5 by 2: q = 2", d(2) + d(5.5), fld64(0) + fld64(8) + [0xD9, 0xF8] + status(0) + fstp64(2) + POP,
     u16(C3) + d(1.5))
case("fprem -7 by 2: q = 3", d(2) + d(-7), fld64(0) + fld64(8) + [0xD9, 0xF8] + status(0) + fstp64(2) + POP,
     u16(C3 | C1) + d(-1))
case("fprem 13 by -4: q = 3", d(-4) + d(13), fld64(0) + fld64(8) + [0xD9, 0xF8] + status(0) + fstp64(2) + POP,
     u16(C3 | C1) + d(1))
case("fprem 1 by infinity", d(math.inf) + d(1), fld64(0) + fld64(8) + [0xD9, 0xF8] + fstp64(0) + POP, d(1))
# fmod as C libraries write it: fprem until C2 is clear.
loop = [0xD9, 0xF8] + FNSTSW_AX + [0xF6, 0xC4, 0x04, 0x75, (-(2 + 2 + 3 + 2)) & 0xFF]
for x, y in [(1e300, 3.0), (-1e300, 7.0), (123456.75, 0.1), (2.0**1000, 3.0**20)]:
    case(f"fmod({x!r}, {y!r}) by an fprem loop", d(y) + d(x), fld64(0) + fld64(8) + loop + fstp64(0) + POP,
         d(math.fmod(x, y)))

# fcmov, after an integer compare: mov eax, n; cmp eax, 1.
def compare(n): return [0xB8] + list(i32(n)) + [0x83, 0xF8, 0x01]
for name, op, ext_, n, taken in [("fcmovb, below", 0xDA, 0, 0, True), ("fcmovb, not below", 0xDA, 0, 2, False),
                                 ("fcmove, equal", 0xDA, 1, 1, True), ("fcmovbe, above", 0xDA, 2, 2, False),
                                 ("fcmovnbe, above", 0xDB, 2, 2, True), ("fcmovne, equal", 0xDB, 1, 1, False),
                                 ("fcmovnb, below", 0xDB, 0, 0, False)]:
    case(name, d(1) + d(2), fld64(0) + fld64(8) + compare(n) + reg(op, ext_, 1) + fstp64(0) + POP,
         d(1 if taken else 2))
case("fcmovu after an unordered fucomi", d(math.nan) + d(5) + d(1),
     fld64(0) + fld64(8) + reg(0xDB, 5, 1) + POP + fld64(16) + reg(0xDA, 3, 1) + fstp64(0) + POP, d(math.nan))

# The control and status words, and the environment.
case("fldcw, fnstcw", u16(0xF7F) + u16(0x37F), fldcw(0) + fnstcw(0) + fldcw(2) + fnstcw(2), u16(0xF7F) + u16(0x37F))
# Intel's manual says fnstenv masks every exception after storing (glibc's feholdexcept counts on it); Rosetta 2's
# doesn't, so the control word just after isn't compared. exec's translator masks them.
case("fnstenv, fldenv", u16(0xE40) + u16(0x37F),
     [0xDB, 0xE2] + fldcw(0) + mem(0xD9, 6, RSI, 0) + fnstcw(28) + mem(0xD9, 4, RSI, 0) + fnstcw(30) + fldcw(2),
     list(u16(0xE40)) + careless(6) + list(u16(0xFFFF)) + careless(18) + careless(2) + list(u16(0xE40)))
case("fninit", u16(0xF7F), fldcw(0) + [0xDB, 0xE3] + fnstcw(0), u16(0x37F))
# fnstsw m16, then only C0 to C3 kept: mov ax, [rsi]; and ax, 0x4700; mov [rsi], ax.
case("fnstsw m16 after fcom, less", d(2) + d(1), fld64(0) + fld64(8) + reg(0xD8, 2, 1) + mem(0xDD, 7, RSI, 0)
     + [0x66, 0x8B, 0x06, 0x66, 0x25, 0x00, 0x47, 0x66, 0x89, 0x06] + POP + POP, u16(C0))
# sahf and lahf: mov ah, n; sahf; lahf; mov [rsi], ah, and the flags sahf set.
case("sahf, lahf: SF ZF PF CF", b'', [0xB4, 0xC5, 0x9E, 0xB4, 0x00, 0x9F, 0x88, 0x66, 0x00] + setcc(2, 1)
     + setcc(4, 2) + setcc(10, 3) + setcc(8, 4), [0xC7, 1, 1, 1, 1])
case("sahf, lahf: none", b'', [0xB4, 0x00, 0x9E, 0xB4, 0xFF, 0x9F, 0x88, 0x66, 0x00] + setcc(2, 1) + setcc(4, 2),
     [0x02, 0, 0])
case("fwait", d(1), [0x9B] + fld64(0) + [0x9B] + fstp64(0), d(1))


def c_bytes(bs):
    return ', '.join(f'0x{b:02x}' for b in bs)


out = []
w = out.append
w("// x87, each instruction run on known inputs and checked byte for byte, on lambda for x86-64. Written by x87.py,")
w("// whose answers are what an x87 stores; the kernels README says how to build it.")
w("")
w("typedef unsigned char u8;")
w("typedef long long i64;")
w("#ifdef __APPLE__")
w("/* Also an x86-64 macOS program, to check the answers on a real x87 (Rosetta 2's). */")
w("#include <stdlib.h>")
w("#include <unistd.h>")
w("static void say(const char *s) { int n = 0; while (s[n]) n++; write(1, s, n); }")
w("static void finish(int failures) { exit(failures); }")
w("#else")
w("static i64 sys(i64 n, i64 a, i64 b, i64 c) {")
w('    i64 ret;')
w('    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");')
w("    return ret;")
w("}")
w("static void say(const char *s) { int n = 0; while (s[n]) n++; sys(1, 1, (i64)s, n); }")
w("static void finish(int failures) { sys(231, failures, 0, 0); }")
w("#endif")
w("static int failures;")
w("static void check(int ok, const char *what) {")
w('    say(ok ? "ok      " : "FAILED  ");')
w("    say(what);")
w('    say("\\n");')
w("    if (!ok) failures++;")
w("}")
w("/* Bytes `care` marks are compared; the rest of the 64 must still be 0xaa, as they were before. */")
w("static int same(const u8 *out, const u8 *want, const u8 *care, int n) {")
w("    for (int i = 0; i < 64; i++) {")
w("        if (i >= n) { if (out[i] != 0xaa) return 0; }")
w("        else if (care[i] && out[i] != want[i]) return 0;")
w("    }")
w("    return 1;")
w("}")
w("")
for k, (name, inputs, code, want, reference) in enumerate(cases):
    w(f"static void case{k}(u8 *out, const u8 *in) {{")
    w(f'    __asm__ volatile("mov %0, %%rsi\\n mov %1, %%rdi\\n .byte {c_bytes(code)}"')
    w('                     :: "r"(out), "r"(in) : "rax", "rsi", "rdi", "memory", "cc", "st", "st(1)", "st(2)",')
    w('                       "st(3)", "st(4)", "st(5)", "st(6)", "st(7)");')
    w("}")
w("")
w("#ifdef __APPLE__")
w("void cmain(void);")
w("int main(void) { cmain(); return 0; }")
w("#else")
w('__asm__(".globl _start\\n_start:\\n xor %rbp, %rbp\\n and $-16, %rsp\\n call cmain\\n hlt\\n");')
w("#endif")
w("void cmain(void) {")
w("    static u8 in[64], out[64];")
for k, (name, inputs, code, want, reference) in enumerate(cases):
    if not reference:
        w("#ifndef __APPLE__")
    data = list(inputs) + [0] * (64 - len(inputs))
    wantb = [0 if b is None else b for b in want]
    care = [0 if b is None else 1 for b in want]
    w(f"    {{ static const u8 x[64] = {{{c_bytes(data)}}};")
    w(f"      static const u8 want[{len(want)}] = {{{c_bytes(wantb)}}}, care[{len(want)}] = {{{c_bytes(care)}}};")
    w("      for (int i = 0; i < 64; i++) { in[i] = x[i]; out[i] = 0xaa; }")
    w(f'      case{k}(out, in); check(same(out, want, care, {len(want)}), "{name}"); }}')
    if not reference:
        w("#endif")
w("    finish(failures);")
w("}")
print('\n'.join(out))
