//! The stub's seccomp filter: the one place the host kernel checks what a stub's own system calls may be.
//!
//! A program's calls never run on the host. With Syscall User Dispatch they arrive as `SIGSYS` before seccomp sees
//! them; without it, this filter traps them (`SECCOMP_RET_TRAP`), which is the same `SIGSYS`. Either way they reach the
//! handler. The handler's own page is the one place real calls come from, and only these, with the arguments checked
//! as far as seccomp can (it sees registers, not memory):
//!
//! - `rt_sigreturn`, the handler's way back to the program;
//! - `futex` on the mailbox's state word, waiting or waking;
//! - `arch_prctl`, setting `fs` and `gs` (where the kernel doesn't let the handler do it itself);
//! - in `fast` mode, `mmap` of guest memory (`GUEST_FD`, `MAP_SHARED | MAP_FIXED`), `mprotect` and `munmap`, below the
//!   stub.
//!
//! Anything else from the handler's page kills the stub. In `strict` mode the handler can't map at all; the trampoline
//! can, which a program can't use, as it isn't exempt from dispatch and Carbon opens the selector only while the stub
//! is stopped under `ptrace`. Calls from anywhere else trap, and so does any other architecture's (`int 0x80`).

use super::handler;

/// A classic BPF instruction.
#[derive(Clone, Copy)]
struct Insn {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

const LD_W_ABS: u16 = 0x20;
const ALU_AND_K: u16 = 0x54;
const JEQ_K: u16 = 0x15;
const JGT_K: u16 = 0x25;
const JGE_K: u16 = 0x35;
const RET_K: u16 = 0x06;

pub const RET_KILL_PROCESS: u32 = 0x8000_0000;
pub const RET_TRAP: u32 = 0x0003_0000;
pub const RET_ALLOW: u32 = 0x7fff_0000;

/// `seccomp_data`'s offsets: the call's number, its architecture, the instruction pointer's halves, and the arguments'.
const NR: u32 = 0;
const ARCH: u32 = 4;
const IP_LOW: u32 = 8;
const IP_HIGH: u32 = 12;
fn arg_low(n: u32) -> u32 {
    16 + 8 * n
}
fn arg_high(n: u32) -> u32 {
    20 + 8 * n
}

/// Where a jump goes: a label, or one of the filter's verdicts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum To {
    Next,
    Label(&'static str),
    Allow,
    Trap,
    Kill,
}

enum Op {
    Load(u32),
    And(u32),
    Jump { code: u16, k: u32, yes: To, no: To },
    Label(&'static str),
}

/// A filter written with labels, assembled with every jump resolved.
#[derive(Default)]
struct Program(Vec<Op>);

impl Program {
    fn load(&mut self, at: u32) -> &mut Self {
        self.0.push(Op::Load(at));
        self
    }
    fn and(&mut self, k: u32) -> &mut Self {
        self.0.push(Op::And(k));
        self
    }
    fn jump(&mut self, code: u16, k: u32, yes: To, no: To) -> &mut Self {
        self.0.push(Op::Jump { code, k, yes, no });
        self
    }
    fn eq(&mut self, k: u32, yes: To, no: To) -> &mut Self {
        self.jump(JEQ_K, k, yes, no)
    }
    fn label(&mut self, name: &'static str) -> &mut Self {
        self.0.push(Op::Label(name));
        self
    }

    /// Jumps to `yes` if the 64-bit value at `low`/`high` is below `bound`, else `no`.
    fn below(&mut self, low: u32, high: u32, bound: u64, yes: To, no: To, scratch: &'static str) -> &mut Self {
        self.load(high).jump(JGT_K, (bound >> 32) as u32, no, To::Next).eq((bound >> 32) as u32, To::Label(scratch), yes);
        self.label(scratch).load(low).jump(JGE_K, bound as u32, no, yes)
    }

