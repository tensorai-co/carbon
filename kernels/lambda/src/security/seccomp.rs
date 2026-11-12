//! seccomp: strict mode, and filters in classic BPF that look at each system call before lambda does, as Chrome,
//! Firefox, systemd's services and container runtimes sandbox themselves.
//!
//! A filter is checked as it's installed (every jump in range, a return at the end, loads only of `seccomp_data`'s
//! words, nothing Linux's checker refuses) and run on `struct seccomp_data`: the call's number as the program's ISA
//! numbers it, the audit arch, the address after the call and its six arguments. Every filter runs, newest first, and
//! the most restrictive answer wins: kill the process, kill the thread, SIGSYS, an errno, or allow (logging, or with
//! no tracer or listener for `SECCOMP_RET_TRACE` and `SECCOMP_RET_USER_NOTIF`, ENOSYS).
//!
//! Filters are a process's, not each thread's: one installed by any thread holds for all of them, as with
//! `SECCOMP_FILTER_FLAG_TSYNC`. They're inherited by children and kept through `execve`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use exec::Isa;
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use crate::abi::*;
use crate::process::Task;
use crate::process::creds::CAP_SYS_ADMIN;

const SECCOMP_SET_MODE_STRICT: u64 = 0;
const SECCOMP_SET_MODE_FILTER: u64 = 1;
const SECCOMP_GET_ACTION_AVAIL: u64 = 2;
const SECCOMP_GET_NOTIF_SIZES: u64 = 3;
const SECCOMP_FILTER_FLAG_TSYNC: u64 = 1;
const SECCOMP_FILTER_FLAG_LOG: u64 = 2;
const SECCOMP_FILTER_FLAG_SPEC_ALLOW: u64 = 4;
const SECCOMP_FILTER_FLAG_TSYNC_ESRCH: u64 = 16;

pub(crate) const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
pub(crate) const SECCOMP_RET_KILL_THREAD: u32 = 0;
pub(crate) const SECCOMP_RET_TRAP: u32 = 0x0003_0000;
pub(crate) const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
pub(crate) const SECCOMP_RET_USER_NOTIF: u32 = 0x7fc0_0000;
pub(crate) const SECCOMP_RET_TRACE: u32 = 0x7ff0_0000;
pub(crate) const SECCOMP_RET_LOG: u32 = 0x7ffc_0000;
pub(crate) const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_RET_ACTION_FULL: u32 = 0xffff_0000;
const SECCOMP_RET_DATA: u32 = 0xffff;
/// The most instructions one filter has, and all of a process's filters together (each costing 4 more).
const BPF_MAXINSNS: usize = 4096;
const MAX_INSNS_PER_PATH: usize = 32768;
/// `struct seccomp_data`'s size, and the scratch memory's.
const DATA_SIZE: u32 = 64;
const MEMWORDS: u32 = 16;
/// The highest errno an `SECCOMP_RET_ERRNO` gives.
const MAX_ERRNO: u32 = 4095;

/// `AUDIT_ARCH_X86_64` and `AUDIT_ARCH_AARCH64`.
pub(crate) fn audit_arch(isa: Isa) -> u32 {
    match isa {
        Isa::Arm64 => 0xc000_00b7,
        _ => 0xc000_003e,
    }
}

/// One `struct sock_filter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Insn {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

/// The classic BPF opcodes seccomp takes.
mod op {
    pub const LD_W_ABS: u16 = 0x20;
    pub const LD_W_LEN: u16 = 0x80;
    pub const LDX_W_LEN: u16 = 0x81;
    pub const LD_IMM: u16 = 0x00;
    pub const LDX_IMM: u16 = 0x01;
    pub const LD_MEM: u16 = 0x60;
    pub const LDX_MEM: u16 = 0x61;
    pub const ST: u16 = 0x02;
    pub const STX: u16 = 0x03;
    pub const RET_K: u16 = 0x06;
    pub const RET_A: u16 = 0x16;
    pub const TAX: u16 = 0x07;
    pub const TXA: u16 = 0x87;
    pub const JA: u16 = 0x05;
    pub const ALU: u16 = 0x04;
    pub const JMP: u16 = 0x05;
    pub const SRC_X: u16 = 0x08;
}

