//! COM objects atlas implements, and the first of them: the shell link (`CLSID_ShellLink`), a shortcut, with
//! `IShellLinkW`, `IShellLinkA` and `IPersistFile`.
//!
//! **Objects.** An object is one block in the program's heap (from ntdll's task allocator): a pointer for each of its
//! interfaces, each to that interface's vtable after them, whose entries are the addresses of ole32's host exports.
//! Calling a method traps into Rust as any export does, `this` its first argument. What the object holds is the
//! process's (`ProcState::com`), by the block's address; an interface pointer is found in it by where it falls.
//! `QueryInterface`, `AddRef` and `Release` are the same exports in every vtable; the last `Release` frees the block.
//!
//! **Shell links** keep a target, its arguments, working folder, description, icon, show command and hot key, and
//! `IPersistFile` saves and loads them as `.lnk` files in the Shell Link Binary File Format ([MS-SHLLINK]): the header,
//! the target as `LinkInfo`'s local path (in ANSI and in UTF-16), and the strings in UTF-16. A link Windows made, with
//! the target only as an item ID list, loads with its strings but no target, as the shell namespace's IDs for files
//! aren't read yet.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::process::Task;

const S_OK: u64 = 0;
const S_FALSE: u64 = 1;
const E_NOINTERFACE: u64 = 0x8000_4002;
const E_POINTER: u64 = 0x8000_4003;
const E_FAIL: u64 = 0x8000_4005;
const E_INVALIDARG: u64 = 0x8007_0057;

/// A GUID as it's laid out in memory.
pub type Guid = [u8; 16];

pub const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    let a = a.to_le_bytes();
    let (b, c) = (b.to_le_bytes(), c.to_le_bytes());
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]
}

/// The COM GUIDs Windows numbers from {xxxxxxxx-0000-0000-C000-000000000046}.
const fn ole(a: u32) -> Guid {
    guid(a, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46])
}

pub const CLSID_SHELL_LINK: Guid = ole(0x0002_1401);
const IID_IUNKNOWN: Guid = ole(0);
const IID_ISHELLLINKW: Guid = ole(0x0002_14f9);
const IID_ISHELLLINKA: Guid = ole(0x0002_14ee);
const IID_IPERSIST: Guid = ole(0x0000_010c);
const IID_IPERSISTFILE: Guid = ole(0x0000_010b);

/// An object of atlas's.
#[derive(Debug, Clone)]
pub enum Object {
    ShellLink(ShellLink),
    /// DirectWrite's (`os/dwrite.rs`): one interface each, the ones it derives from answered by the same pointer.
    Dwrite(super::dwrite::Dw),
}

impl Object {
    /// How many interface pointers its block starts with.
    fn interfaces(&self) -> u64 {
        match self {
            Object::ShellLink(_) => LINK_INTERFACES.len() as u64,
            Object::Dwrite(_) => 1,
        }
    }
}

/// What's kept of an object: its references and what it is.
#[derive(Debug, Clone)]
pub struct Com {
    pub refs: u32,
    pub object: Object,
}

/// A shortcut, as `IShellLink` sets and gets it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellLink {
    pub target: String,
    pub arguments: String,
    pub directory: String,
    pub description: String,
    pub relative: String,
    pub icon: String,
    pub icon_index: i32,
    pub show: u32,
    pub hotkey: u16,
    /// The file it was loaded from or saved to (`IPersistFile::GetCurFile`).
    pub file: Option<String>,
    pub dirty: bool,
}

// ---- .lnk files ----

const HAS_LINK_TARGET_ID_LIST: u32 = 0x1;
const HAS_LINK_INFO: u32 = 0x2;
const HAS_NAME: u32 = 0x4;
const HAS_RELATIVE_PATH: u32 = 0x8;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const HAS_ICON_LOCATION: u32 = 0x40;
const IS_UNICODE: u32 = 0x80;

