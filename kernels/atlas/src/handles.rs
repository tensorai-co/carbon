//! A process's handles: the console's, and kernel objects'. `-1` and `-2` are the pseudo-handles for the current process
//! and thread, which `Task::object` resolves. A handle may be inheritable: `CreateProcess` with `bInheritHandles` gives
//! the child each of those at the same value, as Windows does.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::files::{File, Find};
use crate::objects::Object;

#[derive(Clone)]
pub enum Handle {
    /// The host's stdout or stderr.
    Console(host::Stream),
    /// The host's stdin.
    Input,
    Object(Arc<Object>),
    /// A file, a folder, a device, or one end of a pipe.
    File(Arc<File>),
    /// A `FindFirstFile` search: what's left of what it found.
    Find(Arc<Find>),
    /// A registry key, by its ID.
    Key(u64),
    /// A section: `CreateFileMapping`'s.
    Section(Arc<crate::sections::Section>),
    /// An I/O completion port.
    Port(Arc<crate::ports::Port>),
    /// An access token: a process's own (`OpenProcessToken`), or a copy (`DuplicateTokenEx`).
    Token(Arc<crate::security::Token>),
    /// A toolhelp snapshot: processes, threads and modules as they were.
    Snapshot(Arc<std::sync::Mutex<crate::os::procapi::Snapshot>>),
}

/// `GetCurrentProcess()` and `GetCurrentThread()`.
pub const CURRENT_PROCESS: u64 = u64::MAX;
pub const CURRENT_THREAD: u64 = u64::MAX - 1;
/// `GetCurrentProcessToken()`, `GetCurrentThreadToken()` and `GetCurrentThreadEffectiveToken()`, for queries.
pub const CURRENT_PROCESS_TOKEN: u64 = u64::MAX - 3;
pub const CURRENT_THREAD_EFFECTIVE_TOKEN: u64 = u64::MAX - 5;

#[derive(Default)]
pub struct Handles {
    map: BTreeMap<u64, Handle>,
    /// The handles a child gets, with `bInheritHandles`.
    inheritable: BTreeSet<u64>,
}

impl Handles {
    /// Adds a handle, giving its value: a multiple of 4, as on Windows.
    pub fn add(&mut self, handle: Handle) -> u64 {
        self.add_with(handle, false)
    }

    /// Adds a handle that a child process may inherit, or not.
    pub fn add_with(&mut self, handle: Handle, inheritable: bool) -> u64 {
        let value = (1..).map(|i| i * 4).find(|v| !self.map.contains_key(v)).unwrap_or(4);
        self.put(value, handle, inheritable);
        value
    }

    /// Puts a handle at a value: what a child's inherited handles get.
    pub fn put(&mut self, value: u64, handle: Handle, inheritable: bool) {
        self.map.insert(value, handle);
        if inheritable {
            self.inheritable.insert(value);
        } else {
            self.inheritable.remove(&value);
        }
    }

    pub fn get(&self, value: u64) -> Option<Handle> {
        self.map.get(&value).cloned()
    }

    pub fn contains(&self, value: u64) -> bool {
        self.map.contains_key(&value)
    }

    /// Every handle, with its value, in order.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &Handle)> {
        self.map.iter().map(|(v, h)| (*v, h))
    }

    /// Whether any handle is one `f` says yes to.
    pub fn any(&self, f: impl Fn(&Handle) -> bool) -> bool {
        self.map.values().any(f)
    }

    pub fn close(&mut self, value: u64) -> bool {
        self.inheritable.remove(&value);
        self.map.remove(&value).is_some()
    }

    /// `HANDLE_FLAG_INHERIT`, for `GetHandleInformation`; None for a handle that isn't there.
    pub fn inheritable(&self, value: u64) -> Option<bool> {
        self.map.contains_key(&value).then(|| self.inheritable.contains(&value))
    }

    /// `SetHandleInformation`'s `HANDLE_FLAG_INHERIT`. Whether the handle's there.
    pub fn set_inheritable(&mut self, value: u64, inheritable: bool) -> bool {
        match self.map.get(&value).cloned() {
            Some(handle) => {
                self.put(value, handle, inheritable);
                true
            }
            None => false,
        }
    }

    /// What a child inherits: each inheritable handle, with its value.
    pub fn inherited(&self) -> Handles {
        let mut child = Handles::default();
        for &value in &self.inheritable {
            if let Some(handle) = self.map.get(&value) {
                child.put(value, handle.clone(), true);
            }
        }
        child
    }

    /// Every handle, taken out: a process that's over lets go of them all at once (a pipe's other end sees it close).
    pub fn take(&mut self) -> Handles {
        std::mem::take(self)
    }
}