/// A checked filter.
#[derive(Debug)]
pub(crate) struct Filter {
    insns: Vec<Insn>,
    /// `SECCOMP_FILTER_FLAG_LOG`: log what it doesn't allow (to lambda's trace).
    pub log: bool,
}

impl Filter {
    /// Checks a program as Linux's `bpf_check_classic` and `seccomp_check_filter` do.
    pub fn check(insns: Vec<Insn>, log: bool) -> Result<Filter, Errno> {
        let len = insns.len();
        if len == 0 || len > BPF_MAXINSNS {
            return Err(EINVAL);
        }
        for (pc, i) in insns.iter().enumerate() {
            let class = i.code & 7;
            let ok = match i.code {
                op::LD_W_ABS => i.k < DATA_SIZE && i.k.is_multiple_of(4),
                op::LD_W_LEN | op::LDX_W_LEN | op::LD_IMM | op::LDX_IMM | op::RET_K | op::RET_A | op::TAX | op::TXA => true,
                op::LD_MEM | op::LDX_MEM | op::ST | op::STX => i.k < MEMWORDS,
                op::JA => pc + 1 + (i.k as usize) < len,
                c if class == op::ALU => {
                    let x = c & op::SRC_X != 0;
                    match c & 0xf0 {
                        // ADD SUB MUL OR AND XOR
                        0x00 | 0x10 | 0x20 | 0x40 | 0x50 | 0xa0 => true,
                        // DIV MOD: not by a constant 0.
                        0x30 | 0x90 => x || i.k != 0,
                        // LSH RSH: not by a constant 32 or more.
                        0x60 | 0x70 => x || i.k < 32,
                        // NEG
                        0x80 => !x,
                        _ => false,
                    }
                }
                c if class == op::JMP => matches!(c & 0xf0, 0x10..=0x40) && pc + 1 + (i.jt as usize) < len && pc + 1 + (i.jf as usize) < len,
                _ => false,
            };
            if !ok {
                return Err(EINVAL);
            }
        }
        if !matches!(insns[len - 1].code, op::RET_K | op::RET_A) {
            return Err(EINVAL);
        }
        Ok(Filter { insns, log })
    }

    /// Runs it on `struct seccomp_data`, as sixteen words.
    pub fn run(&self, data: &[u32; 16]) -> u32 {
        let (mut a, mut x, mut mem, mut pc) = (0u32, 0u32, [0u32; MEMWORDS as usize], 0usize);
        while let Some(i) = self.insns.get(pc) {
            pc += 1;
            match i.code {
                op::LD_W_ABS => a = data[(i.k / 4) as usize],
                op::LD_W_LEN => a = DATA_SIZE,
                op::LDX_W_LEN => x = DATA_SIZE,
                op::LD_IMM => a = i.k,
                op::LDX_IMM => x = i.k,
                op::LD_MEM => a = mem[i.k as usize],
                op::LDX_MEM => x = mem[i.k as usize],
                op::ST => mem[i.k as usize] = a,
                op::STX => mem[i.k as usize] = x,
                op::RET_K => return i.k,
                op::RET_A => return a,
                op::TAX => x = a,
                op::TXA => a = x,
                op::JA => pc += i.k as usize,
                c if c & 7 == op::ALU => {
                    let v = if c & op::SRC_X != 0 { x } else { i.k };
                    a = match c & 0xf0 {
                        0x00 => a.wrapping_add(v),
                        0x10 => a.wrapping_sub(v),
                        0x20 => a.wrapping_mul(v),
                        // Dividing by an X of 0 ends the program, returning 0.
                        0x30 | 0x90 if v == 0 => return 0,
                        0x30 => a / v,
                        0x90 => a % v,
                        0x40 => a | v,
                        0x50 => a & v,
                        0x60 => a.checked_shl(v).unwrap_or(0),
                        0x70 => a.checked_shr(v).unwrap_or(0),
                        0x80 => a.wrapping_neg(),
                        _ => a ^ v,
                    };
                }
                c => {
                    let v = if c & op::SRC_X != 0 { x } else { i.k };
                    let taken = match c & 0xf0 {
                        0x10 => a == v,
                        0x20 => a > v,
                        0x30 => a >= v,
                        _ => a & v != 0,
                    };
                    pc += usize::from(if taken { i.jt } else { i.jf });
                }
            }
        }
        // Can't be reached: a checked program ends in a return.
        SECCOMP_RET_KILL_PROCESS
    }
}

