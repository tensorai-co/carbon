//! Every system DLL, as a submodule: its `dll!` declaration, and the host Rust behind its host exports. atlas makes
//! each DLL's image from its declaration (`images`), and nothing is ever a file of its own.

pub mod advapi32;
pub mod bthprops;
pub mod bcrypt;
pub mod bcryptprimitives;
pub mod cfgmgr32;
pub mod com;
pub mod combase;
pub mod comctl32;
pub mod comdlg32;
pub mod credui;
pub mod crypt32;
pub mod cryptui;
pub mod d3d11;
pub mod d3d12;
pub mod d3d9;
pub mod dbghelp;
pub mod dhcpcsvc;
pub mod dwmapi;
pub mod dwrite;
pub mod dxgi;
pub mod dxva2;
pub mod esent;
pub mod etw;
pub mod fileapi;
pub mod fontsub;
pub mod gdi32;
pub mod hid;
pub mod imagehlp;
pub mod imm32;
pub mod ioapi;
pub mod iphlpapi;
pub mod kernel32;
pub mod localeapi;
pub mod memapi;
pub mod menuapi;
pub mod mf;
pub mod mfplat;
pub mod mfreadwrite;
pub mod msdmo;
pub mod msimg32;
pub mod mswsock;
pub mod ncrypt;
pub mod ndfapi;
pub mod netapi32;
pub mod nlsapi;
pub mod ole32;
pub mod oleacc;
pub mod oleaut32;
pub mod opengl32;
pub mod pdh;
pub mod pipeapi;
pub mod powrprof;
pub mod procapi;
pub mod propsys;
pub mod psapi;
pub mod resapi;
pub mod rpcrt4;
pub mod secapi;
pub mod secur32;
pub mod setupapi;
pub mod shcore;
pub mod shell32;
pub mod shellapi;
pub mod shlwapi;
pub mod synch;
pub mod sysapi;
pub mod ucrtbase;
pub mod uiautomationcore;
pub mod urlmon;
pub mod user32;
pub mod userenv;
pub mod usp10;
pub mod uxtheme;
pub mod version;
pub mod volumes;
pub mod wevtapi;
pub mod winhttp;
pub mod wininet;
pub mod winmm;
pub mod winspool;
pub mod wintrust;
pub mod winusb;
pub mod winuser;
pub mod ws2_32;
pub mod wsock32;
pub mod wtsapi32;

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
    /// Its ordinal, where it isn't the one after the export before it's (programs import some by Windows' own
    /// numbers, which have gaps: ws2_32's `socket` is #23, `WSAStartup` #115).
    pub ordinal: Option<u16>,
}

pub struct Dll {
    /// Its file name, as Windows has it.
    pub name: &'static str,
    pub exports: &'static [Export],
    /// Its first export's ordinal, the rest following in order: 1, unless programs import its first by an ordinal of
    /// Windows' own (comctl32's InitCommonControls is #17).
    pub first_ordinal: u16,
}

impl Dll {
    /// Each export's ordinal: its own, or the one after the export before it's (the first's `first_ordinal`).
    pub fn ordinals(&self) -> Vec<u16> {
        let mut next = self.first_ordinal;
        self.exports
            .iter()
            .map(|e| {
                let o = e.ordinal.unwrap_or(next);
                next = o + 1;
                o
            })
            .collect()
    }

    /// The export with an ordinal.
    pub fn by_ordinal(&self, ordinal: u16) -> Option<&Export> {
        self.ordinals().iter().position(|&o| o == ordinal).map(|i| &self.exports[i])
    }
}