/// A shell link as a `.lnk` file: the header (with the target's attributes and size, if it's known), `LinkInfo` with
/// the target as a local path, and the strings it has.
pub fn to_lnk(link: &ShellLink, target_attributes: u32, target_size: u32) -> Vec<u8> {
    let strings = [(HAS_NAME, &link.description), (HAS_RELATIVE_PATH, &link.relative), (HAS_WORKING_DIR, &link.directory), (HAS_ARGUMENTS, &link.arguments), (HAS_ICON_LOCATION, &link.icon)];
    let mut flags = IS_UNICODE;
    if !link.target.is_empty() {
        flags |= HAS_LINK_INFO;
    }
    for (flag, s) in strings {
        if !s.is_empty() {
            flags |= flag;
        }
    }
    let mut b = Vec::new();
    b.extend(0x4cu32.to_le_bytes());
    b.extend(CLSID_SHELL_LINK);
    b.extend(flags.to_le_bytes());
    b.extend(target_attributes.to_le_bytes());
    b.extend([0u8; 24]); // creation, access and write times
    b.extend(target_size.to_le_bytes());
    b.extend(link.icon_index.to_le_bytes());
    b.extend(link.show.max(1).to_le_bytes());
    b.extend(link.hotkey.to_le_bytes());
    b.extend([0u8; 10]);
    if !link.target.is_empty() {
        // LinkInfo, with the Unicode offsets (a 0x24-byte header): VolumeID (a fixed disk, no label), then the local
        // base path in ANSI, an empty common path suffix, and both again in UTF-16.
        let volume: Vec<u8> = [17u32, 3, 0, 0x10].iter().flat_map(|v| v.to_le_bytes()).chain([0]).collect();
        let ansi: Vec<u8> = crate::ansi::bytes(&link.target).into_iter().chain([0]).collect();
        let wide: Vec<u8> = link.target.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        let volume_at = 0x24u32;
        let base_at = volume_at + volume.len() as u32;
        let suffix_at = base_at + ansi.len() as u32;
        let base_wide_at = suffix_at + 1;
        let suffix_wide_at = base_wide_at + wide.len() as u32;
        let size = suffix_wide_at + 2;
        for v in [size, 0x24, 1, volume_at, base_at, 0, suffix_at, base_wide_at, suffix_wide_at] {
            b.extend(v.to_le_bytes());
        }
        b.extend(volume);
        b.extend(ansi);
        b.push(0);
        b.extend(wide);
        b.extend([0, 0]);
    }
    for (_, s) in strings {
        if !s.is_empty() {
            let units: Vec<u16> = s.encode_utf16().collect();
            b.extend((units.len() as u16).to_le_bytes());
            b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        }
    }
    b.extend(0u32.to_le_bytes());
    b
}

/// A `.lnk` file's shell link, if it's one.
pub fn from_lnk(b: &[u8]) -> Option<ShellLink> {
    let u16_at = |o: usize| b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    if u32_at(0)? != 0x4c || b.get(4..20)? != CLSID_SHELL_LINK {
        return None;
    }
    let flags = u32_at(20)?;
    let mut link = ShellLink {
        icon_index: u32_at(56)? as i32,
        show: u32_at(60)?,
        hotkey: u16_at(64)?,
        ..ShellLink::default()
    };
    let mut at = 0x4c;
    if flags & HAS_LINK_TARGET_ID_LIST != 0 {
        at += 2 + usize::from(u16_at(at)?);
    }
    if flags & HAS_LINK_INFO != 0 {
        let info = b.get(at..at + u32_at(at)? as usize)?;
        let i32_at = |o: usize| info.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize);
        let (header, info_flags) = (i32_at(4)?, i32_at(8)?);
        let cstr = |o: usize| info.get(o..).map(|s| s.split(|&c| c == 0).next().unwrap_or(&[]).to_vec()).unwrap_or_default();
        let wstr = |o: usize| -> String {
            let units: Vec<u16> = info.get(o..).unwrap_or(&[]).as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).collect();
            String::from_utf16_lossy(&units)
        };
        if info_flags & 1 != 0 {
            link.target = if header >= 0x24 && i32_at(28)? != 0 {
                wstr(i32_at(28)?) + &wstr(i32_at(32)?)
            } else {
                crate::ansi::text(&cstr(i32_at(16)?)) + &crate::ansi::text(&cstr(i32_at(24)?))
            };
        }
        at += info.len();
    }
    let unicode = flags & IS_UNICODE != 0;
    for (flag, slot) in [(HAS_NAME, 0), (HAS_RELATIVE_PATH, 1), (HAS_WORKING_DIR, 2), (HAS_ARGUMENTS, 3), (HAS_ICON_LOCATION, 4)] {
        if flags & flag == 0 {
            continue;
        }
        let n = usize::from(u16_at(at)?);
        at += 2;
        let s = if unicode {
            let units: Vec<u16> = b.get(at..at + 2 * n)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            at += 2 * n;
            String::from_utf16_lossy(&units)
        } else {
            let s = crate::ansi::text(b.get(at..at + n)?);
            at += n;
            s
        };
        *[&mut link.description, &mut link.relative, &mut link.directory, &mut link.arguments, &mut link.icon][slot] = s;
    }
    Some(link)
}

