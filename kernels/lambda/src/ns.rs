//! Namespaces: what a process sees of the machine, held by each process and copied or swapped by `clone`, `unshare`
//! and `setns`. lambda gives a process its own mount, UTS (host and domain names) and IPC (System V objects and POSIX
//! message queues) namespace; of the PID, network, user, cgroup and time namespaces there's one, the machine's,
//! which every process shares and which `/proc/<pid>/ns` and `lsns` still show.
//!
//! Each namespace has an inode of its own, as Linux's nsfs gives it, so `/proc/<pid>/ns/<kind>` reads as
//! `<kind>:[<inode>]` and `lsns` tells them apart; two processes in one namespace share its `Arc`, so they share the
//! same inode and the same objects.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::admin::Names;
use crate::fs::mount::MountNs;
use crate::ipc::{MsgTable, SemTable};
use crate::mm::shared::ShmTable;

/// nsfs inode numbers, from where Linux's begin.
static NEXT_NS: AtomicU64 = AtomicU64::new(0xf000_0000);

pub(crate) fn next_ns_ino() -> u64 {
    NEXT_NS.fetch_add(1, Ordering::Relaxed)
}

/// nsfs's device number (`st_dev` of a `/proc/<pid>/ns/<kind>` file).
pub(crate) const NSFS_DEV: u64 = 0x4;

/// The kinds of namespace, with the `CLONE_NEW*` bit each is made by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NsKind {
    Mnt,
    Uts,
    Ipc,
    Pid,
    Net,
    User,
    Cgroup,
    Time,
}

impl NsKind {
    /// How it's named in `/proc/<pid>/ns` and in `<kind>:[<inode>]`.
    pub fn name(self) -> &'static str {
        match self {
            NsKind::Mnt => "mnt",
            NsKind::Uts => "uts",
            NsKind::Ipc => "ipc",
            NsKind::Pid => "pid",
            NsKind::Net => "net",
            NsKind::User => "user",
            NsKind::Cgroup => "cgroup",
            NsKind::Time => "time",
        }
    }

    /// Its `CLONE_NEW*` flag.
    pub fn flag(self) -> u64 {
        match self {
            NsKind::Mnt => 0x0002_0000,
            NsKind::Uts => 0x0400_0000,
            NsKind::Ipc => 0x0800_0000,
            NsKind::User => 0x1000_0000,
            NsKind::Pid => 0x2000_0000,
            NsKind::Net => 0x4000_0000,
            NsKind::Cgroup => 0x0200_0000,
            NsKind::Time => 0x0000_0080,
        }
    }

    /// Whether lambda can give a process its own: mount, UTS and IPC.
    pub fn separable(self) -> bool {
        matches!(self, NsKind::Mnt | NsKind::Uts | NsKind::Ipc)
    }

    pub const ALL: [NsKind; 8] = [NsKind::Mnt, NsKind::Uts, NsKind::Ipc, NsKind::Pid, NsKind::Net, NsKind::User, NsKind::Cgroup, NsKind::Time];
}

/// A UTS namespace: a host and a domain name of its own.
#[derive(Debug)]
pub(crate) struct UtsNs {
    pub id: u64,
    pub names: Mutex<Names>,
}

impl UtsNs {
    pub fn new(names: Names) -> Arc<UtsNs> {
        Arc::new(UtsNs { id: next_ns_ino(), names: Mutex::new(names) })
    }

    /// A copy, with its own inode: `unshare` and `clone` of a UTS namespace.
    pub fn duplicate(&self) -> Arc<UtsNs> {
        UtsNs::new(kernels_core::lock(&self.names).clone())
    }
}

/// An IPC namespace: its own System V shared memory, message queues and semaphore sets, and POSIX message queues.
#[derive(Default)]
pub(crate) struct IpcNs {
    pub id: u64,
    pub shm: Mutex<ShmTable>,
    pub msg: Mutex<MsgTable>,
    pub sem: Mutex<SemTable>,
    pub mqueues: Mutex<std::collections::BTreeMap<String, Arc<crate::ipc::mqueue::MQueue>>>,
}

impl IpcNs {
    pub fn new() -> Arc<IpcNs> {
        Arc::new(IpcNs { id: next_ns_ino(), ..IpcNs::default() })
    }
}

/// A namespace lambda keeps only one of (PID, network, user, cgroup, time): nothing of the machine but its inode.
#[derive(Debug)]
pub(crate) struct Singleton {
    pub id: u64,
}

impl Singleton {
    fn new() -> Arc<Singleton> {
        Arc::new(Singleton { id: next_ns_ino() })
    }
}

