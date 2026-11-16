//! DIR: a folder's files and folders, as cmd lists them, with `/A`, `/B`, `/S`, `/O` and `/W`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use programs::*;

use crate::expand::full_path;
use crate::shell::{Shell, attributes, file_time, find};

struct Options {
    bare: bool,
    tree: bool,
    wide: bool,
    /// `/A`: which attributes must be set (`+`) or clear (`-`); None for the usual (not hidden or system).
    attrs: Option<Vec<(u32, bool)>>,
    order: Vec<(char, bool)>,
}

fn size(f: &WIN32_FIND_DATAW) -> u64 {
    (u64::from(f.size_high) << 32) | u64::from(f.size_low)
}

fn shown(f: &WIN32_FIND_DATAW, o: &Options) -> bool {
    match &o.attrs {
        None => f.attributes & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) == 0,
        Some(wanted) => wanted.iter().all(|&(bit, set)| (f.attributes & bit != 0) == set),
    }
}

fn sort(entries: &mut [WIN32_FIND_DATAW], order: &[(char, bool)]) {
    if order.is_empty() {
        entries.sort_by_key(|f| narrow(&f.name).to_ascii_lowercase());
        return;
    }
    entries.sort_by(|a, b| {
        for &(key, reverse) in order {
            let o = match key {
                'n' => narrow(&a.name).to_ascii_lowercase().cmp(&narrow(&b.name).to_ascii_lowercase()),
                's' => size(a).cmp(&size(b)),
                'e' => ext(&narrow(&a.name)).cmp(&ext(&narrow(&b.name))),
                'd' => (a.written.high, a.written.low).cmp(&(b.written.high, b.written.low)),
                // Folders first.
                'g' => (b.attributes & FILE_ATTRIBUTE_DIRECTORY).cmp(&(a.attributes & FILE_ATTRIBUTE_DIRECTORY)),
                _ => core::cmp::Ordering::Equal,
            };
            let o = if reverse { o.reverse() } else { o };
            if o != core::cmp::Ordering::Equal {
                return o;
            }
        }
        core::cmp::Ordering::Equal
    });
}

fn ext(name: &str) -> String {
    name.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase())
}

