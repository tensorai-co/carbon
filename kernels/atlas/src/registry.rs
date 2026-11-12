//! The registry: a tree of keys, each with values, in memory, made with Windows' defaults on a fresh machine and saved
//! in the machine's own layer, where Windows keeps its hives (`C:\Windows\System32\config`), in atlas's own format.
//!
//! Names fold case and keep it, as Windows' do. `HKEY_CURRENT_USER` is a key under `HKEY_USERS`, and
//! `HKEY_CLASSES_ROOT` is `HKEY_LOCAL_MACHINE\SOFTWARE\Classes`, as Windows maps them.

use std::collections::{BTreeMap, HashMap};

pub const REG_NONE: u32 = 0;
pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;
pub const REG_BINARY: u32 = 3;
pub const REG_DWORD: u32 = 4;
pub const REG_MULTI_SZ: u32 = 7;
pub const REG_QWORD: u32 = 11;

pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_KEY_DELETED: u32 = 1018;

/// Where the registry is saved on the machine's disk.
pub const SAVED: &str = "/Windows/System32/config/REGISTRY";
const MAGIC: &[u8] = b"atlas registry 1\n";
/// The user's SID, under `HKEY_USERS`.
pub const USER: &str = "S-1-5-21-0-0-0-1000";

#[derive(Debug, Clone)]
pub struct Value {
    pub name: String,
    pub kind: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Key {
    pub name: String,
    pub parent: u64,
    /// By name, folded: key IDs.
    pub subkeys: BTreeMap<String, u64>,
    /// By name, folded. The default value's name is "".
    pub values: BTreeMap<String, Value>,
    /// Not saved: `REG_OPTION_VOLATILE`.
    pub volatile: bool,
    /// Nanoseconds since 1970.
    pub modified: u64,
    pub deleted: bool,
}

/// The predefined keys a program reaches the rest by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum Root {
    ClassesRoot,
    CurrentUser,
    LocalMachine,
    Users,
    CurrentConfig,
}

impl Root {
    /// From a predefined handle: `HKEY_CLASSES_ROOT` (0x80000000) and on, sign-extended or not.
    pub fn from_handle(handle: u64) -> Option<Root> {
        if handle >> 32 != 0 && handle >> 32 != 0xffff_ffff {
            return None;
        }
        Some(match handle as u32 {
            0x8000_0000 => Root::ClassesRoot,
            0x8000_0001 => Root::CurrentUser,
            0x8000_0002 => Root::LocalMachine,
            0x8000_0003 => Root::Users,
            0x8000_0005 => Root::CurrentConfig,
            _ => return None,
        })
    }
}

pub struct Registry {
    keys: HashMap<u64, Key>,
    next: u64,
    machine: u64,
    users: u64,
}

fn fold(name: &str) -> String {
    name.to_lowercase()
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// The text of a string value: up to its first NUL.
pub fn text(data: &[u8]) -> String {
    let units: Vec<u16> = data.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

/// What `%NAME%` stands for in a `REG_EXPAND_SZ`: the machine's own folders and names.
pub fn variable(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "systemroot" | "windir" => "C:\\Windows",
        "systemdrive" | "homedrive" => "C:",
        "programfiles" | "programw6432" => "C:\\Program Files",
        "programfiles(x86)" => "C:\\Program Files (x86)",
        "programdata" | "allusersprofile" => "C:\\ProgramData",
        "commonprogramfiles" => "C:\\Program Files\\Common Files",
        "userprofile" => "C:\\Users\\carbon",
        "homepath" => "\\Users\\carbon",
        "appdata" => "C:\\Users\\carbon\\AppData\\Roaming",
        "localappdata" => "C:\\Users\\carbon\\AppData\\Local",
        "temp" | "tmp" => "C:\\Windows\\Temp",
        "username" => "carbon",
        "computername" | "userdomain" => "CARBON",
        "os" => "Windows_NT",
        _ => return None,
    })
}