// ---- Vtables ----

/// IShellLinkW's methods, in order, and IShellLinkA's: the exports that are them.
const SHELL_LINK_W: [&str; 21] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease",
    "AtlasLinkGetPathW", "AtlasLinkGetIDList", "AtlasLinkSetIDList", "AtlasLinkGetDescriptionW", "AtlasLinkSetDescriptionW",
    "AtlasLinkGetWorkingDirectoryW", "AtlasLinkSetWorkingDirectoryW", "AtlasLinkGetArgumentsW", "AtlasLinkSetArgumentsW",
    "AtlasLinkGetHotkey", "AtlasLinkSetHotkey", "AtlasLinkGetShowCmd", "AtlasLinkSetShowCmd",
    "AtlasLinkGetIconLocationW", "AtlasLinkSetIconLocationW", "AtlasLinkSetRelativePathW", "AtlasLinkResolve", "AtlasLinkSetPathW",
];
const SHELL_LINK_A: [&str; 21] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease",
    "AtlasLinkGetPathA", "AtlasLinkGetIDList", "AtlasLinkSetIDList", "AtlasLinkGetDescriptionA", "AtlasLinkSetDescriptionA",
    "AtlasLinkGetWorkingDirectoryA", "AtlasLinkSetWorkingDirectoryA", "AtlasLinkGetArgumentsA", "AtlasLinkSetArgumentsA",
    "AtlasLinkGetHotkey", "AtlasLinkSetHotkey", "AtlasLinkGetShowCmd", "AtlasLinkSetShowCmd",
    "AtlasLinkGetIconLocationA", "AtlasLinkSetIconLocationA", "AtlasLinkSetRelativePathA", "AtlasLinkResolve", "AtlasLinkSetPathA",
];
const PERSIST_FILE: [&str; 9] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease",
    "AtlasLinkGetClassID", "AtlasLinkIsDirty", "AtlasLinkLoad", "AtlasLinkSave", "AtlasLinkSaveCompleted", "AtlasLinkGetCurFile",
];

/// A shell link's interfaces, by their pointers' order in it: IShellLinkW, IPersistFile, IShellLinkA.
const LINK_INTERFACES: [&[&str]; 3] = [&SHELL_LINK_W, &PERSIST_FILE, &SHELL_LINK_A];

/// The strings a shell link has, by which method names them.
#[derive(Clone, Copy)]
enum Field {
    Description,
    Directory,
    Arguments,
}

impl Task {
    pub(crate) fn read_guid(&mut self, cx: &mut Cx<'_>, at: u64) -> Option<Guid> {
        let b = self.mem(cx, false, |t| user::read_vec(&t.space(), at, 16)).ok()?;
        b.try_into().ok()
    }

    /// Makes an object with these interfaces: its block, with their vtables of `dll`'s exports. Its address.
    pub(crate) fn make_object(&mut self, cx: &mut Cx<'_>, dll: &str, interfaces: &[&[&str]], object: Object) -> Result<u64, Reply> {
        let w = self.abi().width;
        let mut addresses = Vec::new();
        for names in interfaces {
            let mut table = Vec::new();
            for name in names.iter() {
                let Some(at) = self.system_export(dll, name) else { return Ok(0) };
                table.push(at);
            }
            addresses.push(table);
        }
        // The interface pointers, then the vtables, in a block allocated first so they can point into it.
        let count = interfaces.len() + addresses.iter().map(Vec::len).sum::<usize>();
        let base = self.task_alloc(cx, &vec![0u8; count * w as usize])?;
        if base == 0 {
            return Ok(0);
        }
        let abi = self.abi();
        let mut bytes = Vec::new();
        let mut table_at = base + interfaces.len() as u64 * w;
        for table in &addresses {
            bytes.extend(abi.ptr(table_at));
            table_at += table.len() as u64 * w;
        }
        for table in &addresses {
            for &a in table {
                bytes.extend(abi.ptr(a));
            }
        }
        let _ = self.mem(cx, true, |t| user::write(&mut t.space(), base, &bytes));
        kernels_core::lock(&self.process.state).com.insert(base, Com { refs: 1, object });
        Ok(base)
    }

