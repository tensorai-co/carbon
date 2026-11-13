//! Delivering exceptions to the program: a fault, a bad instruction or a breakpoint becomes an EXCEPTION_RECORD and a
//! CONTEXT on the program's stack, and the thread carries on in ntdll's dispatcher, which finds the program's handler.
//! `CONTINUE` (NtContinue) is how it comes back: the thread takes the registers a CONTEXT says.

use atlas_abi::{context as ctx, status};
use exec::isa::x86::{RAX, RBP, RBX, RCX, RDI, RDX, RSI, RSP, USER_FLAGS};
use exec::{FpRegs, Isa, Prot, Regs};
use kernels_core::lock;
use kernels_core::thread::{Cx, Flow};
use kernels_core::user;

use crate::process::Task;

/// How many exceptions in a row, with nothing carrying on in between, before atlas gives up on the program: an
/// exception while dispatching one, again and again.
const MOST_NESTED: u32 = 16;
/// Room left below the stack pointer the program had, past any red zone.
const LEAVE: u64 = 256;

/// What went wrong, as an exception.
pub struct Raised {
    pub code: u32,
    pub address: u64,
    pub parameters: Vec<u64>,
}

impl Raised {
    pub fn access_violation(pc: u64, addr: u64, access: Prot) -> Raised {
        let kind = if access.exec() {
            8
        } else if access.write() {
            1
        } else {
            0
        };
        Raised { code: status::ACCESS_VIOLATION, address: pc, parameters: vec![kind, addr] }
    }

    pub fn simple(code: u32, pc: u64) -> Raised {
        Raised { code, address: pc, parameters: Vec::new() }
    }
}

impl Task {
    /// Hands an exception to the program's dispatcher, or ends the process if it can't take one.
    pub fn raise(&mut self, cx: &mut Cx<'_>, raised: Raised, what: &str) -> Flow {
        let dispatcher = lock(&self.process.state).dispatcher;
        self.nested += 1;
        if dispatcher == 0 || self.nested > MOST_NESTED {
            return self.die(raised.code, what);
        }
        let (regs, fp) = cx.turn(|t| (t.thread().regs(), t.thread().fp()));
        let x86 = self.isa == Isa::X86_32;
        let context = if x86 { encode_x86(&regs, &fp) } else { encode(&regs, &fp) };
        // EXCEPTION_RECORD, in this ISA's layout: its address and parameters are pointer-sized.
        let abi = self.abi();
        let (record, width) = (abi.record, abi.width as usize);
        let mut rec = vec![0u8; record.size as usize];
        rec[0..4].copy_from_slice(&raised.code.to_le_bytes());
        rec[record.address as usize..][..width].copy_from_slice(&abi.ptr(raised.address));
        rec[record.parameter_count as usize..][..4].copy_from_slice(&(raised.parameters.len() as u32).to_le_bytes());
        for (i, p) in raised.parameters.iter().take(15).enumerate() {
            rec[record.parameters as usize + i * width..][..width].copy_from_slice(&abi.ptr(*p));
        }
        let context_at = (regs.sp() - LEAVE - context.len() as u64 - rec.len() as u64) & !15;
        let record_at = context_at + context.len() as u64;
        let mut bytes = context;
        bytes.extend(&rec);
        let mut new = regs;
        let mut frame = Vec::new();
        let frame_at = match &mut new {
            Regs::Arm64(r) => {
                r.x[0] = record_at;
                r.x[1] = context_at;
                r.x[29] = 0;
                r.x[30] = 0;
                r.sp = context_at;
                r.pc = dispatcher;
                context_at
            }
            // As if called, stdcall: a return address of 0, then the record and the context.
            Regs::X86(r) if x86 => {
                r.gpr[RSP] = context_at - 12;
                r.rip = dispatcher;
                r.rflags &= !0x400;
                frame = [0u32, record_at as u32, context_at as u32].iter().flat_map(|v| v.to_le_bytes()).collect();
                context_at - 12
            }
            Regs::X86(r) => {
                // As if called: a return address of 0, and 32 bytes of home space above it.
                r.gpr[exec::isa::x86::RCX] = record_at;
                r.gpr[exec::isa::x86::RDX] = context_at;
                r.gpr[exec::isa::x86::RSP] = context_at - 40;
                r.rip = dispatcher;
                r.rflags &= !0x400;
                frame = vec![0u8; 40];
                context_at - 40
            }
        };
        frame.extend(bytes);
        if self.mem(cx, true, |t| user::write(&mut t.space(), frame_at, &frame)).is_err() {
            return self.die(raised.code, &format!("{what}, and there's no stack left to handle it"));
        }
        cx.turn(|t| t.thread().set_regs(&new));
        Flow::Run
    }

