//! Checks against LLVM: the assembler's encodings and the decoder's lengths are the ones `llvm-mc` gives.

use alloc::vec::Vec;

use crate::decode::x86::decode;
use crate::emit::arm64::{exit, translate};
use crate::emit::asm::{Asm, Cond, Shift, vec};

#[test]
fn encodings_match_llvm() {
    let cases: [(u32, u32, &str); 29] = [
        (0xd2a24683, { let mut a = Asm::new(); a.movz(true, 3, 0x1234, 1); a.code[0] }, "movz x3, #0x1234, lsl #16"),
        (0xaa0403f1, { let mut a = Asm::new(); a.mov(true, 17, 4); a.code[0] }, "mov x17, x4"),
        (0xea1502bf, { let mut a = Asm::new(); a.ands(true, 31, 21, 21); a.code[0] }, "ands xzr, x21, x21"),
        (0x8b030e31, { let mut a = Asm::new(); a.add_shift(true, 17, 17, 3, Shift::Lsl, 3); a.code[0] }, "add x17, x17, x3, lsl #3"),
        (0xeb95fedf, { let mut a = Asm::new(); a.subs_shift(true, 31, 22, 21, Shift::Asr, 63); a.code[0] }, "subs xzr, x22, x21, asr #63"),
        (0xd1002084, { let mut a = Asm::new(); a.sub_imm(true, 4, 4, 8); a.code[0] }, "sub x4, x4, #8"),
        (0x53081e76, { let mut a = Asm::new(); a.lsl_imm(false, 22, 19, 24); a.code[0] }, "lsl w22, w19, #24"),
        (0xd3483c13, { let mut a = Asm::new(); a.ubfx(true, 19, 0, 8, 8); a.code[0] }, "ubfx x19, x0, #8, #8"),
        (0xb3781ea0, { let mut a = Asm::new(); a.bfi(true, 0, 21, 8, 8); a.code[0] }, "bfi x0, x21, #8, #8"),
        (0x9b547c16, { let mut a = Asm::new(); a.smulh(22, 0, 20); a.code[0] }, "smulh x22, x0, x20"),
        (0x9ad40815, { let mut a = Asm::new(); a.udiv(true, 21, 0, 20); a.code[0] }, "udiv x21, x0, x20"),
        (0x9a933295, { let mut a = Asm::new(); a.csel(true, 21, 20, 19, Cond::Cc); a.code[0] }, "csel x21, x20, x19, lo"),
        (0x9a9f07f7, { let mut a = Asm::new(); a.cset(true, 23, Cond::Ne); a.code[0] }, "cset x23, ne"),
        (0x39400233, { let mut a = Asm::new(); a.ldr(8, 19, 17); a.code[0] }, "ldrb w19, [x17]"),
        (0xf9000093, { let mut a = Asm::new(); a.str(64, 19, 4); a.code[0] }, "str x19, [x4]"),
        (0x3dc00223, { let mut a = Asm::new(); a.ldr_q(3, 17); a.code[0] }, "ldr q3, [x17]"),
        (0xc8f3fe34, { let mut a = Asm::new(); a.casal(64, 19, 20, 17); a.code[0] }, "casal x19, x20, [x17]"),
        (0xf8f40233, { let mut a = Asm::new(); a.ldaddal(64, 20, 19, 17); a.code[0] }, "ldaddal x20, x19, [x17]"),
        (0x38f48233, { let mut a = Asm::new(); a.swpal(8, 20, 19, 17); a.code[0] }, "swpalb w20, w19, [x17]"),
        (0x54ffffa3, { let mut a = Asm::new(); a.b_cond(Cond::Cc, -3); a.code[0] }, "b.lo #-12"),
        (0xd4200060, { let mut a = Asm::new(); a.brk(3); a.code[0] }, "brk #0x3"),
        (0xd53b4217, { let mut a = Asm::new(); a.mrs_nzcv(23); a.code[0] }, "mrs x23, NZCV"),
        (0xd500401f, { let mut a = Asm::new(); a.cfinv(); a.code[0] }, "cfinv"),
        (0xba020020, { let mut a = Asm::new(); a.adcs(true, 0, 1, 2); a.code[0] }, "adcs x0, x1, x2"),
        (0x7a050083, { let mut a = Asm::new(); a.sbcs(false, 3, 4, 5); a.code[0] }, "sbcs w3, w4, w5"),
        (0x4e20a8bf, { let mut a = Asm::new(); a.cmlt0_16b(31, 5); a.code[0] }, "cmlt v31.16b, v5.16b, #0"),
        (0x0e31bbfc, { let mut a = Asm::new(); a.addv_8b(28, 31); a.code[0] }, "addv b28, v31.8b"),
        (0x9e67021e, { let mut a = Asm::new(); a.fmov_d_x(30, 16); a.code[0] }, "fmov d30, x16"),
        (0x6e1807de, { let mut a = Asm::new(); a.ins_d1_d0(30, 30); a.code[0] }, "mov v30.d[1], v30.d[0]"),
    ];
    for (llvm, ours, text) in cases {
        assert_eq!(ours, llvm, "{text}");
    }
}