/// A process's seccomp mode: strict, or its filters, newest first.
#[derive(Debug, Default)]
pub(crate) struct Seccomp {
    pub strict: bool,
    pub filters: Vec<Arc<Filter>>,
}

impl Seccomp {
    /// What `PR_GET_SECCOMP` says: 0, 1 (strict) or 2 (filters).
    pub fn mode(this: &Option<Arc<Seccomp>>) -> u64 {
        match this {
            None => 0,
            Some(s) if s.strict => 1,
            Some(_) => 2,
        }
    }

    fn instructions(&self) -> usize {
        self.filters.iter().map(|f| f.insns.len() + 4).sum()
    }
}

/// What a filtered call comes to.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Verdict {
    Allow,
    /// Not done; this is its result.
    Return(Errno),
    /// SIGSYS for it, with this as its `si_errno`.
    Trap(u16),
    KillThread,
    KillProcess,
    /// Strict mode's answer to anything else: SIGKILL.
    Sigkill,
}

/// What `seccomp` answers for a call: the most restrictive of every filter's answer, as Linux compares them.
pub(crate) fn judge(seccomp: &Seccomp, isa: Isa, nr: u64, pc: u64, args: &[u64; 6]) -> (Verdict, bool) {
    if seccomp.strict {
        let allowed: [u64; 4] = match isa {
            Isa::Arm64 => [63, 64, 93, 139],
            _ => [0, 1, 60, 15],
        };
        // Strict mode's other answer is SIGKILL.
        return (if allowed.contains(&nr) { Verdict::Allow } else { Verdict::Sigkill }, true);
    }
    let mut data = [0u32; 16];
    data[0] = nr as u32;
    data[1] = audit_arch(isa);
    data[2] = pc as u32;
    data[3] = (pc >> 32) as u32;
    for (i, arg) in args.iter().enumerate() {
        data[4 + i * 2] = *arg as u32;
        data[5 + i * 2] = (*arg >> 32) as u32;
    }
    let (mut ret, mut log) = (SECCOMP_RET_ALLOW, false);
    for filter in &seccomp.filters {
        let got = filter.run(&data);
        if ((got & SECCOMP_RET_ACTION_FULL) as i32) < ((ret & SECCOMP_RET_ACTION_FULL) as i32) {
            ret = got;
            log = filter.log;
        }
    }
    let data = ret & SECCOMP_RET_DATA;
    let verdict = match ret & SECCOMP_RET_ACTION_FULL {
        SECCOMP_RET_ALLOW => return (Verdict::Allow, false),
        SECCOMP_RET_LOG => return (Verdict::Allow, true),
        SECCOMP_RET_ERRNO => Verdict::Return(Errno(data.min(MAX_ERRNO) as i32)),
        // No tracer, no listener: the call fails as there's no one to decide it.
        SECCOMP_RET_TRACE | SECCOMP_RET_USER_NOTIF => Verdict::Return(ENOSYS),
        SECCOMP_RET_TRAP => Verdict::Trap(data as u16),
        SECCOMP_RET_KILL_THREAD => Verdict::KillThread,
        // KILL_PROCESS, and anything Linux doesn't know, which it treats as that.
        _ => Verdict::KillProcess,
    };
    (verdict, log || matches!(verdict, Verdict::KillThread | Verdict::KillProcess))
}