    /// `CONTINUE`: the thread takes the registers in the CONTEXT at `at`.
    pub fn continue_from(&mut self, cx: &mut Cx<'_>, at: u64) -> Flow {
        if self.isa == Isa::X86_32 {
            return self.continue_x86(cx, at);
        }
        let x86 = self.isa != exec::Isa::Arm64;
        let size = if x86 { ctx::X64_SIZE } else { ctx::ARM64_SIZE };
        let Ok(bytes) = self.mem(cx, false, |t| user::read_vec(&t.space(), at, size as usize)) else {
            return self.die(status::ACCESS_VIOLATION, "NtContinue with a bad CONTEXT");
        };
        let get = |o: u64| u64::from_le_bytes(bytes[o as usize..o as usize + 8].try_into().unwrap_or_default());
        let get32 = |o: u64| u32::from_le_bytes(bytes[o as usize..o as usize + 4].try_into().unwrap_or_default());
        cx.turn(|t| {
            let mut regs = t.thread().regs();
            let mut fp = t.thread().fp();
            match (&mut regs, &mut fp) {
                (Regs::Arm64(r), FpRegs::Arm64(f)) => {
                    for i in 0..31 {
                        r.x[i] = get(ctx::ARM64_X + i as u64 * 8);
                    }
                    r.sp = get(ctx::ARM64_SP);
                    r.pc = get(ctx::ARM64_PC);
                    r.nzcv = u64::from(get32(ctx::ARM64_CPSR) & 0xf000_0000);
                    for i in 0..32 {
                        let o = (ctx::ARM64_V + i as u64 * 16) as usize;
                        f.v[i] = u128::from_le_bytes(bytes[o..o + 16].try_into().unwrap_or_default());
                    }
                    f.fpcr = get32(ctx::ARM64_FPCR);
                    f.fpsr = get32(ctx::ARM64_FPSR);
                }
                (Regs::X86(r), FpRegs::X86(f)) => {
                    for i in 0..16 {
                        r.gpr[i] = get(ctx::X64_GPRS + i as u64 * 8);
                    }
                    r.rip = get(ctx::X64_RIP);
                    r.rflags = r.rflags & !USER_FLAGS | u64::from(get32(ctx::X64_EFLAGS)) & USER_FLAGS;
                    f.fxsave.copy_from_slice(&bytes[0x100..0x300]);
                }
                _ => {}
            }
            t.thread().set_regs(&regs);
            t.thread().set_fp(&fp);
        });
        self.nested = 0;
        Flow::Run
    }

    /// x86's `CONTINUE`: only what its ContextFlags say it has. ntdll's own stubs (`RtlRaiseException`, `RtlUnwind`)
    /// capture the control and integer registers and leave the floating-point areas as they were on the stack.
    fn continue_x86(&mut self, cx: &mut Cx<'_>, at: u64) -> Flow {
        const CONTROL: u32 = 1;
        const INTEGER: u32 = 2;
        const FLOATING_POINT: u32 = 8;
        const EXTENDED_REGISTERS: u32 = 0x20;
        let Ok(bytes) = self.mem(cx, false, |t| user::read_vec(&t.space(), at, ctx::X86_SIZE as usize)) else {
            return self.die(status::ACCESS_VIOLATION, "NtContinue with a bad CONTEXT");
        };
        let get = |o: u64| u64::from(u32::from_le_bytes(bytes[o as usize..o as usize + 4].try_into().unwrap_or_default()));
        let flags = get(ctx::X86_FLAGS) as u32;
        cx.turn(|t| {
            let mut regs = t.thread().regs();
            let mut fp = t.thread().fp();
            if let (Regs::X86(r), FpRegs::X86(f)) = (&mut regs, &mut fp) {
                if flags & CONTROL != 0 {
                    r.gpr[RBP] = get(ctx::X86_EBP);
                    r.rip = get(ctx::X86_EIP);
                    r.gpr[RSP] = get(ctx::X86_ESP);
                    r.rflags = r.rflags & !USER_FLAGS | get(ctx::X86_EFLAGS) & USER_FLAGS;
                }
                if flags & INTEGER != 0 {
                    for (reg, o) in [(RDI, ctx::X86_EDI), (RSI, ctx::X86_ESI), (RBX, ctx::X86_EBX), (RDX, ctx::X86_EDX), (RCX, ctx::X86_ECX), (RAX, ctx::X86_EAX)] {
                        r.gpr[reg] = get(o);
                    }
                }
                if flags & EXTENDED_REGISTERS != 0 {
                    let o = ctx::X86_EXTENDED as usize;
                    f.fxsave.copy_from_slice(&bytes[o..o + 512]);
                } else if flags & FLOATING_POINT != 0 {
                    let o = ctx::X86_FLOAT_SAVE as usize;
                    f.fxsave[0..4].copy_from_slice(&bytes[o..o + 4]);
                }
            }
            t.thread().set_regs(&regs);
            t.thread().set_fp(&fp);
        });
        self.nested = 0;
        Flow::Run
    }
}

