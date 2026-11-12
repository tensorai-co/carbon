//! The edge: each ISA's system call numbers, mapped to one [`Sys`], with each ISA's quirks fixed here, so that
//! nothing past this file knows which ISA a call came from. x86-64's old calls (`open`, `stat`, `fork`, …) become their
//! modern `*at` forms, and `clone`'s arguments are put in ARM64's order.

mod arm64;
mod x86_64;

use exec::Isa;

use crate::abi::{AT_FDCWD, AT_REMOVEDIR, AT_SYMLINK_NOFOLLOW, CLONE_VFORK, CLONE_VM, O_CREAT, O_TRUNC, O_WRONLY, SIGCHLD, open_flags_from};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sys {
    // Files.
    Read,
    Write,
    Readv,
    Writev,
    Pread64,
    Pwrite64,
    Openat,
    Close,
    Lseek,
    Fstat,
    Newfstatat,
    Statx,
    Getdents64,
    Ioctl,
    Fcntl,
    Dup,
    Dup2,
    Dup3,
    Pipe2,
    Faccessat,
    Readlinkat,
    Mkdirat,
    Unlinkat,
    Renameat2,
    Symlinkat,
    Ftruncate,
    Statfs,
    Fstatfs,
    /// Advice about reading a file: accepted, and ignored.
    Fadvise64,
    Fsync,
    Getcwd,
    Chdir,
    Fchdir,
    Umask,
    /// Accepted, and does nothing: permissions, owners, times.
    Fchmodat,
    Fchmod,
    Fchownat,
    Fchown,
    Linkat,
    Flock,
    CloseRange,
    Utimensat,
    /// Extended attributes: none are kept yet. Setting and removing say so; listing finds none.
    Setxattr,
    Lsetxattr,
    Fsetxattr,
    Getxattr,
    Lgetxattr,
    Fgetxattr,
    Listxattr,
    Llistxattr,
    Flistxattr,
    Removexattr,
    Lremovexattr,
    Fremovexattr,
    Setxattrat,
    Getxattrat,
    Listxattrat,
    Removexattrat,
    Mknodat,
    Openat2,
    FileGetattr,
    FileSetattr,
    /// `fchmodat` with flags (`AT_SYMLINK_NOFOLLOW`, `AT_EMPTY_PATH`).
    Fchmodat2,
    /// The path form of `ftruncate`.
    Truncate,
    Fallocate,
    /// `readv` and `writev` at an offset; the `2` forms take `RWF_*` flags, and -1 for the file's own offset.
    Preadv,
    Pwritev,
    Preadv2,
    Pwritev2,
    /// Copying between files without the program's memory: `sendfile`, `copy_file_range`, `splice`, `tee`, and
    /// `vmsplice` between a pipe and memory.
    Sendfile,
    CopyFileRange,
    Splice,
    Tee,
    Vmsplice,
    /// Advice about reading ahead: checked, and nothing more.
    Readahead,
    /// The disk is one: each of these syncs it all.
    Sync,
    Syncfs,
    SyncFileRange,
    /// File handles: the disk gives none, so there are none to open.
    NameToHandleAt,
    OpenByHandleAt,
    IoprioGet,
    IoprioSet,
    /// x86-64's `utime`, with a `utimbuf` of seconds.
    Utime,
    /// x86-64's `futimesat`, with two `timeval`s (`utimes` is one from the working directory).
    Futimesat,
    /// x86-64's `getdents`, with the old `linux_dirent` layout.
    Getdents,
    // Memory.
    Mmap,
    Munmap,
    Mprotect,
    Mremap,
    Madvise,
    Brk,
    Msync,
    Mincore,
    Mlock,
    Mlock2,
    Munlock,
    Mlockall,
    Munlockall,
    Mseal,
    SetMempolicy,
    GetMempolicy,
    Mbind,
    MigratePages,
    MovePages,
    SetMempolicyHomeNode,
    /// Protection keys: none to give.
    PkeyAlloc,
    PkeyFree,
    PkeyMprotect,
    /// Shadow stacks: not supported, as on a CPU without them.
    MapShadowStack,
    ProcessMadvise,
    ProcessMrelease,
    Cachestat,
    /// Nothing swaps: EPERM.
    Swapon,
    Swapoff,
    MemfdCreate,
    MemfdSecret,
    Shmget,
    Shmat,
    Shmdt,
    Shmctl,
    Msgget,
    Msgsnd,
    Msgrcv,
    Msgctl,
    Semget,
    /// `semtimedop` (`semop` is one with no timeout).
    Semtimedop,
    Semop,
    Semctl,
    MqOpen,
    MqUnlink,
    MqTimedsend,
    MqTimedreceive,
    MqNotify,
    MqGetsetattr,
    FutexWait,
    FutexWake,
    FutexRequeue,
    FutexWaitv,
    InotifyInit1,
    InotifyAddWatch,
    InotifyRmWatch,
    IoSetup,
    IoDestroy,
    IoSubmit,
    IoCancel,
    IoGetevents,
    IoPgetevents,
    Capget,
    Capset,
    Seccomp,
    LsmGetSelfAttr,
    LsmSetSelfAttr,
    LsmListModules,
    LandlockCreateRuleset,
    LandlockAddRule,
    LandlockRestrictSelf,
    Bpf,
    PerfEventOpen,
    Sethostname,
    Setdomainname,
    Reboot,
    Syslog,
    Vhangup,
    Quotactl,
    QuotactlFd,
    KexecLoad,
    KexecFileLoad,
    InitModule,
    FinitModule,
    DeleteModule,
    Acct,
    Iopl,
    Ioperm,
    Mount,
    Umount2,
    PivotRoot,
    Chroot,
    Fchroot,
    Unshare,
    Setns,
    Listns,
    Fsopen,
    Fsconfig,
    Fsmount,
    Fspick,
    MoveMount,
    OpenTree,
    OpenTreeAttr,
    MountSetattr,
    // Processes and threads.
    Clone,
    Clone3,
    Execve,
    Exit,
    ExitGroup,
    Wait4,
    Getpid,
    Getppid,
    Gettid,
    Getuid,
    Geteuid,
    Getgid,
    Getegid,
    Getresuid,
    Getresgid,
    Getgroups,
    Setpgid,
    Getpgid,
    Setsid,
    Getsid,
    SetTidAddress,
    SetRobustList,
    Rseq,
    Futex,
    SchedYield,
    SchedGetaffinity,
    Prlimit64,
    /// `prlimit64` on this process, getting or setting.
    Getrlimit,
    Setrlimit,
    Prctl,
    ArchPrctl,
    Getrusage,
    Times,
    Waitid,
    Execveat,
    GetRobustList,
    /// Restarting a call by its own number: there's never one to restart, as lambda restarts calls where they were.
    RestartSyscall,
    Personality,
    Getpriority,
    Setpriority,
    SchedSetscheduler,
    SchedGetscheduler,
    SchedSetparam,
    SchedGetparam,
    SchedGetPriorityMax,
    SchedGetPriorityMin,
    SchedRrGetInterval,
    SchedSetattr,
    SchedGetattr,
    SchedSetaffinity,
    Getcpu,
    Kcmp,
    /// Tracing another process's threads, and reading and writing another process's memory.
    Ptrace,
    ProcessVmReadv,
    ProcessVmWritev,
    Statmount,
    Listmount,
    FanotifyInit,
    FanotifyMark,
    RemapFilePages,
    /// x86-64's old calls: a mounted file system's free space, the file system types, the LDT, and the uprobe
    /// trampolines' calls (only ever right from a trampoline, and there are none).
    Ustat,
    Sysfs,
    ModifyLdt,
    Uprobe,
    Uretprobe,
    // Signals.
    RtSigaction,
    RtSigprocmask,
    RtSigreturn,
    RtSigpending,
    RtSigsuspend,
    Sigaltstack,
    Kill,
    Tkill,
    Tgkill,
    RtSigtimedwait,
    RtSigqueueinfo,
    RtTgsigqueueinfo,
    /// x86-64's `pause`.
    Pause,
    /// x86-64's `alarm`.
    Alarm,
    Getitimer,
    Setitimer,
    TimerCreate,
    TimerSettime,
    TimerGettime,
    TimerGetoverrun,
    TimerDelete,
    /// Setting the clock: EPERM, as the machine's real time is the host's.
    ClockSettime,
    Settimeofday,
    /// Reading the clock's discipline (`adjtimex` is `clock_adjtime` on real time).
    Adjtimex,
    ClockAdjtime,
    PidfdOpen,
    PidfdSendSignal,
    PidfdGetfd,
    // Time.
    ClockGettime,
    ClockGetres,
    Gettimeofday,
    /// x86-64's `time`: the wall clock's seconds.
    Time,
    Nanosleep,
    ClockNanosleep,
    // The machine.
    Uname,
    Sysinfo,
    Getrandom,
    // Sockets.
    Socket,
    Socketpair,
    Bind,
    Listen,
    /// `accept4` (`accept` is one with no flags).
    Accept4,
    Connect,
    Getsockname,
    Getpeername,
    Sendto,
    Recvfrom,
    Setsockopt,
    Getsockopt,
    Shutdown,
    Sendmsg,
    Recvmsg,
    Sendmmsg,
    Recvmmsg,
    // Not yet: these say so quietly, as programs try them and carry on.
    Membarrier,
    // Waiting on files.
    Ppoll,
    /// x86-64's `poll`, with its timeout in milliseconds.
    Poll,
    Pselect6,
    /// x86-64's `select`, with a `timeval`.
    Select,
    /// `signalfd4` (x86-64's `signalfd` is one with no flags).
    Signalfd,
    /// `epoll_create1` (x86-64's `epoll_create` is one with no flags).
    EpollCreate1,
    EpollCtl,
    /// `epoll_pwait`, with its timeout in milliseconds (x86-64's `epoll_wait` is one with no mask).
    EpollPwait,
    /// `epoll_pwait2`, with a `timespec`.
    EpollPwait2,
    /// `eventfd2` (x86-64's `eventfd` is one with no flags).
    Eventfd2,
    TimerfdCreate,
    TimerfdSettime,
    TimerfdGettime,
    Setuid,
    Setgid,
    Setreuid,
    Setregid,
    Setresuid,
    Setresgid,
    Setgroups,
    Setfsuid,
    Setfsgid,
}

