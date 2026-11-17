//! `ADD` of a local archive: a tar, plain or gzipped, found by what it holds (not its name, as Docker finds it), its
//! entries written into a machine's folder with their modes, owners, times and links.

use crate::error::{Error, Result};
use crate::files::Op;
use crate::Files;

/// What an archive is compressed with, from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Packed {
    Tar,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

/// Whether `bytes` start an archive `ADD` unpacks, and how it's compressed.
pub(crate) fn detect(bytes: &[u8]) -> Option<Packed> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        Some(Packed::Gzip)
    } else if bytes.starts_with(b"BZh") {
        Some(Packed::Bzip2)
    } else if bytes.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        Some(Packed::Xz)
    } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        Some(Packed::Zstd)
    } else if bytes.get(257..262) == Some(b"ustar") {
        Some(Packed::Tar)
    } else {
        None
    }
}

/// One tar entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub path: String,
    pub kind: u8,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u64,
    pub link: String,
    pub data: std::ops::Range<usize>,
}

fn octal(field: &[u8]) -> u64 {
    // GNU's base-256 for big numbers: the first byte's top bit set.
    if field.first().is_some_and(|b| b & 0x80 != 0) {
        return field[1..].iter().fold(u64::from(field[0] & 0x7f), |n, &b| n << 8 | u64::from(b));
    }
    field.iter().skip_while(|&&b| b == b' ' || b == 0).take_while(|&&b| (b'0'..=b'7').contains(&b)).fold(0, |n, &b| n * 8 + u64::from(b - b'0'))
}

fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// A tar's entries: ustar, with GNU long names and links and PAX's `path` and `linkpath`.
pub(crate) fn entries(tar: &[u8]) -> Result<Vec<Entry>> {
    let bad = || Error::invalid("the archive is cut short or isn't a tar");
    let mut out = Vec::new();
    let (mut long_name, mut long_link): (Option<String>, Option<String>) = (None, None);
    let mut at = 0;
    while at + 512 <= tar.len() {
        let h = &tar[at..at + 512];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let size = octal(&h[124..136]) as usize;
        let start = at + 512;
        let end = start.checked_add(size).filter(|&e| e <= tar.len()).ok_or_else(bad)?;
        at = start + size.div_ceil(512) * 512;
        let kind = h[156];
        match kind {
            b'L' => {
                long_name = Some(text(&tar[start..end]));
                continue;
            }
            b'K' => {
                long_link = Some(text(&tar[start..end]));
                continue;
            }
            b'x' => {
                // PAX records: "LEN key=value\n".
                let records = String::from_utf8_lossy(&tar[start..end]).into_owned();
                for record in records.lines() {
                    let Some((_, kv)) = record.split_once(' ') else { continue };
                    if let Some(v) = kv.strip_prefix("path=") {
                        long_name = Some(v.to_string());
                    } else if let Some(v) = kv.strip_prefix("linkpath=") {
                        long_link = Some(v.to_string());
                    }
                }
                continue;
            }
            b'g' => continue,
            _ => {}
        }
        let mut path = text(&h[..100]);
        if &h[257..262] == b"ustar" {
            let prefix = text(&h[345..500]);
            if !prefix.is_empty() {
                path = format!("{prefix}/{path}");
            }
        }
        let path = long_name.take().unwrap_or(path);
        let link = long_link.take().unwrap_or_else(|| text(&h[157..257]));
        out.push(Entry { path, kind, mode: octal(&h[100..108]) as u32 & 0o7777, uid: octal(&h[108..116]) as u32, gid: octal(&h[116..124]) as u32, mtime: octal(&h[136..148]), link, data: start..end });
    }
    Ok(out)
}

/// A path in an archive made safe under `dest`: no `..` past it, no leading `/`.
fn under(dest: &str, path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| format!("{}/{}", dest.trim_end_matches('/'), parts.join("/")))
}

/// Unpacks an archive's bytes into the machine's folder `dest`: owners as the archive has them unless `owner` names
/// one. How many entries it wrote.
pub(crate) fn unpack(files: &Files, bytes: &[u8], dest: &str, owner: Option<&str>) -> Result<usize> {
    let tar = match detect(bytes) {
        Some(Packed::Tar) => bytes.to_vec(),
        Some(Packed::Gzip) => images::gunzip(bytes).map_err(Error::invalid)?,
        Some(other) => return Err(Error::unsupported(format!("an archive compressed with {other:?} isn't unpacked yet (gzip and plain tar are)"))),
        None => return Err(Error::invalid("not an archive")),
    };
    let entries = entries(&tar)?;
    files.batch(|op| {
        op(Op::Mkdir { path: dest.to_string(), mode: None, owner: None, parents: true, modified: None }, &[])?;
        let mut n = 0;
        let mut dirs = Vec::new();
        for e in &entries {
            let Some(path) = under(dest, &e.path) else { continue };
            let who = Some(owner.map_or_else(|| format!("{}:{}", e.uid, e.gid), String::from));
            let ns = Some(e.mtime.saturating_mul(1_000_000_000));
            match e.kind {
                b'0' | 0 | b'7' => {
                    op(Op::Write { path: path.clone(), len: e.data.len() as u64, mode: Some(e.mode), owner: who, parents: true, append: false, modified: ns }, &tar[e.data.clone()])?;
                }
                b'5' => {
                    op(Op::Mkdir { path: path.clone(), mode: Some(e.mode), owner: who.clone(), parents: true, modified: None }, &[])?;
                    dirs.push((path, e.mode, who, ns));
                }
                b'2' => {
                    let _ = op(Op::Remove { path: path.clone(), recursive: false }, &[]);
                    op(Op::Symlink { path, target: e.link.clone(), owner: who }, &[])?;
                }
                b'1' => {
                    let Some(to) = under(dest, &e.link) else { continue };
                    let _ = op(Op::Remove { path: path.clone(), recursive: false }, &[]);
                    op(Op::Link { path, to }, &[])?;
                }
                // Devices and FIFOs: nothing a build's ADD carries.
                _ => continue,
            }
            n += 1;
        }
        // Folders' times last, since what was put in them changed them.
        for (path, mode, who, ns) in dirs.into_iter().rev() {
            op(Op::Mkdir { path, mode: Some(mode), owner: who, parents: true, modified: ns }, &[])?;
        }
        Ok(n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_under_the_destination() {
        assert_eq!(under("/opt", "./a/b").as_deref(), Some("/opt/a/b"));
        assert_eq!(under("/opt/", "/etc/x").as_deref(), Some("/opt/etc/x"));
        assert_eq!(under("/opt", "a/../../x"), None);
        assert_eq!(under("/opt", "./"), None);
    }

    #[test]
    fn archives_found_by_what_they_hold() {
        assert_eq!(detect(&[0x1f, 0x8b, 8, 0]), Some(Packed::Gzip));
        assert_eq!(detect(b"BZh91AY"), Some(Packed::Bzip2));
        assert_eq!(detect(b"hello"), None);
        let tar = crate::files::tests::rootfs_tar(&[("a/b.txt", b"hi", 0o640)]);
        assert_eq!(detect(&tar), Some(Packed::Tar));
        let e = entries(&tar).unwrap();
        assert_eq!((e[0].path.as_str(), e[0].mode, &tar[e[0].data.clone()]), ("a/b.txt", 0o640, &b"hi"[..]));
    }
}