    fn assemble(&self) -> Vec<Insn> {
        // Instructions are everything but labels; verdicts go at the end, one each.
        let mut at = std::collections::BTreeMap::new();
        let mut n = 0usize;
        for op in &self.0 {
            match op {
                Op::Label(name) => {
                    at.insert(*name, n);
                }
                _ => n += 1,
            }
        }
        let (allow, trap, kill) = (n, n + 1, n + 2);
        let mut out = Vec::new();
        for op in &self.0 {
            let here = out.len();
            let target = |to: To| -> u8 {
                let dest = match to {
                    To::Next => here + 1,
                    To::Label(name) => *at.get(name).unwrap_or_else(|| panic!("no label {name}")),
                    To::Allow => allow,
                    To::Trap => trap,
                    To::Kill => kill,
                };
                let offset = dest.checked_sub(here + 1).expect("a jump backwards");
                u8::try_from(offset).expect("a jump too far")
            };
            match *op {
                Op::Load(k) => out.push(Insn { code: LD_W_ABS, jt: 0, jf: 0, k }),
                Op::And(k) => out.push(Insn { code: ALU_AND_K, jt: 0, jf: 0, k }),
                Op::Jump { code, k, yes, no } => out.push(Insn { code, jt: target(yes), jf: target(no), k }),
                Op::Label(_) => {}
            }
        }
        out.push(Insn { code: RET_K, jt: 0, jf: 0, k: RET_ALLOW });
        out.push(Insn { code: RET_K, jt: 0, jf: 0, k: RET_TRAP });
        out.push(Insn { code: RET_K, jt: 0, jf: 0, k: RET_KILL_PROCESS });
        out
    }
}

/// Who may map guest memory into the stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapper {
    /// The handler, when Carbon asks it to (`fast`).
    Handler,
    /// The trampoline, while Carbon has the stub stopped (`strict`).
    Trampoline,
}

/// The filter, as the bytes of its instructions, for a stub at `base` whose guest memory is at `guest_fd`.
pub fn filter(base: u64, guest_fd: i32, mapper: Mapper) -> Vec<u8> {
    let handler_page = base;
    let trampoline_page = base + handler::TRAMPOLINE_PAGE;
    let state = base + handler::SHARED + handler::STATE;
    let mut p = Program::default();
    p.load(ARCH).eq(super::sys::AUDIT_ARCH_X86_64, To::Next, To::Trap);
    // From the handler's page?
    p.load(IP_HIGH).eq((handler_page >> 32) as u32, To::Next, To::Label("trampoline?"));
    p.load(IP_LOW).and(0xffff_f000).eq(handler_page as u32, To::Label("handler"), To::Label("trampoline?"));
    // From the trampoline's (strict only)?
    p.label("trampoline?");
    if mapper == Mapper::Trampoline {
        p.load(IP_HIGH).eq((trampoline_page >> 32) as u32, To::Next, To::Trap);
        p.load(IP_LOW).and(0xffff_f000).eq(trampoline_page as u32, To::Label("mapping"), To::Trap);
    } else {
        p.eq(0, To::Trap, To::Trap);
    }

    p.label("handler").load(NR);
    p.eq(libc::SYS_rt_sigreturn as u32, To::Allow, To::Next);
    p.eq(libc::SYS_futex as u32, To::Label("futex"), To::Next);
    p.eq(libc::SYS_arch_prctl as u32, To::Label("arch_prctl"), To::Next);
    if mapper == Mapper::Handler {
        p.eq(libc::SYS_mmap as u32, To::Label("mmap"), To::Next);
        p.eq(libc::SYS_mprotect as u32, To::Label("below"), To::Next);
        p.eq(libc::SYS_munmap as u32, To::Label("below"), To::Kill);
    } else {
        p.eq(0, To::Kill, To::Kill);
    }

    // futex(&mailbox.state, FUTEX_WAIT or FUTEX_WAKE, ...)
    p.label("futex").load(arg_low(0)).eq(state as u32, To::Next, To::Kill);
    p.load(arg_high(0)).eq((state >> 32) as u32, To::Next, To::Kill);
    p.load(arg_low(1)).eq(libc::FUTEX_WAIT as u32, To::Allow, To::Next).eq(libc::FUTEX_WAKE as u32, To::Allow, To::Kill);

    // arch_prctl(ARCH_SET_GS or ARCH_SET_FS, ...)
    p.label("arch_prctl").load(arg_low(0));
    p.eq(0x1001, To::Allow, To::Next).eq(0x1002, To::Allow, To::Kill);

    // The mapping calls, from the handler (fast) or the trampoline (strict).
    p.label("mapping").load(NR);
    p.eq(libc::SYS_mmap as u32, To::Label("mmap"), To::Next);
    p.eq(libc::SYS_mprotect as u32, To::Label("below"), To::Next);
    p.eq(libc::SYS_munmap as u32, To::Label("below"), To::Kill);
    // mmap(below the stub, len, prot, MAP_SHARED | MAP_FIXED, GUEST_FD, offset)
    p.label("mmap").load(arg_low(4)).eq(guest_fd as u32, To::Next, To::Kill);
    p.load(arg_low(3)).eq((libc::MAP_SHARED | libc::MAP_FIXED) as u32, To::Label("below"), To::Kill);
    // The address is below the stub.
    p.label("below");
    p.below(arg_low(0), arg_high(0), base, To::Allow, To::Kill, "below low");

    p.assemble().iter().flat_map(|i| [&i.code.to_le_bytes()[..], &[i.jt, i.jf], &i.k.to_le_bytes()].concat()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the filter the way the kernel would, for one call.
    fn run(filter: &[u8], arch: u32, nr: u32, ip: u64, args: [u64; 6]) -> u32 {
        let (mut pc, mut acc) = (0usize, 0u32);
        loop {
            let i = &filter[pc * 8..pc * 8 + 8];
            let (code, jt, jf, k) = (u16::from_le_bytes([i[0], i[1]]), usize::from(i[2]), usize::from(i[3]), u32::from_le_bytes([i[4], i[5], i[6], i[7]]));
            pc += 1;
            match code {
                LD_W_ABS => {
                    acc = match k {
                        NR => nr,
                        ARCH => arch,
                        IP_LOW => ip as u32,
                        IP_HIGH => (ip >> 32) as u32,
                        k if (16..64).contains(&k) => {
                            let arg = args[((k - 16) / 8) as usize];
                            if (k - 16) % 8 == 0 { arg as u32 } else { (arg >> 32) as u32 }
                        }
                        other => panic!("unknown offset {other}"),
                    }
                }
                ALU_AND_K => acc &= k,
                JEQ_K => pc += if acc == k { jt } else { jf },
                JGT_K => pc += if acc > k { jt } else { jf },
                JGE_K => pc += if acc >= k { jt } else { jf },
                RET_K => return k,
                other => panic!("unknown instruction {other:#x}"),
            }
        }
    }

    const BASE: u64 = 0x7fff_fffe_c000;
    const X86_64: u32 = crate::sandbox::sys::AUDIT_ARCH_X86_64;

    #[test]
    fn a_program_s_calls_trap_and_the_handler_s_are_checked() {
        let f = filter(BASE, 100, Mapper::Handler);
        let handler = BASE + 0x180;
        let state = BASE + handler::SHARED;
        let guest_map = [0x40_0000, 0x1000, 3, (libc::MAP_SHARED | libc::MAP_FIXED) as u64, 100, 0];
        // A program's calls, from anywhere but the handler's page, trap: as do the trampoline's, and int 0x80's.
        for ip in [0x40_1000, BASE + 0x1002, BASE + 0x3000, BASE - 1] {
            assert_eq!(run(&f, X86_64, 39, ip, [0; 6]), RET_TRAP, "{ip:#x}");
            assert_eq!(run(&f, X86_64, libc::SYS_mmap as u32, ip, guest_map), RET_TRAP);
        }
        assert_eq!(run(&f, crate::sandbox::sys::AUDIT_ARCH_I386, 1, handler, [0; 6]), RET_TRAP);
        // The handler's own.
        assert_eq!(run(&f, X86_64, libc::SYS_rt_sigreturn as u32, handler, [0; 6]), RET_ALLOW);
        assert_eq!(run(&f, X86_64, libc::SYS_futex as u32, handler, [state, 0, 1, 0, 0, 0]), RET_ALLOW);
        assert_eq!(run(&f, X86_64, libc::SYS_futex as u32, handler, [state, 1, 1, 0, 0, 0]), RET_ALLOW);
        assert_eq!(run(&f, X86_64, libc::SYS_futex as u32, handler, [state + 4, 0, 1, 0, 0, 0]), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_futex as u32, handler, [state, 9, 1, 0, 0, 0]), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_arch_prctl as u32, handler, [0x1002, 0, 0, 0, 0, 0]), RET_ALLOW);
        assert_eq!(run(&f, X86_64, libc::SYS_arch_prctl as u32, handler, [0x3001, 0, 0, 0, 0, 0]), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_arch_prctl as u32, handler, [0x1003, 0, 0, 0, 0, 0]), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_mmap as u32, handler, guest_map), RET_ALLOW);
        assert_eq!(run(&f, X86_64, libc::SYS_mmap as u32, handler, [0x40_0000, 0x1000, 3, libc::MAP_SHARED as u64, 100, 0]), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_mmap as u32, handler, [0x40_0000, 0x1000, 3, guest_map[3], 3, 0]), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_mmap as u32, handler, [BASE, 0x1000, 3, guest_map[3], 100, 0]), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_mprotect as u32, handler, [BASE - 0x1000, 0x1000, 7, 0, 0, 0]), RET_ALLOW);
        assert_eq!(run(&f, X86_64, libc::SYS_mprotect as u32, handler, [BASE + 0x2000, 0x1000, 7, 0, 0, 0]), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_munmap as u32, handler, [1 << 46, 0x1000, 0, 0, 0, 0]), RET_ALLOW);
        assert_eq!(run(&f, X86_64, libc::SYS_munmap as u32, handler, [1 << 47, 0x1000, 0, 0, 0, 0]), RET_KILL_PROCESS);
        // Anything else from the handler's page.
        for nr in [0, 1, 2, 56, 57, 59, 157, 317, 322] {
            assert_eq!(run(&f, X86_64, nr, handler, [0; 6]), RET_KILL_PROCESS, "call {nr}");
        }
    }

    #[test]
    fn in_strict_mode_only_the_trampoline_maps() {
        let f = filter(BASE, 100, Mapper::Trampoline);
        let guest_map = [0x40_0000, 0x1000, 3, (libc::MAP_SHARED | libc::MAP_FIXED) as u64, 100, 0];
        assert_eq!(run(&f, X86_64, libc::SYS_mmap as u32, BASE + 0x180, guest_map), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_mprotect as u32, BASE + 0x180, guest_map), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_mmap as u32, BASE + 0x1002, guest_map), RET_ALLOW);
        assert_eq!(run(&f, X86_64, 39, BASE + 0x1002, [0; 6]), RET_KILL_PROCESS);
        assert_eq!(run(&f, X86_64, libc::SYS_futex as u32, BASE + 0x180, [BASE + handler::SHARED, 0, 0, 0, 0, 0]), RET_ALLOW);
        assert_eq!(run(&f, X86_64, 39, 0x40_1000, [0; 6]), RET_TRAP);
    }
}
