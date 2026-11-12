//! Linux's numbers and structures, as lambda needs them. Where ISAs differ (open's flags, `struct stat`), the
//! canonical values are x86-64's, and `sys` converts ARM64's at the edge.

// A table of Linux's numbers: not every one is used yet.
#![allow(dead_code)]

use exec::Isa;

/// An error number; system calls return its negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Errno(pub i32);

pub const EPERM: Errno = Errno(1);
pub const ENOENT: Errno = Errno(2);
pub const ESRCH: Errno = Errno(3);
pub const EINTR: Errno = Errno(4);
pub const EIO: Errno = Errno(5);
pub const ENOEXEC: Errno = Errno(8);
pub const EBADF: Errno = Errno(9);
pub const ECHILD: Errno = Errno(10);
pub const EAGAIN: Errno = Errno(11);
pub const ENOMEM: Errno = Errno(12);
pub const EFAULT: Errno = Errno(14);
pub const EEXIST: Errno = Errno(17);
pub const EXDEV: Errno = Errno(18);
pub const EROFS: Errno = Errno(30);
pub const ENXIO: Errno = Errno(6);
pub const ENOTDIR: Errno = Errno(20);
pub const EISDIR: Errno = Errno(21);
pub const EINVAL: Errno = Errno(22);
pub const EMFILE: Errno = Errno(24);
pub const ENOTTY: Errno = Errno(25);
pub const ESPIPE: Errno = Errno(29);
pub const EPIPE: Errno = Errno(32);
pub const ERANGE: Errno = Errno(34);
pub const ENAMETOOLONG: Errno = Errno(36);
pub const ENOSYS: Errno = Errno(38);
pub const ENOTEMPTY: Errno = Errno(39);
pub const ELOOP: Errno = Errno(40);
pub const ENODATA: Errno = Errno(61);
pub const ENOTSOCK: Errno = Errno(88);
pub const ENOTSUP: Errno = Errno(95);
pub const EAFNOSUPPORT: Errno = Errno(97);
pub const ETIMEDOUT: Errno = Errno(110);
pub const EACCES: Errno = Errno(13);
pub const ENODEV: Errno = Errno(19);
pub const EDOM: Errno = Errno(33);
pub const EDESTADDRREQ: Errno = Errno(89);
pub const EMSGSIZE: Errno = Errno(90);
pub const ENOPROTOOPT: Errno = Errno(92);
pub const EPROTONOSUPPORT: Errno = Errno(93);
pub const ESOCKTNOSUPPORT: Errno = Errno(94);
pub const EOPNOTSUPP: Errno = ENOTSUP;
pub const EADDRINUSE: Errno = Errno(98);
pub const EADDRNOTAVAIL: Errno = Errno(99);
pub const ENETUNREACH: Errno = Errno(101);
pub const ECONNABORTED: Errno = Errno(103);
pub const ECONNRESET: Errno = Errno(104);
pub const EISCONN: Errno = Errno(106);
pub const ENOTCONN: Errno = Errno(107);
pub const ECONNREFUSED: Errno = Errno(111);
pub const EHOSTUNREACH: Errno = Errno(113);
pub const EALREADY: Errno = Errno(114);
pub const EINPROGRESS: Errno = Errno(115);
pub const EFBIG: Errno = Errno(27);
pub const ENOSPC: Errno = Errno(28);
pub const EBUSY: Errno = Errno(16);
pub const E2BIG: Errno = Errno(7);
pub const ESTALE: Errno = Errno(116);
pub const EOVERFLOW: Errno = Errno(75);

impl From<layers::Error> for Errno {
    fn from(e: layers::Error) -> Errno {
        match e {
            layers::Error::NotFound => ENOENT,
            layers::Error::NotDir => ENOTDIR,
            layers::Error::IsDir => EISDIR,
            layers::Error::Exists => EEXIST,
            layers::Error::NotEmpty => ENOTEMPTY,
            layers::Error::Loop => ELOOP,
            layers::Error::BadName => EINVAL,
            layers::Error::CrossFs => EXDEV,
            layers::Error::Io => EIO,
        }
    }
}

impl From<kernels_core::user::Fault> for Errno {
    fn from(_: kernels_core::user::Fault) -> Errno {
        EFAULT
    }
}

pub const AT_FDCWD: i32 = -100;
pub const AT_SYMLINK_NOFOLLOW: u64 = 0x100;
pub const AT_REMOVEDIR: u64 = 0x200;
pub const AT_EMPTY_PATH: u64 = 0x1000;

// open's flags, as x86-64 has them.
pub const O_ACCMODE: u32 = 3;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_CREAT: u32 = 0o100;
pub const O_EXCL: u32 = 0o200;
pub const O_TRUNC: u32 = 0o1000;
pub const O_APPEND: u32 = 0o2000;
pub const O_NOCTTY: u32 = 0o400;
pub const O_NONBLOCK: u32 = 0o4000;
pub const O_DIRECT: u32 = 0o40000;
pub const O_LARGEFILE: u32 = 0o100000;
pub const O_DIRECTORY: u32 = 0o200000;
pub const O_NOFOLLOW: u32 = 0o400000;
/// A handle to the file itself, not to read or write: a symbolic link, with `O_NOFOLLOW`.
pub const O_PATH: u32 = 0o10000000;
pub const O_CLOEXEC: u32 = 0o2000000;

