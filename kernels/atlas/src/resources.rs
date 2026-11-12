//! Resources, as a PE image keeps them: the resource directory (types, then names, then languages), string tables, and
//! version information (`VS_VERSIONINFO`). Directories are read through a reader of RVAs, so the same code reads a
//! loaded module's memory and a file's bytes; and they're built, for the DLLs atlas makes.

use std::collections::BTreeMap;

pub const RT_STRING: u16 = 6;
pub const RT_VERSION: u16 = 16;

pub const ERROR_RESOURCE_DATA_NOT_FOUND: u32 = 1812;
pub const ERROR_RESOURCE_TYPE_NOT_FOUND: u32 = 1813;
pub const ERROR_RESOURCE_NAME_NOT_FOUND: u32 = 1814;
pub const ERROR_RESOURCE_LANG_NOT_FOUND: u32 = 1815;

/// A type's, name's or language's ID: a number, or a name (kept in upper case, as resource compilers store them).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Id {
    Name(String),
    Num(u16),
}

impl Id {
    /// A name as a program gives it: `#123` is the number 123; other names compare in upper case.
    pub fn named(s: &str) -> Id {
        match s.strip_prefix('#').and_then(|n| n.parse().ok()) {
            Some(n) => Id::Num(n),
            None => Id::Name(s.to_uppercase()),
        }
    }
}

/// One entry of a directory: its ID, where what it leads to is (an RVA), and whether that's another directory.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: Id,
    pub at: u32,
    pub directory: bool,
}

/// What's found: where its `IMAGE_RESOURCE_DATA_ENTRY` is, where its bytes are and how many, and its language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub entry: u32,
    pub data: u32,
    pub size: u32,
    pub lang: u16,
}

/// A resource directory, read through `read(rva, length)`, rooted at `root` (the resource data directory's RVA).
pub struct Tree<R: FnMut(u32, usize) -> Option<Vec<u8>>> {
    pub root: u32,
    read: R,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

impl<R: FnMut(u32, usize) -> Option<Vec<u8>>> Tree<R> {
    pub fn new(root: u32, read: R) -> Tree<R> {
        Tree { root, read }
    }

    /// A directory's entries, in its order: named ones first, then numbered, each in order.
    pub fn entries(&mut self, at: u32) -> Option<Vec<Entry>> {
        let head = (self.read)(at, 16)?;
        let count = (usize::from(u16_at(&head, 12)) + usize::from(u16_at(&head, 14))).min(4096);
        let list = (self.read)(at + 16, 8 * count)?;
        (0..count)
            .map(|i| {
                let (name, offset) = (u32_at(&list, 8 * i), u32_at(&list, 8 * i + 4));
                let id = if name & 0x8000_0000 != 0 {
                    let string = self.root + (name & 0x7fff_ffff);
                    let len = usize::from(u16_at(&(self.read)(string, 2)?, 0));
                    let units: Vec<u16> = (self.read)(string + 2, 2 * len)?.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    Id::Name(String::from_utf16_lossy(&units).to_uppercase())
                } else {
                    Id::Num(name as u16)
                };
                Some(Entry { id, at: self.root + (offset & 0x7fff_ffff), directory: offset & 0x8000_0000 != 0 })
            })
            .collect()
    }

    /// The types there are; a type's names; a name's languages.
    pub fn types(&mut self) -> Option<Vec<Entry>> {
        self.entries(self.root)
    }

    fn child(&mut self, at: u32, id: &Id) -> Option<Entry> {
        self.entries(at)?.into_iter().find(|e| e.id == *id && e.directory)
    }

    pub fn names(&mut self, kind: &Id) -> Option<Vec<Entry>> {
        let t = self.child(self.root, kind)?;
        self.entries(t.at)
    }

    pub fn languages(&mut self, kind: &Id, name: &Id) -> Option<Vec<Entry>> {
        let t = self.child(self.root, kind)?;
        let n = self.child(t.at, name)?;
        self.entries(n.at)
    }

