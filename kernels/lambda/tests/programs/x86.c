// General-purpose x86-64 instructions the translator learned from real programs: 8- and 16-bit rotates, multiplies
// and divides, locked bit tests and bit tests on memory, fxsave/fxrstor, and the string instructions. Each runs in inline assembly on values
// the compiler can't see, and is checked against what the compiler works out from the same constants, so it passes
// on real x86 hardware too. x86-64 only. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned char u8;

#if !defined(__x86_64__)
#error "x86.c is for x86-64"
#endif

static i64 sys(i64 n, i64 a, i64 b, i64 c) {
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(1, 1, (i64)s, len(s)); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}

// Values the compiler can't fold.
static volatile u64 v_in[8] = {0x1234, 0xa5, 0x8001, 0xfedc, 0x7f, 0x81, 0xff, 0x3};
static u64 in(int i) { return v_in[i]; }

// Rotates: the result and CF.
#define ROT(width, op, c_imm, value, expect, expect_cf, name)                                                         \
    {                                                                                                                  \
        u64 v = (value); u8 cf;                                                                                        \
        __asm__ volatile(op " $" #c_imm ", %" width "0\n setc %1" : "+r"(v), "=r"(cf) : : "cc");                       \
        check((u16)v == (u16)(expect) && cf == (expect_cf), name);                                                     \
    }
static u8 rol8(u8 v, int n) { n &= 7; return (u8)(v << n | v >> ((8 - n) & 7)); }
static u16 rol16(u16 v, int n) { n &= 15; return (u16)(v << n | v >> ((16 - n) & 15)); }
static u8 ror8(u8 v, int n) { n &= 7; return (u8)(v >> n | v << ((8 - n) & 7)); }
static u16 ror16(u16 v, int n) { n &= 15; return (u16)(v >> n | v << ((16 - n) & 15)); }

void cmain(i64 *sp) {
    (void)sp;
    say("x86-64 instructions, translated or not\n");

    // 8- and 16-bit rotates by an immediate: the value, and CF (rol: the low bit after; ror: the top bit after).
    u64 a5 = in(1), b = in(0);
    ROT("b", "rolb", 3, a5, rol8(0xa5, 3), rol8(0xa5, 3) & 1, "rol al, 3");
    ROT("b", "rorb", 3, a5, ror8(0xa5, 3), ror8(0xa5, 3) >> 7, "ror al, 3");
    ROT("b", "rolb", 8, a5, 0xa5, 0xa5 & 1, "rol al, 8: all the way round, CF still set from it");
    ROT("b", "rolb", 13, a5, rol8(0xa5, 13), rol8(0xa5, 13) & 1, "rol al, 13");
    ROT("w", "rolw", 8, b, 0x3412, 0x3412 & 1, "rol ax, 8: a 16-bit byte swap");
    ROT("w", "rorw", 4, b, ror16(0x1234, 4), ror16(0x1234, 4) >> 15, "ror ax, 4");
    ROT("w", "rolw", 17, b, rol16(0x1234, 17), rol16(0x1234, 17) & 1, "rol ax, 17");
    {
        // By cl, and a count of 0 (masked to 5 bits: 32) leaves the value and CF alone.
        u64 v = in(3); u8 cf; u64 c = 5;
        __asm__ volatile("stc\n rolw %%cl, %w0\n setc %1" : "+r"(v), "=r"(cf) : "c"(c) : "cc");
        u64 w = in(3); u8 cf2; u64 c2 = 32;
        __asm__ volatile("stc\n rolw %%cl, %w0\n setc %1" : "+r"(w), "=r"(cf2) : "c"(c2) : "cc");
        check((u16)v == rol16(0xfedc, 5) && cf == (rol16(0xfedc, 5) & 1) && (u16)w == 0xfedc && cf2 == 1, "rol ax, cl; and by 32, nothing changes");
    }
    {
        // A rotate keeps the rest of the register.
        u64 v = 0x1111111111111100 | in(1);
        __asm__ volatile("rolb $4, %b0" : "+r"(v) : : "cc");
        check(v == (0x1111111111111100 | rol8(0xa5, 4)), "rol al leaves the rest of rax");
    }

    {
        // stc, clc and cmc.
        u8 set, cleared, flipped, back;
        __asm__ volatile("stc\n setc %0\n clc\n setc %1\n cmc\n setc %2\n cmc\n setc %3" : "=r"(set), "=r"(cleared), "=r"(flipped), "=r"(back) : : "cc");
        check(set == 1 && cleared == 0 && flipped == 1 && back == 0, "stc, clc, and cmc both ways");
    }

    // 8- and 16-bit multiplies, one operand: ax = al * r8; dx:ax = ax * r16. CF and OF if the high half matters.
    {
        u64 ax = in(4), r = in(6); u8 cf;
        __asm__ volatile("mulb %b2\n setc %1" : "+a"(ax), "=r"(cf) : "q"(r) : "cc");
        check((u16)ax == 0x7f * 0xff && cf == 1, "mul r8: ax = al * r8, CF set");
        ax = in(7); r = in(5);
        __asm__ volatile("imulb %b2\n setc %1" : "+a"(ax), "=r"(cf) : "q"(r) : "cc");
        check((u16)ax == (u16)(3 * -127) && cf == 1, "imul r8, negative");
        u64 dx = 0x5555;
        ax = in(3); r = in(0);
        __asm__ volatile("mulw %w3\n setc %2" : "+a"(ax), "+d"(dx), "=r"(cf) : "r"(r) : "cc");
        u32 p = 0xfedcu * 0x1234u;
        check((u16)ax == (u16)p && (u16)dx == (u16)(p >> 16) && cf == 1, "mul r16: dx:ax");
        ax = in(7); r = in(0);
        __asm__ volatile("imulw %w3\n setc %2" : "+a"(ax), "+d"(dx), "=r"(cf) : "r"(r) : "cc");
        check((u16)ax == 3 * 0x1234 && (u16)dx == 0 && cf == 0, "imul r16, small: CF clear");
    }
    {
        // Two operands, 16 bits.
        u64 x = in(2), y = in(7); u8 cf;
        __asm__ volatile("imulw %w2, %w0\n setc %1" : "+r"(x), "=r"(cf) : "r"(y) : "cc");
        check((u16)x == (u16)((short)0x8001 * 3) && cf == 1, "imul r16, r16: overflows");
        u16 mem = (u16)in(0);
        x = in(7);
        __asm__ volatile("imulw %2, %w0\n setc %1" : "+r"(x), "=r"(cf) : "m"(mem) : "cc");
        check((u16)x == 3 * 0x1234 && cf == 0, "imul r16, m16");
    }

    // 8- and 16-bit divides.
    {
        u64 ax = 1000 + (in(7) - 3), r = in(4);
        __asm__ volatile("divb %b1" : "+a"(ax) : "q"(r) : "cc");
        check((u8)ax == 1000 / 0x7f && (u8)(ax >> 8) == 1000 % 0x7f, "div r8: al quotient, ah remainder");
        ax = (u16)-100 + (in(7) - 3); r = in(7);
        __asm__ volatile("idivb %b1" : "+a"(ax) : "q"(r) : "cc");
        check((signed char)ax == -100 / 3 && (signed char)(ax >> 8) == -100 % 3, "idiv r8, negative");
        u64 dx = 0x0001; ax = 0x2345 + (in(7) - 3); r = in(0);
        __asm__ volatile("divw %w2" : "+a"(ax), "+d"(dx) : "r"(r) : "cc");
        check((u16)ax == 0x12345 / 0x1234 && (u16)dx == 0x12345 % 0x1234, "div r16: dx:ax / r16");
        dx = 0xffff; ax = (u16)-30000 + (in(7) - 3); r = in(7);
        __asm__ volatile("idivw %w2" : "+a"(ax), "+d"(dx) : "r"(r) : "cc");
        check((short)ax == -30000 / 3 && (short)dx == -30000 % 3, "idiv r16, negative");
    }

    // Bit tests: locked, on memory, and with a register's bit number reaching past the operand.
    {
        static u32 words[4] = {0, 0, 0, 0};
        u8 cf;
        __asm__ volatile("lock btsl $5, %0\n setc %1" : "+m"(words[1]), "=r"(cf) : : "cc");
        u8 cf2;
        __asm__ volatile("lock btsl $5, %0\n setc %1" : "+m"(words[1]), "=r"(cf2) : : "cc");
        check(words[1] == 1 << 5 && cf == 0 && cf2 == 1, "lock bts: set, and CF says it was");
        __asm__ volatile("lock btrl $5, %0\n setc %1" : "+m"(words[1]), "=r"(cf) : : "cc");
        __asm__ volatile("lock btcq $63, %0\n setc %1" : "+m"(*(u64 *)&words[2]), "=r"(cf2) : : "cc");
        check(words[1] == 0 && cf == 1 && words[3] == 0x80000000u && cf2 == 0, "lock btr, and lock btc on 64 bits");
        // bit 40 from words[0]: words[1]'s bit 8. And -1 from words[2]: words[1]'s top bit.
        u64 n = 32 + in(7) * 3 - 1;
        __asm__ volatile("btsl %k1, %0" : "+m"(words[0]) : "r"(n) : "cc", "memory");
        i64 m = -(i64)(in(7) - 2);
        __asm__ volatile("btsl %k1, %0" : "+m"(words[2]) : "r"(m) : "cc", "memory");
        __asm__ volatile("btl %k2, %1\n setc %0" : "=r"(cf) : "m"(words[0]), "r"(n) : "cc", "memory");
        check(words[1] == (1u << 8 | 0x80000000u) && cf == 1 && words[0] == 0, "bts and bt on memory, the bit number reaching the next words, and back");
    }

    // fxsave and fxrstor: the image's layout, and every register back as it was.
    {
        static u8 image[512] __attribute__((aligned(16)));
        static u8 clean[512] __attribute__((aligned(16)));
        double x = (double)in(0), y = 0.5;
        u64 got[2], st;
        __asm__ volatile(
            "fldl %3\n fldl %4\n"
            "movq %5, %%xmm7\n pinsrw $7, %k5, %%xmm7\n"
            "fxsave %0\n"
            "fxsave %1\n"
            "pxor %%xmm7, %%xmm7\n fstp %%st(0)\n fstp %%st(0)\n fldz\n fldz\n"
            "fxrstor %1\n"
            "movdqu %%xmm7, %2\n"
            "fstpl %3\n fstpl %4\n"
            : "=m"(image), "+m"(clean), "=m"(got), "+m"(x), "+m"(y)
            : "r"(in(3))
            : "xmm7", "st", "st(1)", "memory");
        st = (u64)x;
        u16 fcw = *(u16 *)image;
        u32 mxcsr = *(u32 *)(image + 24);
        check(fcw == 0x37f && mxcsr == 0x1f80, "fxsave: the control word and MXCSR where they go");
        check(*(u64 *)(image + 160 + 7 * 16) == in(3) && *(u16 *)(image + 160 + 7 * 16 + 14) == (u16)in(3),
              "fxsave: xmm7 at its place in the image");
        // ST(0) was 0.5: 80 bits of 1.0 * 2^-1.
        check(*(u64 *)(image + 32) == 0x8000000000000000ul && *(u16 *)(image + 40) == 0x3ffe, "fxsave: ST(0) as an 80-bit number");
        check(got[0] == in(3) && (u16)(got[1] >> 48) == (u16)in(3) && x == 0.5 && st == 0, "fxrstor: xmm7 and the x87 stack back");
        (void)st;
    }

    // The string instructions: scas, cmps and lods, once and repeated, up and down; rcx and the pointers after, and the
    // flags of the last compare.
    {
        static char text[] = "carbon\0tail";
        u64 rcx = (u64)-1, at = (u64)text;
        u8 zf;
        __asm__ volatile("repne scasb\n setz %2" : "+c"(rcx), "+D"(at), "=r"(zf) : "a"(in(7) - 3) : "cc", "memory");
        check(~rcx - 1 == 6 && at == (u64)text + 7 && zf == 1, "repne scasb: strlen, stopping past the NUL");
    }
    {
        static char text[] = "carbon";
        u64 rcx = 3, at = (u64)text;
        u8 zf;
        __asm__ volatile("repne scasb\n setz %2" : "+c"(rcx), "+D"(at), "=r"(zf) : "a"((u64)'z') : "cc", "memory");
        check(rcx == 0 && at == (u64)text + 3 && zf == 0, "repne scasb: rcx running out first, ZF clear");
    }
    {
        static char a[] = "abcdefgh", b[] = "abcdXfgh";
        u64 rcx = 8, si = (u64)a, di = (u64)b;
        u8 zf, cf;
        __asm__ volatile("repe cmpsb\n setz %3\n setc %4" : "+c"(rcx), "+S"(si), "+D"(di), "=r"(zf), "=r"(cf) : : "cc", "memory");
        // 'e' - 'X' is positive: no borrow.
        check(rcx == 3 && si == (u64)a + 5 && di == (u64)b + 5 && zf == 0 && cf == 0, "repe cmpsb: the first difference, and its flags");
        rcx = 8, si = (u64)a, di = (u64)a;
        __asm__ volatile("repe cmpsb\n setz %3" : "+c"(rcx), "+S"(si), "+D"(di), "=r"(zf) : : "cc", "memory");
        check(rcx == 0 && si == (u64)a + 8 && zf == 1, "repe cmpsb: all equal");
    }
    {
        static u16 a[] = {1, 2, 3, 4}, b[] = {1, 2, 7, 4};
        u64 rcx = 4, si = (u64)&a[3], di = (u64)&b[3];
        u8 zf, cf;
        __asm__ volatile("std\n repe cmpsw\n cld\n setz %3\n setc %4" : "+c"(rcx), "+S"(si), "+D"(di), "=r"(zf), "=r"(cf) : : "cc", "memory");
        // Down: 4 = 4, then 3 < 7 borrows.
        check(rcx == 2 && si == (u64)&a[1] && di == (u64)&b[1] && zf == 0 && cf == 1, "std; repe cmpsw: stepping down, CF from a borrow");
    }
    {
        static u64 words[] = {5, 6, 0x123456789abcdefull, 8};
        u64 rcx = 4, at = (u64)words, rax = 0x123456789abcdefull;
        u8 zf;
        __asm__ volatile("repne scasq\n setz %2" : "+c"(rcx), "+D"(at), "=r"(zf) : "a"(rax) : "cc", "memory");
        check(rcx == 1 && at == (u64)&words[3] && zf == 1, "repne scasq: a quadword found");
    }
    {
        static u8 bytes[] = {0x11, 0x22, 0x33};
        u64 before = in(3) | 0xffffffffffff0000ull, rax = before, si = (u64)bytes;
        __asm__ volatile("lodsb\n lodsb" : "+a"(rax), "+S"(si) : : "memory");
        check(rax == ((before & ~0xffull) | 0x22) && si == (u64)bytes + 2, "lodsb: al only, rsi stepping");
        static u64 q[] = {0x1122334455667788ull};
        si = (u64)q;
        __asm__ volatile("lodsq" : "+a"(rax), "+S"(si) : : "memory");
        check(rax == q[0] && si == (u64)q + 8, "lodsq");
    }
    {
        // A repeated compare with rcx 0 reads nothing and leaves the flags: ZF set by the cmp before stays.
        u64 rcx = 0, at = 0;
        u8 zf;
        __asm__ volatile("cmp %%rcx, %%rcx\n repne scasb\n setz %2" : "+c"(rcx), "+D"(at), "=r"(zf) : "a"(in(7)) : "cc", "memory");
        check(rcx == 0 && at == 0 && zf == 1, "repne scasb with rcx 0: nothing read, the flags as they were");
    }

    say(failures ? "some checks failed\n" : "all passed\n");
    sys(231, failures, 0, 0);
}
