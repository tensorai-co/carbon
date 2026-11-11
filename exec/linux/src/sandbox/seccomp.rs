//! The stub's seccomp filter. Syscall User Dispatch sends a program's system calls to Carbon, but the stub's own
//! trampoline is allowed through, and a program could jump to it. So the stub may only make the calls Carbon makes on
//! its behalf: `mmap`, `mprotect` and `munmap` of its own address space (whose only file is guest memory), and exit.
//! Anything else, or any other architecture's calls, kills it.

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
const RET_KILL_PROCESS: u32 = 0x8000_0000;
const RET_ALLOW: u32 = 0x7fff_0000;

/// `seccomp_data`'s offsets.
const NR: u32 = 0;
const ARCH: u32 = 4;

const ALLOWED: [u32; 5] = [libc::SYS_mmap as u32, libc::SYS_mprotect as u32, libc::SYS_munmap as u32, libc::SYS_exit as u32, libc::SYS_exit_group as u32];

/// The filter, as the bytes of its instructions.
pub fn filter() -> Vec<u8> {
    let n = ALLOWED.len() as u8;
    // 0: arch, 1: x86-64 or kill, 2: nr, 3..3+n: allowed or next, 3+n: kill, 4+n: allow
    let kill = 3 + n;
    let mut program = vec![
        Insn { code: LD_W_ABS, jt: 0, jf: 0, k: ARCH },
        Insn { code: JEQ_K, jt: 0, jf: kill - 2, k: super::sys::AUDIT_ARCH_X86_64 },
        Insn { code: LD_W_ABS, jt: 0, jf: 0, k: NR },
    ];
    for (i, nr) in ALLOWED.iter().enumerate() {
        let at = 3 + i as u8;
        program.push(Insn { code: JEQ_K, jt: kill + 1 - (at + 1), jf: 0, k: *nr });
    }
    program.push(Insn { code: RET_K, jt: 0, jf: 0, k: RET_KILL_PROCESS });
    program.push(Insn { code: RET_K, jt: 0, jf: 0, k: RET_ALLOW });
    program.iter().flat_map(|i| [&i.code.to_le_bytes()[..], &[i.jt, i.jf], &i.k.to_le_bytes()].concat()).collect()
}

/// How many instructions `filter` has.
pub fn len() -> u16 {
    ALLOWED.len() as u16 + 5
}

#[cfg(test)]
mod tests {
    /// Runs the filter the way the kernel would, for one call.
    fn run(filter: &[u8], arch: u32, nr: u32) -> u32 {
        let mut pc = 0;
        let mut acc = 0;
        loop {
            let i = &filter[pc * 8..pc * 8 + 8];
            let (code, jt, jf, k) = (u16::from_le_bytes([i[0], i[1]]), usize::from(i[2]), usize::from(i[3]), u32::from_le_bytes([i[4], i[5], i[6], i[7]]));
            pc += 1;
            match code {
                super::LD_W_ABS => acc = if k == super::NR { nr } else { arch },
                super::JEQ_K => pc += if acc == k { jt } else { jf },
                super::RET_K => return k,
                other => panic!("unknown instruction {other:#x}"),
            }
        }
    }

    #[test]
    fn allows_only_what_carbon_needs() {
        let filter = super::filter();
        assert_eq!(filter.len(), usize::from(super::len()) * 8);
        let x86_64 = crate::sandbox::sys::AUDIT_ARCH_X86_64;
        for nr in super::ALLOWED {
            assert_eq!(run(&filter, x86_64, nr), super::RET_ALLOW);
        }
        // read, write, open, clone, fork, execve, prctl, seccomp, execveat, and x32's mmap.
        for nr in [0, 1, 2, 56, 57, 59, 157, 317, 322, 0x4000_0009] {
            assert_eq!(run(&filter, x86_64, nr), super::RET_KILL_PROCESS, "call {nr}");
        }
        assert_eq!(run(&filter, crate::sandbox::sys::AUDIT_ARCH_I386, 9), super::RET_KILL_PROCESS);
    }
}