/// The namespaces a process is in.
#[derive(Clone)]
pub(crate) struct Namespaces {
    pub mnt: Arc<MountNs>,
    pub uts: Arc<UtsNs>,
    pub ipc: Arc<IpcNs>,
    pub pid: Arc<Singleton>,
    pub net: Arc<Singleton>,
    pub user: Arc<Singleton>,
    pub cgroup: Arc<Singleton>,
    pub time: Arc<Singleton>,
}

impl Namespaces {
    /// The machine's at boot: one of each.
    pub fn boot(mnt: Arc<MountNs>, names: Names) -> Namespaces {
        Namespaces { mnt, uts: UtsNs::new(names), ipc: IpcNs::new(), pid: Singleton::new(), net: Singleton::new(), user: Singleton::new(), cgroup: Singleton::new(), time: Singleton::new() }
    }

    /// A namespace's inode.
    pub fn id(&self, kind: NsKind) -> u64 {
        match kind {
            NsKind::Mnt => self.mnt.id(),
            NsKind::Uts => self.uts.id,
            NsKind::Ipc => self.ipc.id,
            NsKind::Pid => self.pid.id,
            NsKind::Net => self.net.id,
            NsKind::User => self.user.id,
            NsKind::Cgroup => self.cgroup.id,
            NsKind::Time => self.time.id,
        }
    }

    /// Gives a new namespace of each separable kind in `flags`, in place.
    pub fn unshare(&mut self, flags: u64) {
        if flags & NsKind::Mnt.flag() != 0 {
            self.mnt = self.mnt.duplicate();
        }
        if flags & NsKind::Uts.flag() != 0 {
            self.uts = self.uts.duplicate();
        }
        if flags & NsKind::Ipc.flag() != 0 {
            self.ipc = IpcNs::new();
        }
    }
}

/// Every `CLONE_NEW*` flag, and the ones lambda can act on (mount, UTS, IPC).
pub(crate) const NEW_ALL: u64 = 0x0002_0080 | 0x0400_0000 | 0x0800_0000 | 0x1000_0000 | 0x2000_0000 | 0x4000_0000 | 0x0200_0000;
pub(crate) const NEW_SUPPORTED: u64 = 0x0002_0000 | 0x0400_0000 | 0x0800_0000;

/// A reference to one namespace, as an open `/proc/<pid>/ns/<kind>` file holds it, for `setns`.
#[derive(Clone)]
pub(crate) enum NsRef {
    Mnt(Arc<MountNs>),
    Uts(Arc<UtsNs>),
    Ipc(Arc<IpcNs>),
    Single(NsKind, Arc<Singleton>),
}

impl NsRef {
    pub fn kind(&self) -> NsKind {
        match self {
            NsRef::Mnt(_) => NsKind::Mnt,
            NsRef::Uts(_) => NsKind::Uts,
            NsRef::Ipc(_) => NsKind::Ipc,
            NsRef::Single(kind, _) => *kind,
        }
    }

    pub fn id(&self) -> u64 {
        match self {
            NsRef::Mnt(ns) => ns.id(),
            NsRef::Uts(ns) => ns.id,
            NsRef::Ipc(ns) => ns.id,
            NsRef::Single(_, ns) => ns.id,
        }
    }
}

impl Namespaces {
    /// A reference to this process's namespace of `kind`, as `/proc/<pid>/ns/<kind>` gives.
    pub fn reference(&self, kind: NsKind) -> NsRef {
        match kind {
            NsKind::Mnt => NsRef::Mnt(self.mnt.clone()),
            NsKind::Uts => NsRef::Uts(self.uts.clone()),
            NsKind::Ipc => NsRef::Ipc(self.ipc.clone()),
            NsKind::Pid => NsRef::Single(kind, self.pid.clone()),
            NsKind::Net => NsRef::Single(kind, self.net.clone()),
            NsKind::User => NsRef::Single(kind, self.user.clone()),
            NsKind::Cgroup => NsRef::Single(kind, self.cgroup.clone()),
            NsKind::Time => NsRef::Single(kind, self.time.clone()),
        }
    }

    /// Joins the namespace `r` names (`setns`): for a kind lambda keeps one of, this is already it.
    pub fn join(&mut self, r: &NsRef) {
        match r {
            NsRef::Mnt(ns) => self.mnt = ns.clone(),
            NsRef::Uts(ns) => self.uts = ns.clone(),
            NsRef::Ipc(ns) => self.ipc = ns.clone(),
            NsRef::Single(..) => {}
        }
    }
}

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use crate::abi::*;
use crate::fs::Node;
use crate::process::Task;
use crate::process::creds::CAP_SYS_ADMIN;