/// The vector and floating-point encodings SSE is lowered to, each checked against `llvm-mc`.
#[test]
fn sse_encodings_match_llvm() {
    let cases: [(u32, u32, &str); 80] = [
        (0x4e238441, { let mut a = Asm::new(); a.v3(vec::ADD, 0, 1, 2, 3); a.code[0] }, "add v1.16b, v2.16b, v3.16b"),
        (0x6ee38441, { let mut a = Asm::new(); a.v3(vec::SUB, 3, 1, 2, 3); a.code[0] }, "sub v1.2d, v2.2d, v3.2d"),
        (0x6e668ca4, { let mut a = Asm::new(); a.v3(vec::CMEQ, 1, 4, 5, 6); a.code[0] }, "cmeq v4.8h, v5.8h, v6.8h"),
        (0x4ea634a4, { let mut a = Asm::new(); a.v3(vec::CMGT, 2, 4, 5, 6); a.code[0] }, "cmgt v4.4s, v5.4s, v6.4s"),
        (0x4e230c41, { let mut a = Asm::new(); a.v3(vec::SQADD, 0, 1, 2, 3); a.code[0] }, "sqadd v1.16b, v2.16b, v3.16b"),
        (0x6e630c41, { let mut a = Asm::new(); a.v3(vec::UQADD, 1, 1, 2, 3); a.code[0] }, "uqadd v1.8h, v2.8h, v3.8h"),
        (0x4e632c41, { let mut a = Asm::new(); a.v3(vec::SQSUB, 1, 1, 2, 3); a.code[0] }, "sqsub v1.8h, v2.8h, v3.8h"),
        (0x6e232c41, { let mut a = Asm::new(); a.v3(vec::UQSUB, 0, 1, 2, 3); a.code[0] }, "uqsub v1.16b, v2.16b, v3.16b"),
        (0x4e636441, { let mut a = Asm::new(); a.v3(vec::SMAX, 1, 1, 2, 3); a.code[0] }, "smax v1.8h, v2.8h, v3.8h"),
        (0x6e236441, { let mut a = Asm::new(); a.v3(vec::UMAX, 0, 1, 2, 3); a.code[0] }, "umax v1.16b, v2.16b, v3.16b"),
        (0x4e636c41, { let mut a = Asm::new(); a.v3(vec::SMIN, 1, 1, 2, 3); a.code[0] }, "smin v1.8h, v2.8h, v3.8h"),
        (0x6e236c41, { let mut a = Asm::new(); a.v3(vec::UMIN, 0, 1, 2, 3); a.code[0] }, "umin v1.16b, v2.16b, v3.16b"),
        (0x4e639c41, { let mut a = Asm::new(); a.v3(vec::MUL, 1, 1, 2, 3); a.code[0] }, "mul v1.8h, v2.8h, v3.8h"),
        (0x6e231441, { let mut a = Asm::new(); a.v3(vec::URHADD, 0, 1, 2, 3); a.code[0] }, "urhadd v1.16b, v2.16b, v3.16b"),
        (0x4e033841, { let mut a = Asm::new(); a.v3(vec::ZIP1, 0, 1, 2, 3); a.code[0] }, "zip1 v1.16b, v2.16b, v3.16b"),
        (0x4ec37841, { let mut a = Asm::new(); a.v3(vec::ZIP2, 3, 1, 2, 3); a.code[0] }, "zip2 v1.2d, v2.2d, v3.2d"),
        (0x4e23d441, { let mut a = Asm::new(); a.v3(vec::FADD, 0, 1, 2, 3); a.code[0] }, "fadd v1.4s, v2.4s, v3.4s"),
        (0x6e23d441, { let mut a = Asm::new(); a.v3(vec::FADDP, 0, 1, 2, 3); a.code[0] }, "faddp v1.4s, v2.4s, v3.4s"),
        (0x6e63d441, { let mut a = Asm::new(); a.v3(vec::FADDP, 1, 1, 2, 3); a.code[0] }, "faddp v1.2d, v2.2d, v3.2d"),
        (0x4ec31841, { let mut a = Asm::new(); a.v3(vec::UZP1, 3, 1, 2, 3); a.code[0] }, "uzp1 v1.2d, v2.2d, v3.2d"),
        (0x4e835841, { let mut a = Asm::new(); a.v3(vec::UZP2, 2, 1, 2, 3); a.code[0] }, "uzp2 v1.4s, v2.4s, v3.4s"),
        (0x4e832841, { let mut a = Asm::new(); a.v3(vec::TRN1, 2, 1, 2, 3); a.code[0] }, "trn1 v1.4s, v2.4s, v3.4s"),
        (0x4e836841, { let mut a = Asm::new(); a.v3(vec::TRN2, 2, 1, 2, 3); a.code[0] }, "trn2 v1.4s, v2.4s, v3.4s"),
        (0x4e080441, { let mut a = Asm::new(); a.dup_d0(1, 2); a.code[0] }, "dup v1.2d, v2.d[0]"),
        (0x4ee3d441, { let mut a = Asm::new(); a.v3(vec::FSUB, 1, 1, 2, 3); a.code[0] }, "fsub v1.2d, v2.2d, v3.2d"),
        (0x6e23dc41, { let mut a = Asm::new(); a.v3(vec::FMUL, 0, 1, 2, 3); a.code[0] }, "fmul v1.4s, v2.4s, v3.4s"),
        (0x6e63fc41, { let mut a = Asm::new(); a.v3(vec::FDIV, 1, 1, 2, 3); a.code[0] }, "fdiv v1.2d, v2.2d, v3.2d"),
        (0x4e23e441, { let mut a = Asm::new(); a.v3(vec::FCMEQ, 0, 1, 2, 3); a.code[0] }, "fcmeq v1.4s, v2.4s, v3.4s"),
        (0x6e63e441, { let mut a = Asm::new(); a.v3(vec::FCMGE, 1, 1, 2, 3); a.code[0] }, "fcmge v1.2d, v2.2d, v3.2d"),
        (0x6ea3e441, { let mut a = Asm::new(); a.v3(vec::FCMGT, 0, 1, 2, 3); a.code[0] }, "fcmgt v1.4s, v2.4s, v3.4s"),
        (0x6ee1f841, { let mut a = Asm::new(); a.v2(vec::FSQRT, 1, 1, 2); a.code[0] }, "fsqrt v1.2d, v2.2d"),
        (0xcb56735a, { let mut a = Asm::new(); a.sub_shift(26, 26, 22, Shift::Lsr, 28); a.code[0] }, "sub x26, x26, x22, lsr #28"),
        (0xf8737a96, { let mut a = Asm::new(); a.ldr_index(22, 20, 19); a.code[0] }, "ldr x22, [x20, x19, lsl #3]"),
        (0xd61f0280, { let mut a = Asm::new(); a.br(20); a.code[0] }, "br x20"),
        (0x1e644041, { let mut a = Asm::new(); a.f_unary(8, true, 1, 2); a.code[0] }, "frintn d1, d2"),
        (0x1e654041, { let mut a = Asm::new(); a.f_unary(10, true, 1, 2); a.code[0] }, "frintm d1, d2"),
        (0x1e64c041, { let mut a = Asm::new(); a.f_unary(9, true, 1, 2); a.code[0] }, "frintp d1, d2"),
        (0x1e65c041, { let mut a = Asm::new(); a.f_unary(11, true, 1, 2); a.code[0] }, "frintz d1, d2"),
        (0x1e614230, { let mut a = Asm::new(); a.f_unary(2, true, 16, 17); a.code[0] }, "fneg d16, d17"),
        (0x1e60c230, { let mut a = Asm::new(); a.f_unary(1, true, 16, 17); a.code[0] }, "fabs d16, d17"),
        (0x9e63027c, { let mut a = Asm::new(); a.ucvtf(true, true, 28, 19); a.code[0] }, "ucvtf d28, x19"),
        (0x1f5dc39e, { let mut a = Asm::new(); a.fmsub(true, 30, 28, 29, 16); a.code[0] }, "fmsub d30, d28, d29, d16"),
        (0x93dafed6, { let mut a = Asm::new(); a.extr(22, 22, 26, 63); a.code[0] }, "extr x22, x22, x26, #63"),
        (0x6e621c3c, { let mut a = Asm::new(); a.bsl(28, 1, 2); a.code[0] }, "bsl v28.16b, v1.16b, v2.16b"),
        (0x6e205bbc, { let mut a = Asm::new(); a.v_not(28, 29); a.code[0] }, "mvn v28.16b, v29.16b"),
        (0x4f155483, { let mut a = Asm::new(); a.shl(1, 3, 4, 5); a.code[0] }, "shl v3.8h, v4.8h, #5"),
        (0x6f210483, { let mut a = Asm::new(); a.ushr(2, 3, 4, 31); a.code[0] }, "ushr v3.4s, v4.4s, #31"),
        (0x6f410483, { let mut a = Asm::new(); a.ushr(3, 3, 4, 63); a.code[0] }, "ushr v3.2d, v4.2d, #63"),
        (0x4f100483, { let mut a = Asm::new(); a.sshr(1, 3, 4, 16); a.code[0] }, "sshr v3.8h, v4.8h, #16"),
        (0x4e1e0041, { let mut a = Asm::new(); a.tbl(1, 2, false, 30); a.code[0] }, "tbl v1.16b, { v2.16b }, v30.16b"),
        (0x4e1e2381, { let mut a = Asm::new(); a.tbl(1, 28, true, 30); a.code[0] }, "tbl v1.16b, { v28.16b, v29.16b }, v30.16b"),
        (0x6e084441, { let mut a = Asm::new(); a.ins_elem(3, 1, 0, 2, 1); a.code[0] }, "mov v1.d[0], v2.d[1]"),
        (0x6e180441, { let mut a = Asm::new(); a.ins_elem(3, 1, 1, 2, 0); a.code[0] }, "mov v1.d[1], v2.d[0]"),
        (0x4e161e63, { let mut a = Asm::new(); a.ins_gen(1, 3, 5, 19); a.code[0] }, "mov v3.h[5], w19"),
        (0x4e181e7e, { let mut a = Asm::new(); a.ins_gen(3, 30, 1, 19); a.code[0] }, "mov v30.d[1], x19"),
        (0x0e1e3c60, { let mut a = Asm::new(); a.umov(1, 0, 3, 7); a.code[0] }, "umov w0, v3.h[7]"),
        (0x0e1c3f93, { let mut a = Asm::new(); a.umov(2, 19, 28, 3); a.code[0] }, "mov w19, v28.s[3]"),
        (0x4e183f93, { let mut a = Asm::new(); a.umov(3, 19, 28, 1); a.code[0] }, "mov x19, v28.d[1]"),
        (0x0e21483c, { let mut a = Asm::new(); a.narrow(0, false, false, 28, 1); a.code[0] }, "sqxtn v28.8b, v1.8h"),
        (0x6e21285c, { let mut a = Asm::new(); a.narrow(0, true, true, 28, 2); a.code[0] }, "sqxtun2 v28.16b, v2.8h"),
        (0x4e61485c, { let mut a = Asm::new(); a.narrow(1, false, true, 28, 2); a.code[0] }, "sqxtn2 v28.8h, v2.4s"),
        (0x1e62283c, { let mut a = Asm::new(); a.f_scalar(2, true, 28, 1, 2); a.code[0] }, "fadd d28, d1, d2"),
        (0x1e22183c, { let mut a = Asm::new(); a.f_scalar(1, false, 28, 1, 2); a.code[0] }, "fdiv s28, s1, s2"),
        (0x1e61c07c, { let mut a = Asm::new(); a.fsqrt(true, 28, 3); a.code[0] }, "fsqrt d28, d3"),
        (0x1e7f2020, { let mut a = Asm::new(); a.fcmp(true, 1, 31); a.code[0] }, "fcmp d1, d31"),
        (0x1e224c3c, { let mut a = Asm::new(); a.fcsel(false, 28, 1, 2, Cond::Mi); a.code[0] }, "fcsel s28, s1, s2, mi"),
        (0x1e22c03c, { let mut a = Asm::new(); a.fcvt(false, 28, 1); a.code[0] }, "fcvt d28, s1"),
        (0x1e62403c, { let mut a = Asm::new(); a.fcvt(true, 28, 1); a.code[0] }, "fcvt s28, d1"),
        (0x9e62027c, { let mut a = Asm::new(); a.scvtf(true, true, 28, 19); a.code[0] }, "scvtf d28, x19"),
        (0x1e22027c, { let mut a = Asm::new(); a.scvtf(false, false, 28, 19); a.code[0] }, "scvtf s28, w19"),
        (0x9e780393, { let mut a = Asm::new(); a.fcvtzs(true, true, 19, 28); a.code[0] }, "fcvtzs x19, d28"),
        (0x9e380033, { let mut a = Asm::new(); a.fcvtzs(true, false, 19, 1); a.code[0] }, "fcvtzs x19, s1"),
        (0x1e67c03c, { let mut a = Asm::new(); a.frinti(true, 28, 1); a.code[0] }, "frinti d28, d1"),
        (0xd53b4413, { let mut a = Asm::new(); a.mrs_fpcr(19); a.code[0] }, "mrs x19, FPCR"),
        (0xd51b4418, { let mut a = Asm::new(); a.msr_fpcr(24); a.code[0] }, "msr FPCR, x24"),
        (0x0e617841, { let mut a = Asm::new(); a.put(0x0E61_7800 | 2 << 5 | 1); a.code[0] }, "fcvtl v1.2d, v2.2s"),
        (0x0e616841, { let mut a = Asm::new(); a.put(0x0E61_6800 | 2 << 5 | 1); a.code[0] }, "fcvtn v1.2s, v2.2d"),
        (0x4e21d841, { let mut a = Asm::new(); a.put(0x4E21_D800 | 2 << 5 | 1); a.code[0] }, "scvtf v1.4s, v2.4s"),
        (0x4ea1b841, { let mut a = Asm::new(); a.put(0x4EA1_B800 | 2 << 5 | 1); a.code[0] }, "fcvtzs v1.4s, v2.4s"),
        (0x6ea19841, { let mut a = Asm::new(); a.put(0x6EA1_9800 | 2 << 5 | 1); a.code[0] }, "frinti v1.4s, v2.4s"),
    ];
    for (llvm, ours, text) in cases {
        assert_eq!(ours, llvm, "{text}");
    }
}