/// A 32-bit thread's registers as x86's CONTEXT: everything, with x87's state both as `fnsave` has it (FloatSave)
/// and as `fxsave` does (ExtendedRegisters), and the segments Windows gives a 32-bit process.
fn encode_x86(regs: &Regs, fp: &FpRegs) -> Vec<u8> {
    let (Regs::X86(r), FpRegs::X86(f)) = (regs, fp) else { return Vec::new() };
    let mut b = vec![0u8; ctx::X86_SIZE as usize];
    let mut put = |o: u64, v: u32| b[o as usize..o as usize + 4].copy_from_slice(&v.to_le_bytes());
    put(ctx::X86_FLAGS, ctx::X86_FULL);
    for (o, v) in [(ctx::X86_SEG_GS, 0x2b), (ctx::X86_SEG_FS, 0x53), (ctx::X86_SEG_FS + 4, 0x2b), (ctx::X86_SEG_FS + 8, 0x2b), (ctx::X86_SEG_CS, 0x23), (ctx::X86_SEG_SS, 0x2b)] {
        put(o, v);
    }
    for (reg, o) in [(RDI, ctx::X86_EDI), (RSI, ctx::X86_ESI), (RBX, ctx::X86_EBX), (RDX, ctx::X86_EDX), (RCX, ctx::X86_ECX), (RAX, ctx::X86_EAX), (RBP, ctx::X86_EBP), (RSP, ctx::X86_ESP)] {
        put(o, r.gpr[reg] as u32);
    }
    put(ctx::X86_EIP, r.rip as u32);
    put(ctx::X86_EFLAGS, r.rflags as u32);
    // FloatSave: the control and status words, every register tagged empty, then the eight 10-byte registers.
    let save = ctx::X86_FLOAT_SAVE as usize;
    b[save..save + 4].copy_from_slice(&f.fxsave[0..4]);
    b[save + 8..save + 10].copy_from_slice(&[0xff, 0xff]);
    for i in 0..8 {
        b[save + 28 + i * 10..save + 38 + i * 10].copy_from_slice(&f.fxsave[32 + i * 16..42 + i * 16]);
    }
    let ext = ctx::X86_EXTENDED as usize;
    b[ext..ext + 512].copy_from_slice(&f.fxsave);
    b
}

/// A thread's registers as a CONTEXT.
fn encode(regs: &Regs, fp: &FpRegs) -> Vec<u8> {
    match (regs, fp) {
        (Regs::Arm64(r), FpRegs::Arm64(f)) => {
            let mut b = vec![0u8; ctx::ARM64_SIZE as usize];
            b[0..4].copy_from_slice(&ctx::ARM64_FULL.to_le_bytes());
            b[4..8].copy_from_slice(&(r.nzcv as u32).to_le_bytes());
            for (i, x) in r.x.iter().enumerate() {
                let o = (ctx::ARM64_X + i as u64 * 8) as usize;
                b[o..o + 8].copy_from_slice(&x.to_le_bytes());
            }
            b[ctx::ARM64_SP as usize..][..8].copy_from_slice(&r.sp.to_le_bytes());
            b[ctx::ARM64_PC as usize..][..8].copy_from_slice(&r.pc.to_le_bytes());
            for (i, v) in f.v.iter().enumerate() {
                let o = (ctx::ARM64_V + i as u64 * 16) as usize;
                b[o..o + 16].copy_from_slice(&v.to_le_bytes());
            }
            b[ctx::ARM64_FPCR as usize..][..4].copy_from_slice(&f.fpcr.to_le_bytes());
            b[ctx::ARM64_FPSR as usize..][..4].copy_from_slice(&f.fpsr.to_le_bytes());
            b
        }
        (Regs::X86(r), FpRegs::X86(f)) => {
            let mut b = vec![0u8; ctx::X64_SIZE as usize];
            b[ctx::X64_FLAGS as usize..][..4].copy_from_slice(&ctx::X64_FULL.to_le_bytes());
            b[ctx::X64_MXCSR as usize..][..4].copy_from_slice(&f.fxsave[24..28]);
            b[0x38..0x40].copy_from_slice(&[0x33, 0, 0x2b, 0, 0x2b, 0, 0x53, 0]); // cs, ds, es, fs
            b[0x40..0x44].copy_from_slice(&[0x2b, 0, 0x2b, 0]); // gs, ss
            b[ctx::X64_EFLAGS as usize..][..4].copy_from_slice(&(r.rflags as u32).to_le_bytes());
            for (i, x) in r.gpr.iter().enumerate() {
                let o = (ctx::X64_GPRS + i as u64 * 8) as usize;
                b[o..o + 8].copy_from_slice(&x.to_le_bytes());
            }
            b[ctx::X64_RIP as usize..][..8].copy_from_slice(&r.rip.to_le_bytes());
            b[0x100..0x300].copy_from_slice(&f.fxsave);
            b
        }
        _ => Vec::new(),
    }
}