    /// `CoCreateInstance` for a class of atlas's: the object asked for through `iid`, written to `out`. None for a
    /// class it doesn't have.
    pub fn create_com_object(&mut self, cx: &mut Cx<'_>, clsid: u64, iid: u64, out: u64) -> Result<Option<u64>, Reply> {
        let (Some(class), Some(iid)) = (self.read_guid(cx, clsid), self.read_guid(cx, iid)) else { return Ok(Some(E_POINTER)) };
        if class != CLSID_SHELL_LINK {
            return Ok(None);
        }
        let base = self.make_object(cx, "ole32.dll", &LINK_INTERFACES, Object::ShellLink(ShellLink { show: 1, ..ShellLink::default() }))?;
        if base == 0 {
            return Ok(Some(E_FAIL));
        }
        let hr = self.query_interface(cx, base, &iid, out)?;
        self.release(cx, base)?;
        Ok(Some(hr))
    }

    /// The object an interface pointer is part of: its block's address and the interface's place in it.
    fn object_of(&self, this: u64) -> Option<(u64, usize)> {
        let w = self.abi().width;
        let state = kernels_core::lock(&self.process.state);
        let (&base, com) = state.com.range(..=this).next_back()?;
        let count = com.object.interfaces();
        let at = this - base;
        (at < count * w && at.is_multiple_of(w)).then_some((base, (at / w) as usize))
    }

    pub(crate) fn query_interface(&mut self, cx: &mut Cx<'_>, this: u64, iid: &Guid, out: u64) -> Result<u64, Reply> {
        if out == 0 {
            return Ok(E_POINTER);
        }
        let Some((base, _)) = self.object_of(this) else { return Ok(E_INVALIDARG) };
        let w = self.abi().width;
        let dw = match &kernels_core::lock(&self.process.state).com.get(&base).map(|c| &c.object) {
            Some(Object::Dwrite(dw)) => Some(super::dwrite::answers(dw, iid)),
            _ => None,
        };
        let found = match (dw, *iid) {
            (Some(yes), _) => (yes || *iid == IID_IUNKNOWN).then_some(base),
            (None, IID_IUNKNOWN | IID_ISHELLLINKW) => Some(base),
            (None, IID_IPERSIST | IID_IPERSISTFILE) => Some(base + w),
            (None, IID_ISHELLLINKA) => Some(base + 2 * w),
            _ => None,
        };
        let _ = self.write_ptr(cx, out, found.unwrap_or(0));
        match found {
            Some(_) => {
                self.add_ref(base);
                Ok(S_OK)
            }
            None => Ok(E_NOINTERFACE),
        }
    }

    pub(crate) fn add_ref(&mut self, this: u64) -> u64 {
        let Some((base, _)) = self.object_of(this) else { return 0 };
        let mut state = kernels_core::lock(&self.process.state);
        state.com.get_mut(&base).map_or(0, |c| {
            c.refs += 1;
            u64::from(c.refs)
        })
    }

    pub(crate) fn release(&mut self, cx: &mut Cx<'_>, this: u64) -> Result<u64, Reply> {
        let Some((base, _)) = self.object_of(this) else { return Ok(0) };
        let left = {
            let mut state = kernels_core::lock(&self.process.state);
            let Some(c) = state.com.get_mut(&base) else { return Ok(0) };
            c.refs -= 1;
            let left = c.refs;
            if left == 0 {
                state.com.remove(&base);
            }
            left
        };
        if left == 0 {
            self.task_free(cx, base)?;
        }
        Ok(u64::from(left))
    }

