//! The stub's seccomp filter, the last of its locks. A program's system calls never reach the host at all: Syscall User
//! Dispatch, with no part of the stub exempt, sends every one to Carbon, as `PTRACE_SYSEMU` does where there's no
//! dispatch. Carbon's own calls on the stub's behalf go through its trampoline, so the filter allows only those: `mmap`,
//! `mprotect` and `munmap` of the stub's own address space (whose only file is guest memory) and exit, made from the
//! trampoline's own `syscall`. Anything else, from anywhere else, or any other architecture's calls, kills it; or,
//! where calls are caught with `PTRACE_SYSEMU`, is handed to Carbon (`SECCOMP_RET_TRACE`): Linux stops a call for
//! `PTRACE_SYSEMU` before seccomp sees it, but gVisor asks seccomp first, so there a program's every call comes this
//! way, as a `ptrace` seccomp event, which Carbon reads as the call it is.

/// A classic BPF instruction.
#[derive(Clone, Copy)]
struct Insn {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

const LD_W_ABS: u16 = 0x20;
const JEQ_K: u16 = 0x15;
const RET_K: u16 = 0x06;
pub const RET_KILL_PROCESS: u32 = 0x8000_0000;
/// Stops the stub for its tracer, with `PTRACE_O_TRACESECCOMP`.
pub const RET_TRACE: u32 = 0x7ff0_0000;
const RET_ALLOW: u32 = 0x7fff_0000;

/// `seccomp_data`'s offsets: the call's number, its architecture, and the instruction pointer's two halves.
const NR: u32 = 0;
const ARCH: u32 = 4;
const IP_LOW: u32 = 8;
const IP_HIGH: u32 = 12;

const ALLOWED: [u32; 5] = [libc::SYS_mmap as u32, libc::SYS_mprotect as u32, libc::SYS_munmap as u32, libc::SYS_exit as u32, libc::SYS_exit_group as u32];

/// The instructions before the allowed numbers: arch and its check, both halves of the address and their checks, nr.
const HEAD: usize = 7;

/// The filter, as the bytes of its instructions, for a trampoline whose `syscall` returns to `after`, answering
/// `otherwise` for every call it doesn't allow.
pub fn filter(after: u64, otherwise: u32) -> Vec<u8> {
    let n = ALLOWED.len();
    let kill = (HEAD + n) as u8;
    // Jumps count from the next instruction.
    let to_kill = |at: usize| kill - (at as u8 + 1);
    let mut program = vec![
        Insn { code: LD_W_ABS, jt: 0, jf: 0, k: ARCH },
        Insn { code: JEQ_K, jt: 0, jf: to_kill(1), k: super::sys::AUDIT_ARCH_X86_64 },
        Insn { code: LD_W_ABS, jt: 0, jf: 0, k: IP_LOW },
        Insn { code: JEQ_K, jt: 0, jf: to_kill(3), k: after as u32 },
        Insn { code: LD_W_ABS, jt: 0, jf: 0, k: IP_HIGH },
        Insn { code: JEQ_K, jt: 0, jf: to_kill(5), k: (after >> 32) as u32 },
        Insn { code: LD_W_ABS, jt: 0, jf: 0, k: NR },
    ];
    for (i, nr) in ALLOWED.iter().enumerate() {
        let at = HEAD + i;
        program.push(Insn { code: JEQ_K, jt: to_kill(at) + 1, jf: 0, k: *nr });
    }
    program.push(Insn { code: RET_K, jt: 0, jf: 0, k: otherwise });
    program.push(Insn { code: RET_K, jt: 0, jf: 0, k: RET_ALLOW });
    program.iter().flat_map(|i| [&i.code.to_le_bytes()[..], &[i.jt, i.jf], &i.k.to_le_bytes()].concat()).collect()
}

/// How many instructions `filter` has.
pub fn len() -> u16 {
    (HEAD + ALLOWED.len() + 2) as u16
}

#[cfg(test)]
mod tests {
    /// Runs the filter the way the kernel would, for one call.
    fn run(filter: &[u8], arch: u32, nr: u32, ip: u64) -> u32 {
        let mut pc = 0;
        let mut acc = 0;
        loop {
            let i = &filter[pc * 8..pc * 8 + 8];
            let (code, jt, jf, k) = (u16::from_le_bytes([i[0], i[1]]), usize::from(i[2]), usize::from(i[3]), u32::from_le_bytes([i[4], i[5], i[6], i[7]]));
            pc += 1;
            match code {
                super::LD_W_ABS => {
                    acc = match k {
                        super::NR => nr,
                        super::ARCH => arch,
                        super::IP_LOW => ip as u32,
                        super::IP_HIGH => (ip >> 32) as u32,
                        other => panic!("unknown offset {other}"),
                    }
                }
                super::JEQ_K => pc += if acc == k { jt } else { jf },
                super::RET_K => return k,
                other => panic!("unknown instruction {other:#x}"),
            }
        }
    }

    #[test]
    fn allows_only_what_carbon_needs_from_where_it_makes_it() {
        let after = 0x7fff_ffff_d07a;
        let filter = super::filter(after, super::RET_KILL_PROCESS);
        assert_eq!(filter.len(), usize::from(super::len()) * 8);
        let x86_64 = crate::sandbox::sys::AUDIT_ARCH_X86_64;
        for nr in super::ALLOWED {
            assert_eq!(run(&filter, x86_64, nr, after), super::RET_ALLOW);
            // The same call from anywhere else, as a program's jump into the middle of something.
            assert_eq!(run(&filter, x86_64, nr, after + 0x1000), super::RET_KILL_PROCESS);
            assert_eq!(run(&filter, x86_64, nr, after ^ (1 << 40)), super::RET_KILL_PROCESS);
        }
        // read, write, open, clone, fork, execve, prctl, seccomp, execveat, and x32's mmap.
        for nr in [0, 1, 2, 56, 57, 59, 157, 317, 322, 0x4000_0009] {
            assert_eq!(run(&filter, x86_64, nr, after), super::RET_KILL_PROCESS, "call {nr}");
        }
        assert_eq!(run(&filter, crate::sandbox::sys::AUDIT_ARCH_I386, 9, after), super::RET_KILL_PROCESS);
        // Caught with PTRACE_SYSEMU: the same, but handed to Carbon.
        let filter = super::filter(after, super::RET_TRACE);
        assert_eq!(run(&filter, x86_64, libc::SYS_mmap as u32, after), super::RET_ALLOW);
        assert_eq!(run(&filter, x86_64, 0, after), super::RET_TRACE);
        assert_eq!(run(&filter, x86_64, libc::SYS_mmap as u32, after + 8), super::RET_TRACE);
    }
}