impl Task {
    /// `seccomp`.
    pub(crate) fn seccomp(&mut self, cx: &mut Cx<'_>, operation: u64, flags: u64, args: u64) -> Result<u64, Errno> {
        match operation {
            SECCOMP_SET_MODE_STRICT => {
                if flags != 0 || args != 0 {
                    return Err(EINVAL);
                }
                self.set_seccomp(None)
            }
            SECCOMP_SET_MODE_FILTER => {
                let known = SECCOMP_FILTER_FLAG_TSYNC | SECCOMP_FILTER_FLAG_LOG | SECCOMP_FILTER_FLAG_SPEC_ALLOW | SECCOMP_FILTER_FLAG_TSYNC_ESRCH;
                // `SECCOMP_FILTER_FLAG_NEW_LISTENER` (user notification) isn't here.
                if flags & !known != 0 {
                    return Err(EINVAL);
                }
                let filter = self.read_filter(cx, args, flags & SECCOMP_FILTER_FLAG_LOG != 0)?;
                self.set_seccomp(Some(filter))
            }
            SECCOMP_GET_ACTION_AVAIL => {
                if flags != 0 {
                    return Err(EINVAL);
                }
                let action = self.mem(cx, false, |t| user::read_u32(&t.space(), args))?;
                match action {
                    SECCOMP_RET_KILL_PROCESS | SECCOMP_RET_KILL_THREAD | SECCOMP_RET_TRAP | SECCOMP_RET_ERRNO | SECCOMP_RET_TRACE | SECCOMP_RET_LOG | SECCOMP_RET_ALLOW => Ok(0),
                    _ => Err(EOPNOTSUPP),
                }
            }
            SECCOMP_GET_NOTIF_SIZES => {
                if flags != 0 {
                    return Err(EINVAL);
                }
                // struct seccomp_notif, seccomp_notif_resp and seccomp_data.
                let sizes: Vec<u8> = [80u16, 24, 64].iter().flat_map(|s| s.to_le_bytes()).collect();
                self.mem(cx, true, |t| user::write(&mut t.space(), args, &sizes))?;
                Ok(0)
            }
            _ => Err(EINVAL),
        }
    }