    /// Works on the shell link an interface pointer is part of.
    fn link<R>(&self, this: u64, f: impl FnOnce(&mut ShellLink) -> R) -> Option<R> {
        let (base, _) = self.object_of(this)?;
        let mut state = kernels_core::lock(&self.process.state);
        match &mut state.com.get_mut(&base)?.object {
            Object::ShellLink(link) => Some(f(link)),
            Object::Dwrite(_) => None,
        }
    }

    /// Works on the DirectWrite object an interface pointer is.
    pub(crate) fn dw<R>(&self, this: u64, f: impl FnOnce(&mut super::dwrite::Dw) -> R) -> Option<R> {
        let (base, _) = self.object_of(this)?;
        let mut state = kernels_core::lock(&self.process.state);
        match &mut state.com.get_mut(&base)?.object {
            Object::Dwrite(dw) => Some(f(dw)),
            Object::ShellLink(_) => None,
        }
    }

    /// Writes a string into a buffer of `room` characters, UTF-16 or ANSI, cut and terminated.
    fn put_string(&mut self, cx: &mut Cx<'_>, at: u64, room: u64, s: &str, wide: bool) {
        let room = (room as u32 as usize).min(1 << 16);
        if at == 0 || room == 0 {
            return;
        }
        let bytes: Vec<u8> = if wide {
            s.encode_utf16().take(room - 1).chain([0]).flat_map(u16::to_le_bytes).collect()
        } else {
            crate::ansi::bytes(s).into_iter().take(room - 1).chain([0]).collect()
        };
        let _ = self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes));
    }
}

fn value(result: Result<u64, Reply>) -> Reply {
    match result {
        Ok(v) => Reply::Value(v),
        Err(reply) => reply,
    }
}

// ---- IUnknown ----

pub fn com_query_interface(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(iid) = task.read_guid(cx, a[1]) else { return Reply::Value(E_POINTER) };
    value(task.query_interface(cx, a[0], &iid, a[2]))
}

pub fn com_add_ref(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.add_ref(a[0]))
}

pub fn com_release(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.release(cx, a[0]))
}

// ---- IShellLinkW and IShellLinkA ----

fn get_field(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], field: Field, wide: bool) -> Reply {
    let Some(s) = task.link(a[0], |l| match field {
        Field::Description => l.description.clone(),
        Field::Directory => l.directory.clone(),
        Field::Arguments => l.arguments.clone(),
    }) else {
        return Reply::Value(E_INVALIDARG);
    };
    task.put_string(cx, a[1], a[2], &s, wide);
    Reply::Value(S_OK)
}

fn set_field(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], field: Field, wide: bool) -> Reply {
    let s = task.text_arg(cx, a[1], wide);
    let done = task.link(a[0], |l| {
        *match field {
            Field::Description => &mut l.description,
            Field::Directory => &mut l.directory,
            Field::Arguments => &mut l.arguments,
        } = s;
        l.dirty = true;
    });
    Reply::Value(if done.is_some() { S_OK } else { E_INVALIDARG })
}

/// `GetPath(file, room, &find_data, flags)`: the target, and WIN32_FIND_DATA for it; S_FALSE for none.
fn get_path(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(target) = task.link(a[0], |l| l.target.clone()) else { return Reply::Value(E_INVALIDARG) };
    task.put_string(cx, a[1], a[2], &target, wide);
    if a[3] != 0 {
        // WIN32_FIND_DATA: attributes, three times, the size's halves, two reserved, then the name (from 44).
        let mut b = vec![0u8; if wide { 592 } else { 320 }];
        if let Ok(info) = task.path_info(&target) {
            b[0..4].copy_from_slice(&info.attributes.to_le_bytes());
            b[28..32].copy_from_slice(&((info.size >> 32) as u32).to_le_bytes());
            b[32..36].copy_from_slice(&(info.size as u32).to_le_bytes());
        }
        let name = target.rsplit('\\').next().unwrap_or("");
        let text: Vec<u8> = if wide { name.encode_utf16().take(259).flat_map(u16::to_le_bytes).collect() } else { crate::ansi::bytes(name).into_iter().take(259).collect() };
        b[44..44 + text.len()].copy_from_slice(&text);
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &b));
    }
    Reply::Value(if target.is_empty() { S_FALSE } else { S_OK })
}