/// ARM64's `O_DIRECTORY`, `O_NOFOLLOW`, `O_DIRECT` and `O_LARGEFILE`, which differ from x86-64's.
const ARM64_SWAPPED: [(u32, u32); 4] = [(0o40000, O_DIRECTORY), (0o100000, O_NOFOLLOW), (0o200000, O_DIRECT), (0o400000, O_LARGEFILE)];

/// open's flags as `isa` gives them, to x86-64's.
pub fn open_flags_from(isa: Isa, flags: u32) -> u32 {
    if isa != Isa::Arm64 {
        return flags;
    }
    let moved = ARM64_SWAPPED.iter().fold(0, |m, (arm, _)| m | arm);
    ARM64_SWAPPED.iter().fold(flags & !moved, |out, (arm, x86)| if flags & arm != 0 { out | x86 } else { out })
}

/// open's flags, from x86-64's, as `isa` has them.
pub fn open_flags_to(isa: Isa, flags: u32) -> u32 {
    if isa != Isa::Arm64 {
        return flags;
    }
    let moved = ARM64_SWAPPED.iter().fold(0, |m, (_, x86)| m | x86);
    ARM64_SWAPPED.iter().fold(flags & !moved, |out, (arm, x86)| if flags & x86 != 0 { out | arm } else { out })
}

// File types, in st_mode.
pub const S_IFMT: u32 = 0o170000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFLNK: u32 = 0o120000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFBLK: u32 = 0o060000;
pub const S_IFIFO: u32 = 0o010000;
pub const S_IFSOCK: u32 = 0o140000;

// Directory entry types, for getdents64.
pub const DT_FIFO: u8 = 1;
pub const DT_CHR: u8 = 2;
pub const DT_DIR: u8 = 4;
pub const DT_REG: u8 = 8;
pub const DT_LNK: u8 = 10;

// mmap.
pub const PROT_READ: u64 = 1;
pub const PROT_WRITE: u64 = 2;
pub const PROT_EXEC: u64 = 4;
pub const MAP_SHARED: u64 = 0x01;
pub const MAP_PRIVATE: u64 = 0x02;
pub const MAP_FIXED: u64 = 0x10;
pub const MAP_ANONYMOUS: u64 = 0x20;
pub const MAP_FIXED_NOREPLACE: u64 = 0x100000;
pub const MREMAP_MAYMOVE: u64 = 1;
pub const MREMAP_FIXED: u64 = 2;
pub const MADV_DONTNEED: u64 = 4;

// clone.
pub const CSIGNAL: u64 = 0xff;
pub const CLONE_VM: u64 = 0x100;
pub const CLONE_FS: u64 = 0x200;
pub const CLONE_FILES: u64 = 0x400;
pub const CLONE_SIGHAND: u64 = 0x800;
pub const CLONE_PIDFD: u64 = 0x1000;
pub const CLONE_VFORK: u64 = 0x4000;
pub const CLONE_THREAD: u64 = 0x10000;
pub const CLONE_SETTLS: u64 = 0x80000;
pub const CLONE_PARENT_SETTID: u64 = 0x100000;
pub const CLONE_CHILD_CLEARTID: u64 = 0x200000;
pub const CLONE_CHILD_SETTID: u64 = 0x1000000;

// wait4.
pub const WNOHANG: u64 = 1;

// futex.
pub const FUTEX_WAIT: u64 = 0;
pub const FUTEX_WAKE: u64 = 1;
pub const FUTEX_REQUEUE: u64 = 3;
pub const FUTEX_CMP_REQUEUE: u64 = 4;
pub const FUTEX_WAKE_OP: u64 = 5;
pub const FUTEX_WAIT_BITSET: u64 = 9;
pub const FUTEX_WAKE_BITSET: u64 = 10;
pub const FUTEX_CMD_MASK: u64 = 0x7f;
pub const FUTEX_PRIVATE_FLAG: u64 = 128;

// Signals.
pub const SIGKILL: i32 = 9;
pub const SIGSEGV: i32 = 11;
pub const SIGILL: i32 = 4;
pub const SIGTRAP: i32 = 5;
pub const SIGBUS: i32 = 7;
pub const SIGFPE: i32 = 8;
pub const SIGCHLD: i32 = 17;

// fcntl.
pub const F_DUPFD: u64 = 0;
pub const F_GETFD: u64 = 1;
pub const F_SETFD: u64 = 2;
pub const F_GETFL: u64 = 3;
pub const F_SETFL: u64 = 4;
pub const F_DUPFD_CLOEXEC: u64 = 1030;
pub const FD_CLOEXEC: u64 = 1;

// ioctl.
pub const FIONBIO: u64 = 0x5421;

