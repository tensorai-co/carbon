//! What a Windows program asks of the system, against what atlas has: every DLL and function a PE imports (statically
//! and delay-loaded, by name or ordinal, from a 32-bit or 64-bit image), each one found in atlas's own declarations
//! (`os`'s DLLs, and ntdll's exports in `atlas-guest`), declared but not written yet (`missing`), not declared, or in a
//! DLL atlas doesn't have (or one the program brings beside itself). API sets (`api-ms-win-*`) are followed to the DLL
//! that has their functions. `carbon atlas imports` prints it, for a file or a folder of them.

use std::collections::HashMap;
use std::sync::OnceLock;

use exec::Isa;

use crate::images::pe::Pe;
use crate::os::{DLLS, Kind};

/// Where an import stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Status {
    /// Written, in host Rust.
    Host,
    /// Written, in ntdll (in the program).
    Guest,
    /// Declared, but a stub that ends the process naming itself.
    Stub,
    /// The DLL's there, but not this function: the program doesn't load.
    Undeclared,
    /// atlas has no such DLL: the program doesn't load.
    NoDll,
    /// A DLL the program brings itself (found beside it), so not atlas's to provide.
    Program,
}

impl Status {
    /// Whether it's there to call.
    pub fn present(self) -> bool {
        matches!(self, Status::Host | Status::Guest | Status::Program)
    }
}

/// One imported function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The DLL as the program names it.
    pub dll: String,
    /// The DLL it's looked for in: the same, or where an API set leads.
    pub target: String,
    /// Its name, or its ordinal.
    pub name: Option<String>,
    pub ordinal: Option<u16>,
    /// Delay-loaded: only when it's first called, so a gap here doesn't stop the program starting.
    pub delayed: bool,
    pub status: Status,
}

impl Import {
    /// How it's shown: its name, or `#ordinal`.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("#{}", self.ordinal.unwrap_or(0)))
    }
}

/// What a PE file imports.
#[derive(Debug, Clone)]
pub struct Report {
    pub isa: Isa,
    pub is_dll: bool,
    pub imports: Vec<Import>,
}

/// An API set's DLL: `api-ms-win-core-synch-l1-2-0` is kernel32's (where atlas keeps kernelbase's functions),
/// `api-ms-win-crt-*` ucrtbase's, and so on. None for one that isn't known (`ext-ms-*` sets may be absent on Windows
/// too).
pub fn api_set(dll: &str) -> Option<&'static str> {
    let name = dll.to_ascii_lowercase();
    let name = name.strip_suffix(".dll").unwrap_or(&name);
    let rest = name.strip_prefix("api-ms-win-")?;
    const SETS: &[(&str, &str)] = &[
        ("crt-", "ucrtbase.dll"),
        ("core-rtlsupport-", "ntdll.dll"),
        ("core-registry-", "advapi32.dll"),
        ("core-com-", "combase.dll"),
        ("core-winrt-", "combase.dll"),
        ("core-", "kernel32.dll"),
        ("security-", "advapi32.dll"),
        ("eventing-", "advapi32.dll"),
        ("service-", "advapi32.dll"),
        ("shcore-", "shcore.dll"),
        ("shell-", "shell32.dll"),
        ("gdi-", "gdi32.dll"),
        ("ntuser-", "user32.dll"),
        ("rtcore-ntuser-", "user32.dll"),
        ("downlevel-", "kernel32.dll"),
    ];
    SETS.iter().find(|(prefix, _)| rest.starts_with(prefix)).map(|(_, dll)| *dll)
}

/// The DLL's name, folded: lower case, with `.dll`.
fn folded(dll: &str) -> String {
    let lower = dll.to_ascii_lowercase();
    if lower.ends_with(".dll") || lower.contains('.') { lower } else { format!("{lower}.dll") }
}