fn set_path(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let path = task.text_arg(cx, a[1], wide);
    let done = task.link(a[0], |l| {
        l.target = path;
        l.dirty = true;
    });
    Reply::Value(if done.is_some() { S_OK } else { E_INVALIDARG })
}

/// `GetIconLocation(path, room, &index)`.
fn get_icon_location(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some((icon, index)) = task.link(a[0], |l| (l.icon.clone(), l.icon_index)) else { return Reply::Value(E_INVALIDARG) };
    task.put_string(cx, a[1], a[2], &icon, wide);
    if a[3] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &index.to_le_bytes()));
    }
    Reply::Value(S_OK)
}

fn set_icon_location(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let icon = task.text_arg(cx, a[1], wide);
    let done = task.link(a[0], |l| {
        l.icon = icon;
        l.icon_index = a[2] as i32;
        l.dirty = true;
    });
    Reply::Value(if done.is_some() { S_OK } else { E_INVALIDARG })
}

fn set_relative_path(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let path = task.text_arg(cx, a[1], wide);
    let done = task.link(a[0], |l| {
        l.relative = path;
        l.dirty = true;
    });
    Reply::Value(if done.is_some() { S_OK } else { E_INVALIDARG })
}

macro_rules! both {
    ($($w:ident $a:ident => $f:ident $(($field:expr))?;)*) => {
        $(
            pub fn $w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
                $f(task, cx, a, $($field,)? true)
            }
            pub fn $a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
                $f(task, cx, a, $($field,)? false)
            }
        )*
    };
}

both! {
    link_get_path_w link_get_path_a => get_path;
    link_set_path_w link_set_path_a => set_path;
    link_get_description_w link_get_description_a => get_field(Field::Description);
    link_set_description_w link_set_description_a => set_field(Field::Description);
    link_get_working_directory_w link_get_working_directory_a => get_field(Field::Directory);
    link_set_working_directory_w link_set_working_directory_a => set_field(Field::Directory);
    link_get_arguments_w link_get_arguments_a => get_field(Field::Arguments);
    link_set_arguments_w link_set_arguments_a => set_field(Field::Arguments);
    link_get_icon_location_w link_get_icon_location_a => get_icon_location;
    link_set_icon_location_w link_set_icon_location_a => set_icon_location;
    link_set_relative_path_w link_set_relative_path_a => set_relative_path;
}

/// `GetIDList(&pidl)`: the target's item ID list; S_FALSE and NULL for none.
pub fn link_get_id_list(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(target) = task.link(a[0], |l| l.target.clone()) else { return Reply::Value(E_INVALIDARG) };
    if target.is_empty() {
        let _ = task.write_ptr(cx, a[1], 0);
        return Reply::Value(S_FALSE);
    }
    match task.make_pidl(cx, Some(&target), 0) {
        Ok(pidl) => {
            let _ = task.write_ptr(cx, a[1], pidl);
            Reply::Value(S_OK)
        }
        Err(reply) => reply,
    }
}

/// `SetIDList(pidl)`: the target is what the list is.
pub fn link_set_id_list(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(path) = task.pidl_path(cx, a[1]) else { return Reply::Value(E_INVALIDARG) };
    let done = task.link(a[0], |l| {
        l.target = path;
        l.dirty = true;
    });
    Reply::Value(if done.is_some() { S_OK } else { E_INVALIDARG })
}

pub fn link_get_hotkey(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(key) = task.link(a[0], |l| l.hotkey) else { return Reply::Value(E_INVALIDARG) };
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &key.to_le_bytes()));
    Reply::Value(S_OK)
}

pub fn link_set_hotkey(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let done = task.link(a[0], |l| {
        l.hotkey = a[1] as u16;
        l.dirty = true;
    });
    Reply::Value(if done.is_some() { S_OK } else { E_INVALIDARG })
}

pub fn link_get_show_cmd(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(show) = task.link(a[0], |l| l.show) else { return Reply::Value(E_INVALIDARG) };
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &show.to_le_bytes()));
    Reply::Value(S_OK)
}

pub fn link_set_show_cmd(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let done = task.link(a[0], |l| {
        l.show = a[1] as u32;
        l.dirty = true;
    });
    Reply::Value(if done.is_some() { S_OK } else { E_INVALIDARG })
}