pub fn dir(shell: &mut Shell, args: &[String]) {
    let mut o = Options { bare: false, tree: false, wide: false, attrs: None, order: Vec::new() };
    let mut paths = Vec::new();
    for a in args {
        if let Some(sw) = a.strip_prefix('/') {
            let lower = sw.to_ascii_lowercase();
            let (key, value) = (lower.chars().next().unwrap_or(' '), lower.get(1..).unwrap_or("").trim_start_matches(':'));
            match key {
                'b' => o.bare = true,
                's' => o.tree = true,
                'w' | 'd' => o.wide = true,
                'a' => {
                    let mut wanted = Vec::new();
                    let mut set = true;
                    for c in value.chars() {
                        let bit = match c {
                            '-' => {
                                set = false;
                                continue;
                            }
                            'd' => FILE_ATTRIBUTE_DIRECTORY,
                            'h' => FILE_ATTRIBUTE_HIDDEN,
                            's' => FILE_ATTRIBUTE_SYSTEM,
                            'r' => FILE_ATTRIBUTE_READONLY,
                            'a' => FILE_ATTRIBUTE_ARCHIVE,
                            'l' => FILE_ATTRIBUTE_REPARSE_POINT,
                            _ => continue,
                        };
                        wanted.push((bit, set));
                        set = true;
                    }
                    o.attrs = Some(wanted);
                }
                'o' => {
                    let mut reverse = false;
                    for c in value.chars() {
                        if c == '-' {
                            reverse = true;
                        } else {
                            o.order.push((c, reverse));
                            reverse = false;
                        }
                    }
                    if o.order.is_empty() {
                        o.order = vec![('g', false), ('n', false)];
                    }
                }
                _ => {}
            }
        } else {
            paths.push(a.replace('"', ""));
        }
    }
    if paths.is_empty() {
        paths.push(String::from("."));
    }
    if !o.bare {
        let root = wide("C:\\");
        let mut serial = 0u32;
        unsafe { GetVolumeInformationW(root.as_ptr(), core::ptr::null_mut(), 0, &mut serial, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), 0) };
        out(" Volume in drive C has no label.\r\n");
        out(&format!(" Volume Serial Number is {:04X}-{:04X}\r\n", serial >> 16, serial & 0xffff));
    }
    let (mut total_files, mut total_bytes, mut total_dirs) = (0u64, 0u64, 0u64);
    let mut found_any = false;
    for path in paths {
        // A folder lists what's in it; anything else is a pattern in its folder.
        let is_folder = attributes(&path).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0) && !path.contains(['*', '?']);
        let (folder, pattern) = if is_folder {
            (full_path(&path), String::from("*"))
        } else {
            match path.rfind(['\\', '/']) {
                Some(at) => (full_path(&path[..=at]), path[at + 1..].to_string()),
                None => (crate::expand::full_path("."), path.clone()),
            }
        };
        let mut folders = vec![folder.trim_end_matches('\\').to_string()];
        let mut i = 0;
        while i < folders.len() {
            let dir = folders[i].clone();
            let base = if dir.ends_with(':') { format!("{dir}\\") } else { dir.clone() };
            let mut entries: Vec<WIN32_FIND_DATAW> = find(&format!("{}\\{pattern}", base.trim_end_matches('\\'))).into_iter().filter(|f| shown(f, &o)).collect();
            sort(&mut entries, &o.order);
            if o.tree {
                for f in find(&format!("{}\\*", base.trim_end_matches('\\'))) {
                    let name = narrow(&f.name);
                    if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && name != "." && name != ".." {
                        folders.push(format!("{}\\{name}", base.trim_end_matches('\\')));
                    }
                }
            }
            i += 1;
            if entries.is_empty() {
                continue;
            }
            found_any = true;
            if o.bare {
                for f in &entries {
                    let name = narrow(&f.name);
                    if name == "." || name == ".." {
                        continue;
                    }
                    if o.tree {
                        out(&format!("{}\\{name}\r\n", base.trim_end_matches('\\')));
                    } else {
                        out(&format!("{name}\r\n"));
                    }
                }
                continue;
            }
            out(&format!("\r\n Directory of {base}\r\n\r\n"));
            let (mut files, mut bytes, mut dirs) = (0u64, 0u64, 0u64);
            if o.wide {
                let mut line = String::new();
                for (n, f) in entries.iter().enumerate() {
                    let name = narrow(&f.name);
                    let cell = if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 { format!("[{name}]") } else { name };
                    line.push_str(&format!("{cell:<18}"));
                    if n % 5 == 4 {
                        out(&format!("{}\r\n", line.trim_end()));
                        line.clear();
                    }
                }
                if !line.is_empty() {
                    out(&format!("{}\r\n", line.trim_end()));
                }
            }
            for f in &entries {
                let name = narrow(&f.name);
                if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    dirs += 1;
                    if !o.wide {
                        let tag = if f.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 { "<SYMLINKD>" } else { "<DIR>     " };
                        out(&format!("{}    {tag}     {name}\r\n", file_time(f.written)));
                    }
                } else {
                    files += 1;
                    bytes += size(f);
                    if !o.wide {
                        if f.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                            out(&format!("{}    <SYMLINK>      {name}\r\n", file_time(f.written)));
                        } else {
                            out(&format!("{}    {:>14} {name}\r\n", file_time(f.written), grouped(size(f))));
                        }
                    }
                }
            }
            if o.tree {
                out(&format!("{files:>16} File(s) {:>14} bytes\r\n", grouped(bytes)));
            } else {
                total_dirs += dirs;
            }
            total_files += files;
            total_bytes += bytes;
        }
    }
    if !found_any {
        if !o.bare {
            out("\r\n");
        }
        err("File Not Found\r\n");
        shell.errorlevel = 1;
        return;
    }
    if !o.bare {
        let mut free = 0u64;
        let root = wide("C:\\");
        unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free, core::ptr::null_mut(), core::ptr::null_mut()) };
        if o.tree {
            out(&format!("\r\n     Total Files Listed:\r\n{total_files:>16} File(s) {:>14} bytes\r\n", grouped(total_bytes)));
        } else {
            out(&format!("{total_files:>16} File(s) {:>14} bytes\r\n", grouped(total_bytes)));
        }
        out(&format!("{total_dirs:>16} Dir(s) {:>15} bytes free\r\n", grouped(free)));
    }
}
