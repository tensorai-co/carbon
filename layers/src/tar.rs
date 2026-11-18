//! Tar archives and file systems, both ways. `import` fills a file system from one (ustar, with GNU long names and PAX
//! records), as an image's layers come: each entry's mode, owner, group and device number go to `meta`, which makes
//! the kernel's own `M` from them. `export` writes one: a whole tree, or what changed from another tree (a layer, with
//! OCI's whiteouts for what's gone), each file's `M` told as an entry by the kernel.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::{Error, Fs, Ino, Kind, New, ROOT, Result};

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

/// What a PAX header said of the entry after it.
#[derive(Default)]
struct Pax {
    size: Option<u64>,
    uid: Option<u64>,
    gid: Option<u64>,
    mtime: Option<u64>,
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
    // A PAX header's numbers, for the entry after it: size, owner, group, time.
    let mut pax: Pax = Pax::default();
    while at + 512 <= archive.len() {
        let h = &archive[at..at + 512];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let size = pax.size.take().unwrap_or_else(|| octal(&h[124..136])) as usize;
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
                // PAX: the path, link path and the numbers a ustar header can't hold.
                for record in text(data).split('\n') {
                    if let Some((_, kv)) = record.split_once(' ') {
                        let number = |v: &str| v.split('.').next().and_then(|n| n.parse::<u64>().ok());
                        if let Some(path) = kv.strip_prefix("path=") {
                            long_name = Some(path.to_string());
                        } else if let Some(path) = kv.strip_prefix("linkpath=") {
                            long_link = Some(path.to_string());
                        } else if let Some(v) = kv.strip_prefix("size=") {
                            pax.size = number(v);
                        } else if let Some(v) = kv.strip_prefix("uid=") {
                            pax.uid = number(v);
                        } else if let Some(v) = kv.strip_prefix("gid=") {
                            pax.gid = number(v);
                        } else if let Some(v) = kv.strip_prefix("mtime=") {
                            pax.mtime = number(v);
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
            uid: pax.uid.take().unwrap_or_else(|| octal(&h[108..116])) as u32,
            gid: pax.gid.take().unwrap_or_else(|| octal(&h[116..124])) as u32,
            rdev: octal(&h[329..337]) << 8 | octal(&h[337..345]),
        };
        let mtime = pax.mtime.take().unwrap_or_else(|| octal(&h[136..148])) * 1_000_000_000;
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

const S_IFMT: u32 = 0o170000;
const S_IFIFO: u32 = 0o010000;
const S_IFCHR: u32 = 0o020000;
const S_IFBLK: u32 = 0o060000;

/// Writes `fs` as a tar, to `out` a piece at a time: all of it, or with `base`, only what differs from `base` (a
/// layer, as OCI has them, for an image built on `base`'s). Each file's `M` is told as an entry by `entry`, with the
/// type bits for a device or FIFO. A file or link that's new or changed is written whole (its contents, mode, owner,
/// time or target changed); a folder, when its own mode, owner or time changed, or it's new; whatever's gone, as a
/// whiteout (`.wh.NAME`). A file with other names written before it in the same archive is a hard link to the first.
/// Names are in each folder's order, so the same trees always make the same bytes. Gives how many entries it wrote:
/// none means nothing differs. Sockets aren't written (a tar has no kind for them), nor extended attributes.
pub fn export<M: Clone>(fs: &Fs<M>, base: Option<&Fs<M>>, mut entry: impl FnMut(&M) -> Entry, out: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<usize> {
    let mut w = Writer { out, entries: 0, firsts: BTreeMap::new() };
    let base = base.map(|b| (b, ROOT));
    w.dir(fs, ROOT, base, "", &mut entry)?;
    (w.out)(&[0u8; 1024])?;
    Ok(w.entries)
}

struct Writer<'o> {
    out: &'o mut dyn FnMut(&[u8]) -> Result<()>,
    entries: usize,
    /// Files with more than one name, by inode, and the first name each was written as.
    firsts: BTreeMap<Ino, String>,
}

impl Writer<'_> {
    /// A folder's entries: what's gone from `base`'s, then each of its own, new, changed or (a folder) looked into.
    fn dir<M: Clone>(&mut self, fs: &Fs<M>, dir: Ino, base: Option<(&Fs<M>, Ino)>, prefix: &str, entry: &mut impl FnMut(&M) -> Entry) -> Result<()> {
        if let Some((b, bdir)) = base {
            for (name, _, _) in b.entries(bdir)? {
                if matches!(fs.lookup(dir, &name), Err(Error::NotFound)) {
                    self.header(&format!("{prefix}.wh.{name}"), b'0', Entry { mode: 0, uid: 0, gid: 0, rdev: 0 }, 0, 0, "")?;
                }
            }
        }
        for (name, ino, kind) in fs.entries(dir)? {
            let path = format!("{prefix}{name}");
            let old = base.and_then(|(b, bdir)| b.lookup(bdir, &name).ok().map(|o| (b, o))).filter(|(b, o)| b.stat(*o).is_ok_and(|s| s.kind == kind));
            match (kind, old) {
                (Kind::Dir, Some((b, o))) => {
                    if differs(fs, ino, b, o, entry)? {
                        self.write(fs, ino, &path, entry)?;
                    }
                    self.dir(fs, ino, Some((b, o)), &format!("{path}/"), entry)?;
                }
                (Kind::Dir, None) => {
                    self.write(fs, ino, &path, entry)?;
                    self.dir(fs, ino, None, &format!("{path}/"), entry)?;
                }
                (_, Some((b, o))) => {
                    if differs(fs, ino, b, o, entry)? || !same_contents(fs, ino, b, o)? {
                        self.write(fs, ino, &path, entry)?;
                    }
                }
                (_, None) => self.write(fs, ino, &path, entry)?,
            }
        }
        Ok(())
    }

    /// One file, folder or link, whole.
    fn write<M: Clone>(&mut self, fs: &Fs<M>, ino: Ino, path: &str, entry: &mut impl FnMut(&M) -> Entry) -> Result<()> {
        let stat = fs.stat(ino)?;
        let e = entry(&fs.meta(ino)?);
        let mtime = stat.modified / 1_000_000_000;
        match stat.kind {
            Kind::Dir => self.header(&format!("{path}/"), b'5', e, 0, mtime, ""),
            Kind::Symlink => self.header(path, b'2', e, 0, mtime, &fs.read_link(ino)?),
            Kind::File => {
                let special = match e.mode & S_IFMT {
                    S_IFCHR => Some(b'3'),
                    S_IFBLK => Some(b'4'),
                    S_IFIFO => Some(b'6'),
                    0 | 0o100000 => None,
                    _ => return Ok(()),
                };
                if let Some(kind) = special {
                    return self.header(path, kind, e, 0, mtime, "");
                }
                if stat.links > 1 {
                    if let Some(first) = self.firsts.get(&ino).cloned() {
                        return self.header(path, b'1', e, 0, mtime, &first);
                    }
                    self.firsts.insert(ino, path.to_string());
                }
                self.header(path, b'0', e, stat.size, mtime, "")?;
                let mut piece = vec![0u8; 1 << 20];
                let mut at = 0u64;
                while at < stat.size {
                    let want = (stat.size - at).min(piece.len() as u64) as usize;
                    let n = fs.read(ino, at, &mut piece[..want])?;
                    if n == 0 {
                        return Err(Error::Io);
                    }
                    (self.out)(&piece[..n])?;
                    at += n as u64;
                }
                let pad = (stat.size.next_multiple_of(512) - stat.size) as usize;
                (self.out)(&[0u8; 512][..pad])
            }
        }
    }

    /// A ustar header, after a PAX one for whatever doesn't fit it (a long path or link, a big owner, size or time).
    fn header(&mut self, path: &str, kind: u8, e: Entry, size: u64, mtime: u64, link: &str) -> Result<()> {
        let mut pax = String::new();
        let mut record = |key: &str, value: &str| {
            // Each record's length counts its own digits.
            let body = format!(" {key}={value}\n");
            let mut len = body.len() + 1;
            while format!("{len}").len() + body.len() != len {
                len += 1;
            }
            pax.push_str(&format!("{len}{body}"));
        };
        if path.len() > 100 {
            record("path", path);
        }
        if link.len() > 100 {
            record("linkpath", link);
        }
        for (key, value, max) in [("uid", u64::from(e.uid), 0o7777777), ("gid", u64::from(e.gid), 0o7777777), ("size", size, 0o77777777777), ("mtime", mtime, 0o77777777777)] {
            if value > max {
                record(key, &format!("{value}"));
            }
        }
        if !pax.is_empty() {
            let name: String = format!("PaxHeaders/{}", path.trim_end_matches('/')).chars().take(100).collect();
            (self.out)(&ustar(&name, b'x', Entry { mode: 0o644, uid: 0, gid: 0, rdev: 0 }, pax.len() as u64, mtime, ""))?;
            (self.out)(pax.as_bytes())?;
            let pad = pax.len().next_multiple_of(512) - pax.len();
            (self.out)(&[0u8; 512][..pad])?;
        }
        (self.out)(&ustar(path, kind, e, size, mtime, link))?;
        self.entries += 1;
        Ok(())
    }
}

/// The header blocks for a plain file of `size` bytes at `path` in an archive (a PAX one first if the path is long),
/// mode 644 and owned by root, for archives of files that aren't a file system's (an image's blobs, as `docker save`
/// writes them). Its contents go after, padded to 512 bytes.
pub fn file_header(path: &str, size: u64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut sink = |b: &[u8]| {
        out.extend_from_slice(b);
        Ok(())
    };
    let mut w = Writer { out: &mut sink, entries: 0, firsts: BTreeMap::new() };
    let _ = w.header(path, b'0', Entry { mode: 0o644, uid: 0, gid: 0, rdev: 0 }, size, 0, "");
    out
}

/// A ustar header block; fields too long or too big are cut (a PAX header before it says them in full).
fn ustar(path: &str, kind: u8, e: Entry, size: u64, mtime: u64, link: &str) -> [u8; 512] {
    fn put(h: &mut [u8; 512], at: usize, len: usize, bytes: &[u8]) {
        let n = bytes.len().min(len);
        h[at..at + n].copy_from_slice(&bytes[..n]);
    }
    let mut h = [0u8; 512];
    let num = |value: u64, len: usize| {
        let max = (1u64 << (3 * (len - 1))) - 1;
        format!("{:0width$o}\0", value.min(max), width = len - 1)
    };
    put(&mut h, 0, 100, path.as_bytes());
    put(&mut h, 100, 8, num(u64::from(e.mode & 0o7777), 8).as_bytes());
    put(&mut h, 108, 8, num(u64::from(e.uid), 8).as_bytes());
    put(&mut h, 116, 8, num(u64::from(e.gid), 8).as_bytes());
    put(&mut h, 124, 12, num(size, 12).as_bytes());
    put(&mut h, 136, 12, num(mtime, 12).as_bytes());
    put(&mut h, 148, 8, b"        ");
    put(&mut h, 156, 1, &[kind]);
    put(&mut h, 157, 100, link.as_bytes());
    put(&mut h, 257, 6, b"ustar\0");
    put(&mut h, 263, 2, b"00");
    if matches!(kind, b'3' | b'4') {
        put(&mut h, 329, 8, num(e.rdev >> 8, 8).as_bytes());
        put(&mut h, 337, 8, num(e.rdev & 0xff, 8).as_bytes());
    }
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    put(&mut h, 148, 8, format!("{sum:06o}\0 ").as_bytes());
    h
}

/// Whether a file's own metadata differs from another's: its kind of entry (mode, owner, device), time, size, links'
/// target.
fn differs<M: Clone>(fs: &Fs<M>, ino: Ino, base: &Fs<M>, old: Ino, entry: &mut impl FnMut(&M) -> Entry) -> Result<bool> {
    let (a, b) = (fs.stat(ino)?, base.stat(old)?);
    if a.kind != b.kind || a.modified / 1_000_000_000 != b.modified / 1_000_000_000 || entry(&fs.meta(ino)?) != entry(&base.meta(old)?) {
        return Ok(true);
    }
    match a.kind {
        Kind::Dir => Ok(false),
        Kind::Symlink => Ok(fs.read_link(ino)? != base.read_link(old)?),
        Kind::File => Ok(a.size != b.size),
    }
}

/// Whether two files of the same size hold the same bytes: by their chunks' names when both have them, else read.
fn same_contents<M: Clone>(fs: &Fs<M>, ino: Ino, base: &Fs<M>, old: Ino) -> Result<bool> {
    if fs.stat(ino)?.kind != Kind::File {
        return Ok(true);
    }
    if let (Some(a), Some(b)) = (fs.chunk_names(ino)?, base.chunk_names(old)?) {
        return Ok(a == b);
    }
    let size = fs.stat(ino)?.size;
    let (mut x, mut y) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    let mut at = 0u64;
    while at < size {
        let want = (size - at).min(x.len() as u64) as usize;
        let (n, m) = (fs.read(ino, at, &mut x[..want])?, base.read(old, at, &mut y[..want])?);
        if n != m || x[..n] != y[..m] {
            return Ok(false);
        }
        if n == 0 {
            break;
        }
        at += n as u64;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use alloc::boxed::Box;
    use alloc::vec::Vec;

    /// A kernel's `M`, for tests: mode, owner, group, device.
    type M = [u32; 4];

    fn meta(e: Entry) -> M {
        [e.mode, e.uid, e.gid, e.rdev as u32]
    }

    fn entry(m: &M) -> Entry {
        Entry { mode: m[0], uid: m[1], gid: m[2], rdev: u64::from(m[3]) }
    }

    fn tar_of(fs: &Fs<M>, base: Option<&Fs<M>>) -> (Vec<u8>, usize) {
        let mut out = Vec::new();
        let n = export(fs, base, entry, &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        (out, n)
    }

    const S: u64 = 1_000_000_000;

    fn tree() -> Fs<M> {
        let mut fs: Fs<M> = Fs::building([0o755, 0, 0, 0], false, Box::new(MemoryStore::default()));
        let etc = fs.create(ROOT, "etc", New::Dir, [0o755, 0, 0, 0], 5 * S).unwrap();
        let f = fs.create(etc, "hostname", New::File, [0o644, 0, 0, 0], 6 * S).unwrap();
        fs.write(f, 0, b"box\n", 6 * S).unwrap();
        let big = fs.create(etc, "big", New::File, [0o600, 1000, 1000, 0], 7 * S).unwrap();
        fs.write(big, 0, &vec![7u8; 200_000], 7 * S).unwrap();
        fs.link(etc, "big-again", big, 7 * S).unwrap();
        fs.create(etc, "link", New::Symlink("hostname".into()), [0o777, 0, 0, 0], 8 * S).unwrap();
        fs.create(etc, "fifo", New::File, [0o010644, 0, 0, 0], 8 * S).unwrap();
        fs.create(etc, "null", New::File, [0o020666, 0, 0, (1 << 8) | 3], 8 * S).unwrap();
        let long = "d".repeat(120);
        let deep = fs.create(ROOT, &long, New::Dir, [0o700, 70_000_000, 5, 0], 9 * S).unwrap();
        let g = fs.create(deep, "file", New::File, [0o640, 70_000_000, 5, 0], 9 * S).unwrap();
        fs.write(g, 0, b"far", 9 * S).unwrap();
        for ino in [etc, f, big, deep, g] {
            let at = fs.stat(ino).unwrap().modified;
            fs.set_modified(ino, at).unwrap();
        }
        let freeze = fs.freeze().unwrap();
        let store = fs.store.take().unwrap().into_inner();
        Fs::open(Some(&freeze), store, [0o755, 0, 0, 0], false).unwrap()
    }

    fn imported(base: Option<&Fs<M>>, layers: &[&[u8]]) -> Fs<M> {
        let mut fs: Fs<M> = Fs::building([0o755, 0, 0, 0], false, Box::new(MemoryStore::default()));
        if let Some(b) = base {
            import(&mut fs, &tar_of(b, None).0, meta).unwrap();
        }
        for layer in layers {
            import(&mut fs, layer, meta).unwrap();
        }
        fs
    }

    #[test]
    fn a_tree_written_and_read_back_is_the_same_tree() {
        let fs = tree();
        let (bytes, n) = tar_of(&fs, None);
        assert_eq!(n, 9, "two folders, seven files and links");
        assert_eq!(tar_of(&fs, None).0, bytes, "the same tree makes the same bytes");
        let back = imported(None, &[&bytes]);
        assert_eq!(tar_of(&back, Some(&fs)).1, 0, "nothing differs");
        assert_eq!(tar_of(&fs, Some(&back)).1, 0);
        let big = back.resolve(ROOT, "/etc/big", true).unwrap();
        assert_eq!(back.stat(big).unwrap().links, 2, "a hard link stays one");
        assert_eq!(back.meta(back.resolve(ROOT, "/etc/null", true).unwrap()).unwrap(), [0o020666, 0, 0, (1 << 8) | 3]);
        let far = back.resolve(ROOT, &alloc::format!("/{}/file", "d".repeat(120)), true).unwrap();
        assert_eq!((back.contents(far).unwrap(), back.meta(far).unwrap()[1]), (b"far".to_vec(), 70_000_000), "a long path, a big owner");
    }

    #[test]
    fn a_layer_is_what_changed() {
        let base = tree();
        let mut next = tree();
        let etc = next.resolve(ROOT, "/etc", true).unwrap();
        let host = next.lookup(etc, "hostname").unwrap();
        next.write(host, 0, b"new\n", 20 * S).unwrap();
        next.remove(etc, "link", 20 * S).unwrap();
        let long = "d".repeat(120);
        let deep = next.lookup(ROOT, &long).unwrap();
        next.remove(deep, "file", 20 * S).unwrap();
        next.remove(ROOT, &long, 20 * S).unwrap();
        let added = next.create(ROOT, "srv", New::Dir, [0o755, 0, 0, 0], 21 * S).unwrap();
        next.create(added, "x", New::File, [0o644, 0, 0, 0], 21 * S).unwrap();
        let big = next.lookup(etc, "big").unwrap();
        next.set_meta(big, [0o644, 1000, 1000, 0]).unwrap();
        let (layer, n) = tar_of(&next, Some(&base));
        // /etc (its time), hostname, big and its other name, two whiteouts, /srv and its file.
        assert!(n <= 9, "{n} entries");
        let text = String::from_utf8_lossy(&layer);
        assert!(text.contains(".wh.link") && text.contains(&alloc::format!(".wh.{long}")));
        assert!(!text.contains("etc/null"), "what didn't change isn't in it");
        let applied = imported(Some(&base), &[&layer]);
        assert_eq!(tar_of(&applied, Some(&next)).1, 0, "the base and the layer make the new tree");
        assert_eq!(tar_of(&next, Some(&applied)).1, 0);
        assert_eq!(tar_of(&next, Some(&next)).1, 0, "a tree doesn't differ from itself");
    }
}
