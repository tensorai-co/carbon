//! shlwapi.dll: paths; the rest is declared so programs that import it load, and says it isn't written yet.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "SHLWAPI.dll";
    host IsOS(1) @ 437 => is_os;
    missing SHCreateMemStream @ 12;
    missing QISearch @ 219;
    host PathFileExistsW(1) => path_file_exists_w;
    missing SHDeleteKeyA;
    host PathFileExistsA(1) => path_file_exists_a;
    // Not written yet: what larger programs import.
    missing AssocQueryStringW;
    missing PathFindExtensionW;
    missing PathFindFileNameW;
    host PathMatchSpecW(2) => path_match_spec_w;
    missing PathRemoveExtensionW;
}

/// `PathFileExistsW(path)` and `A`: whether there's a file or folder there.
fn path_file_exists(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let found = task.path_arg(cx, a[0], wide).is_some_and(|p: String| task.path_info(&p).is_ok());
    Reply::Value(u64::from(found))
}

fn path_file_exists_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_file_exists(task, cx, a, true)
}

fn path_file_exists_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_file_exists(task, cx, a, false)
}

/// `PathMatchSpecW(path, specs)`: whether the path matches any of the `;`-separated wildcard specs (`*.txt; *.log`),
/// as `FindFirstFile` matches, without regard to case.
fn path_match_spec_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 || a[1] == 0 {
        return Reply::Value(0);
    }
    let path = task.text_arg(cx, a[0], true);
    let specs = task.text_arg(cx, a[1], true);
    Reply::Value(u64::from(matches_any(&path, &specs)))
}

fn matches_any(path: &str, specs: &str) -> bool {
    specs.split(';').map(str::trim).filter(|s| !s.is_empty()).any(|spec| crate::files::matches(spec, path))
}

/// `IsOS(feature)`: what the machine is, as Windows 10 Pro on its own: an NT workstation, professional, not a member of
/// a domain, not a server, not a tablet PC, with fast user switching and the welcome screen.
fn is_os(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let yes = match a[0] as u32 {
        // OS_WINDOWS (0) is the 9x line.
        0 => false,
        // OS_NT, OS_WIN2000ORGREATER, OS_XPORGREATER and the like.
        1 | 7..=10 | 13 | 19..=20 => true,
        // OS_PROFESSIONAL, OS_FASTUSERSWITCHING, OS_WELCOMELOGONUI.
        17 | 25 | 26 => true,
        // OS_WOW6432: a 32-bit program on 64-bit Windows.
        30 => task.isa == exec::Isa::X86_32,
        // OS_DOMAINMEMBER, OS_ANYSERVER, OS_TABLETPC, OS_MEDIACENTER, OS_SERVERADMINUI and the rest.
        _ => false,
    };
    Reply::Value(u64::from(yes))
}

#[cfg(test)]
mod tests {
    #[test]
    fn specs_each_tried() {
        assert!(super::matches_any("C:\\logs\\a.LOG", "*.txt; *.log"));
        assert!(!super::matches_any("a.bin", "*.txt;*.log"));
        assert!(super::matches_any("README", "*.*"));
    }
}