/// Expands `%NAME%`s, leaving any it doesn't know as they are.
pub fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => match variable(&after[..end]) {
                Some(value) => {
                    out.push_str(value);
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

impl Registry {
    /// A fresh machine's registry.
    pub fn new() -> Registry {
        let mut r = Registry { keys: HashMap::new(), next: 1, machine: 0, users: 0 };
        r.machine = r.add(0, "HKEY_LOCAL_MACHINE", false, 0);
        r.users = r.add(0, "HKEY_USERS", false, 0);
        r.defaults();
        r
    }

    fn add(&mut self, parent: u64, name: &str, volatile: bool, now: u64) -> u64 {
        let id = self.next;
        self.next += 1;
        self.keys.insert(id, Key { name: name.to_string(), parent, subkeys: BTreeMap::new(), values: BTreeMap::new(), volatile, modified: now, deleted: false });
        if let Some(p) = self.keys.get_mut(&parent) {
            p.subkeys.insert(fold(name), id);
        }
        id
    }

    pub fn key(&self, id: u64) -> Result<&Key, u32> {
        match self.keys.get(&id) {
            Some(k) if !k.deleted => Ok(k),
            Some(_) => Err(ERROR_KEY_DELETED),
            None => Err(ERROR_KEY_DELETED),
        }
    }

    fn key_mut(&mut self, id: u64) -> Result<&mut Key, u32> {
        match self.keys.get_mut(&id) {
            Some(k) if !k.deleted => Ok(k),
            _ => Err(ERROR_KEY_DELETED),
        }
    }

    /// The key a predefined handle stands for, made if it isn't there.
    pub fn root(&mut self, root: Root) -> u64 {
        let (base, path) = match root {
            Root::LocalMachine => return self.machine,
            Root::Users => return self.users,
            Root::CurrentUser => (self.users, USER.to_string()),
            Root::ClassesRoot => (self.machine, "SOFTWARE\\Classes".to_string()),
            Root::CurrentConfig => (self.machine, "SYSTEM\\CurrentControlSet\\Hardware Profiles\\Current".to_string()),
        };
        self.create(base, &path, false, 0).map(|(id, _)| id).unwrap_or(self.machine)
    }

    fn parts(path: &str) -> impl Iterator<Item = &str> {
        path.split('\\').filter(|p| !p.is_empty())
    }

    /// The key at `path` under `base`.
    pub fn open(&self, base: u64, path: &str) -> Result<u64, u32> {
        let mut at = base;
        self.key(at)?;
        for part in Registry::parts(path) {
            at = *self.key(at)?.subkeys.get(&fold(part)).ok_or(ERROR_FILE_NOT_FOUND)?;
        }
        Ok(at)
    }

    /// The key at `path` under `base`, made (with any keys on the way) if it isn't there. True if it was made.
    pub fn create(&mut self, base: u64, path: &str, volatile: bool, now: u64) -> Result<(u64, bool), u32> {
        let mut at = base;
        self.key(at)?;
        let mut made = false;
        for part in Registry::parts(path) {
            at = match self.key(at)?.subkeys.get(&fold(part)) {
                Some(&id) => id,
                None => {
                    // A volatile key can't have lasting keys under it.
                    let parent_volatile = self.key(at)?.volatile;
                    if parent_volatile && !volatile {
                        return Err(1021); // ERROR_CHILD_MUST_BE_VOLATILE
                    }
                    made = true;
                    self.add(at, part, volatile, now)
                }
            };
        }
        Ok((at, made))
    }

    pub fn value(&self, key: u64, name: &str) -> Result<&Value, u32> {
        self.key(key)?.values.get(&fold(name)).ok_or(ERROR_FILE_NOT_FOUND)
    }

    pub fn set_value(&mut self, key: u64, name: &str, kind: u32, data: Vec<u8>, now: u64) -> Result<(), u32> {
        let k = self.key_mut(key)?;
        k.values.insert(fold(name), Value { name: name.to_string(), kind, data });
        k.modified = now;
        Ok(())
    }

    pub fn delete_value(&mut self, key: u64, name: &str, now: u64) -> Result<(), u32> {
        let k = self.key_mut(key)?;
        k.values.remove(&fold(name)).ok_or(ERROR_FILE_NOT_FOUND)?;
        k.modified = now;
        Ok(())
    }

    /// Deletes the key at `path` under `base`. With `tree`, its subkeys go too; without, it mustn't have any.
    pub fn delete_key(&mut self, base: u64, path: &str, tree: bool) -> Result<(), u32> {
        let id = self.open(base, path)?;
        let key = self.key(id)?;
        if id == self.machine || id == self.users || key.parent == 0 {
            return Err(ERROR_ACCESS_DENIED);
        }
        if !tree && !key.subkeys.is_empty() {
            return Err(ERROR_ACCESS_DENIED);
        }
        self.remove(id);
        Ok(())
    }

    /// `RegDeleteTree` with no subkey: everything in the key, but not the key.
    pub fn clear(&mut self, id: u64) -> Result<(), u32> {
        let subkeys: Vec<u64> = self.key(id)?.subkeys.values().copied().collect();
        for sub in subkeys {
            self.remove(sub);
        }
        self.key_mut(id)?.values.clear();
        Ok(())
    }

    fn remove(&mut self, id: u64) {
        let Some(key) = self.keys.get_mut(&id) else { return };
        key.deleted = true;
        let (parent, name, subkeys) = (key.parent, fold(&key.name), std::mem::take(&mut key.subkeys));
        for sub in subkeys.into_values() {
            self.remove(sub);
        }
        if let Some(p) = self.keys.get_mut(&parent) {
            p.subkeys.remove(&name);
        }
    }

    /// Lets go of a deleted key once nothing has it open.
    pub fn forget(&mut self, id: u64) {
        if self.keys.get(&id).is_some_and(|k| k.deleted) {
            self.keys.remove(&id);
        }
    }

    /// A key's full path, from its hive's root.
    fn path(&self, mut id: u64) -> String {
        let mut parts = Vec::new();
        while let Some(k) = self.keys.get(&id) {
            parts.push(k.name.clone());
            id = k.parent;
        }
        parts.reverse();
        parts.join("\\")
    }

    // ---- Saving and loading ----

    /// Every lasting key and its values, in atlas's format: each key's path, then its values.
    pub fn save(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        let mut put = |bytes: &[u8]| {
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend(bytes);
        };
        let mut stack = vec![self.machine, self.users];
        while let Some(id) = stack.pop() {
            let Ok(key) = self.key(id) else { continue };
            if key.volatile {
                continue;
            }
            put(self.path(id).as_bytes());
            put(&(key.values.len() as u32).to_le_bytes());
            for v in key.values.values() {
                put(v.name.as_bytes());
                put(&v.kind.to_le_bytes());
                put(&v.data);
            }
            stack.extend(key.subkeys.values().rev());
        }
        out
    }

    /// A registry from what `save` wrote, or None if it isn't one.
    pub fn load(bytes: &[u8]) -> Option<Registry> {
        let mut at = bytes.strip_prefix(MAGIC)?;
        let mut take = || -> Option<&[u8]> {
            let len = u32::from_le_bytes(at.get(..4)?.try_into().ok()?) as usize;
            let (field, rest) = at.get(4..)?.split_at_checked(len)?;
            at = rest;
            Some(field)
        };
        let mut r = Registry { keys: HashMap::new(), next: 1, machine: 0, users: 0 };
        r.machine = r.add(0, "HKEY_LOCAL_MACHINE", false, 0);
        r.users = r.add(0, "HKEY_USERS", false, 0);
        while let Some(path) = take() {
            let path = String::from_utf8_lossy(path).into_owned();
            let (hive, rest) = path.split_once('\\').unwrap_or((&path, ""));
            let base = if hive.eq_ignore_ascii_case("HKEY_USERS") { r.users } else { r.machine };
            let (id, _) = r.create(base, rest, false, 0).ok()?;
            let count = u32::from_le_bytes(take()?.try_into().ok()?);
            for _ in 0..count {
                let name = String::from_utf8_lossy(take()?).into_owned();
                let kind = u32::from_le_bytes(take()?.try_into().ok()?);
                let data = take()?.to_vec();
                r.set_value(id, &name, kind, data, 0).ok()?;
            }
        }
        Some(r)
    }

    // ---- A fresh machine's ----

    fn put(&mut self, base: u64, path: &str, values: &[(&str, u32, Vec<u8>)]) {
        if let Ok((id, _)) = self.create(base, path, false, 0) {
            for (name, kind, data) in values {
                let _ = self.set_value(id, name, *kind, data.clone(), 0);
            }
        }
    }

    fn defaults(&mut self) {
        let sz = |s: &str| (REG_SZ, utf16z(s));
        let expand_sz = |s: &str| (REG_EXPAND_SZ, utf16z(s));
        let dword = |v: u32| (REG_DWORD, v.to_le_bytes().to_vec());
        let m = self.machine;
        let v = |pairs: Vec<(&'static str, (u32, Vec<u8>))>| pairs.into_iter().map(|(n, (k, d))| (n, k, d)).collect::<Vec<_>>();
        self.put(m, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", &v(vec![
            ("ProductName", sz("Windows 10 Pro")),
            ("EditionID", sz("Professional")),
            ("InstallationType", sz("Client")),
            ("CurrentVersion", sz("6.3")),
            ("CurrentBuild", sz(&crate::system::BUILD.to_string())),
            ("CurrentBuildNumber", sz(&crate::system::BUILD.to_string())),
            ("CurrentMajorVersionNumber", dword(crate::system::MAJOR)),
            ("CurrentMinorVersionNumber", dword(crate::system::MINOR)),
            ("DisplayVersion", sz("22H2")),
            ("SystemRoot", sz("C:\\Windows")),
            ("RegisteredOwner", sz("carbon")),
        ]));
        self.put(m, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion", &v(vec![
            ("ProgramFilesDir", sz("C:\\Program Files")),
            ("ProgramFilesDir (x86)", sz("C:\\Program Files (x86)")),
            ("CommonFilesDir", sz("C:\\Program Files\\Common Files")),
            ("ProgramFilesPath", expand_sz("%ProgramFiles%")),
        ]));
        self.put(m, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall", &[]);
        self.put(m, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths", &[]);
        self.put(m, "SOFTWARE\\Classes", &[]);
        self.put(m, "SOFTWARE\\WOW6432Node", &[]);
        self.put(m, "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment", &v(vec![
            ("Path", expand_sz("%SystemRoot%\\system32;%SystemRoot%;%SystemRoot%\\System32\\Wbem")),
            ("PATHEXT", sz(".COM;.EXE;.BAT;.CMD")),
            ("TEMP", expand_sz("%SystemRoot%\\TEMP")),
            ("TMP", expand_sz("%SystemRoot%\\TEMP")),
            ("windir", expand_sz("%SystemRoot%")),
            ("OS", sz("Windows_NT")),
            ("NUMBER_OF_PROCESSORS", sz("1")),
        ]));
        self.put(m, "SYSTEM\\CurrentControlSet\\Control\\ComputerName\\ComputerName", &v(vec![("ComputerName", sz("CARBON"))]));
        self.put(m, "SYSTEM\\CurrentControlSet\\Control\\Nls\\Language", &v(vec![("Default", sz("0409")), ("InstallLanguage", sz("0409"))]));
        self.put(m, "SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation", &v(vec![("TimeZoneKeyName", sz("UTC"))]));
        self.put(m, "SYSTEM\\CurrentControlSet\\Hardware Profiles\\Current", &[]);
        self.put(m, "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0", &v(vec![
            ("ProcessorNameString", sz("Carbon virtual processor")),
            ("~MHz", dword(3000)),
            ("Identifier", sz("Carbon")),
        ]));
        let u = self.users;
        let user = USER;
        self.put(u, &format!("{user}\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Folders"), &v(vec![
            ("AppData", sz("C:\\Users\\carbon\\AppData\\Roaming")),
            ("Local AppData", sz("C:\\Users\\carbon\\AppData\\Local")),
            ("Personal", sz("C:\\Users\\carbon\\Documents")),
            ("Desktop", sz("C:\\Users\\carbon\\Desktop")),
        ]));
        self.put(u, &format!("{user}\\Environment"), &v(vec![("TEMP", expand_sz("%SystemRoot%\\TEMP")), ("TMP", expand_sz("%SystemRoot%\\TEMP"))]));
        self.put(u, &format!("{user}\\Control Panel\\International"), &v(vec![("LocaleName", sz("en-US")), ("sDecimal", sz(".")), ("sThousand", sz(","))]));
        self.put(u, &format!("{user}\\Software\\Classes"), &[]);
        self.put(u, ".DEFAULT", &[]);
    }
}

impl Default for Registry {
    fn default() -> Registry {
        Registry::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_values_and_saving() {
        let mut r = Registry::new();
        let user = r.root(Root::CurrentUser);
        let (key, made) = r.create(user, "Software\\Carbon\\Test", false, 1).unwrap();
        assert!(made);
        assert_eq!(r.create(user, "software\\CARBON\\test", false, 1).unwrap(), (key, false));
        r.set_value(key, "Name", REG_SZ, utf16z("atlas"), 1).unwrap();
        assert_eq!(text(&r.value(key, "NAME").unwrap().data), "atlas");
        let (volatile, _) = r.create(user, "Software\\Carbon\\Gone", true, 1).unwrap();
        r.set_value(volatile, "x", REG_DWORD, vec![1, 0, 0, 0], 1).unwrap();
        let saved = r.save();
        let back = Registry::load(&saved).unwrap();
        let mut back = back;
        let user = back.root(Root::CurrentUser);
        let key = back.open(user, "Software\\Carbon\\Test").unwrap();
        assert_eq!(back.key(key).unwrap().name, "Test");
        assert_eq!(text(&back.value(key, "name").unwrap().data), "atlas");
        assert_eq!(back.open(user, "Software\\Carbon\\Gone"), Err(ERROR_FILE_NOT_FOUND));
        let machine = back.root(Root::LocalMachine);
        assert!(back.open(machine, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion").is_ok());
        assert_eq!(back.delete_key(user, "Software\\Carbon", false), Err(ERROR_ACCESS_DENIED));
        back.delete_key(user, "Software\\Carbon", true).unwrap();
        assert_eq!(back.key(key).map(|_| ()), Err(ERROR_KEY_DELETED));
    }

    #[test]
    fn expanding() {
        assert_eq!(expand("%SystemRoot%\\system32;%Nope%;100%"), "C:\\Windows\\system32;%Nope%;100%");
    }
}
