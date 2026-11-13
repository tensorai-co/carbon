//! `carbon atlas imports PATH... [--all]`: what Windows programs import, against what atlas has. For one file, each
//! DLL it imports and what of it isn't there; for several (or a folder), a line a file and then what's wanted most,
//! by how many files want it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use atlas::imports::{self, Import, Report, Status, Tally};
use exec::Isa;

fn is_pe(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "exe" | "dll" | "sys" | "ocx" | "cpl"))
}

/// The PE files under each path: a file itself, or each one in a folder (all the way down).
fn files(paths: &[String]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else if is_pe(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        if path.is_dir() {
            walk(&path, &mut out);
        } else {
            out.push(path);
        }
    }
    out
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn kind(report: &Report) -> String {
    let (format, isa) = match report.isa {
        Isa::Arm64 => ("PE32+", "ARM64"),
        Isa::X86_64 => ("PE32+", "x86-64"),
        _ => ("PE32", "x86 (32-bit, WoW64)"),
    };
    format!("{format} {isa} {}", if report.is_dll { "DLL" } else { "EXE" })
}

/// Whether it would load as it is: nothing it imports statically is undeclared or in a DLL atlas lacks.
fn loads(report: &Report) -> bool {
    report.imports.iter().all(|i| i.delayed || !matches!(i.status, Status::Undeclared | Status::NoDll))
}

/// `1 import`, `2 imports`.
fn of(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn counts(imports: &[&Import]) -> String {
    let n = |s: &[Status]| imports.iter().filter(|i| s.contains(&i.status)).count();
    let mut parts = vec![format!("{} there", n(&[Status::Host, Status::Guest]))];
    for (count, one, many) in [
        (n(&[Status::Stub]), "stub", "stubs"),
        (n(&[Status::Undeclared]), "not declared", "not declared"),
        (n(&[Status::NoDll]), "in a DLL atlas lacks", "in DLLs atlas lacks"),
        (n(&[Status::Program]), "the program's own", "the program's own"),
    ] {
        if count > 0 {
            parts.push(of(count, one, many));
        }
    }
    parts.join(", ")
}

fn list(names: &[String]) -> String {
    let mut out = String::new();
    let mut width = 0;
    for (i, n) in names.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
            width += 2;
        }
        if width + n.len() > 100 {
            out.push_str("\n        ");
            width = 0;
        }
        out.push_str(n);
        width += n.len();
    }
    out
}

/// One file in full: each DLL, and what of it isn't there (or everything, with `all`).
fn detail(path: &Path, report: &Report, all: bool) {
    let dlls: HashSet<&str> = report.imports.iter().map(|i| i.dll.as_str()).collect();
    let refs: Vec<&Import> = report.imports.iter().collect();
    println!("{}: {}, {} from {}: {}", path.display(), kind(report), of(refs.len(), "import", "imports"), of(dlls.len(), "DLL", "DLLs"), counts(&refs));
    println!("  {}", if loads(report) { "loads as it is" } else { "doesn't load yet: it imports what atlas lacks" });
    let mut by_dll: BTreeMap<String, Vec<&Import>> = BTreeMap::new();
    for i in &report.imports {
        by_dll.entry(i.dll.clone()).or_default().push(i);
    }
    for (dll, imports) in by_dll {
        let target = &imports[0].target;
        let via = if !target.eq_ignore_ascii_case(&dll) { format!(" (an API set: {target})") } else { String::new() };
        let delayed = if imports.iter().all(|i| i.delayed) { ", delay-loaded" } else { "" };
        println!("  {dll}{via}{delayed}: {}, {}", of(imports.len(), "import", "imports"), counts(&imports));
        let groups: &[(Status, &str)] = if all {
            &[(Status::Host, "host"), (Status::Guest, "in ntdll"), (Status::Stub, "stubs"), (Status::Undeclared, "not declared"), (Status::NoDll, "no such DLL"), (Status::Program, "the program's own")]
        } else {
            &[(Status::Stub, "stubs"), (Status::Undeclared, "not declared"), (Status::NoDll, "no such DLL")]
        };
        for (status, what) in groups {
            let mut names: Vec<String> = imports.iter().filter(|i| i.status == *status).map(|i| i.label()).collect();
            names.sort();
            names.dedup();
            if !names.is_empty() {
                println!("      {what}: {}", list(&names));
            }
        }
    }
}

pub fn run(args: Vec<String>) {
    let all = args.iter().any(|a| a == "--all");
    let paths: Vec<String> = args.into_iter().filter(|a| a != "--all").collect();
    if paths.is_empty() {
        crate::fail("atlas imports needs a file or a folder: carbon atlas imports setup.exe [--all]");
    }
    let found = files(&paths);
    if found.is_empty() {
        crate::fail("no EXE or DLL files there");
    }
    // The DLLs the program brings: every one scanned, and every one beside each file.
    let mut provided: HashSet<String> = found.iter().map(|p| name_of(p)).collect();
    for p in &found {
        if let Some(dir) = p.parent().and_then(|d| std::fs::read_dir(if d.as_os_str().is_empty() { Path::new(".") } else { d }).ok()) {
            provided.extend(dir.flatten().map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()).filter(|n| n.ends_with(".dll")));
        }
    }
    let is_provided = |dll: &str| provided.contains(dll);
    let mut tally = Tally::default();
    let mut loading = 0;
    for path in &found {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                continue;
            }
        };
        let report = match imports::scan(&bytes, &is_provided) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                continue;
            }
        };
        if found.len() == 1 {
            detail(path, &report, all);
            return;
        }
        let refs: Vec<&Import> = report.imports.iter().collect();
        let mark = if loads(&report) { "loads" } else { "doesn't load" };
        println!("{:<14} {}  {}: {}", mark, path.display(), kind(&report), counts(&refs));
        loading += usize::from(loads(&report));
        tally.add(&report);
    }
    println!("\n{} files, {} of them load as they are", tally.files, loading);
    let mut dlls: Vec<(&String, &usize)> = tally.dlls.iter().collect();
    dlls.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    if !dlls.is_empty() {
        println!("\nDLLs atlas lacks, by how many files import them:");
        for (dll, n) in dlls {
            println!("  {n:>5}  {dll}");
        }
    }
    let mut functions: Vec<_> = tally.functions.iter().filter(|(_, (_, s))| *s != Status::NoDll).collect();
    functions.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    if !functions.is_empty() {
        println!("\nFunctions atlas's DLLs lack, by how many files import them:");
        for ((dll, name), (n, status)) in functions.iter().take(if all { usize::MAX } else { 60 }) {
            let what = if *status == Status::Stub { "stub" } else { "not declared" };
            println!("  {n:>5}  {dll}!{name} ({what})");
        }
        if !all && functions.len() > 60 {
            println!("  … {} more (--all for every one)", functions.len() - 60);
        }
    }
}
