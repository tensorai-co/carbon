//! Every system DLL, as a submodule: its `dll!` declaration, and the host Rust behind its host exports. atlas makes
//! each DLL's image from its declaration (`images`), and nothing is ever a file of its own.

pub mod advapi32;
pub mod comctl32;
pub mod fileapi;
pub mod gdi32;
pub mod ioapi;
pub mod kernel32;
pub mod memapi;
pub mod nlsapi;
pub mod ole32;
pub mod pipeapi;
pub mod procapi;
pub mod resapi;
pub mod secapi;
pub mod shell32;
pub mod synch;
pub mod sysapi;
pub mod ucrtbase;
pub mod user32;
pub mod version;

use kernels_core::thread::Cx;

use crate::process::Task;

/// What a host export gives back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// Its return value.
    Value(u64),
    /// The process is ending, with this exit code (`ExitProcess`).
    ExitProcess(u32),
    /// This thread is ending (`ExitThread`).
    ExitThread(u32),
    /// This thread or its process is ending already, for some other reason.
    Ended,
}

/// A host export's body: the calling task, its CPU, and the arguments as the program passed them.
pub type HostFn = fn(&mut Task, &mut Cx<'_>, &[u64]) -> Reply;

#[derive(Clone, Copy)]
pub enum Kind {
    /// Host Rust, reached through a trap stub, taking this many arguments.
    Host { args: usize, run: HostFn },
    /// Code in `atlas-guest` (`ntdll`), which the export forwards to.
    Guest(&'static str),
    /// Not written yet: a stub that says so.
    Missing,
}

pub struct Export {
    pub name: &'static str,
    pub kind: Kind,
}

pub struct Dll {
    /// Its file name, as Windows has it.
    pub name: &'static str,
    pub exports: &'static [Export],
    /// Its first export's ordinal, the rest following in order: 1, unless programs import its first by an ordinal of
    /// Windows' own (comctl32's InitCommonControls is #17).
    pub first_ordinal: u16,
}

/// Declares a DLL: each export is `host Name(args) => function`, `guest Name => NtdllName` (or just `guest Name`), or
/// `missing Name`. `"NAME.dll" @ 17;` numbers its exports from 17.
macro_rules! dll {
    ($file:literal $(@ $first:literal)?; $($kind:ident $name:ident $(($args:literal))? $(=> $target:ident)?;)*) => {
        pub static DLL: $crate::os::Dll = $crate::os::Dll {
            name: $file,
            exports: &[$(dll!(@export $kind $name $(($args))? $(=> $target)?)),*],
            first_ordinal: dll!(@first $($first)?),
        };
    };
    (@first) => { 1 };
    (@first $first:literal) => { $first };
    (@export host $name:ident ($args:literal) => $run:ident) => {
        $crate::os::Export { name: stringify!($name), kind: $crate::os::Kind::Host { args: $args, run: $run } }
    };
    (@export guest $name:ident => $target:ident) => {
        $crate::os::Export { name: stringify!($name), kind: $crate::os::Kind::Guest(stringify!($target)) }
    };
    (@export guest $name:ident) => {
        $crate::os::Export { name: stringify!($name), kind: $crate::os::Kind::Guest(stringify!($name)) }
    };
    (@export missing $name:ident) => {
        $crate::os::Export { name: stringify!($name), kind: $crate::os::Kind::Missing }
    };
}
pub(crate) use dll;

/// Every system DLL atlas has.
pub static DLLS: &[&Dll] = &[&kernel32::DLL, &user32::DLL, &advapi32::DLL, &gdi32::DLL, &shell32::DLL, &version::DLL, &ucrtbase::DLL, &comctl32::DLL, &ole32::DLL];

/// The DLL called `name` (`kernel32`, `KERNEL32.DLL`, …), by its index in [`DLLS`].
pub fn find(name: &str) -> Option<usize> {
    let name = name.to_ascii_lowercase();
    let name = name.strip_suffix(".dll").unwrap_or(&name);
    DLLS.iter().position(|d| d.name.to_ascii_lowercase().strip_suffix(".dll") == Some(name))
}

/// The call number of export `index` of DLL `dll`: every export of every DLL has one.
pub fn number(dll: usize, index: usize) -> u64 {
    atlas_abi::call::HOST_EXPORTS + DLLS[..dll].iter().map(|d| d.exports.len()).sum::<usize>() as u64 + index as u64
}

/// The export a call number is for.
pub fn by_number(nr: u64) -> Option<(&'static Dll, &'static Export)> {
    let mut at = nr.checked_sub(atlas_abi::call::HOST_EXPORTS)? as usize;
    for dll in DLLS {
        if at < dll.exports.len() {
            return Some((dll, &dll.exports[at]));
        }
        at -= dll.exports.len();
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_export_is_declared_once() {
        // The export table is looked up by name, sorted: a name twice is found as either.
        for dll in super::DLLS {
            let mut names: Vec<_> = dll.exports.iter().map(|e| e.name).collect();
            names.sort_unstable();
            let twice: Vec<_> = names.windows(2).filter(|w| w[0] == w[1]).map(|w| w[0]).collect();
            assert!(twice.is_empty(), "{} declares {twice:?} more than once", dll.name);
        }
    }
}