#[test]
fn lengths_match_llvm() {
    let cases: [(&str, &[u8]); 90] = [
        ("mov rax, qword ptr [rip + 4660]", &[0x48, 0x8b, 0x05, 0x34, 0x12, 0x00, 0x00]),
        ("mov eax, dword ptr [rbx + 4*rcx + 8]", &[0x8b, 0x44, 0x8b, 0x08]),
        ("mov byte ptr [rsp - 128], 127", &[0xc6, 0x44, 0x24, 0x80, 0x7f]),
        ("mov qword ptr fs:[40], rdx", &[0x64, 0x48, 0x89, 0x14, 0x25, 0x28, 0x00, 0x00, 0x00]),
        ("movabs r11, 1234605616436508552", &[0x49, 0xbb, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]),
        ("mov r9w, 4660", &[0x66, 0x41, 0xb9, 0x34, 0x12]),
        ("lea rdi, [rsi + 8*r8 - 4096]", &[0x4a, 0x8d, 0xbc, 0xc6, 0x00, 0xf0, 0xff, 0xff]),
        ("lea eax, [ebx + 4]", &[0x67, 0x8d, 0x43, 0x04]),
        ("add rax, 305419896", &[0x48, 0x05, 0x78, 0x56, 0x34, 0x12]),
        ("add al, 5", &[0x04, 0x05]),
        ("sub dword ptr [rbp - 8], 1", &[0x83, 0x6d, 0xf8, 0x01]),
        ("cmp byte ptr [rax], 0", &[0x80, 0x38, 0x00]),
        ("xor eax, eax", &[0x31, 0xc0]),
        ("and r15, r14", &[0x4d, 0x21, 0xf7]),
        ("or ax, 256", &[0x66, 0x0d, 0x00, 0x01]),
        ("test edi, edi", &[0x85, 0xff]),
        ("test byte ptr [rcx + 4], 128", &[0xf6, 0x41, 0x04, 0x80]),
        ("inc r12", &[0x49, 0xff, 0xc4]),
        ("dec dword ptr [rax]", &[0xff, 0x08]),
        ("lock  add qword ptr [rdi], rsi", &[0xf0, 0x48, 0x01, 0x37]),
        ("lock  cmpxchg qword ptr [rdx], rcx", &[0xf0, 0x48, 0x0f, 0xb1, 0x0a]),
        ("lock  xadd dword ptr [rsi], eax", &[0xf0, 0x0f, 0xc1, 0x06]),
        ("xchg qword ptr [rbx], rax", &[0x48, 0x87, 0x03]),
        ("push rbp", &[0x55]),
        ("push 305419896", &[0x68, 0x78, 0x56, 0x34, 0x12]),
        ("push qword ptr [rax + 8]", &[0xff, 0x70, 0x08]),
        ("pop r13", &[0x41, 0x5d]),
        ("call qword ptr [rip + 64]", &[0xff, 0x15, 0x40, 0x00, 0x00, 0x00]),
        ("call rax", &[0xff, 0xd0]),
        ("ret", &[0xc3]),
        ("ret 16", &[0xc2, 0x10, 0x00]),
        ("jmp qword ptr [8*rax + 8192]", &[0xff, 0x24, 0xc5, 0x00, 0x20, 0x00, 0x00]),
        ("imul rax, rbx", &[0x48, 0x0f, 0xaf, 0xc3]),
        ("imul ecx, dword ptr [rdx], 74565", &[0x69, 0x0a, 0x45, 0x23, 0x01, 0x00]),
        ("imul edx, eax, 7", &[0x6b, 0xd0, 0x07]),
        ("mul rcx", &[0x48, 0xf7, 0xe1]),
        ("idiv dword ptr [rsp + 4]", &[0xf7, 0x7c, 0x24, 0x04]),
        ("div r8", &[0x49, 0xf7, 0xf0]),
        ("neg rax", &[0x48, 0xf7, 0xd8]),
        ("not ecx", &[0xf7, 0xd1]),
        ("shl rax, 3", &[0x48, 0xc1, 0xe0, 0x03]),
        ("sar edx, cl", &[0xd3, 0xfa]),
        ("shr byte ptr [rax]", &[0xd0, 0x28]),
        ("rol ecx, 5", &[0xc1, 0xc1, 0x05]),
        ("movzx eax, byte ptr [rdi]", &[0x0f, 0xb6, 0x07]),
        ("movzx ecx, word ptr [rsi + 2]", &[0x0f, 0xb7, 0x4e, 0x02]),
        ("movsx rax, byte ptr [rdi]", &[0x48, 0x0f, 0xbe, 0x07]),
        ("movsxd rax, dword ptr [rdi]", &[0x48, 0x63, 0x07]),
        ("cdqe", &[0x48, 0x98]),
        ("cqo", &[0x48, 0x99]),
        ("cdq", &[0x99]),
        ("cmove rax, rcx", &[0x48, 0x0f, 0x44, 0xc1]),
        ("cmovl edx, dword ptr [rax]", &[0x0f, 0x4c, 0x10]),
        ("sete al", &[0x0f, 0x94, 0xc0]),
        ("setne byte ptr [rcx]", &[0x0f, 0x95, 0x01]),
        ("bsf rax, rcx", &[0x48, 0x0f, 0xbc, 0xc1]),
        ("bsr edx, eax", &[0x0f, 0xbd, 0xd0]),
        ("tzcnt rax, rdi", &[0xf3, 0x48, 0x0f, 0xbc, 0xc7]),
        ("lzcnt ecx, edx", &[0xf3, 0x0f, 0xbd, 0xca]),
        ("bswap rax", &[0x48, 0x0f, 0xc8]),
        ("syscall", &[0x0f, 0x05]),
        ("cpuid", &[0x0f, 0xa2]),
        ("rdtsc", &[0x0f, 0x31]),
        ("int3", &[0xcc]),
        ("int 128", &[0xcd, 0x80]),
        ("ud2", &[0x0f, 0x0b]),
        ("nop", &[0x90]),
        ("nop dword ptr [rax + rax]", &[0x0f, 0x1f, 0x04, 0x00]),
        ("endbr64", &[0xf3, 0x0f, 0x1e, 0xfa]),
        ("pause", &[0xf3, 0x90]),
        ("mfence", &[0x0f, 0xae, 0xf0]),
        ("rep  stosb byte ptr es:[rdi], al", &[0xf3, 0xaa]),
        ("rep  movsq qword ptr es:[rdi], qword ptr [rsi]", &[0xf3, 0x48, 0xa5]),
        ("movdqa xmm0, xmmword ptr [rsi]", &[0x66, 0x0f, 0x6f, 0x06]),
        ("movdqu xmmword ptr [rdi + 16], xmm15", &[0xf3, 0x44, 0x0f, 0x7f, 0x7f, 0x10]),
        ("movups xmm1, xmm2", &[0x0f, 0x10, 0xca]),
        ("movaps xmmword ptr [rsp], xmm3", &[0x0f, 0x29, 0x1c, 0x24]),
        ("movss xmm0, dword ptr [rax]", &[0xf3, 0x0f, 0x10, 0x00]),
        ("movsd qword ptr [rax], xmm9", &[0xf2, 0x44, 0x0f, 0x11, 0x08]),
        ("pxor xmm0, xmm0", &[0x66, 0x0f, 0xef, 0xc0]),
        ("xorps xmm1, xmm1", &[0x0f, 0x57, 0xc9]),
        ("pcmpeqb xmm1, xmmword ptr [rdi]", &[0x66, 0x0f, 0x74, 0x0f]),
        ("pmovmskb eax, xmm1", &[0x66, 0x0f, 0xd7, 0xc1]),
        ("movq rax, xmm0", &[0x66, 0x48, 0x0f, 0x7e, 0xc0]),
        ("movd xmm1, ecx", &[0x66, 0x0f, 0x6e, 0xc9]),
        ("movq xmm2, qword ptr [rdi]", &[0xf3, 0x0f, 0x7e, 0x17]),
        ("movq qword ptr [rsp], xmm4", &[0x66, 0x0f, 0xd6, 0x24, 0x24]),
        ("pand xmm0, xmm1", &[0x66, 0x0f, 0xdb, 0xc1]),
        ("pandn xmm2, xmm3", &[0x66, 0x0f, 0xdf, 0xd3]),
        ("por xmm4, xmm5", &[0x66, 0x0f, 0xeb, 0xe5]),
    ];
    for (text, bytes) in cases {
        let inst = decode(0x1000, bytes).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        assert_eq!(usize::from(inst.len), bytes.len(), "{text}");
    }
}