/// Declares a DLL: each export is `host Name(args) => function`, `guest Name => NtdllName` (or just `guest Name`), or
/// `missing Name`, numbered on from the one before; `@ 23` after its name gives it an ordinal of its own (the next
/// follow on from there). `"NAME.dll" @ 17;` numbers its exports from 17.
macro_rules! dll {
    ($file:literal $(@ $first:literal)?; $($kind:ident $name:ident $(($args:literal))? $(@ $ord:literal)? $(=> $target:ident)?;)*) => {
        pub static DLL: $crate::os::Dll = $crate::os::Dll {
            name: $file,
            exports: &[$(dll!(@export $kind $name $(($args))? [$($ord)?] $(=> $target)?)),*],
            first_ordinal: dll!(@first $($first)?),
        };
    };
    (@first) => { 1 };
    (@first $first:literal) => { $first };
    (@ord) => { None };
    (@ord $ord:literal) => { Some($ord) };
    (@export host $name:ident ($args:literal) [$($ord:literal)?] => $run:ident) => {
        $crate::os::Export { name: stringify!($name), kind: $crate::os::Kind::Host { args: $args, run: $run }, ordinal: dll!(@ord $($ord)?) }
    };
    (@export guest $name:ident [$($ord:literal)?] => $target:ident) => {
        $crate::os::Export { name: stringify!($name), kind: $crate::os::Kind::Guest(stringify!($target)), ordinal: dll!(@ord $($ord)?) }
    };
    (@export guest $name:ident [$($ord:literal)?]) => {
        $crate::os::Export { name: stringify!($name), kind: $crate::os::Kind::Guest(stringify!($name)), ordinal: dll!(@ord $($ord)?) }
    };
    (@export missing $name:ident [$($ord:literal)?]) => {
        $crate::os::Export { name: stringify!($name), kind: $crate::os::Kind::Missing, ordinal: dll!(@ord $($ord)?) }
    };
}
pub(crate) use dll;

/// Every system DLL atlas has.
pub static DLLS: &[&Dll] = &[&kernel32::DLL, &user32::DLL, &advapi32::DLL, &gdi32::DLL, &shell32::DLL, &version::DLL, &ucrtbase::DLL, &comctl32::DLL, &ole32::DLL, &psapi::DLL, &oleaut32::DLL, &bcrypt::DLL, &crypt32::DLL, &ws2_32::DLL, &wsock32::DLL,
    &cfgmgr32::DLL, &comdlg32::DLL, &dwmapi::DLL, &dwrite::DLL, &dxgi::DLL, &dxva2::DLL, &hid::DLL, &imagehlp::DLL, &imm32::DLL, &iphlpapi::DLL, &msdmo::DLL, &msimg32::DLL, &mswsock::DLL, &ncrypt::DLL, &opengl32::DLL, &pdh::DLL, &powrprof::DLL, &rpcrt4::DLL, &secur32::DLL, &setupapi::DLL, &shlwapi::DLL, &usp10::DLL, &winhttp::DLL, &wininet::DLL, &winmm::DLL, &wtsapi32::DLL, &shcore::DLL,
    &bcryptprimitives::DLL, &credui::DLL, &cryptui::DLL, &d3d11::DLL, &d3d12::DLL, &d3d9::DLL, &dbghelp::DLL, &dhcpcsvc::DLL, &esent::DLL, &fontsub::DLL, &mf::DLL, &mfplat::DLL, &mfreadwrite::DLL, &ndfapi::DLL, &netapi32::DLL, &oleacc::DLL, &propsys::DLL, &uiautomationcore::DLL, &urlmon::DLL, &userenv::DLL, &uxtheme::DLL, &wevtapi::DLL, &wintrust::DLL, &winusb::DLL, &combase::DLL, &winspool::DLL, &bthprops::DLL];

/// The DLL called `name` (`kernel32`, `KERNEL32.DLL`, …), by its index in [`DLLS`].
pub fn find(name: &str) -> Option<usize> {
    // `.dll` if it has no extension; another extension (`.drv`, `.cpl`) is part of its name.
    let name = name.to_ascii_lowercase();
    let name = if name.contains('.') { name } else { format!("{name}.dll") };
    DLLS.iter().position(|d| d.name.to_ascii_lowercase() == name)
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

    #[test]
    fn each_ordinal_is_one_export() {
        for dll in super::DLLS {
            let mut ordinals = dll.ordinals();
            ordinals.sort_unstable();
            let twice: Vec<_> = ordinals.windows(2).filter(|w| w[0] == w[1]).map(|w| w[0]).collect();
            assert!(twice.is_empty(), "{} gives ordinals {twice:?} to more than one export", dll.name);
        }
    }
}