/// ntdll's exports for an ISA, from atlas-guest's image (x86-64's for a 32-bit program, until there's an i686 one).
fn ntdll_exports(isa: Isa) -> &'static [String] {
    static ARM64: OnceLock<Vec<String>> = OnceLock::new();
    static X86_64: OnceLock<Vec<String>> = OnceLock::new();
    let (cell, isa) = match isa {
        Isa::Arm64 => (&ARM64, Isa::Arm64),
        _ => (&X86_64, Isa::X86_64),
    };
    cell.get_or_init(|| {
        let Some(bytes) = crate::images::guest(isa) else { return Vec::new() };
        let Ok(pe) = Pe::parse(bytes) else { return Vec::new() };
        pe.layout().ok().and_then(|image| pe.export_names(&image).ok()).unwrap_or_default()
    })
}

/// Where an import of `name` (or else `ordinal`) from `dll` stands, `provided` saying which DLLs the program brings.
pub fn status(dll: &str, name: Option<&str>, ordinal: Option<u16>, isa: Isa, provided: &dyn Fn(&str) -> bool) -> Status {
    let dll = folded(dll);
    if dll == "ntdll.dll" {
        let Some(name) = name else { return Status::Undeclared };
        return if ntdll_exports(isa).iter().any(|e| e == name) { Status::Guest } else { Status::Undeclared };
    }
    let Some(index) = crate::os::find(&dll) else {
        return if provided(&dll) { Status::Program } else { Status::NoDll };
    };
    let d = DLLS[index];
    let found = match (name, ordinal) {
        (Some(name), _) => d.exports.iter().find(|e| e.name == name),
        (None, Some(ordinal)) => ordinal.checked_sub(d.first_ordinal).and_then(|i| d.exports.get(usize::from(i))),
        (None, None) => None,
    };
    match found.map(|e| e.kind) {
        Some(Kind::Host { .. }) => Status::Host,
        Some(Kind::Guest(_)) => Status::Guest,
        Some(Kind::Missing) => Status::Stub,
        None => Status::Undeclared,
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn cstr(b: &[u8], at: usize) -> Option<String> {
    let rest = b.get(at..)?;
    let end = rest.iter().position(|&c| c == 0)?;
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
}

/// The functions a thunk table names: each by name (through its hint/name entry) or by ordinal.
fn thunks(image: &[u8], at: usize, wide: bool) -> Vec<(Option<String>, Option<u16>)> {
    let mut out = Vec::new();
    let step = if wide { 8 } else { 4 };
    for i in 0..65536 {
        let entry = if wide { u64_at(image, at + i * step) } else { u32_at(image, at + i * step).map(u64::from) };
        let Some(entry) = entry.filter(|&e| e != 0) else { break };
        let by_ordinal = if wide { entry >> 63 != 0 } else { entry >> 31 != 0 };
        if by_ordinal {
            out.push((None, Some(entry as u16)));
        } else {
            out.push((cstr(image, (entry & 0x7fff_ffff) as usize + 2), None));
        }
    }
    out
}

/// What a PE file imports, each import placed: `provided` says which DLLs the program brings beside itself.
pub fn scan(bytes: &[u8], provided: &dyn Fn(&str) -> bool) -> Result<Report, String> {
    let pe = Pe::parse(bytes).map_err(|_| "not a PE image atlas reads (an EXE or DLL for ARM64, x86-64 or x86)".to_string())?;
    let image = pe.layout().map_err(|_| "its sections don't fit its image".to_string())?;
    let wide = pe.isa != Isa::X86_32;
    let mut raw: Vec<(String, Option<String>, Option<u16>, bool)> = Vec::new();
    // Static imports: a descriptor per DLL, its lookup table (or its address table, where there's none).
    if let Some((rva, _)) = pe.directory(1) {
        let mut at = rva as usize;
        while let (Some(lookup), Some(name), Some(iat)) = (u32_at(&image, at), u32_at(&image, at + 12), u32_at(&image, at + 16)) {
            if name == 0 && iat == 0 {
                break;
            }
            let Some(dll) = cstr(&image, name as usize) else { break };
            let table = if lookup != 0 { lookup } else { iat };
            for (n, o) in thunks(&image, table as usize, wide) {
                raw.push((dll.clone(), n, o, false));
            }
            at += 20;
        }
    }
    // Delay-loaded imports: a descriptor per DLL, its fields RVAs (or, from old linkers, addresses).
    if let Some((rva, _)) = pe.directory(13) {
        let mut at = rva as usize;
        while let (Some(attrs), Some(name), Some(names)) = (u32_at(&image, at), u32_at(&image, at + 4), u32_at(&image, at + 16)) {
            if name == 0 {
                break;
            }
            let fix = |v: u32| if attrs & 1 != 0 { v as usize } else { (u64::from(v)).wrapping_sub(pe.image_base) as usize };
            let Some(dll) = cstr(&image, fix(name)) else { break };
            for (n, o) in thunks(&image, fix(names), wide) {
                raw.push((dll.clone(), n, o, true));
            }
            at += 32;
        }
    }
    let imports = raw
        .into_iter()
        .map(|(dll, name, ordinal, delayed)| {
            let target = api_set(&dll).map_or_else(|| folded(&dll), str::to_string);
            let status = status(&target, name.as_deref(), ordinal, pe.isa, provided);
            Import { dll, target, name, ordinal, delayed, status }
        })
        .collect();
    Ok(Report { isa: pe.isa, is_dll: pe.is_dll, imports })
}

/// Many files' imports added up: for each DLL atlas lacks, and each function it lacks, how many files want it.
#[derive(Debug, Default)]
pub struct Tally {
    pub files: usize,
    /// DLL (folded) → files importing it, for DLLs atlas doesn't have.
    pub dlls: HashMap<String, usize>,
    /// (DLL, function) → files importing it, and where it stands, for functions not there to call.
    pub functions: HashMap<(String, String), (usize, Status)>,
}

impl Tally {
    pub fn add(&mut self, report: &Report) {
        self.files += 1;
        let mut dlls: Vec<&str> = report.imports.iter().filter(|i| i.status == Status::NoDll).map(|i| i.target.as_str()).collect();
        dlls.sort_unstable();
        dlls.dedup();
        for d in dlls {
            *self.dlls.entry(d.to_string()).or_default() += 1;
        }
        let mut seen = std::collections::HashSet::new();
        for i in report.imports.iter().filter(|i| !i.status.present()) {
            let key = (i.target.clone(), i.label());
            if seen.insert(key.clone()) {
                self.functions.entry(key).or_insert((0, i.status)).0 += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_sets_lead_to_their_dlls() {
        assert_eq!(api_set("api-ms-win-core-synch-l1-2-0.dll"), Some("kernel32.dll"));
        assert_eq!(api_set("API-MS-WIN-CRT-RUNTIME-L1-1-0.DLL"), Some("ucrtbase.dll"));
        assert_eq!(api_set("api-ms-win-core-rtlsupport-l1-1-0"), Some("ntdll.dll"));
        assert_eq!(api_set("kernel32.dll"), None);
    }

    #[test]
    fn imports_stand_as_atlas_declares_them() {
        let none = |_: &str| false;
        assert_eq!(status("KERNEL32.dll", Some("CreateFileW"), None, Isa::X86_64, &none), Status::Host);
        assert_eq!(status("kernel32", Some("HeapAlloc"), None, Isa::X86_64, &none), Status::Guest);
        assert_eq!(status("kernel32.dll", Some("CreateFiber"), None, Isa::X86_64, &none), Status::Guest);
        assert_eq!(status("user32.dll", Some("CreateWindowExA"), None, Isa::X86_64, &none), Status::Host);
        assert_eq!(status("gdi32.dll", Some("BitBlt"), None, Isa::X86_64, &none), Status::Stub);
        assert_eq!(status("kernel32.dll", Some("NoSuchFunction"), None, Isa::X86_64, &none), Status::Undeclared);
        assert_eq!(status("comctl32.dll", Some("InitCommonControlsEx"), None, Isa::X86_64, &none), Status::Host);
        assert_eq!(status("comctl32.dll", None, Some(17), Isa::X86_64, &none), Status::Host);
        assert_eq!(status("nosuch.dll", Some("f"), None, Isa::X86_64, &none), Status::NoDll);
        assert_eq!(status("mylib.dll", Some("f"), None, Isa::X86_64, &|d| d == "mylib.dll"), Status::Program);
        assert_eq!(status("user32.dll", None, None, Isa::X86_64, &none), Status::Undeclared);
    }

    /// A PE with no sections, its import tables in its headers: kernel32 (two by name), a DLL atlas hasn't (one by
    /// ordinal), and another delay-loaded.
    fn image(wide: bool) -> Vec<u8> {
        let mut b = vec![0u8; 0x600];
        let mut put = |at: usize, v: &[u8]| b[at..at + v.len()].copy_from_slice(v);
        put(0, b"MZ");
        put(0x3c, &0x40u32.to_le_bytes());
        put(0x40, b"PE\0\0");
        let (opt, dirs) = (0x58, if wide { 0x58 + 112 } else { 0x58 + 96 });
        put(0x44, &(if wide { 0x8664u16 } else { 0x14c }).to_le_bytes());
        put(0x44 + 16, &((dirs - opt + 16 * 8) as u16).to_le_bytes());
        put(0x44 + 18, &2u16.to_le_bytes());
        put(opt, &(if wide { 0x20bu16 } else { 0x10b }).to_le_bytes());
        put(opt + 56, &0x600u32.to_le_bytes());
        put(opt + 60, &0x600u32.to_le_bytes());
        put(dirs - 4, &16u32.to_le_bytes());
        put(dirs + 8, &[0x00, 0x02, 0, 0, 60, 0, 0, 0]);
        put(dirs + 13 * 8, &[0x80, 0x02, 0, 0, 64, 0, 0, 0]);
        for (at, lookup, name) in [(0x200, 0x300u32, 0x480u32), (0x214, 0x340, 0x490)] {
            put(at, &lookup.to_le_bytes());
            put(at + 12, &name.to_le_bytes());
            put(at + 16, &lookup.to_le_bytes());
        }
        put(0x280, &1u32.to_le_bytes());
        put(0x284, &0x4a0u32.to_le_bytes());
        put(0x290, &0x380u32.to_le_bytes());
        let ordinal = if wide { 1u64 << 63 } else { 1 << 31 };
        for (at, entries) in [(0x300, &[0x400u64, 0x420][..]), (0x340, &[ordinal | 17]), (0x380, &[0x440])] {
            for (i, e) in entries.iter().enumerate() {
                if wide { put(at + i * 8, &e.to_le_bytes()) } else { put(at + i * 4, &(*e as u32).to_le_bytes()) }
            }
        }
        for (at, s) in [(0x402, "CreateFileW"), (0x422, "CreateFiber"), (0x442, "CoCreateInstance"), (0x480, "KERNEL32.dll"), (0x490, "MISSING1.dll"), (0x4a0, "nope32.dll")] {
            put(at, s.as_bytes());
        }
        b
    }

    #[test]
    fn a_pe_of_either_width_scans_alike() {
        for (wide, isa) in [(false, Isa::X86_32), (true, Isa::X86_64)] {
            let report = scan(&image(wide), &|_| false).unwrap();
            assert_eq!(report.isa, isa);
            let got: Vec<(&str, String, bool, Status)> = report.imports.iter().map(|i| (i.dll.as_str(), i.label(), i.delayed, i.status)).collect();
            assert_eq!(
                got,
                [
                    ("KERNEL32.dll", "CreateFileW".into(), false, Status::Host),
                    ("KERNEL32.dll", "CreateFiber".into(), false, Status::Guest),
                    ("MISSING1.dll", "#17".into(), false, Status::NoDll),
                    ("nope32.dll", "CoCreateInstance".into(), true, Status::NoDll),
                ]
            );
            let brought = scan(&image(wide), &|d| d == "missing1.dll").unwrap();
            assert_eq!(brought.imports[2].status, Status::Program);
        }
    }
}
