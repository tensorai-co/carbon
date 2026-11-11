#!/usr/bin/env python3
"""Writes sse.c: a test of SSE and SSE2 instructions, each run on known inputs and checked byte for byte against
what this file works out x86 does. The answers come from here, not from any translator, and the program passes on
real x86 hardware too, which checks this file's model of x86.

    python3 sse.py > sse.c
"""

import math
import struct

def u8s(b): return list(b)
def pack(fmt, xs): return bytes(struct.pack('<' + fmt * len(xs), *xs))
def unpack(fmt, b): return list(struct.unpack('<' + fmt * (len(b) // struct.calcsize(fmt)), b))

A = bytes(range(0x10, 0x20))
B = bytes([0x80, 0x7f, 0xff, 0x01, 0x40, 0xc0, 0x00, 0xfe, 0x33, 0xcc, 0x7f, 0x80, 0x01, 0x02, 0x03, 0x04])
W1 = pack('h', [1000, -2000, 32767, -32768, 7, -7, 30000, -30000])
W2 = pack('h', [3000, 500, 1, -1, -7, 7, 10000, -10000])
D1 = pack('i', [100000, -5, 0x7fffffff, -0x80000000])
D2 = pack('i', [7, -5, 1, 1])

def lanes(fmt, a, b, f):
    return pack(fmt, [f(x, y) for x, y in zip(unpack(fmt, a), unpack(fmt, b))])

def wrap(bits, signed):
    def w(v):
        v &= (1 << bits) - 1
        return v - (1 << bits) if signed and v >> (bits - 1) else v
    return w

def sat(lo, hi): return lambda v: max(lo, min(hi, v))

cases = []

def case(name, asm, a, b, want, out='xmm1'):
    cases.append((name, asm, a, b, want, out))

# Packed integer arithmetic.
for name, fmt, f in [
    ('paddb', 'B', lambda x, y: (x + y) & 0xff), ('paddw', 'H', lambda x, y: (x + y) & 0xffff),
    ('paddd', 'I', lambda x, y: (x + y) & 0xffffffff), ('paddq', 'Q', lambda x, y: (x + y) & (2**64 - 1)),
    ('psubb', 'B', lambda x, y: (x - y) & 0xff), ('psubw', 'H', lambda x, y: (x - y) & 0xffff),
    ('psubd', 'I', lambda x, y: (x - y) & 0xffffffff), ('psubq', 'Q', lambda x, y: (x - y) & (2**64 - 1)),
    ('paddsb', 'b', lambda x, y: sat(-128, 127)(x + y)), ('paddsw', 'h', lambda x, y: sat(-32768, 32767)(x + y)),
    ('paddusb', 'B', lambda x, y: min(255, x + y)), ('paddusw', 'H', lambda x, y: min(65535, x + y)),
    ('psubsb', 'b', lambda x, y: sat(-128, 127)(x - y)), ('psubsw', 'h', lambda x, y: sat(-32768, 32767)(x - y)),
    ('psubusb', 'B', lambda x, y: max(0, x - y)), ('psubusw', 'H', lambda x, y: max(0, x - y)),
    ('pminub', 'B', min), ('pmaxub', 'B', max), ('pminsw', 'h', min), ('pmaxsw', 'h', max),
    ('pavgb', 'B', lambda x, y: (x + y + 1) >> 1), ('pavgw', 'H', lambda x, y: (x + y + 1) >> 1),
    ('pmullw', 'h', lambda x, y: wrap(16, True)(x * y)),
    ('pcmpeqb', 'B', lambda x, y: 0xff if x == y else 0), ('pcmpeqw', 'H', lambda x, y: 0xffff if x == y else 0),
    ('pcmpeqd', 'I', lambda x, y: 0xffffffff if x == y else 0),
    ('pcmpgtb', 'b', lambda x, y: -1 if x > y else 0), ('pcmpgtw', 'h', lambda x, y: -1 if x > y else 0),
    ('pcmpgtd', 'i', lambda x, y: -1 if x > y else 0),
]:
    for (a, b) in [(A, B), (W1, W2), (D1, D2)]:
        if (fmt in 'Bb' and a is A) or (fmt in 'Hh' and a is W1) or (fmt in 'IiQ' and a is D1):
            case(name, f'{name} %%xmm2, %%xmm1', a, b, lanes(fmt, a, b, f))
case('paddb, memory', 'paddb (%2), %%xmm1', A, B, lanes('B', A, B, lambda x, y: (x + y) & 0xff))

# Logic.
case('pand', 'pand %%xmm2, %%xmm1', A, B, bytes(x & y for x, y in zip(A, B)))
case('pandn', 'pandn %%xmm2, %%xmm1', A, B, bytes(~x & y & 0xff for x, y in zip(A, B)))
case('por', 'por %%xmm2, %%xmm1', A, B, bytes(x | y for x, y in zip(A, B)))
case('pxor', 'pxor %%xmm2, %%xmm1', A, B, bytes(x ^ y for x, y in zip(A, B)))
case('andnps', 'andnps %%xmm2, %%xmm1', A, B, bytes(~x & y & 0xff for x, y in zip(A, B)))

# Unpacks and packs.
def interleave(a, b, size, high):
    n = 16 // size
    ea = [a[i * size:(i + 1) * size] for i in range(n)]
    eb = [b[i * size:(i + 1) * size] for i in range(n)]
    half = range(n // 2, n) if high else range(n // 2)
    return b''.join(ea[i] + eb[i] for i in half)
for name, size, high in [('punpcklbw', 1, 0), ('punpcklwd', 2, 0), ('punpckldq', 4, 0), ('punpcklqdq', 8, 0),
                         ('punpckhbw', 1, 1), ('punpckhwd', 2, 1), ('punpckhdq', 4, 1), ('punpckhqdq', 8, 1),
                         ('unpcklps', 4, 0), ('unpckhps', 4, 1), ('unpcklpd', 8, 0), ('unpckhpd', 8, 1)]:
    case(name, f'{name} %%xmm2, %%xmm1', A, B, interleave(A, B, size, high))
case('packsswb', 'packsswb %%xmm2, %%xmm1', W1, W2, pack('b', [sat(-128, 127)(v) for v in unpack('h', W1) + unpack('h', W2)]))
case('packuswb', 'packuswb %%xmm2, %%xmm1', W1, W2, pack('B', [sat(0, 255)(v) for v in unpack('h', W1) + unpack('h', W2)]))
case('packssdw', 'packssdw %%xmm2, %%xmm1', D1, D2, pack('h', [sat(-32768, 32767)(v) for v in unpack('i', D1) + unpack('i', D2)]))

# Shuffles.
def pick(elems, imm, n, count):
    return [elems[(imm >> (2 * i)) & (n - 1)] for i in range(count)]
d = unpack('I', A); case('pshufd $0x1b', 'pshufd $0x1b, %%xmm2, %%xmm1', B, A, pack('I', pick(d, 0x1b, 4, 4)))
w = unpack('H', A)
case('pshuflw $0x1b', 'pshuflw $0x1b, %%xmm2, %%xmm1', B, A, pack('H', pick(w[:4], 0x1b, 4, 4) + w[4:]))
case('pshufhw $0x1b', 'pshufhw $0x1b, %%xmm2, %%xmm1', B, A, pack('H', w[:4] + pick(w[4:], 0x1b, 4, 4)))
s1, s2 = unpack('I', A), unpack('I', B)
for imm in (0x4e, 0xb1):
    case(f'shufps ${imm:#x}', f'shufps ${imm:#x}, %%xmm2, %%xmm1', A, B,
         pack('I', [s1[imm & 3], s1[(imm >> 2) & 3], s2[(imm >> 4) & 3], s2[(imm >> 6) & 3]]))
q1, q2 = unpack('Q', A), unpack('Q', B)
for imm in (1, 2):
    case(f'shufpd ${imm}', f'shufpd ${imm}, %%xmm2, %%xmm1', A, B, pack('Q', [q1[imm & 1], q2[(imm >> 1) & 1]]))

# Shifts by an immediate.
def shifted(fmt, bits, a, f): return pack(fmt, [f(v) & ((1 << bits) - 1) for v in unpack(fmt, a)])
for name, fmt, bits, n, f in [
    ('psllw', 'H', 16, 3, lambda v: v << 3), ('psrlw', 'H', 16, 3, lambda v: v >> 3), ('pslld', 'I', 32, 5, lambda v: v << 5),
    ('psrld', 'I', 32, 5, lambda v: v >> 5), ('psllq', 'Q', 64, 7, lambda v: v << 7), ('psrlq', 'Q', 64, 7, lambda v: v >> 7),
    ('psllq', 'Q', 64, 64, lambda v: 0),
]:
    case(f'{name} ${n}', f'{name} ${n}, %%xmm1', W1, W1, shifted(fmt, bits, W1, f))
case('psraw $3', 'psraw $3, %%xmm1', W1, W1, pack('h', [v >> 3 for v in unpack('h', W1)]))
case('psrad $5', 'psrad $5, %%xmm1', D1, D1, pack('i', [v >> 5 for v in unpack('i', D1)]))
case('psrad $40', 'psrad $40, %%xmm1', D1, D1, pack('i', [v >> 31 for v in unpack('i', D1)]))
# Shifts by a count in a register: its low 64 bits, any count past the width emptying (or sign-filling) each element.
def count(n): return pack('Q', [n, 0x55])
for name, fmt, bits, signed in [('psllw', 'H', 16, False), ('psrlw', 'H', 16, False), ('psraw', 'h', 16, True),
                                ('pslld', 'I', 32, False), ('psrld', 'I', 32, False), ('psrad', 'i', 32, True),
                                ('psllq', 'Q', 64, False), ('psrlq', 'Q', 64, False)]:
    a = W1 if bits == 16 else D1
    for n in [0, 3, bits - 1, bits, 300, 2**40]:
        left = name.startswith('psll')
        def f(v, n=n, left=left):
            if left:
                return (v << n) if n < bits else 0
            if signed:
                return v >> min(n, bits - 1)
            return (v >> n) if n < bits else 0
        want = pack(fmt, [wrap(bits, signed)(f(v)) for v in unpack(fmt, a)])
        case(f'{name} by {n}', f'{name} %%xmm2, %%xmm1', a, count(n), want)

# The multiplies that widen: high halves, pairs added, the even doublewords' full products; and psadbw's sums.
case('pmulhw', 'pmulhw %%xmm2, %%xmm1', W1, W2, pack('h', [(x * y) >> 16 for x, y in zip(unpack('h', W1), unpack('h', W2))]))
case('pmulhuw', 'pmulhuw %%xmm2, %%xmm1', W1, W2, pack('H', [(x * y) >> 16 for x, y in zip(unpack('H', W1), unpack('H', W2))]))
P = pack('h', [-32768] * 8)
for name, a, b in [('pmaddwd', W1, W2), ('pmaddwd, both -32768', P, P)]:
    p = [x * y for x, y in zip(unpack('h', a), unpack('h', b))]
    case(name, 'pmaddwd %%xmm2, %%xmm1', a, b, pack('i', [wrap(32, True)(p[i] + p[i + 1]) for i in range(0, 8, 2)]))
dw1, dw2 = unpack('I', D1), unpack('I', D2)
case('pmuludq', 'pmuludq %%xmm2, %%xmm1', D1, D2, pack('Q', [dw1[0] * dw2[0], dw1[2] * dw2[2]]))
case('psadbw', 'psadbw %%xmm2, %%xmm1', A, B, pack('Q', [sum(abs(x - y) for x, y in zip(A[h:h + 8], B[h:h + 8])) for h in (0, 8)]))
case('pslldq $3', 'pslldq $3, %%xmm1', A, A, bytes(3) + A[:13])
case('psrldq $3', 'psrldq $3, %%xmm1', A, A, A[3:] + bytes(3))

# Half moves.
case('movhlps', 'movhlps %%xmm2, %%xmm1', A, B, B[8:] + A[8:])
case('movlhps', 'movlhps %%xmm2, %%xmm1', A, B, A[:8] + B[:8])
case('movhps, load', 'movhps (%2), %%xmm1', A, B, A[:8] + B[:8])
case('movlps, load', 'movlps (%2), %%xmm1', A, B, B[:8] + A[8:])
case('pinsrw $3', 'movl $0xbeef, %%eax\\n pinsrw $3, %%eax, %%xmm1', A, B, A[:6] + b'\xef\xbe' + A[8:])

# Floating point.
F1 = pack('f', [1.5, -2.0, 100.0, 0.25]); F2 = pack('f', [0.5, 4.0, -3.0, 0.25])
G1 = pack('d', [6.0, -1.5]); G2 = pack('d', [2.0, 0.5])
def single(x): return struct.unpack('<f', struct.pack('<f', x))[0]
for name, op in [('add', lambda x, y: x + y), ('sub', lambda x, y: x - y), ('mul', lambda x, y: x * y), ('div', lambda x, y: x / y)]:
    case(f'{name}ps', f'{name}ps %%xmm2, %%xmm1', F1, F2, pack('f', [single(op(x, y)) for x, y in zip(unpack('f', F1), unpack('f', F2))]))
    case(f'{name}pd', f'{name}pd %%xmm2, %%xmm1', G1, G2, pack('d', [op(x, y) for x, y in zip(unpack('d', G1), unpack('d', G2))]))
    # Scalar: only lane 0; the rest of the destination stays.
    case(f'{name}ss', f'{name}ss %%xmm2, %%xmm1', F1, F2, pack('f', [single(op(unpack('f', F1)[0], unpack('f', F2)[0]))]) + F1[4:])
    case(f'{name}sd', f'{name}sd %%xmm2, %%xmm1', G1, G2, pack('d', [op(6.0, 2.0)]) + G1[8:])
case('addsd, memory', 'addsd (%2), %%xmm1', G1, G2, pack('d', [8.0]) + G1[8:])
case('sqrtps', 'sqrtps %%xmm2, %%xmm1', F1, pack('f', [4.0, 9.0, 0.25, 2.25]), pack('f', [2.0, 3.0, 0.5, 1.5]))
case('sqrtsd', 'sqrtsd %%xmm2, %%xmm1', G1, pack('d', [16.0, 4.0]), pack('d', [4.0]) + G1[8:])
NAN = struct.unpack('<f', b'\x00\x00\xc0\x7f')[0]
M1 = pack('f', [1.0, NAN, 0.0, -0.0]); M2 = pack('f', [2.0, 5.0, -0.0, 0.0])
# x86's min and max: the first if it's strictly less (greater), else the second: a NaN or two zeros give the second.
case('minps', 'minps %%xmm2, %%xmm1', M1, M2, pack('f', [1.0, 5.0]) + M2[8:])
case('maxps', 'maxps %%xmm2, %%xmm1', M1, M2, pack('f', [2.0, 5.0]) + M2[8:])
case('minsd', 'minsd %%xmm2, %%xmm1', G1, G2, pack('d', [2.0]) + G1[8:])
case('maxsd', 'maxsd %%xmm2, %%xmm1', G1, G2, pack('d', [6.0]) + G1[8:])
case('cvtss2sd', 'cvtss2sd %%xmm2, %%xmm1', G1, F1, pack('d', [1.5]) + G1[8:])
case('cvtsd2ss', 'cvtsd2ss %%xmm2, %%xmm1', F1, G2, pack('f', [2.0]) + F1[4:])
case('cvtps2pd', 'cvtps2pd %%xmm2, %%xmm1', A, F1, pack('d', [1.5, -2.0]))
case('cvtpd2ps', 'cvtpd2ps %%xmm2, %%xmm1', A, G1, pack('f', [6.0, -1.5]) + bytes(8))
case('cvtdq2ps', 'cvtdq2ps %%xmm2, %%xmm1', A, pack('i', [1, -2, 3, 100]), pack('f', [1.0, -2.0, 3.0, 100.0]))
case('cvttps2dq', 'cvttps2dq %%xmm2, %%xmm1', A, F1, pack('i', [1, -2, 100, 0]))
case('cvtsi2sd, 32-bit', 'movl $-7, %%eax\\n cvtsi2sd %%eax, %%xmm1', G1, G2, pack('d', [-7.0]) + G1[8:])
case('cvtsi2sd, 64-bit', 'movq $-7, %%rax\\n cvtsi2sdq %%rax, %%xmm1', G1, G2, pack('d', [-7.0]) + G1[8:])
case('cvtsi2ss', 'movl $9, %%eax\\n cvtsi2ss %%eax, %%xmm1', F1, F2, pack('f', [9.0]) + F1[4:])

# Compares into masks: eq, lt, le, unord, neq, nlt, nle, ord, with a NaN in lane 1.
C1 = pack('f', [1.0, NAN, 3.0, 4.0]); C2 = pack('f', [1.0, 2.0, 2.0, 5.0])
x, y = unpack('f', C1), unpack('f', C2)
preds = [lambda a, b: a == b, lambda a, b: a < b, lambda a, b: a <= b, lambda a, b: math.isnan(a) or math.isnan(b),
         lambda a, b: not a == b, lambda a, b: not a < b, lambda a, b: not a <= b, lambda a, b: not (math.isnan(a) or math.isnan(b))]
for p, f in enumerate(preds):
    case(f'cmpps ${p}', f'cmpps ${p}, %%xmm2, %%xmm1', C1, C2, pack('I', [0xffffffff if f(a, b) else 0 for a, b in zip(x, y)]))
case('cmpsd $1', 'cmpsd $1, %%xmm2, %%xmm1', G2, G1, pack('Q', [2**64 - 1]) + G2[8:])

# Results in general registers: the value in rax, as 16 bytes.
def r64(v): return pack('Q', [v & (2**64 - 1), 0])
case('pmovmskb', 'pmovmskb %%xmm1, %%eax\\n movq %%rax, %%xmm1', B, B, r64(sum(1 << i for i, v in enumerate(B) if v & 0x80)))
case('movmskps', 'movmskps %%xmm1, %%eax\\n movq %%rax, %%xmm1', F1, F1, r64(0b0010))
case('movmskpd', 'movmskpd %%xmm1, %%eax\\n movq %%rax, %%xmm1', G1, G1, r64(0b10))
case('pextrw $5', 'pextrw $5, %%xmm1, %%eax\\n movq %%rax, %%xmm1', A, A, r64(unpack('H', A)[5]))
case('cvttsd2si, 64-bit', 'cvttsd2si %%xmm1, %%rax\\n movq %%rax, %%xmm1', pack('d', [-2.7, 0]), A, r64(-2))
case('cvtsd2si, nearest-even', 'cvtsd2si %%xmm1, %%rax\\n movq %%rax, %%xmm1', pack('d', [2.5, 0]), A, r64(2))
case('cvttsd2si, a NaN: indefinite', 'cvttsd2si %%xmm1, %%rax\\n movq %%rax, %%xmm1', pack('d', [float('nan'), 0]), A, r64(1 << 63))
case('cvttsd2si, too big: indefinite', 'cvttsd2si %%xmm1, %%rax\\n movq %%rax, %%xmm1', pack('d', [1e30, 0]), A, r64(1 << 63))
case('cvttsd2si, 32-bit too big: indefinite', 'cvttsd2si %%xmm1, %%eax\\n movq %%rax, %%xmm1', pack('d', [3e9, 0]), A, r64(0x80000000))
case('cvttsd2si, 32-bit 2^31 - 1', 'cvttsd2si %%xmm1, %%eax\\n movq %%rax, %%xmm1', pack('d', [2147483647.0, 0]), A, r64(0x7fffffff))
case('cvttss2si, 32-bit', 'cvttss2si %%xmm1, %%eax\\n movq %%rax, %%xmm1', F2, A, r64(0))

# Flags from ucomisd: ZF, PF and CF, as setz, setp and setc into al, ah and bl.
def flags_case(name, a, b, zf, pf, cf):
    asm = ('xorl %%eax, %%eax\\n xorl %%ebx, %%ebx\\n ucomisd %%xmm2, %%xmm1\\n setz %%al\\n setp %%ah\\n setc %%bl\\n'
           ' shll $16, %%ebx\\n orl %%ebx, %%eax\\n movq %%rax, %%xmm1')
    case(name, asm, pack('d', [a, 0]), pack('d', [b, 0]), r64(zf | pf << 8 | cf << 16))
flags_case('ucomisd, greater', 2.0, 1.0, 0, 0, 0)
flags_case('ucomisd, less', 1.0, 2.0, 0, 0, 1)
flags_case('ucomisd, equal', 2.0, 2.0, 1, 0, 0)
flags_case('ucomisd, unordered', float('nan'), 2.0, 1, 1, 1)

# MXCSR: its default, and rounding toward zero through it.
# Without its exception flags (bits 0 to 5), which earlier cases' inexact results set on real hardware.
case('stmxcsr', 'stmxcsr (%0)\\n andl $-64, (%0)\\n movd (%0), %%xmm1', A, A, pack('I', [0x1f80, 0, 0, 0]))
case('ldmxcsr: round toward zero', 'movl $0x7f80, (%0)\\n ldmxcsr (%0)\\n cvtsd2si %%xmm1, %%rax\\n movl $0x1f80, (%0)\\n'
     ' ldmxcsr (%0)\\n movq %%rax, %%xmm1', pack('d', [2.7, 0]), A, r64(2))

def c_bytes(b): return '{' + ', '.join(f'0x{x:02x}' for x in b) + '}'

print('''// SSE and SSE2, each instruction run on known inputs and checked byte for byte, on lambda for x86-64. Written by
// sse.py, whose model of x86 gives the answers; tests/programs/README or the kernels README says how to build it.

typedef unsigned char u8;
typedef long long i64;
static i64 sys(i64 n, i64 a, i64 b, i64 c) {
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return ret;
}
static void say(const char *s) { int n = 0; while (s[n]) n++; sys(1, 1, (i64)s, n); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\\n");
    if (!ok) failures++;
}
static int same(const u8 *a, const u8 *b) { for (int i = 0; i < 16; i++) if (a[i] != b[i]) return 0; return 1; }
''')
for i, (name, asm, a, b, want, out) in enumerate(cases):
    print(f'static void case{i}(u8 *out, const u8 *a, const u8 *b) {{')
    print(f'    __asm__ volatile("movdqu (%1), %%xmm1\\n movdqu (%2), %%xmm2\\n {asm}\\n movdqu %%xmm1, (%0)"')
    print('                     :: "r"(out), "r"(a), "r"(b) : "rax", "rbx", "xmm1", "xmm2", "memory", "cc");')
    print('}')
print('''
__asm__(".globl _start\\n_start:\\n xor %rbp, %rbp\\n and $-16, %rsp\\n call cmain\\n hlt\\n");
void cmain(void) {
    say("SSE and SSE2 on lambda, x86-64\\n");
    __attribute__((aligned(16))) u8 a[16], b[16], out[16];''')
for i, (name, asm, a, b, want, out) in enumerate(cases):
    print(f'    {{ static const u8 x[16] = {c_bytes(a)}, y[16] = {c_bytes(b)}, want[16] = {c_bytes(want)};')
    print(f'      for (int i = 0; i < 16; i++) {{ a[i] = x[i]; b[i] = y[i]; out[i] = 0; }}')
    print(f'      case{i}(out, a, b); check(same(out, want), "{name}"); }}')
print('    sys(231, failures, 0, 0);\n}')