/// What a program asked for, with its arguments as every ISA would give them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Call {
    pub sys: Sys,
    pub args: [u64; 6],
}

const CWD: u64 = AT_FDCWD as i64 as u64;

/// The call `nr` with `args`, from a program in `isa`, or None if lambda doesn't know it.
pub fn decode(isa: Isa, nr: u64, args: [u64; 6]) -> Option<Call> {
    let [a, b, c, d, e, f] = args;
    let call = |sys, args| Some(Call { sys, args });
    match isa {
        Isa::Arm64 => {
            let sys = arm64::sys(nr)?;
            match sys {
                Sys::Openat => call(sys, [a, b, u64::from(open_flags_from(isa, c as u32)), d, e, f]),
                _ => call(sys, args),
            }
        }
        Isa::X86_64 => match x86_64::old(nr) {
            Some(Old::Open) => call(Sys::Openat, [CWD, a, b, c, 0, 0]),
            Some(Old::Creat) => call(Sys::Openat, [CWD, a, u64::from(O_CREAT | O_WRONLY | O_TRUNC), b, 0, 0]),
            Some(Old::Stat) => call(Sys::Newfstatat, [CWD, a, b, 0, 0, 0]),
            Some(Old::Lstat) => call(Sys::Newfstatat, [CWD, a, b, AT_SYMLINK_NOFOLLOW, 0, 0]),
            Some(Old::Access) => call(Sys::Faccessat, [CWD, a, b, 0, 0, 0]),
            Some(Old::Pipe) => call(Sys::Pipe2, [a, 0, 0, 0, 0, 0]),
            Some(Old::Fork) => call(Sys::Clone, [SIGCHLD as u64, 0, 0, 0, 0, 0]),
            Some(Old::Vfork) => call(Sys::Clone, [CLONE_VM | CLONE_VFORK | SIGCHLD as u64, 0, 0, 0, 0, 0]),
            Some(Old::Rename) => call(Sys::Renameat2, [CWD, a, CWD, b, 0, 0]),
            Some(Old::Mkdir) => call(Sys::Mkdirat, [CWD, a, b, 0, 0, 0]),
            Some(Old::Mknod) => call(Sys::Mknodat, [CWD, a, b, c, 0, 0]),
            Some(Old::Rmdir) => call(Sys::Unlinkat, [CWD, a, AT_REMOVEDIR, 0, 0, 0]),
            Some(Old::Unlink) => call(Sys::Unlinkat, [CWD, a, 0, 0, 0, 0]),
            Some(Old::Symlink) => call(Sys::Symlinkat, [a, CWD, b, 0, 0, 0]),
            Some(Old::Readlink) => call(Sys::Readlinkat, [CWD, a, b, c, 0, 0]),
            Some(Old::Chmod) => call(Sys::Fchmodat, [CWD, a, b, 0, 0, 0]),
            Some(Old::Chown) => call(Sys::Fchownat, [CWD, a, b, c, 0, 0]),
            Some(Old::Lchown) => call(Sys::Fchownat, [CWD, a, b, c, AT_SYMLINK_NOFOLLOW, 0]),
            Some(Old::Link) => call(Sys::Linkat, [CWD, a, CWD, b, 0, 0]),
            Some(Old::Getpgrp) => call(Sys::Getpgid, [0; 6]),
            Some(Old::Renameat) => call(Sys::Renameat2, [a, b, c, d, 0, 0]),
            Some(Old::Poll) => call(Sys::Poll, [a, b, c, 0, 0, 0]),
            Some(Old::Accept) => call(Sys::Accept4, [a, b, c, 0, 0, 0]),
            Some(Old::Signalfd) => call(Sys::Signalfd, [a, b, c, 0, 0, 0]),
            // A size that isn't positive is EINVAL, as flags epoll_create1 doesn't know are.
            Some(Old::EpollCreate) => call(Sys::EpollCreate1, [if a as i32 > 0 { 0 } else { u64::MAX }, 0, 0, 0, 0, 0]),
            Some(Old::EpollWait) => call(Sys::EpollPwait, [a, b, c, d, 0, 0]),
            Some(Old::Eventfd) => call(Sys::Eventfd2, [a, 0, 0, 0, 0, 0]),
            Some(Old::InotifyInit) => call(Sys::InotifyInit1, [0; 6]),
            Some(Old::Utimes) => call(Sys::Futimesat, [CWD, a, b, 0, 0, 0]),
            None => {
                let sys = x86_64::sys(nr)?;
                match sys {
                    // x86-64's clone is (flags, stack, parent_tid, child_tid, tls); ARM64's swaps the last two.
                    Sys::Clone => call(sys, [a, b, c, e, d, f]),
                    _ => call(sys, args),
                }
            }
        },
        Isa::X86_32 => None,
    }
}