#[test]
fn a_block_ends_at_its_branch() {
    // add rax, rbx; cmp byte ptr [rax + 8], 0; je +1; nop; nop
    let bytes = [0x48, 0x01, 0xd8, 0x80, 0x78, 0x08, 0x00, 0x74, 0x01, 0x90, 0x90];
    let t = translate(0x40_0000, &bytes, 64);
    let pcs: Vec<u64> = t.map.iter().map(|(_, pc)| *pc).collect();
    assert_eq!(pcs, [0x40_0000, 0x40_0003, 0x40_0007]);
    assert!(t.cacheable);
    // It ends on the branch's target: a chain exit, with the x86 address after it.
    let n = t.code.len();
    let word = |at: usize| u32::from_le_bytes(t.code[at..at + 4].try_into().unwrap());
    assert_eq!(word(n - 12), 0xD420_0000 | u32::from(exit::CHAIN) << 5);
    assert_eq!(u64::from(word(n - 8)) | u64::from(word(n - 4)) << 32, 0x40_000a);
}

#[test]
fn a_return_looks_itself_up() {
    // ret: the lookup table's probe, then an exit with the address in x16 if it misses.
    let t = translate(0x40_0000, &[0xc3], 64);
    let last = u32::from_le_bytes(t.code[t.code.len() - 4..].try_into().unwrap());
    assert_eq!(last, 0xD420_0000 | u32::from(exit::JUMP) << 5);
    let words: Vec<u32> = t.code.chunks(4).map(|w| u32::from_le_bytes(w.try_into().unwrap())).collect();
    assert!(words.contains(&0xD61F_0280), "br x20");
}

