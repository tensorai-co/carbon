//! Filling a file system from a tar archive (ustar, with GNU long names and PAX paths), as an image's layers come.
//! Each entry's mode, owner, group and device number go to `meta`, which makes the kernel's own `M` from them.

use alloc::string::{String, ToString};

use crate::{Error, Fs, Ino, New, ROOT, Result};

/// What a tar entry says about a file, for the kernel's `M`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Permission bits, with the type bits for devices and FIFOs.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// A device's major and minor numbers, packed as `major << 8 | minor`.
    pub rdev: u64,
}

fn octal(field: &[u8]) -> u64 {
    field.iter().skip_while(|&&b| b == b' ').take_while(|&&b| (b'0'..=b'7').contains(&b)).fold(0, |n, &b| n * 8 + u64::from(b - b'0'))
}

fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Adds every entry of `archive` to `fs`, over what's there. Whiteouts (`.wh.` files, as OCI layers have) remove what
/// they name. Gives how many entries it added.
pub fn import<M: Clone>(fs: &mut Fs<M>, archive: &[u8], mut meta: impl FnMut(Entry) -> M) -> Result<usize> {
    let mut at = 0;
    let mut added = 0;
    let (mut long_name, mut long_link): (Option<String>, Option<String>) = (None, None);
    while at + 512 <= archive.len() {
        let h = &archive[at..at + 512];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let size = octal(&h[124..136]) as usize;
        let data = archive.get(at + 512..at + 512 + size).ok_or(Error::BadName)?;
        at += 512 + size.next_multiple_of(512);
        let kind = h[156];
        match kind {
            b'L' => {
                long_name = Some(text(data));
                continue;
            }
            b'K' => {
                long_link = Some(text(data));
                continue;
            }
            b'x' => {
                // PAX: only the path and link path matter here.
                for record in text(data).split('\n') {
                    if let Some((_, kv)) = record.split_once(' ') {
                        if let Some(path) = kv.strip_prefix("path=") {
                            long_name = Some(path.to_string());
                        } else if let Some(path) = kv.strip_prefix("linkpath=") {
                            long_link = Some(path.to_string());
                        }
                    }
                }
                continue;
            }
            b'g' => continue,
            _ => {}
        }
        let mut name = long_name.take().unwrap_or_else(|| {
            let prefix = if &h[257..262] == b"ustar" { text(&h[345..500]) } else { String::new() };
            let name = text(&h[0..100]);
            if prefix.is_empty() { name } else { alloc::format!("{prefix}/{name}") }
        });
        let link = long_link.take().unwrap_or_else(|| text(&h[157..257]));
        while let Some(rest) = name.strip_prefix("./") {
            name = rest.to_string();
        }
        let path = name.trim_end_matches('/');
        if path.is_empty() || path == "." {
            continue;
        }
        let entry = Entry {
            mode: octal(&h[100..108]) as u32,
            uid: octal(&h[108..116]) as u32,
            gid: octal(&h[116..124]) as u32,
            rdev: octal(&h[329..337]) << 8 | octal(&h[337..345]),
        };
        let mtime = octal(&h[136..148]) * 1_000_000_000;
        let (dir, last) = make_parents(fs, path, &mut meta)?;
        if let Some(gone) = last.strip_prefix(".wh.") {
            if gone == ".wh..opq" {
                for (child, _, _) in fs.entries(dir)? {
                    remove_all(fs, dir, &child)?;
                }
            } else {
                let _ = remove_all(fs, dir, gone);
            }
            continue;
        }
        // A new entry replaces an old one, unless both are directories.
        if let Ok(old) = fs.lookup(dir, last) {
            if kind == b'5' && fs.stat(old)?.kind == crate::Kind::Dir {
                fs.set_meta(old, meta(entry))?;
                continue;
            }
            remove_all(fs, dir, last)?;
        }
        let ino = match kind {
            b'5' => fs.create(dir, last, New::Dir, meta(entry), mtime)?,
            b'2' => fs.create(dir, last, New::Symlink(link), meta(entry), mtime)?,
            b'1' => {
                let target = fs.resolve(ROOT, link.trim_start_matches("./"), false)?;
                fs.link(dir, last, target, mtime)?;
                continue;
            }
            b'3' | b'4' | b'6' => {
                let type_bits = match kind {
                    b'3' => 0o020000,
                    b'4' => 0o060000,
                    _ => 0o010000,
                };
                fs.create(dir, last, New::File, meta(Entry { mode: entry.mode | type_bits, ..entry }), mtime)?
            }
            _ => {
                let ino = fs.create(dir, last, New::File, meta(entry), mtime)?;
                fs.write(ino, 0, data, mtime)?;
                ino
            }
        };
        fs.set_modified(ino, mtime)?;
        added += 1;
    }
    Ok(added)
}

/// The directory `path`'s last part goes in, made if it isn't there, and that last part.
fn make_parents<'p, M: Clone>(fs: &mut Fs<M>, path: &'p str, meta: &mut impl FnMut(Entry) -> M) -> Result<(Ino, &'p str)> {
    let (dirs, last) = path.rsplit_once('/').unwrap_or(("", path));
    let mut at = ROOT;
    for part in dirs.split('/').filter(|p| !p.is_empty() && *p != ".") {
        at = match fs.lookup(at, part) {
            Ok(ino) if fs.stat(ino)?.kind == crate::Kind::Dir => ino,
            Ok(_) => return Err(Error::NotDir),
            Err(Error::NotFound) => fs.create(at, part, New::Dir, meta(Entry { mode: 0o755, uid: 0, gid: 0, rdev: 0 }), 0)?,
            Err(e) => return Err(e),
        };
    }
    Ok((at, last))
}

fn remove_all<M: Clone>(fs: &mut Fs<M>, dir: Ino, name: &str) -> Result<()> {
    let ino = fs.lookup(dir, name)?;
    if fs.stat(ino)?.kind == crate::Kind::Dir {
        for (child, _, _) in fs.entries(ino)? {
            remove_all(fs, ino, &child)?;
        }
    }
    fs.remove(dir, name, 0)
}