/// x86-64's calls from before the `*at` forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Old {
    Open,
    Creat,
    Stat,
    Lstat,
    Access,
    Pipe,
    Fork,
    Vfork,
    Rename,
    Renameat,
    Mkdir,
    Mknod,
    Rmdir,
    Unlink,
    Symlink,
    Readlink,
    Chmod,
    Chown,
    Lchown,
    Link,
    Getpgrp,
    Poll,
    Accept,
    Signalfd,
    EpollCreate,
    EpollWait,
    Eventfd,
    InotifyInit,
    Utimes,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_call_either_isa() {
        let arm = decode(Isa::Arm64, 56, [CWD, 1, 0o40000, 0, 0, 0]).unwrap();
        let x86 = decode(Isa::X86_64, 2, [1, 0o200000, 0, 0, 0, 0]).unwrap();
        assert_eq!(arm, x86);
        let clone = decode(Isa::X86_64, 56, [1, 2, 3, 4, 5, 0]).unwrap();
        assert_eq!(clone.args, [1, 2, 3, 5, 4, 0]);
        assert_eq!(decode(Isa::Arm64, 93, [0; 6]).unwrap().sys, Sys::Exit);
        assert_eq!(decode(Isa::X86_64, 60, [0; 6]).unwrap().sys, Sys::Exit);
        assert_eq!(decode(Isa::X86_64, 201, [0; 6]).unwrap().sys, Sys::Time);
        // x86-64's old forms carry nothing past their own arguments.
        assert_eq!(decode(Isa::X86_64, 282, [3, 4, 8, 99, 0, 0]).unwrap(), decode(Isa::Arm64, 74, [3, 4, 8, 0, 0, 0]).unwrap());
        assert_eq!(decode(Isa::X86_64, 232, [3, 4, 5, 6, 7, 8]).unwrap(), decode(Isa::Arm64, 22, [3, 4, 5, 6, 0, 0]).unwrap());
        assert_eq!(decode(Isa::X86_64, 213, [0; 6]).unwrap().args[0], u64::MAX);
        assert_eq!(decode(Isa::X86_64, 43, [3, 4, 5, 6, 0, 0]).unwrap(), decode(Isa::Arm64, 242, [3, 4, 5, 0, 0, 0]).unwrap());
        // x86-64's utimes is futimesat from the working directory.
        assert_eq!(decode(Isa::X86_64, 235, [1, 2, 0, 0, 0, 0]).unwrap(), decode(Isa::X86_64, 261, [CWD, 1, 2, 0, 0, 0]).unwrap());
        let files = [(76, 45), (285, 47), (295, 69), (296, 70), (40, 71), (278, 75), (275, 76), (276, 77), (162, 81), (277, 84), (187, 213), (303, 264), (304, 265), (306, 267), (326, 285), (327, 286), (328, 287), (251, 30), (252, 31), (452, 452), (101, 117), (310, 270), (311, 271), (300, 262), (301, 263), (216, 234), (457, 457), (458, 458)];
        for (x86, arm) in [(41, 198), (42, 203), (44, 206), (45, 207), (46, 211), (47, 212), (53, 199), (55, 209), (299, 243), (307, 269)].into_iter().chain(files) {
            assert_eq!(decode(Isa::X86_64, x86, [0; 6]).unwrap().sys, decode(Isa::Arm64, arm, [0; 6]).unwrap().sys);
        }
    }
}