#[test]
fn lookup_entries_and_chain_branches() {
    use crate::emit::arm64::{CACHE_BASE, chain_branch, lookup_entry};
    let (index, entry) = lookup_entry(0x40_1234, CACHE_BASE + 0x100);
    assert_eq!(index, 0x1234 & 0x3fff);
    assert_eq!(entry, ((0x40_1234u64 >> 14) + 1) << 28 | 0x40);
    assert_eq!(chain_branch(0x1000, 0x1008), Some(0x1400_0002));
    assert_eq!(chain_branch(0x1008, 0x1000), Some(0x17FF_FFFE));
    assert_eq!(chain_branch(0, 128 << 20), None);
}

#[test]
fn code_that_runs_out_is_a_fetch_exit() {
    // mov rax, imm64, cut short
    let t = translate(0x1000, &[0x48, 0xb8, 1, 2, 3], 64);
    assert!(!t.cacheable);
    let last = u32::from_le_bytes(t.code[t.code.len() - 4..].try_into().unwrap());
    assert_eq!(last, 0xD420_0000 | u32::from(exit::FETCH) << 5);
}

#[test]
fn the_direction_flag_lasts_its_block() {
    let words = |t: &crate::emit::arm64::Translation| -> Vec<u32> { t.code.chunks(4).map(|w| u32::from_le_bytes(w.try_into().unwrap())).collect() };
    let unsupported = 0xD420_0000 | u32::from(exit::UNSUPPORTED) << 5;
    // std; rep movsb; cld; ret: down, then up again before the block ends.
    let t = translate(0x40_0000, &[0xfd, 0xf3, 0xa4, 0xfc, 0xc3], 64);
    let w = words(&t);
    assert!(!w.contains(&unsupported));
    // rsi and rdi step down: sub x6, x6, #1 and sub x7, x7, #1.
    assert!(w.contains(&0xD100_04C6) && w.contains(&0xD100_04E7));
    // std; ret: a block can't end with DF set, so it stops at the ret.
    let t = translate(0x40_0000, &[0xfd, 0xc3], 64);
    assert_eq!(*words(&t).last().unwrap(), unsupported);
}