impl Task {
    /// Validates `CLONE_NEW*` flags for `clone`: the kinds lambda can't separate are refused, and new namespaces need
    /// `CAP_SYS_ADMIN` and a new process.
    pub(crate) fn clone_namespaces(&self, flags: u64, thread: bool) -> Result<(), Errno> {
        let new = flags & NEW_ALL;
        if new == 0 {
            return Ok(());
        }
        if thread || new & !NEW_SUPPORTED != 0 {
            return Err(EINVAL);
        }
        if !lock(&self.process.state).creds.capable(CAP_SYS_ADMIN) {
            return Err(EPERM);
        }
        Ok(())
    }

    /// `unshare`: gives this process its own mount, UTS or IPC namespace.
    pub(crate) fn unshare(&mut self, flags: u64) -> Result<u64, Errno> {
        // Namespaces lambda can't separate, and the flags unshare takes besides (it already keeps each its own).
        const ACCEPTED: u64 = NEW_SUPPORTED | CLONE_FILES | CLONE_FS | 0x0004_0000 | CLONE_SIGHAND | CLONE_VM | CLONE_THREAD | 0x0008_0000;
        if flags & !ACCEPTED != 0 {
            return Err(EINVAL);
        }
        let new = flags & NEW_SUPPORTED;
        if new != 0 && !lock(&self.process.state).creds.capable(CAP_SYS_ADMIN) {
            return Err(EPERM);
        }
        lock(&self.process.state).ns.unshare(new);
        Ok(0)
    }

    /// `setns`: the namespace an fd refers to — an `/proc/<pid>/ns/<kind>` file, or a pidfd (then `nstype` says which
    /// kinds) — is joined.
    pub(crate) fn setns(&mut self, fd: i32, nstype: u64) -> Result<u64, Errno> {
        if nstype & !NEW_ALL != 0 {
            return Err(EINVAL);
        }
        let open = self.file(fd)?;
        let refs: Vec<NsRef> = match &open.node {
            Node::Ns(r) => {
                if nstype != 0 && nstype & r.kind().flag() == 0 {
                    return Err(EINVAL);
                }
                vec![r.clone()]
            }
            Node::Pidfd(process) => {
                if nstype == 0 {
                    return Err(EINVAL);
                }
                let ns = lock(&process.state).ns.clone();
                NsKind::ALL.into_iter().filter(|k| nstype & k.flag() != 0).map(|k| ns.reference(k)).collect()
            }
            _ => return Err(EINVAL),
        };
        // Joining a separable namespace needs privilege; the single ones are already the one there.
        if refs.iter().any(|r| r.kind().separable()) && !lock(&self.process.state).creds.capable(CAP_SYS_ADMIN) {
            return Err(EPERM);
        }
        let mut state = lock(&self.process.state);
        for r in &refs {
            state.ns.join(r);
        }
        Ok(0)
    }

    /// `listns`: the namespace inodes of a kind (or every kind), across the machine's processes.
    pub(crate) fn listns(&mut self, cx: &mut Cx<'_>, req_at: u64, ids_at: u64, count: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 {
            return Err(EINVAL);
        }
        // struct ns_id_req: size, then (padding and) ns_type at offset 8.
        let (size, want_type) = self.mem(cx, false, |t| Ok((user::read_u32(&t.space(), req_at)?, user::read_u32(&t.space(), req_at + 8)?)))?;
        if (size as usize) < 16 {
            return Err(EINVAL);
        }
        let kinds: Vec<NsKind> = NsKind::ALL.into_iter().filter(|k| want_type == 0 || want_type == k.clone_new_type()).collect();
        if want_type != 0 && kinds.is_empty() {
            return Err(EINVAL);
        }
        let mut ids: Vec<u64> = Vec::new();
        for process in lock(&self.k.processes).values() {
            let ns = lock(&process.state).ns.clone();
            for kind in &kinds {
                let id = ns.id(*kind);
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ids.sort_unstable();
        let n = ids.len().min(count as usize);
        let bytes: Vec<u8> = ids[..n].iter().flat_map(|id| id.to_le_bytes()).collect();
        if count > 0 {
            self.mem(cx, true, |t| user::write(&mut t.space(), ids_at, &bytes))?;
        }
        Ok(ids.len() as u64)
    }
}

impl NsKind {
    /// The `CLONE_NEWFOO`-style type `listns` names a kind by: its `CLONE_NEW*` flag.
    fn clone_new_type(self) -> u32 {
        self.flag() as u32
    }
}
