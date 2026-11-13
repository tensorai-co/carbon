//! A new process's environment, as Windows makes it at logon: the registry's system and user `Environment` keys,
//! expanded (a user's `Path` goes after the system's), with the variables Windows sets for each session (the user's
//! folders, the computer's name, the processor), then any the machine was started with. It goes in the process's
//! memory as a block of UTF-16 `NAME=VALUE` strings, sorted by name, as the process parameters' `Environment`. From
//! there, ntdll's environment functions work on it in the program, as Windows' do.

use exec::Isa;

use crate::Kernel;
use crate::registry::{self, REG_EXPAND_SZ, REG_SZ, Root};

const SYSTEM: &str = "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment";

fn set(vars: &mut Vec<(String, String)>, name: &str, value: String) {
    match vars.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        Some(entry) => entry.1 = value,
        None => vars.push((name.to_string(), value)),
    }
}

/// A registry key's string values, expanded.
fn from_key(k: &Kernel, root: Root, path: &str) -> Vec<(String, String)> {
    let mut r = kernels_core::lock(&k.registry);
    let base = r.root(root);
    let Ok(key) = r.open(base, path) else { return Vec::new() };
    let Ok(key) = r.key(key) else { return Vec::new() };
    key.values
        .values()
        .filter(|v| v.kind == REG_SZ || v.kind == REG_EXPAND_SZ)
        .map(|v| {
            let text = registry::text(&v.data);
            (v.name.clone(), if v.kind == REG_EXPAND_SZ { registry::expand(&text) } else { text })
        })
        .collect()
}

/// The first process's variables, sorted by name as Windows keeps them.
pub fn first(k: &Kernel, isa: Isa) -> Vec<(String, String)> {
    let mut vars = from_key(k, Root::LocalMachine, SYSTEM);
    for (name, value) in from_key(k, Root::CurrentUser, "Environment") {
        if name.eq_ignore_ascii_case("Path") {
            let system = vars.iter().find(|(n, _)| n.eq_ignore_ascii_case("Path")).map(|(_, v)| v.clone()).unwrap_or_default();
            set(&mut vars, "Path", if system.is_empty() { value } else { format!("{system};{value}") });
        } else {
            set(&mut vars, &name, value);
        }
    }
    let session = [
        ("ALLUSERSPROFILE", "allusersprofile"),
        ("APPDATA", "appdata"),
        ("CommonProgramFiles", "commonprogramfiles"),
        ("CommonProgramFiles(x86)", "commonprogramfiles(x86)"),
        ("CommonProgramW6432", "commonprogramw6432"),
        ("COMPUTERNAME", "computername"),
        ("HOMEDRIVE", "homedrive"),
        ("HOMEPATH", "homepath"),
        ("LOCALAPPDATA", "localappdata"),
        ("ProgramData", "programdata"),
        ("ProgramFiles", "programfiles"),
        ("ProgramFiles(x86)", "programfiles(x86)"),
        ("ProgramW6432", "programw6432"),
        ("SystemDrive", "systemdrive"),
        ("SystemRoot", "systemroot"),
        ("USERDOMAIN", "userdomain"),
        ("USERNAME", "username"),
        ("USERPROFILE", "userprofile"),
    ];
    for (name, key) in session {
        if let Some(value) = registry::variable(key) {
            set(&mut vars, name, value.to_string());
        }
    }
    set(&mut vars, "NUMBER_OF_PROCESSORS", k.host.cpus().to_string());
    for (name, value) in &k.options.env {
        set(&mut vars, name, value.clone());
    }
    for_isa(&mut vars, isa);
    vars
}

/// The variables that say which kind of program it is, set for one of `isa`, as WoW64 sets them as a process starts
/// (from its parent's, or the logon's): a 32-bit program has PROCESSOR_ARCHITECTURE x86 and the machine's own in
/// PROCESSOR_ARCHITEW6432, and its ProgramFiles and CommonProgramFiles are the (x86) folders; a 64-bit one has its
/// own architecture, no PROCESSOR_ARCHITEW6432, and the folders ProgramW6432 and CommonProgramW6432 name.
pub fn for_isa(vars: &mut Vec<(String, String)>, isa: Isa) {
    let get = |vars: &Vec<(String, String)>, name: &str| vars.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    let (architecture, folders) = match isa {
        Isa::Arm64 => ("ARM64", ["ProgramW6432", "CommonProgramW6432"]),
        Isa::X86_64 => ("AMD64", ["ProgramW6432", "CommonProgramW6432"]),
        Isa::X86_32 => ("x86", ["ProgramFiles(x86)", "CommonProgramFiles(x86)"]),
    };
    set(vars, "PROCESSOR_ARCHITECTURE", architecture.to_string());
    vars.retain(|(n, _)| !n.eq_ignore_ascii_case("PROCESSOR_ARCHITEW6432"));
    if isa == Isa::X86_32 {
        set(vars, "PROCESSOR_ARCHITEW6432", "AMD64".to_string());
    }
    for (name, from) in ["ProgramFiles", "CommonProgramFiles"].into_iter().zip(folders) {
        if let Some(value) = get(vars, from) {
            set(vars, name, value);
        }
    }
    vars.sort_by_key(|(n, _)| n.to_uppercase());
}

/// A block's variables, back from its UTF-16.
pub fn parse(block: &[u8]) -> Vec<(String, String)> {
    let units: Vec<u16> = block.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    units
        .split(|&u| u == 0)
        .take_while(|s| !s.is_empty())
        .filter_map(|s| {
            let text = String::from_utf16_lossy(s);
            // `=C:` and its kind start with `=`: the name is up to the next one.
            let at = text.char_indices().skip(1).find(|&(_, c)| c == '=')?.0;
            Some((text[..at].to_string(), text[at + 1..].to_string()))
        })
        .collect()
}

/// The block: each `NAME=VALUE` and a NUL, then a NUL.
pub fn block(vars: &[(String, String)]) -> Vec<u8> {
    let mut units: Vec<u16> = Vec::new();
    for (name, value) in vars {
        units.extend(format!("{name}={value}").encode_utf16());
        units.push(0);
    }
    units.push(0);
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}