    /// `struct sock_fprog` at `at`: its length and its instructions, checked.
    pub(crate) fn read_filter(&mut self, cx: &mut Cx<'_>, at: u64, log: bool) -> Result<Filter, Errno> {
        let (len, insns_at) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), at)? as u16, user::read_u64(&t.space(), at + 8)?)))?;
        if len == 0 || usize::from(len) > BPF_MAXINSNS {
            return Err(EINVAL);
        }
        let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), insns_at, usize::from(len) * 8))?;
        let insns = bytes
            .chunks(8)
            .map(|b| Insn { code: u16::from_le_bytes([b[0], b[1]]), jt: b[2], jf: b[3], k: u32::from_le_bytes([b[4], b[5], b[6], b[7]]) })
            .collect();
        Filter::check(insns, log)
    }

    /// Strict mode (`None`), or one more filter: only with `no_new_privs` or `CAP_SYS_ADMIN`, and never a change of
    /// mode.
    pub(crate) fn set_seccomp(&mut self, filter: Option<Filter>) -> Result<u64, Errno> {
        let mut state = lock(&self.process.state);
        let current = state.security.seccomp.clone();
        let next = match (filter, current) {
            (None, None) => Seccomp { strict: true, filters: Vec::new() },
            (None, Some(s)) if s.strict => return Ok(0),
            (Some(_), Some(s)) if s.strict => return Err(EINVAL),
            (None, Some(_)) => return Err(EINVAL),
            (Some(filter), current) => {
                if !state.security.no_new_privs && !state.creds.capable(CAP_SYS_ADMIN) {
                    return Err(EACCES);
                }
                let mut filters = vec![Arc::new(filter)];
                filters.extend(current.iter().flat_map(|s| s.filters.iter().cloned()));
                let next = Seccomp { strict: false, filters };
                if next.instructions() > MAX_INSNS_PER_PATH {
                    return Err(ENOMEM);
                }
                next
            }
        };
        state.security.seccomp = Some(Arc::new(next));
        self.process.filtered.store(true, Ordering::Release);
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insn(code: u16, jt: u8, jf: u8, k: u32) -> Insn {
        Insn { code, jt, jf, k }
    }

    /// Allows everything but `nr` 39 on x86-64, which gets EPERM; anything not x86-64 is killed.
    fn sample() -> Vec<Insn> {
        vec![
            insn(op::LD_W_ABS, 0, 0, 4),
            insn(0x15, 1, 0, 0xc000_003e),
            insn(op::RET_K, 0, 0, SECCOMP_RET_KILL_PROCESS),
            insn(op::LD_W_ABS, 0, 0, 0),
            insn(0x15, 0, 1, 39),
            insn(op::RET_K, 0, 0, SECCOMP_RET_ERRNO | 1),
            insn(op::RET_K, 0, 0, SECCOMP_RET_ALLOW),
        ]
    }

    #[test]
    fn checks_what_linux_checks() {
        assert!(Filter::check(sample(), false).is_ok());
        assert_eq!(Filter::check(Vec::new(), false).err(), Some(EINVAL));
        // A jump past the end, a load past seccomp_data, an unaligned load, no return at the end, a division by 0.
        for bad in [
            vec![insn(0x15, 5, 0, 1), insn(op::RET_K, 0, 0, 0)],
            vec![insn(op::LD_W_ABS, 0, 0, 64), insn(op::RET_K, 0, 0, 0)],
            vec![insn(op::LD_W_ABS, 0, 0, 2), insn(op::RET_K, 0, 0, 0)],
            vec![insn(op::LD_IMM, 0, 0, 1)],
            vec![insn(0x34, 0, 0, 0), insn(op::RET_K, 0, 0, 0)],
            vec![insn(0x28, 0, 0, 0), insn(op::RET_K, 0, 0, 0)],
        ] {
            assert_eq!(Filter::check(bad, false).err(), Some(EINVAL));
        }
    }

    #[test]
    fn the_most_restrictive_answer_wins() {
        let allow_all = Arc::new(Filter::check(vec![insn(op::RET_K, 0, 0, SECCOMP_RET_ALLOW)], false).unwrap());
        let sample = Arc::new(Filter::check(sample(), false).unwrap());
        let s = Seccomp { strict: false, filters: vec![allow_all, sample] };
        let args = [0; 6];
        assert!(matches!(judge(&s, Isa::X86_64, 39, 0, &args).0, Verdict::Return(EPERM)));
        assert!(matches!(judge(&s, Isa::X86_64, 40, 0, &args).0, Verdict::Allow));
        assert!(matches!(judge(&s, Isa::Arm64, 40, 0, &args).0, Verdict::KillProcess));
        // Arithmetic, scratch memory and X: (nr * 3) stored, loaded into X, compared.
        let math = Filter::check(
            vec![
                insn(op::LD_W_ABS, 0, 0, 0),
                insn(0x24, 0, 0, 3),
                insn(op::ST, 0, 0, 5),
                insn(op::LDX_MEM, 0, 0, 5),
                insn(op::LD_IMM, 0, 0, 120),
                insn(0x1d, 0, 1, 0),
                insn(op::RET_K, 0, 0, SECCOMP_RET_TRAP | 7),
                insn(op::RET_K, 0, 0, SECCOMP_RET_ALLOW),
            ],
            false,
        )
        .unwrap();
        let s = Seccomp { strict: false, filters: vec![Arc::new(math)] };
        assert!(matches!(judge(&s, Isa::Arm64, 40, 0, &args).0, Verdict::Trap(7)));
        assert!(matches!(judge(&s, Isa::Arm64, 41, 0, &args).0, Verdict::Allow));
    }
}