/// `Resolve(window, flags)`: the target as it is; nothing's looked for if it moved.
pub fn link_resolve(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if task.link(a[0], |_| ()).is_some() { S_OK } else { E_INVALIDARG })
}

// ---- IPersistFile ----

pub fn link_get_class_id(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] == 0 {
        return Reply::Value(E_POINTER);
    }
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &CLSID_SHELL_LINK));
    Reply::Value(S_OK)
}

pub fn link_is_dirty(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(match task.link(a[0], |l| l.dirty) {
        Some(true) => S_OK,
        Some(false) => S_FALSE,
        None => E_INVALIDARG,
    })
}

/// HRESULT_FROM_WIN32.
fn from_win32(e: u32) -> u64 {
    0x8007_0000 | u64::from(e & 0xffff)
}

/// `Load(file, mode)`: the link a `.lnk` file is.
pub fn link_load(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.wstr(cx, a[1]);
    let bytes = match task.read_whole(&name) {
        Ok(b) => b,
        Err(e) => return Reply::Value(from_win32(e)),
    };
    let Some(mut loaded) = from_lnk(&bytes) else { return Reply::Value(E_FAIL) };
    loaded.file = Some(name);
    let done = task.link(a[0], |l| *l = loaded);
    Reply::Value(if done.is_some() { S_OK } else { E_INVALIDARG })
}

/// `Save(file, remember)`: the link as a `.lnk` file there (NULL: the file it came from).
pub fn link_save(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(link) = task.link(a[0], |l| l.clone()) else { return Reply::Value(E_INVALIDARG) };
    let name = if a[1] != 0 { task.wstr(cx, a[1]) } else { match link.file.clone() { Some(f) => f, None => return Reply::Value(E_INVALIDARG) } };
    let (attributes, size) = task.path_info(&link.target).map_or((0, 0), |i| (i.attributes, i.size as u32));
    if let Err(e) = task.write_whole(&name, &to_lnk(&link, attributes, size), false) {
        return Reply::Value(from_win32(e));
    }
    task.link(a[0], |l| {
        l.dirty = false;
        if a[2] != 0 || a[1] == 0 {
            l.file = Some(name);
        }
    });
    Reply::Value(S_OK)
}

pub fn link_save_completed(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(S_OK)
}

/// `GetCurFile(&name)`: its file's name, in a block the program frees; S_FALSE if it has none.
pub fn link_get_cur_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.link(a[0], |l| l.file.clone()) else { return Reply::Value(E_INVALIDARG) };
    let Some(file) = file else {
        let _ = task.write_ptr(cx, a[1], 0);
        return Reply::Value(S_FALSE);
    };
    let text: Vec<u8> = file.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    match task.task_alloc(cx, &text) {
        Ok(at) => {
            let _ = task.write_ptr(cx, a[1], at);
            Reply::Value(S_OK)
        }
        Err(reply) => reply,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_as_laid_out() {
        assert_eq!(CLSID_SHELL_LINK, [0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
    }

    #[test]
    fn a_link_round_trips() {
        let link = ShellLink {
            target: "C:\\Program Files (x86)\\App\\app.exe".into(),
            arguments: "-silent".into(),
            directory: "C:\\Program Files (x86)\\App".into(),
            description: "Runs the app".into(),
            icon: "C:\\Program Files (x86)\\App\\app.exe".into(),
            icon_index: 2,
            show: 3,
            hotkey: 0x0641,
            ..ShellLink::default()
        };
        let bytes = to_lnk(&link, 0x20, 1234);
        assert_eq!(&bytes[0..4], &[0x4c, 0, 0, 0]);
        assert_eq!(u32::from_le_bytes(bytes[52..56].try_into().unwrap()), 1234);
        assert_eq!(from_lnk(&bytes), Some(link));
    }

    #[test]
    fn an_empty_link_and_a_bad_one() {
        let link = ShellLink { show: 1, ..ShellLink::default() };
        assert_eq!(from_lnk(&to_lnk(&link, 0, 0)), Some(link));
        assert_eq!(from_lnk(b"not a link at all, but long enough to have a header's room for one, nearly"), None);
    }
}
