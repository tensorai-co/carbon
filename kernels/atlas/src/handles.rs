//! A process's handles: the console's, and kernel objects'. `-1` and `-2` are the pseudo-handles for the current process
//! and thread, which `Task::object` resolves.

use std::collections::BTreeMap;
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
    File(Arc<File>),
    /// A `FindFirstFile` search: what's left of what it found.
    Find(Arc<Find>),
    /// A registry key, by its ID.
    Key(u64),
}

/// `GetCurrentProcess()` and `GetCurrentThread()`.
pub const CURRENT_PROCESS: u64 = u64::MAX;
pub const CURRENT_THREAD: u64 = u64::MAX - 1;

#[derive(Default)]
pub struct Handles {
    map: BTreeMap<u64, Handle>,
}

impl Handles {
    /// Adds a handle, giving its value: a multiple of 4, as on Windows.
    pub fn add(&mut self, handle: Handle) -> u64 {
        let value = (1..).map(|i| i * 4).find(|v| !self.map.contains_key(v)).unwrap_or(4);
        self.map.insert(value, handle);
        value
    }

    pub fn get(&self, value: u64) -> Option<Handle> {
        self.map.get(&value).cloned()
    }

    /// Whether any handle is one `f` says yes to.
    pub fn any(&self, f: impl Fn(&Handle) -> bool) -> bool {
        self.map.values().any(f)
    }

    pub fn close(&mut self, value: u64) -> bool {
        self.map.remove(&value).is_some()
    }
}