// The auxiliary vector.
pub const AT_NULL: u64 = 0;
pub const AT_PHDR: u64 = 3;
pub const AT_PHENT: u64 = 4;
pub const AT_PHNUM: u64 = 5;
pub const AT_PAGESZ: u64 = 6;
pub const AT_BASE: u64 = 7;
pub const AT_FLAGS: u64 = 8;
pub const AT_ENTRY: u64 = 9;
pub const AT_UID: u64 = 11;
pub const AT_EUID: u64 = 12;
pub const AT_GID: u64 = 13;
pub const AT_EGID: u64 = 14;
pub const AT_PLATFORM: u64 = 15;
pub const AT_HWCAP: u64 = 16;
pub const AT_CLKTCK: u64 = 17;
pub const AT_SECURE: u64 = 23;
pub const AT_SYSINFO_EHDR: u64 = 33;
pub const AT_RANDOM: u64 = 25;
pub const AT_HWCAP2: u64 = 26;
pub const AT_EXECFN: u64 = 31;

// Clocks.
pub const CLOCK_REALTIME: u64 = 0;
pub const TIMER_ABSTIME: u64 = 1;

/// What `stat` says about a file, before it's laid out for an ISA.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stat {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
    pub size: i64,
    pub blksize: i64,
    pub blocks: i64,
    /// Seconds and nanoseconds, for access, modification and change alike.
    pub time: (i64, i64),
}

impl Stat {
    /// `struct stat`, as `isa`'s `fstat` and `newfstatat` write it.
    pub fn encode(&self, isa: Isa) -> Vec<u8> {
        let mut b = Vec::with_capacity(144);
        let (sec, nsec) = self.time;
        if isa == Isa::Arm64 {
            // The generic layout: 128 bytes.
            b.extend(self.dev.to_le_bytes());
            b.extend(self.ino.to_le_bytes());
            b.extend(self.mode.to_le_bytes());
            b.extend(self.nlink.to_le_bytes());
            b.extend(self.uid.to_le_bytes());
            b.extend(self.gid.to_le_bytes());
            b.extend(self.rdev.to_le_bytes());
            b.extend(0u64.to_le_bytes());
            b.extend(self.size.to_le_bytes());
            b.extend((self.blksize as i32).to_le_bytes());
            b.extend(0i32.to_le_bytes());
            b.extend(self.blocks.to_le_bytes());
            for _ in 0..3 {
                b.extend(sec.to_le_bytes());
                b.extend(nsec.to_le_bytes());
            }
            b.extend([0; 8]);
        } else {
            // x86-64's: 144 bytes.
            b.extend(self.dev.to_le_bytes());
            b.extend(self.ino.to_le_bytes());
            b.extend(u64::from(self.nlink).to_le_bytes());
            b.extend(self.mode.to_le_bytes());
            b.extend(self.uid.to_le_bytes());
            b.extend(self.gid.to_le_bytes());
            b.extend(0u32.to_le_bytes());
            b.extend(self.rdev.to_le_bytes());
            b.extend(self.size.to_le_bytes());
            b.extend(self.blksize.to_le_bytes());
            b.extend(self.blocks.to_le_bytes());
            for _ in 0..3 {
                b.extend(sec.to_le_bytes());
                b.extend(nsec.to_le_bytes());
            }
            b.extend([0; 24]);
        }
        b
    }

    /// `struct statx`: the same on every ISA, 256 bytes.
    pub fn encode_statx(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(256);
        b.extend(0x7ffu32.to_le_bytes()); // stx_mask: the basic stats
        b.extend((self.blksize as u32).to_le_bytes());
        b.extend(0u64.to_le_bytes()); // attributes
        b.extend(self.nlink.to_le_bytes());
        b.extend(self.uid.to_le_bytes());
        b.extend(self.gid.to_le_bytes());
        b.extend((self.mode as u16).to_le_bytes());
        b.extend([0; 2]);
        b.extend(self.ino.to_le_bytes());
        b.extend((self.size as u64).to_le_bytes());
        b.extend((self.blocks as u64).to_le_bytes());
        b.extend(0u64.to_le_bytes()); // attributes_mask
        for _ in 0..4 {
            b.extend(self.time.0.to_le_bytes());
            b.extend((self.time.1 as u32).to_le_bytes());
            b.extend([0; 4]);
        }
        b.extend(((self.rdev >> 8) as u32).to_le_bytes());
        b.extend((self.rdev as u32 & 0xff).to_le_bytes());
        b.extend(((self.dev >> 8) as u32).to_le_bytes());
        b.extend((self.dev as u32 & 0xff).to_le_bytes());
        b.resize(256, 0);
        b
    }
}

/// The kernel release and version `uname` gives.
pub const RELEASE: &str = "6.12.0-carbon";
pub const VERSION: &str = "#1 SMP Carbon v2";

/// `struct utsname`: six fields of 65 bytes.
pub fn utsname(machine: &str, nodename: &str, domainname: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(390);
    for field in ["Linux", nodename, RELEASE, VERSION, machine, domainname] {
        let mut f = field.as_bytes().to_vec();
        f.resize(65, 0);
        b.extend(f);
    }
    b
}

/// The ELF machine numbers lambda runs.
pub const EM_X86_64: u16 = 62;
pub const EM_AARCH64: u16 = 183;