    /// A resource of a type and name, in a language: exactly it, or with none given (or neutral), the first of
    /// neutral, the user's (0x400), US English and any English, else the first there is.
    pub fn find(&mut self, kind: &Id, name: &Id, lang: Option<u16>) -> Result<Found, u32> {
        let types = self.types().ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
        let t = types.into_iter().find(|e| e.id == *kind && e.directory).ok_or(ERROR_RESOURCE_TYPE_NOT_FOUND)?;
        let n = self.child(t.at, name).ok_or(ERROR_RESOURCE_NAME_NOT_FOUND)?;
        let langs: Vec<(u16, Entry)> = self
            .entries(n.at)
            .ok_or(ERROR_RESOURCE_NAME_NOT_FOUND)?
            .into_iter()
            .filter(|e| !e.directory)
            .filter_map(|e| match e.id {
                Id::Num(l) => Some((l, e)),
                Id::Name(_) => None,
            })
            .collect();
        let pick = match lang.filter(|&l| l != 0) {
            Some(l) => langs.iter().find(|(x, _)| *x == l).ok_or(ERROR_RESOURCE_LANG_NOT_FOUND)?,
            None => [0u16, 0x400, 0x409]
                .iter()
                .find_map(|want| langs.iter().find(|(x, _)| x == want))
                .or_else(|| langs.iter().find(|(x, _)| x & 0x3ff == 9))
                .or_else(|| langs.first())
                .ok_or(ERROR_RESOURCE_NAME_NOT_FOUND)?,
        };
        let entry = pick.1.at;
        let data = (self.read)(entry, 8).ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
        Ok(Found { entry, data: u32_at(&data, 0), size: u32_at(&data, 4), lang: pick.0 })
    }
}

/// A string table block (`RT_STRING`, 16 strings): string `index`'s UTF-16 units, and where they start in the block.
pub fn table_string(block: &[u8], index: usize) -> Option<(usize, Vec<u16>)> {
    let mut at = 0;
    for i in 0..16 {
        let len = usize::from(u16::from_le_bytes([*block.get(at)?, *block.get(at + 1)?]));
        let start = at + 2;
        if i == index {
            let units = block.get(start..start + 2 * len)?.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            return Some((start, units));
        }
        at = start + 2 * len;
    }
    None
}

/// Builds a resource directory of numbered types, names and languages, to sit at `rva`: its bytes.
pub fn build(rva: u32, resources: &BTreeMap<(u16, u16, u16), Vec<u8>>) -> Vec<u8> {
    let mut tree: BTreeMap<u16, BTreeMap<u16, BTreeMap<u16, &Vec<u8>>>> = BTreeMap::new();
    for ((t, n, l), data) in resources {
        tree.entry(*t).or_default().entry(*n).or_default().insert(*l, data);
    }
    // The directories first (root, each type's, each name's), then the data entries, then the data.
    let dir_size = |entries: usize| 16 + 8 * entries;
    let mut size = dir_size(tree.len());
    let mut type_dirs = Vec::new();
    for names in tree.values() {
        type_dirs.push(size);
        size += dir_size(names.len());
    }
    let mut name_dirs = Vec::new();
    for names in tree.values() {
        for langs in names.values() {
            name_dirs.push(size);
            size += dir_size(langs.len());
        }
    }
    let leaves: Vec<&Vec<u8>> = tree.values().flat_map(|n| n.values().flat_map(|l| l.values().copied())).collect();
    let entries_at = size;
    size += 16 * leaves.len();
    let mut data_at = Vec::new();
    for leaf in &leaves {
        size = size.next_multiple_of(8);
        data_at.push(size);
        size += leaf.len();
    }
    let mut b = vec![0u8; size];
    let dir = |b: &mut Vec<u8>, at: usize, entries: &[(u16, u32)]| {
        b[at + 14..at + 16].copy_from_slice(&(entries.len() as u16).to_le_bytes());
        for (i, (id, to)) in entries.iter().enumerate() {
            b[at + 16 + 8 * i..at + 20 + 8 * i].copy_from_slice(&u32::from(*id).to_le_bytes());
            b[at + 20 + 8 * i..at + 24 + 8 * i].copy_from_slice(&to.to_le_bytes());
        }
    };
    let sub = |offset: usize| offset as u32 | 0x8000_0000;
    let root: Vec<(u16, u32)> = tree.keys().zip(&type_dirs).map(|(t, &at)| (*t, sub(at))).collect();
    dir(&mut b, 0, &root);
    let (mut name_i, mut leaf_i) = (0, 0);
    for (names, &type_at) in tree.values().zip(&type_dirs) {
        let list: Vec<(u16, u32)> = names.keys().enumerate().map(|(k, n)| (*n, sub(name_dirs[name_i + k]))).collect();
        dir(&mut b, type_at, &list);
        for langs in names.values() {
            let list: Vec<(u16, u32)> = langs.keys().enumerate().map(|(k, l)| (*l, (entries_at + 16 * (leaf_i + k)) as u32)).collect();
            dir(&mut b, name_dirs[name_i], &list);
            name_i += 1;
            leaf_i += langs.len();
        }
    }
    for (i, leaf) in leaves.iter().enumerate() {
        let e = entries_at + 16 * i;
        b[e..e + 4].copy_from_slice(&(rva + data_at[i] as u32).to_le_bytes());
        b[e + 4..e + 8].copy_from_slice(&(leaf.len() as u32).to_le_bytes());
        b[data_at[i]..data_at[i] + leaf.len()].copy_from_slice(leaf);
    }
    b
}

// --- Version information ---

/// A node of `VS_VERSIONINFO`: its key, its value (binary or text), and its children.
pub enum Value<'a> {
    Binary(&'a [u8]),
    Text(&'a str),
}

pub struct Node<'a> {
    pub key: &'a str,
    pub value: Option<Value<'a>>,
    pub children: Vec<Node<'a>>,
}

fn pad4(b: &mut Vec<u8>) {
    b.resize(b.len().next_multiple_of(4), 0);
}

impl Node<'_> {
    /// Its bytes: wLength, wValueLength (bytes for binary, characters with the NUL for text), wType, the key, the value,
    /// then the children, each 4-aligned.
    pub fn bytes(&self) -> Vec<u8> {
        let mut b = vec![0u8; 6];
        let (value_len, kind) = match &self.value {
            Some(Value::Binary(v)) => (v.len(), 0u16),
            Some(Value::Text(t)) => (t.encode_utf16().count() + 1, 1),
            None => (0, 1),
        };
        b[2..4].copy_from_slice(&(value_len as u16).to_le_bytes());
        b[4..6].copy_from_slice(&kind.to_le_bytes());
        b.extend(self.key.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        pad4(&mut b);
        match &self.value {
            Some(Value::Binary(v)) => b.extend(*v),
            Some(Value::Text(t)) => b.extend(t.encode_utf16().chain([0]).flat_map(u16::to_le_bytes)),
            None => {}
        }
        for child in &self.children {
            pad4(&mut b);
            b.extend(child.bytes());
        }
        let len = b.len() as u16;
        b[0..2].copy_from_slice(&len.to_le_bytes());
        b
    }
}

/// Finds a value in a version block by its path (`\`, `\VarFileInfo\Translation`,
/// `\StringFileInfo\040904B0\FileVersion`; keys in any case): where its value starts, and its `wValueLength`.
pub fn query(block: &[u8], path: &str) -> Option<(usize, u16)> {
    // A node at `at`: its length, value length, type, and where its value and children start.
    let node = |at: usize| -> Option<(usize, u16, u16, String, usize, usize)> {
        let len = usize::from(u16::from_le_bytes([*block.get(at)?, *block.get(at + 1)?]));
        let value_len = u16::from_le_bytes([*block.get(at + 2)?, *block.get(at + 3)?]);
        let kind = u16::from_le_bytes([*block.get(at + 4)?, *block.get(at + 5)?]);
        let mut k = at + 6;
        let mut key = Vec::new();
        loop {
            let u = u16::from_le_bytes([*block.get(k)?, *block.get(k + 1)?]);
            k += 2;
            if u == 0 {
                break;
            }
            key.push(u);
        }
        let value = (k - at).next_multiple_of(4) + at;
        let value_bytes = if kind == 1 { 2 * usize::from(value_len) } else { usize::from(value_len) };
        let children = (value + value_bytes - at).next_multiple_of(4) + at;
        (len >= 6 && at + len <= block.len()).then(|| (len, value_len, kind, String::from_utf16_lossy(&key), value, children))
    };
    let mut at = 0;
    for part in path.split('\\').filter(|p| !p.is_empty()) {
        let (len, _, _, _, _, mut child) = node(at)?;
        let end = at + len;
        let mut found = None;
        while child + 6 <= end {
            let (clen, _, _, key, _, _) = node(child)?;
            if key.eq_ignore_ascii_case(part) {
                found = Some(child);
                break;
            }
            child = (child + clen).next_multiple_of(4);
        }
        at = found?;
    }
    let (_, value_len, _, _, value, _) = node(at)?;
    Some((value, value_len))
}

/// `VS_FIXEDFILEINFO` for a version `a.b.c.d`, of a DLL (or an application).
pub fn fixed_info(version: [u16; 4], dll: bool) -> Vec<u8> {
    let [a, b, c, d] = version.map(u32::from);
    let mut f = Vec::with_capacity(52);
    for v in [0xfeef_04bd, 0x1_0000, a << 16 | b, c << 16 | d, a << 16 | b, c << 16 | d, 0x3f, 0, 0x4_0004, if dll { 2 } else { 1 }, 0, 0, 0] {
        f.extend(u32::to_le_bytes(v));
    }
    f
}

/// The version information atlas gives a DLL it makes: Windows 10's file version, its name, and US English.
pub fn system_version(file: &str) -> Vec<u8> {
    let version = [10, 0, 19041, 1];
    let text = format!("{}.{}.{}.{}", version[0], version[1], version[2], version[3]);
    let fixed = fixed_info(version, true);
    let translation = [0x0409u16, 0x04b0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    let internal = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    let strings = [("FileDescription", file), ("FileVersion", &text), ("InternalName", internal), ("OriginalFilename", file), ("ProductVersion", &text)];
    Node {
        key: "VS_VERSION_INFO",
        value: Some(Value::Binary(&fixed)),
        children: vec![
            Node {
                key: "StringFileInfo",
                value: None,
                children: vec![Node {
                    key: "040904B0",
                    value: None,
                    children: strings.iter().map(|(k, v)| Node { key: k, value: Some(Value::Text(v)), children: Vec::new() }).collect(),
                }],
            },
            Node { key: "VarFileInfo", value: None, children: vec![Node { key: "Translation", value: Some(Value::Binary(&translation)), children: Vec::new() }] },
        ],
    }
    .bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_built_read_back() {
        let mut r = BTreeMap::new();
        r.insert((16, 1, 0x409), b"version".to_vec());
        r.insert((6, 1, 0), b"strings".to_vec());
        r.insert((6, 1, 0x407), b"german".to_vec());
        r.insert((10, 7, 0x409), b"data".to_vec());
        let rva = 0x3000;
        let bytes = build(rva, &r);
        let mut tree = Tree::new(rva, |at, len| bytes.get((at - rva) as usize..(at - rva) as usize + len).map(<[u8]>::to_vec));
        let types: Vec<Id> = tree.types().unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(types, [Id::Num(6), Id::Num(10), Id::Num(16)]);
        let found = tree.find(&Id::Num(16), &Id::Num(1), None).unwrap();
        assert_eq!((found.size, found.lang), (7, 0x409));
        assert_eq!(&bytes[(found.data - rva) as usize..][..7], b"version");
        // Neutral first; then exactly a language asked for, and not one that isn't there.
        assert_eq!(tree.find(&Id::Num(6), &Id::Num(1), None).unwrap().lang, 0);
        assert_eq!(tree.find(&Id::Num(6), &Id::Num(1), Some(0x407)).unwrap().size, 6);
        assert_eq!(tree.find(&Id::Num(6), &Id::Num(1), Some(0x40c)), Err(ERROR_RESOURCE_LANG_NOT_FOUND));
        assert_eq!(tree.find(&Id::Num(6), &Id::Num(2), None), Err(ERROR_RESOURCE_NAME_NOT_FOUND));
        assert_eq!(tree.find(&Id::Num(3), &Id::Num(1), None), Err(ERROR_RESOURCE_TYPE_NOT_FOUND));
        assert_eq!(Id::named("#12"), Id::Num(12));
        assert_eq!(Id::named("hello"), Id::Name("HELLO".into()));
    }

    #[test]
    fn version_information_both_ways() {
        let block = system_version("KERNEL32.dll");
        let (at, len) = query(&block, "\\").unwrap();
        assert_eq!(len, 52);
        assert_eq!(u32_at(&block, at), 0xfeef_04bd);
        assert_eq!(u32_at(&block, at + 8), 10 << 16);
        let (at, len) = query(&block, "\\StringFileInfo\\040904b0\\FileVersion").unwrap();
        let units: Vec<u16> = block[at..at + 2 * usize::from(len)].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16_lossy(&units), "10.0.19041.1\0");
        let (at, len) = query(&block, "\\VarFileInfo\\Translation").unwrap();
        assert_eq!((len, u16_at(&block, at), u16_at(&block, at + 2)), (4, 0x409, 0x4b0));
        assert_eq!(query(&block, "\\StringFileInfo\\040904b0\\Nothing"), None);
    }

    #[test]
    fn string_tables() {
        let mut block = Vec::new();
        for i in 0..16 {
            let s: Vec<u16> = if i == 3 { "three".encode_utf16().collect() } else { Vec::new() };
            block.extend((s.len() as u16).to_le_bytes());
            block.extend(s.iter().flat_map(|u| u.to_le_bytes()));
        }
        assert_eq!(String::from_utf16_lossy(&table_string(&block, 3).unwrap().1), "three");
        assert!(table_string(&block, 4).unwrap().1.is_empty());
    }
}
