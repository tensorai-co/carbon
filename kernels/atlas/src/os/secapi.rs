//! advapi32's security calls, over `security`: tokens and privileges, SIDs and accounts, security descriptors and
//! ACLs, and SDDL. They set the last error and return a BOOL, as Windows' do.
//!
//! Descriptors and ACLs live in the program's memory, as on Windows: each call reads what it's given (an absolute
//! descriptor's parts through its pointers, a self-relative one's at its offsets), and writes back what it changes.
//! What a call allocates (`AllocateAndInitializeSid`, the string conversions) is `LocalAlloc` memory.

use std::sync::Arc;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use super::kernel32::{global_new, global_release};
use crate::handles::{CURRENT_PROCESS, CURRENT_PROCESS_TOKEN, CURRENT_THREAD, CURRENT_THREAD_EFFECTIVE_TOKEN, Handle};
use crate::objects::{Kind, Object};
use crate::sections::Section;
use crate::process::Task;
use crate::security::*;

const TRUE: u64 = 1;
const FALSE: u64 = 0;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NO_TOKEN: u32 = 1008;
const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;
const ERROR_INVALID_OWNER: u32 = 1307;
const ERROR_INVALID_PRIMARY_GROUP: u32 = 1308;
const ERROR_NO_IMPERSONATION_TOKEN: u32 = 1309;
const ERROR_NO_SUCH_LOGON_SESSION: u32 = 1312;
const ERROR_NO_SUCH_PRIVILEGE: u32 = 1313;
const ERROR_PRIVILEGE_NOT_HELD: u32 = 1314;
const ERROR_NONE_MAPPED: u32 = 1332;
const ERROR_INVALID_SECURITY_DESCR: u32 = 1338;
const ERROR_ALLOTTED_SPACE_EXCEEDED: u32 = 1344;
const ERROR_BAD_DESCRIPTOR_FORMAT: u32 = 1361;

type R = Result<u64, u64>;

fn reply(r: R) -> Reply {
    Reply::Value(r.unwrap_or_else(|e| e))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// A string as a `*W` function gives it (UTF-16) or an `*A` one (ANSI), with its NUL.
fn text(s: &str, unicode: bool) -> Vec<u8> {
    if unicode { wide(s) } else { crate::ansi::bytes(s).into_iter().chain([0]).collect() }
}

/// A string the program passed, UTF-16 or ANSI.
fn read_text(space: &exec::Space<'_>, at: u64, most: usize, unicode: bool) -> Result<String, kernels_core::user::Fault> {
    if unicode { user::read_wstr(space, at, most) } else { user::read_cstr(space, at, most).map(|b| crate::ansi::text(&b)) }
}

impl Task {
    fn failed(&mut self, cx: &mut Cx<'_>, error: u32) -> u64 {
        self.set_last_error(cx, error);
        FALSE
    }

    fn put_u32(&mut self, cx: &mut Cx<'_>, at: u64, v: u32) -> Result<(), u64> {
        self.put(cx, at, &v.to_le_bytes())
    }

    /// A pointer-sized value: a pointer or a handle, as this process's ISA has them.
    fn put_ptr(&mut self, cx: &mut Cx<'_>, at: u64, v: u64) -> Result<(), u64> {
        let bytes = self.abi().ptr(v);
        self.put(cx, at, &bytes)
    }

    fn get_u32(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u32, u64> {
        Ok(u32_at(&self.get(cx, at, 4)?, 0))
    }

    fn get_ptr(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u64, u64> {
        let abi = self.abi();
        Ok(abi.ptr_at(&self.get(cx, at, abi.width as usize)?, 0))
    }

    fn read_sid(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<Sid, u64> {
        let head = self.get(cx, at, 8)?;
        if head[0] != 1 || head[1] > 15 {
            return Err(self.failed(cx, ERROR_INVALID_SID));
        }
        let bytes = self.get(cx, at, 8 + 4 * usize::from(head[1]))?;
        Sid::from_bytes(&bytes).ok_or_else(|| self.failed(cx, ERROR_INVALID_SID))
    }

    fn read_acl(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<Acl, u64> {
        let head = self.get(cx, at, 8)?;
        let size = usize::from(u16_at(&head, 2)).max(8);
        let bytes = self.get(cx, at, size)?;
        Acl::from_bytes(&bytes).ok_or_else(|| self.failed(cx, ERROR_INVALID_ACL))
    }

    /// Where a descriptor's parts are, in either form (owner, group, SACL, DACL; 0 for none), and its control.
    fn descriptor_parts(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<(u16, [u64; 4]), u64> {
        let head = self.get(cx, at, 20)?;
        if head[0] != 1 {
            return Err(self.failed(cx, ERROR_INVALID_SECURITY_DESCR));
        }
        let control = u16_at(&head, 2);
        let mut parts = [0u64; 4];
        if control & SE_SELF_RELATIVE != 0 {
            for (i, p) in parts.iter_mut().enumerate() {
                let offset = u32_at(&head, 4 + 4 * i);
                *p = if offset == 0 { 0 } else { at + u64::from(offset) };
            }
        } else {
            // Absolute: the header, then four pointers from a pointer's size on.
            let abi = self.abi();
            let w = abi.width as usize;
            let head = self.get(cx, at, 5 * w)?;
            for (i, p) in parts.iter_mut().enumerate() {
                *p = abi.ptr_at(&head, w + w * i);
            }
        }
        Ok((control, parts))
    }

    fn read_descriptor(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<Descriptor, u64> {
        let (control, [owner, group, sacl, dacl]) = self.descriptor_parts(cx, at)?;
        let mut d = Descriptor { control: control & !SE_SELF_RELATIVE, ..Descriptor::default() };
        if owner != 0 {
            d.owner = Some(self.read_sid(cx, owner)?);
        }
        if group != 0 {
            d.group = Some(self.read_sid(cx, group)?);
        }
        if control & SE_SACL_PRESENT != 0 && sacl != 0 {
            d.sacl = Some(self.read_acl(cx, sacl)?);
        }
        if control & SE_DACL_PRESENT != 0 && dacl != 0 {
            d.dacl = Some(self.read_acl(cx, dacl)?);
        }
        Ok(d)
    }

    /// The token a handle is: one of its own, or a pseudo-handle: the process's (-4), the thread's (-5), or
    /// whichever this thread acts as (-6).
    pub(crate) fn token_of(&mut self, cx: &mut Cx<'_>, handle: u64) -> Result<Arc<Token>, u64> {
        match handle {
            CURRENT_PROCESS_TOKEN => return Ok(lock(&self.process.state).token.clone()),
            CURRENT_THREAD_EFFECTIVE_TOKEN => return Ok(self.effective_token()),
            h if h == CURRENT_PROCESS_TOKEN - 1 => {
                let token = lock(&self.control.token).clone();
                return token.ok_or_else(|| self.failed(cx, ERROR_NO_TOKEN));
            }
            _ => {}
        }
        let found = lock(&self.process.state).handles.get(handle);
        match found {
            Some(Handle::Token(t)) => Ok(t),
            _ => Err(self.failed(cx, ERROR_INVALID_HANDLE)),
        }
    }

    fn new_token_handle(&mut self, token: Token, inheritable: bool) -> u64 {
        lock(&self.process.state).handles.add_with(Handle::Token(Arc::new(token)), inheritable)
    }

    /// Writes `data` (made for where it's going, `buffer`) if `length` holds it, with its size at `returned`.
    fn put_info(&mut self, cx: &mut Cx<'_>, buffer: u64, length: u64, returned: u64, data: &[u8]) -> R {
        if returned != 0 {
            self.put_u32(cx, returned, data.len() as u32)?;
        }
        if buffer == 0 || (length as u32 as usize) < data.len() {
            return Err(self.failed(cx, ERROR_INSUFFICIENT_BUFFER));
        }
        self.put(cx, buffer, data)?;
        Ok(TRUE)
    }

    /// A new `LocalAlloc` block holding `data`, its address written at `out`.
    fn put_allocated(&mut self, cx: &mut Cx<'_>, out: u64, data: &[u8]) -> R {
        let block = global_new(self, cx, data);
        if block == 0 {
            return Err(self.failed(cx, 8)); // ERROR_NOT_ENOUGH_MEMORY
        }
        self.put_ptr(cx, out, block)?;
        Ok(TRUE)
    }
}

/// A structure with pointers into itself, as token information is: built for where it'll be written.
struct Blob {
    base: u64,
    bytes: Vec<u8>,
    /// A pointer's size in the program it's for.
    width: usize,
}

impl Blob {
    fn new(base: u64, head: usize, width: u64) -> Blob {
        Blob { base, bytes: vec![0; head], width: width as usize }
    }
    fn set_ptr(&mut self, at: usize, v: u64) {
        let w = self.width;
        self.bytes[at..at + w].copy_from_slice(&v.to_le_bytes()[..w]);
    }
    /// SID_AND_ATTRIBUTES' size: a pointer and a DWORD, padded to a pointer's size.
    fn sid_and_attributes_size(width: u64) -> usize {
        2 * width as usize
    }
    fn set_u32(&mut self, at: usize, v: u32) {
        self.bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn set_u64(&mut self, at: usize, v: u64) {
        self.bytes[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    /// Appends `data`, 4-aligned, giving its address.
    fn append(&mut self, data: &[u8]) -> u64 {
        self.bytes.resize(self.bytes.len().next_multiple_of(4), 0);
        let at = self.base + self.bytes.len() as u64;
        self.bytes.extend(data);
        at
    }
    /// A `SID_AND_ATTRIBUTES` at `at`, its SID appended.
    fn sid_and_attributes(&mut self, at: usize, sid: &Sid, attributes: u32) {
        let p = self.append(&sid.to_bytes());
        self.set_ptr(at, p);
        self.set_u32(at + self.width, attributes);
    }
}

/// TOKEN_GROUPS: a count, then SID_AND_ATTRIBUTES from a pointer's size on.
fn groups_blob(base: u64, groups: &[(Sid, u32)], width: u64) -> Vec<u8> {
    let entry = Blob::sid_and_attributes_size(width);
    let mut b = Blob::new(base, width as usize + entry * groups.len(), width);
    b.set_u32(0, groups.len() as u32);
    for (i, (sid, attributes)) in groups.iter().enumerate() {
        b.sid_and_attributes(width as usize + entry * i, sid, *attributes);
    }
    b.bytes
}

fn privileges_blob(privileges: &[(u32, u32)]) -> Vec<u8> {
    let mut b = (privileges.len() as u32).to_le_bytes().to_vec();
    for (luid, attributes) in privileges {
        b.extend(luid.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(attributes.to_le_bytes());
    }
    b
}

/// One pointer to a thing appended after it: `TOKEN_OWNER`, `TOKEN_PRIMARY_GROUP`, `TOKEN_DEFAULT_DACL`.
fn pointer_blob(base: u64, data: Option<Vec<u8>>, width: u64) -> Vec<u8> {
    let mut b = Blob::new(base, width as usize, width);
    if let Some(data) = data {
        let p = b.append(&data);
        b.set_ptr(0, p);
    }
    b.bytes
}

// --- Tokens ---

/// `OpenProcessToken(process, access, &token)`: a handle to that process's own token.
pub fn open_process_token(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let Some(process) = task.process_of(a[0]) else { return Err(task.failed(cx, ERROR_INVALID_HANDLE)) };
        let token = lock(&process.state).token.clone();
        let handle = lock(&task.process.state).handles.add(Handle::Token(token));
        task.put_ptr(cx, a[2], handle)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `OpenThreadToken(thread, access, as self, &token)`: the token it's impersonating, or ERROR_NO_TOKEN.
pub fn open_thread_token(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let _ = task.put_ptr(cx, a[3], 0);
        let Some(thread) = task.thread_of(a[0]) else { return Err(task.failed(cx, ERROR_INVALID_HANDLE)) };
        let token = lock(&thread.token).clone();
        let Some(token) = token else { return Err(task.failed(cx, ERROR_NO_TOKEN)) };
        let handle = lock(&task.process.state).handles.add(Handle::Token(token));
        task.put_ptr(cx, a[3], handle)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `GetTokenInformation(token, class, buffer, length, &returned)`.
pub fn get_token_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let token = task.token_of(cx, a[0])?;
        let (class, buffer) = (a[1] as u32, a[2]);
        let s = lock(&token.state).clone();
        let u32s = |v: u32| v.to_le_bytes().to_vec();
        let w = task.abi().width;
        let data = match class {
            1 => {
                let mut b = Blob::new(buffer, Blob::sid_and_attributes_size(w), w);
                b.sid_and_attributes(0, &s.user, if s.user_deny_only { SE_GROUP_USE_FOR_DENY_ONLY } else { 0 });
                b.bytes
            }
            2 => groups_blob(buffer, &s.groups, w),
            3 => privileges_blob(&s.privileges),
            4 => pointer_blob(buffer, Some(s.owner.to_bytes()), w),
            5 => pointer_blob(buffer, Some(s.primary_group.to_bytes()), w),
            6 => pointer_blob(buffer, s.default_dacl.as_ref().map(Acl::to_bytes), w),
            7 => {
                let mut b = b"User32 \0".to_vec();
                b.extend(0x1_0001u64.to_le_bytes());
                b
            }
            8 => u32s(s.kind),
            9 if s.kind == 2 => u32s(s.level),
            10 => {
                let mut b = Blob::new(buffer, 56, w);
                b.set_u64(0, token.id);
                b.set_u64(8, LOGON);
                b.set_u64(16, i64::MAX as u64);
                b.set_u32(24, s.kind);
                b.set_u32(28, s.level);
                b.set_u32(32, 4096);
                b.set_u32(36, 4096);
                b.set_u32(40, s.groups.len() as u32);
                b.set_u32(44, s.privileges.len() as u32);
                b.set_u64(48, s.modified);
                b.bytes
            }
            11 => groups_blob(buffer, &s.restricted, w),
            30 => groups_blob(buffer, &[], w),
            12 => u32s(s.session),
            17 => LOGON.to_le_bytes().to_vec(),
            18 => u32s(1),
            19 => return Err(task.failed(cx, ERROR_NO_SUCH_LOGON_SESSION)),
            20 => u32s(1),
            23 | 24 | 26 | 29 => u32s(0),
            25 => {
                let mut b = Blob::new(buffer, Blob::sid_and_attributes_size(w), w);
                b.sid_and_attributes(0, &s.integrity(), SE_GROUP_INTEGRITY);
                b.bytes
            }
            27 => u32s(3),
            28 => groups_blob(buffer, &[(logon_sid(), GROUP_DEFAULT | SE_GROUP_LOGON_ID)], w),
            31 => vec![0; 8],
            _ => return Err(task.failed(cx, ERROR_INVALID_PARAMETER)),
        };
        task.put_info(cx, buffer, a[3], a[4], &data)
    })();
    reply(r)
}

/// `SetTokenInformation(token, class, buffer, length)`: the owner, the primary group, the default DACL, and the
/// integrity level, lowered only.
pub fn set_token_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let token = task.token_of(cx, a[0])?;
        let (class, buffer) = (a[1] as u32, a[2]);
        match class {
            4 | 5 => {
                let p = task.get_ptr(cx, buffer)?;
                let sid = task.read_sid(cx, p)?;
                let mut s = lock(&token.state);
                if class == 4 {
                    let may = sid == s.user || s.groups.iter().any(|(g, at)| *g == sid && at & SE_GROUP_OWNER != 0);
                    if !may {
                        drop(s);
                        return Err(task.failed(cx, ERROR_INVALID_OWNER));
                    }
                    s.owner = sid;
                } else {
                    let may = sid == s.user || s.groups.iter().any(|(g, _)| *g == sid);
                    if !may {
                        drop(s);
                        return Err(task.failed(cx, ERROR_INVALID_PRIMARY_GROUP));
                    }
                    s.primary_group = sid;
                }
                s.modified = new_luid();
            }
            6 => {
                let p = task.get_ptr(cx, buffer)?;
                let dacl = if p == 0 { None } else { Some(task.read_acl(cx, p)?) };
                let mut s = lock(&token.state);
                s.default_dacl = dacl;
                s.modified = new_luid();
            }
            25 => {
                let p = task.get_ptr(cx, buffer)?;
                let sid = task.read_sid(cx, p)?;
                let mut s = lock(&token.state);
                let now = s.integrity();
                let rid = |sid: &Sid| sid.subs.last().copied().unwrap_or(0);
                if sid.authority != 16 || sid.subs.len() != 1 || rid(&sid) > rid(&now) {
                    drop(s);
                    return Err(task.failed(cx, if sid.authority == 16 { ERROR_PRIVILEGE_NOT_HELD } else { ERROR_INVALID_PARAMETER }));
                }
                if let Some(g) = s.groups.iter_mut().find(|(_, at)| at & SE_GROUP_INTEGRITY != 0) {
                    g.0 = sid;
                }
                s.modified = new_luid();
            }
            12 => return Err(task.failed(cx, ERROR_PRIVILEGE_NOT_HELD)),
            _ => return Err(task.failed(cx, ERROR_INVALID_PARAMETER)),
        }
        Ok(TRUE)
    })();
    reply(r)
}

/// `AdjustTokenPrivileges(token, disable all, new state, length, previous state, &returned)`: TRUE, with
/// ERROR_NOT_ALL_ASSIGNED as the last error if the token didn't have one of them.
pub fn adjust_token_privileges(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let token = task.token_of(cx, a[0])?;
        let wanted: Vec<(u32, u32)> = if a[1] != 0 {
            Vec::new()
        } else {
            let count = task.get_u32(cx, a[2])? as usize;
            let b = task.get(cx, a[2] + 4, 12 * count.min(64))?;
            (0..count.min(64)).map(|i| (u32_at(&b, 12 * i), u32_at(&b, 12 * i + 8))).collect()
        };
        let mut s = lock(&token.state).clone();
        let mut previous = Vec::new();
        let mut missing = false;
        if a[1] != 0 {
            for p in s.privileges.iter_mut().filter(|p| p.1 & SE_PRIVILEGE_ENABLED != 0) {
                previous.push(*p);
                p.1 &= !SE_PRIVILEGE_ENABLED;
            }
        }
        for (luid, attributes) in wanted {
            let Some(i) = s.privileges.iter().position(|p| p.0 == luid) else {
                missing = true;
                continue;
            };
            let old = s.privileges[i];
            if attributes & SE_PRIVILEGE_REMOVED != 0 {
                s.privileges.remove(i);
            } else {
                s.privileges[i].1 = old.1 & !SE_PRIVILEGE_ENABLED | attributes & SE_PRIVILEGE_ENABLED;
            }
            if s.privileges.get(i) != Some(&old) {
                previous.push(old);
            }
        }
        if a[4] != 0 {
            let old = privileges_blob(&previous);
            task.put_info(cx, a[4], a[3], a[5], &old)?;
        }
        s.modified = new_luid();
        *lock(&token.state) = s;
        task.set_last_error(cx, if missing { ERROR_NOT_ALL_ASSIGNED } else { 0 });
        Ok(TRUE)
    })();
    reply(r)
}

/// `PrivilegeCheck(token, &set, &result)`: whether the set's privileges are enabled (all of them, or any).
pub fn privilege_check(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let token = task.token_of(cx, a[0])?;
        let head = task.get(cx, a[1], 8)?;
        let (count, all) = ((u32_at(&head, 0) as usize).min(64), u32_at(&head, 4) & 1 != 0);
        let mut b = task.get(cx, a[1] + 8, 12 * count)?;
        let s = lock(&token.state).clone();
        let mut held = 0;
        for i in 0..count {
            let luid = u32_at(&b, 12 * i);
            if s.privileges.iter().any(|p| p.0 == luid && p.1 & SE_PRIVILEGE_ENABLED != 0) {
                held += 1;
                let at = u32_at(&b, 12 * i + 8) | 0x8000_0000; // SE_PRIVILEGE_USED_FOR_ACCESS
                b[12 * i + 8..12 * i + 12].copy_from_slice(&at.to_le_bytes());
            }
        }
        task.put(cx, a[1] + 8, &b)?;
        let ok = if all { held == count } else { held > 0 || count == 0 };
        task.put_u32(cx, a[2], u32::from(ok))?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `DuplicateTokenEx(token, access, attributes, level, type, &new)`.
pub fn duplicate_token_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let token = task.token_of(cx, a[0])?;
        let (level, kind) = (a[3] as u32, a[4] as u32);
        if !(1..=2).contains(&kind) || level > 3 {
            return Err(task.failed(cx, ERROR_INVALID_PARAMETER));
        }
        let inheritable = task.inheritable_by(cx, a[2]);
        let handle = task.new_token_handle(token.duplicate(kind, level), inheritable);
        task.put_ptr(cx, a[5], handle)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `CreateRestrictedToken(token, flags, disable count, &SIDs to disable, delete count, &privileges to delete, restrict
/// count, &restricting SIDs, &new)`: a copy of the same kind with those SIDs (the user's too) deny-only, those
/// privileges gone (all but SeChangeNotifyPrivilege with DISABLE_MAX_PRIVILEGE), and the restricting SIDs added. With
/// LUA_TOKEN, Administrators is deny-only and the token is at medium integrity with a standard user's privileges.
pub fn create_restricted_token(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
    const LUA_TOKEN: u32 = 0x4;
    const WRITE_RESTRICTED: u32 = 0x8;
    /// SE_GROUP_ENABLED and SE_GROUP_ENABLED_BY_DEFAULT.
    const ENABLED: u32 = 0x6;
    /// SID_AND_ATTRIBUTES' SIDs: a pointer and a DWORD each, two pointers' room.
    fn sids(task: &mut Task, cx: &mut Cx<'_>, n: u64, at: u64) -> Result<Vec<Sid>, u64> {
        let w = task.abi().width;
        let mut out = Vec::new();
        for i in 0..n.min(1024) {
            let Ok(p) = task.mem(cx, false, |t| user::read_ptr(&t.space(), at + i * 2 * w, w)) else { return Err(task.failed(cx, 998)) };
            out.push(task.read_sid(cx, p)?);
        }
        Ok(out)
    }
    let r = (|| {
        let token = task.token_of(cx, a[0])?;
        let flags = a[1] as u32;
        let disable = sids(task, cx, a[2] as u32 as u64, a[3])?;
        let mut delete = Vec::new();
        for i in 0..(a[4] as u32).min(64) {
            // LUID_AND_ATTRIBUTES: the LUID's low part, its high part, then the attributes.
            let Ok(luid) = task.mem(cx, false, |t| user::read_u32(&t.space(), a[5] + u64::from(i) * 12)) else { return Err(task.failed(cx, 998)) };
            delete.push(luid);
        }
        let restrict = sids(task, cx, a[6] as u32 as u64, a[7])?;
        let mut s = lock(&token.state).clone();
        let deny = |s: &mut TokenState, sid: &Sid| {
            if *sid == s.user {
                s.user_deny_only = true;
            }
            for g in s.groups.iter_mut().filter(|g| g.0 == *sid && g.1 & SE_GROUP_INTEGRITY == 0) {
                g.1 = (g.1 & !ENABLED) | SE_GROUP_USE_FOR_DENY_ONLY;
            }
        };
        for sid in &disable {
            deny(&mut s, sid);
        }
        if flags & LUA_TOKEN != 0 {
            let administrators = Sid::new(5, &[32, 544]);
            deny(&mut s, &administrators);
            for g in s.groups.iter_mut().filter(|g| g.1 & SE_GROUP_INTEGRITY != 0) {
                g.0 = Sid::new(16, &[8192]);
            }
            // A standard user's: shutdown, change notify, undock, working set, time zone.
            s.privileges.retain(|p| matches!(p.0, 19 | 23 | 25 | 33 | 34));
            if s.owner == administrators {
                s.owner = s.user.clone();
            }
        }
        if flags & DISABLE_MAX_PRIVILEGE != 0 {
            s.privileges.retain(|p| p.0 == 23);
        } else {
            s.privileges.retain(|p| !delete.contains(&p.0));
        }
        for sid in restrict {
            if !s.restricted.iter().any(|(r, _)| *r == sid) {
                s.restricted.push((sid, GROUP_DEFAULT));
            }
        }
        s.write_restricted |= flags & WRITE_RESTRICTED != 0;
        s.modified = new_luid();
        let handle = task.new_token_handle(Token::with(s), false);
        task.put_ptr(cx, a[8], handle)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `DuplicateToken(token, level, &new)`: an impersonation token.
pub fn duplicate_token(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    duplicate_token_ex(task, cx, &[a[0], 0, 0, a[1], 2, a[2]])
}

/// `CheckTokenMembership(token, sid, &member)`: an impersonation token's, or with none this thread's (its process's).
pub fn check_token_membership(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let token = if a[0] == 0 { task.effective_token() } else { task.token_of(cx, a[0])? };
        if a[0] != 0 && lock(&token.state).kind != 2 {
            return Err(task.failed(cx, ERROR_NO_IMPERSONATION_TOKEN));
        }
        let sid = task.read_sid(cx, a[1])?;
        let member = lock(&token.state).has(&sid);
        task.put_u32(cx, a[2], u32::from(member))?;
        Ok(TRUE)
    })();
    reply(r)
}

fn lookup_privilege_value(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> R {
    let name = if wide {
        task.mem(cx, false, |t| user::read_wstr(&t.space(), a[1], 256))
    } else {
        task.mem(cx, false, |t| user::read_cstr(&t.space(), a[1], 256)).map(|b| crate::ansi::text(&b))
    };
    let Some(luid) = name.ok().as_deref().and_then(privilege_named) else { return Err(task.failed(cx, ERROR_NO_SUCH_PRIVILEGE)) };
    task.put(cx, a[2], &u64::from(luid).to_le_bytes())?;
    Ok(TRUE)
}

/// `LookupPrivilegeValueW(system, name, &luid)` and `A`.
pub fn lookup_privilege_value_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(lookup_privilege_value(task, cx, a, true))
}

pub fn lookup_privilege_value_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(lookup_privilege_value(task, cx, a, false))
}

/// `LookupPrivilegeNameW(system, &luid, name, &length)`.
pub fn lookup_privilege_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    lookup_privilege_name(task, cx, a, true)
}

pub fn lookup_privilege_name_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    lookup_privilege_name(task, cx, a, false)
}

fn lookup_privilege_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let r = (|| {
        let luid = u64_at(&task.get(cx, a[1], 8)?, 0);
        let Some(name) = u32::try_from(luid).ok().and_then(privilege_name) else { return Err(task.failed(cx, ERROR_NO_SUCH_PRIVILEGE)) };
        let room = task.get_u32(cx, a[3])? as usize;
        if room <= name.len() || a[2] == 0 {
            task.put_u32(cx, a[3], name.len() as u32 + 1)?;
            return Err(task.failed(cx, ERROR_INSUFFICIENT_BUFFER));
        }
        task.put(cx, a[2], &text(name, unicode))?;
        task.put_u32(cx, a[3], name.len() as u32)?;
        Ok(TRUE)
    })();
    reply(r)
}

// --- SIDs ---

/// `AllocateAndInitializeSid(&authority, count, s0, … s7, &sid)`.
pub fn allocate_and_initialize_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let count = a[1] as u8;
        if count == 0 || count > 8 {
            return Err(task.failed(cx, ERROR_INVALID_SID));
        }
        let authority = task.get(cx, a[0], 6)?.iter().fold(0u64, |acc, &b| acc << 8 | u64::from(b));
        let subs: Vec<u32> = a[2..2 + usize::from(count)].iter().map(|&s| s as u32).collect();
        task.put_allocated(cx, a[10], &Sid::new(authority, &subs).to_bytes())
    })();
    reply(r)
}

/// `FreeSid(sid)`: NULL once it's freed.
pub fn free_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if global_release(task, cx, a[0]) { 0 } else { a[0] })
}

/// `InitializeSid(sid, &authority, count)`: the subauthorities left as zeros, for the program to fill in.
pub fn initialize_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let count = a[2] as u8;
        if count > 15 {
            return Err(task.failed(cx, ERROR_INVALID_PARAMETER));
        }
        let authority = task.get(cx, a[1], 6)?;
        let mut b = vec![1, count];
        b.extend(authority);
        task.put(cx, a[0], &b)?;
        Ok(TRUE)
    })();
    reply(r)
}

pub fn get_sid_length_required(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(8 + 4 * u64::from(a[0] as u8))
}

/// `GetSidSubAuthority(sid, n)`, `GetSidSubAuthorityCount(sid)` and `GetSidIdentifierAuthority(sid)`: where in it.
pub fn get_sid_sub_authority(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(a[0] + 8 + 4 * (a[1] & 0xffff_ffff))
}

pub fn get_sid_sub_authority_count(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(a[0] + 1)
}

pub fn get_sid_identifier_authority(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(a[0] + 2)
}

pub fn get_length_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.get(cx, a[0], 2).map_or(0, |b| 8 + 4 * u64::from(b[1])))
}

pub fn is_valid_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[0] != 0 && task.get(cx, a[0], 2).is_ok_and(|b| b[0] == 1 && b[1] <= 15)))
}

/// `EqualSid(a, b)`: TRUE, or FALSE with no error if they differ.
pub fn equal_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let (x, y) = (task.read_sid(cx, a[0])?, task.read_sid(cx, a[1])?);
        task.set_last_error(cx, 0);
        Ok(u64::from(x == y))
    })();
    reply(r)
}

/// `EqualPrefixSid(a, b)`: the same but for their last subauthority.
pub fn equal_prefix_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let (mut x, mut y) = (task.read_sid(cx, a[0])?, task.read_sid(cx, a[1])?);
        let same_count = x.subs.len() == y.subs.len();
        x.subs.pop();
        y.subs.pop();
        task.set_last_error(cx, 0);
        Ok(u64::from(same_count && x == y))
    })();
    reply(r)
}

/// `CopySid(length, dest, source)`.
pub fn copy_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let sid = task.read_sid(cx, a[2])?.to_bytes();
        if (a[0] as u32 as usize) < sid.len() {
            return Err(task.failed(cx, ERROR_INSUFFICIENT_BUFFER));
        }
        task.put(cx, a[1], &sid)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `CreateWellKnownSid(type, domain, sid, &size)`.
pub fn create_well_known_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let domain = if a[1] != 0 { Some(task.read_sid(cx, a[1])?) } else { None };
        let Some(sid) = well_known(a[0] as u32, domain.as_ref()) else { return Err(task.failed(cx, ERROR_INVALID_PARAMETER)) };
        let bytes = sid.to_bytes();
        let room = task.get_u32(cx, a[3])? as usize;
        task.put_u32(cx, a[3], bytes.len() as u32)?;
        if room < bytes.len() || a[2] == 0 {
            return Err(task.failed(cx, ERROR_INSUFFICIENT_BUFFER));
        }
        task.put(cx, a[2], &bytes)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `IsWellKnownSid(sid, type)`.
pub fn is_well_known_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let sid = task.read_sid(cx, a[0]).ok();
    Reply::Value(u64::from(sid.is_some() && well_known(a[1] as u32, None) == sid))
}

/// `ConvertSidToStringSidW(sid, &string)`: `S-1-…`, in `LocalAlloc` memory.
pub fn convert_sid_to_string_sid_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_sid_to_string_sid(task, cx, a, true)
}

pub fn convert_sid_to_string_sid_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_sid_to_string_sid(task, cx, a, false)
}

fn convert_sid_to_string_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let r = (|| {
        let sid = task.read_sid(cx, a[0])?;
        task.put_allocated(cx, a[1], &text(&sid.to_string(), unicode))
    })();
    reply(r)
}

/// `ConvertStringSidToSidW(string, &sid)`: `S-1-…` or an SDDL alias.
pub fn convert_string_sid_to_sid_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_string_sid_to_sid(task, cx, a, true)
}

pub fn convert_string_sid_to_sid_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_string_sid_to_sid(task, cx, a, false)
}

fn convert_string_sid_to_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let r = (|| {
        let s = task.mem(cx, false, |t| read_text(&t.space(), a[0], 512, unicode)).map_err(|_| task.failed(cx, 998))?;
        let Some(sid) = sid_from_sddl(&s) else { return Err(task.failed(cx, ERROR_INVALID_SID)) };
        task.put_allocated(cx, a[1], &sid.to_bytes())
    })();
    reply(r)
}

/// Writes a name into a buffer of `*length` characters: TRUE if it fits; else the length it needs, with its NUL.
fn put_name(task: &mut Task, cx: &mut Cx<'_>, buffer: u64, length: u64, name: &str, unicode: bool) -> Result<bool, u64> {
    let room = task.get_u32(cx, length)? as usize;
    let bytes = text(name, unicode);
    let len = bytes.len() / if unicode { 2 } else { 1 } - 1;
    if room <= len || buffer == 0 {
        task.put_u32(cx, length, len as u32 + 1)?;
        return Ok(false);
    }
    task.put(cx, buffer, &bytes)?;
    task.put_u32(cx, length, len as u32)?;
    Ok(true)
}

/// `LookupAccountSidW(system, sid, name, &length, domain, &length, &use)`.
pub fn lookup_account_sid_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    lookup_account_sid(task, cx, a, true)
}

pub fn lookup_account_sid_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    lookup_account_sid(task, cx, a, false)
}

fn lookup_account_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let r = (|| {
        let sid = task.read_sid(cx, a[1])?;
        let Some((_, domain, name, kind)) = accounts().into_iter().find(|(s, _, _, _)| *s == sid) else {
            return Err(task.failed(cx, ERROR_NONE_MAPPED));
        };
        // Both sizes are given back when either's too small.
        let (n_room, d_room) = (task.get_u32(cx, a[3])? as usize, task.get_u32(cx, a[5])? as usize);
        if n_room <= name.len() || d_room <= domain.len() || a[2] == 0 || a[4] == 0 {
            task.put_u32(cx, a[3], name.len() as u32 + 1)?;
            task.put_u32(cx, a[5], domain.len() as u32 + 1)?;
            return Err(task.failed(cx, ERROR_INSUFFICIENT_BUFFER));
        }
        put_name(task, cx, a[2], a[3], name, unicode)?;
        put_name(task, cx, a[4], a[5], domain, unicode)?;
        task.put_u32(cx, a[6], kind)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `LookupAccountNameW(system, name, sid, &size, domain, &length, &use)`: a name, or `DOMAIN\name`.
pub fn lookup_account_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    lookup_account_name(task, cx, a, true)
}

pub fn lookup_account_name_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    lookup_account_name(task, cx, a, false)
}

fn lookup_account_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let r = (|| {
        let full = task.mem(cx, false, |t| read_text(&t.space(), a[1], 512, unicode)).map_err(|_| task.failed(cx, 998))?;
        let (want_domain, want_name) = full.rsplit_once('\\').map_or((None, full.as_str()), |(d, n)| (Some(d), n));
        let found = accounts().into_iter().find(|(_, d, n, _)| n.eq_ignore_ascii_case(want_name) && want_domain.is_none_or(|w| w.eq_ignore_ascii_case(d)));
        let Some((sid, domain, _, kind)) = found else { return Err(task.failed(cx, ERROR_NONE_MAPPED)) };
        let bytes = sid.to_bytes();
        let (s_room, d_room) = (task.get_u32(cx, a[3])? as usize, task.get_u32(cx, a[5])? as usize);
        if s_room < bytes.len() || d_room <= domain.len() || a[2] == 0 || a[4] == 0 {
            task.put_u32(cx, a[3], bytes.len() as u32)?;
            task.put_u32(cx, a[5], domain.len() as u32 + 1)?;
            return Err(task.failed(cx, ERROR_INSUFFICIENT_BUFFER));
        }
        task.put(cx, a[2], &bytes)?;
        task.put_u32(cx, a[3], bytes.len() as u32)?;
        put_name(task, cx, a[4], a[5], domain, unicode)?;
        task.put_u32(cx, a[6], kind)?;
        Ok(TRUE)
    })();
    reply(r)
}

// --- Security descriptors ---

/// `InitializeSecurityDescriptor(sd, revision)`: an absolute descriptor, empty.
pub fn initialize_security_descriptor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        if a[1] != 1 {
            return Err(task.failed(cx, 1305)); // ERROR_UNKNOWN_REVISION
        }
        // The header and four pointers: 40 bytes, or 20 for a 32-bit program.
        let mut b = vec![0u8; 5 * task.abi().width as usize];
        b[0] = 1;
        task.put(cx, a[0], &b)?;
        Ok(TRUE)
    })();
    reply(r)
}

pub fn is_valid_security_descriptor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let ok = task.read_descriptor(cx, a[0]).is_ok();
    if !ok {
        task.set_last_error(cx, ERROR_INVALID_SECURITY_DESCR);
    }
    Reply::Value(u64::from(ok))
}

/// `GetSecurityDescriptorLength(sd)`: its header and its parts.
pub fn get_security_descriptor_length(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Ok((control, _)) = task.descriptor_parts(cx, a[0]) else { return Reply::Value(0) };
    let Ok(d) = task.read_descriptor(cx, a[0]) else { return Reply::Value(0) };
    let head = if control & SE_SELF_RELATIVE != 0 { 20 } else { 40 };
    Reply::Value((d.to_relative().len() - 20 + head) as u64)
}

/// `GetSecurityDescriptorControl(sd, &control, &revision)`.
pub fn get_security_descriptor_control(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let (control, _) = task.descriptor_parts(cx, a[0])?;
        task.put(cx, a[1], &control.to_le_bytes())?;
        task.put_u32(cx, a[2], 1)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `SetSecurityDescriptorControl(sd, which, values)`: only the inheritance and protection bits may be set so.
pub fn set_security_descriptor_control(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let settable = SE_DACL_AUTO_INHERIT_REQ | SE_SACL_AUTO_INHERIT_REQ | SE_DACL_AUTO_INHERITED | SE_SACL_AUTO_INHERITED | SE_DACL_PROTECTED | SE_SACL_PROTECTED | 0x4000;
        let (which, values) = (a[1] as u16, a[2] as u16);
        if which & !settable != 0 {
            return Err(task.failed(cx, ERROR_INVALID_PARAMETER));
        }
        let (control, _) = task.descriptor_parts(cx, a[0])?;
        let control = control & !which | values & which;
        task.put(cx, a[0] + 2, &control.to_le_bytes())?;
        Ok(TRUE)
    })();
    reply(r)
}

/// An absolute descriptor's part `index` (owner 0, group 1, SACL 2, DACL 3) set to `pointer`, with control bits.
fn set_part(task: &mut Task, cx: &mut Cx<'_>, sd: u64, index: u64, pointer: u64, set: u16, clear: u16) -> R {
    let (control, _) = task.descriptor_parts(cx, sd)?;
    if control & SE_SELF_RELATIVE != 0 {
        return Err(task.failed(cx, ERROR_INVALID_SECURITY_DESCR));
    }
    let w = task.abi().width;
    task.put_ptr(cx, sd + w + w * index, pointer)?;
    task.put(cx, sd + 2, &(control & !clear | set).to_le_bytes())?;
    Ok(TRUE)
}

/// `SetSecurityDescriptorOwner(sd, sid, defaulted)` and `Group`.
pub fn set_security_descriptor_owner(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let flag = if a[2] != 0 { SE_OWNER_DEFAULTED } else { 0 };
    reply(set_part(task, cx, a[0], 0, a[1], flag, SE_OWNER_DEFAULTED))
}

pub fn set_security_descriptor_group(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let flag = if a[2] != 0 { SE_GROUP_DEFAULTED } else { 0 };
    reply(set_part(task, cx, a[0], 1, a[1], flag, SE_GROUP_DEFAULTED))
}

/// `SetSecurityDescriptorDacl(sd, present, dacl, defaulted)` and `Sacl`. Present with no ACL is a NULL DACL.
fn set_acl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], index: u64, present: u16, defaulted: u16) -> R {
    let set = if a[1] != 0 { present | if a[3] != 0 { defaulted } else { 0 } } else { 0 };
    set_part(task, cx, a[0], index, if a[1] != 0 { a[2] } else { 0 }, set, present | defaulted)
}

pub fn set_security_descriptor_dacl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(set_acl(task, cx, a, 3, SE_DACL_PRESENT, SE_DACL_DEFAULTED))
}

pub fn set_security_descriptor_sacl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(set_acl(task, cx, a, 2, SE_SACL_PRESENT, SE_SACL_DEFAULTED))
}

/// `GetSecurityDescriptorOwner(sd, &sid, &defaulted)` and `Group`: pointers into it, in either form.
fn get_sid_part(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], index: usize, defaulted: u16) -> R {
    let (control, parts) = task.descriptor_parts(cx, a[0])?;
    task.put_ptr(cx, a[1], parts[index])?;
    task.put_u32(cx, a[2], u32::from(control & defaulted != 0))?;
    Ok(TRUE)
}

pub fn get_security_descriptor_owner(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(get_sid_part(task, cx, a, 0, SE_OWNER_DEFAULTED))
}

pub fn get_security_descriptor_group(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(get_sid_part(task, cx, a, 1, SE_GROUP_DEFAULTED))
}

/// `GetSecurityDescriptorDacl(sd, &present, &dacl, &defaulted)` and `Sacl`.
fn get_acl_part(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], index: usize, present: u16, defaulted: u16) -> R {
    let (control, parts) = task.descriptor_parts(cx, a[0])?;
    let is = control & present != 0;
    task.put_u32(cx, a[1], u32::from(is))?;
    if is {
        task.put_ptr(cx, a[2], parts[index])?;
        task.put_u32(cx, a[3], u32::from(control & defaulted != 0))?;
    }
    Ok(TRUE)
}

pub fn get_security_descriptor_dacl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(get_acl_part(task, cx, a, 3, SE_DACL_PRESENT, SE_DACL_DEFAULTED))
}

pub fn get_security_descriptor_sacl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(get_acl_part(task, cx, a, 2, SE_SACL_PRESENT, SE_SACL_DEFAULTED))
}

/// `MakeSelfRelativeSD(absolute, relative, &length)`.
pub fn make_self_relative_sd(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let (control, _) = task.descriptor_parts(cx, a[0])?;
        if control & SE_SELF_RELATIVE != 0 {
            return Err(task.failed(cx, ERROR_BAD_DESCRIPTOR_FORMAT));
        }
        let bytes = task.read_descriptor(cx, a[0])?.to_relative();
        let room = task.get_u32(cx, a[2])? as usize;
        task.put_u32(cx, a[2], bytes.len() as u32)?;
        if room < bytes.len() || a[1] == 0 {
            return Err(task.failed(cx, ERROR_INSUFFICIENT_BUFFER));
        }
        task.put(cx, a[1], &bytes)?;
        Ok(TRUE)
    })();
    reply(r)
}

// --- ACLs ---

/// `InitializeAcl(acl, length, revision)`: an empty ACL of that size.
pub fn initialize_acl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let (size, revision) = (a[1] as u32 as usize, a[2] as u8);
        if !(8..=0xffff).contains(&size) || !(2..=4).contains(&revision) {
            return Err(task.failed(cx, ERROR_INVALID_PARAMETER));
        }
        let acl = Acl { revision, aces: Vec::new(), size: size & !3 };
        task.put(cx, a[0], &acl.to_bytes()[..8])?;
        Ok(TRUE)
    })();
    reply(r)
}

pub fn is_valid_acl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.read_acl(cx, a[0]).is_ok()))
}

/// Changes the ACL at `at` as `change` does, if what it then holds fits in its size.
fn change_acl(task: &mut Task, cx: &mut Cx<'_>, at: u64, revision: u8, change: impl FnOnce(&mut Acl) -> Result<(), u32>) -> R {
    let mut acl = task.read_acl(cx, at)?;
    if let Err(e) = change(&mut acl) {
        return Err(task.failed(cx, e));
    }
    acl.revision = acl.revision.max(revision);
    if acl.used() > acl.size {
        return Err(task.failed(cx, ERROR_ALLOTTED_SPACE_EXCEEDED));
    }
    task.put(cx, at, &acl.to_bytes())?;
    Ok(TRUE)
}

/// Appends an allowed, denied or label ACE: `(acl, revision, [flags,] mask, sid)`.
fn add_ace(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], kind: u8, with_flags: bool) -> R {
    let (flags, mask, sid_at) = if with_flags { (a[2] as u8, a[3] as u32, a[4]) } else { (0, a[2] as u32, a[3]) };
    let sid = task.read_sid(cx, sid_at)?;
    change_acl(task, cx, a[0], a[1] as u8, |acl| {
        acl.aces.push(Ace::new(kind, flags, mask, sid));
        Ok(())
    })
}

pub fn add_access_allowed_ace(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(add_ace(task, cx, a, ACCESS_ALLOWED, false))
}

pub fn add_access_allowed_ace_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(add_ace(task, cx, a, ACCESS_ALLOWED, true))
}

pub fn add_access_denied_ace(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(add_ace(task, cx, a, ACCESS_DENIED, false))
}

pub fn add_access_denied_ace_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(add_ace(task, cx, a, ACCESS_DENIED, true))
}

/// `AddMandatoryAce(acl, revision, flags, policy, label)`.
pub fn add_mandatory_ace(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(add_ace(task, cx, a, SYSTEM_MANDATORY_LABEL, true))
}

/// `AddAce(acl, revision, index, aces, length)`: ACEs in their binary form, at an index (`MAXDWORD` the end).
pub fn add_ace_list(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let bytes = task.get(cx, a[3], (a[4] as u32 as usize).min(0xffff))?;
        let mut new = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let Some((ace, len)) = Ace::from_bytes(&bytes[at..]) else { return Err(task.failed(cx, ERROR_INVALID_PARAMETER)) };
            new.push(ace);
            at += len;
        }
        let index = a[2] as u32;
        change_acl(task, cx, a[0], a[1] as u8, |acl| {
            let i = if index == u32::MAX { acl.aces.len() } else { (index as usize).min(acl.aces.len()) };
            acl.aces.splice(i..i, new);
            Ok(())
        })
    })();
    reply(r)
}

/// `DeleteAce(acl, index)`.
pub fn delete_ace(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let index = a[1] as u32 as usize;
    reply(change_acl(task, cx, a[0], 0, |acl| {
        if index >= acl.aces.len() {
            return Err(ERROR_INVALID_PARAMETER);
        }
        acl.aces.remove(index);
        Ok(())
    }))
}

/// `GetAce(acl, index, &ace)`: where the ACE is, in the ACL.
pub fn get_ace(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let acl = task.read_acl(cx, a[0])?;
        let index = a[1] as u32 as usize;
        if index >= acl.aces.len() {
            return Err(task.failed(cx, ERROR_INVALID_PARAMETER));
        }
        let offset = 8 + acl.aces[..index].iter().map(|ace| ace.to_bytes().len()).sum::<usize>();
        task.put_ptr(cx, a[2], a[0] + offset as u64)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `GetAclInformation(acl, info, length, class)`: its revision (1), or its ACE count and bytes used and free (2).
pub fn get_acl_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let acl = task.read_acl(cx, a[0])?;
        let data: Vec<u8> = match a[3] as u32 {
            1 => u32::from(acl.revision).to_le_bytes().to_vec(),
            2 => [acl.aces.len() as u32, acl.used() as u32, (acl.size - acl.used()) as u32].iter().flat_map(|v| v.to_le_bytes()).collect(),
            _ => return Err(task.failed(cx, ERROR_INVALID_PARAMETER)),
        };
        if (a[2] as u32 as usize) < data.len() {
            return Err(task.failed(cx, ERROR_INSUFFICIENT_BUFFER));
        }
        task.put(cx, a[1], &data)?;
        Ok(TRUE)
    })();
    reply(r)
}

// --- SDDL ---

/// `ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, revision, &sd, &size)`: a self-relative descriptor,
/// in `LocalAlloc` memory.
pub fn convert_string_security_descriptor_to_security_descriptor_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_string_security_descriptor_to_security_descriptor(task, cx, a, true)
}

pub fn convert_string_security_descriptor_to_security_descriptor_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_string_security_descriptor_to_security_descriptor(task, cx, a, false)
}

fn convert_string_security_descriptor_to_security_descriptor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let r = (|| {
        if a[1] != 1 {
            return Err(task.failed(cx, 1305));
        }
        let s = task.mem(cx, false, |t| read_text(&t.space(), a[0], 65536, unicode)).map_err(|_| task.failed(cx, 998))?;
        let d = match Descriptor::from_sddl(&s) {
            Ok(d) => d,
            Err(e) => return Err(task.failed(cx, e)),
        };
        let bytes = d.to_relative();
        if a[3] != 0 {
            task.put_u32(cx, a[3], bytes.len() as u32)?;
        }
        task.put_allocated(cx, a[2], &bytes)
    })();
    reply(r)
}

/// `ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, revision, which, &sddl, &length)`: the parts asked
/// for, in `LocalAlloc` memory; the length counts the NUL.
pub fn convert_security_descriptor_to_string_security_descriptor_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_security_descriptor_to_string_security_descriptor(task, cx, a, true)
}

pub fn convert_security_descriptor_to_string_security_descriptor_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_security_descriptor_to_string_security_descriptor(task, cx, a, false)
}

fn convert_security_descriptor_to_string_security_descriptor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let r = (|| {
        if a[1] != 1 {
            return Err(task.failed(cx, 1305));
        }
        let d = task.read_descriptor(cx, a[0])?;
        let s = d.to_sddl(a[2] as u32);
        if a[4] != 0 {
            task.put_u32(cx, a[4], (text(&s, unicode).len() / if unicode { 2 } else { 1 }) as u32)?;
        }
        task.put_allocated(cx, a[3], &text(&s, unicode))
    })();
    reply(r)
}

// --- Descriptors kept on objects, files and keys ---

const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_BAD_TOKEN_TYPE: u32 = 1349;
const ERROR_GENERIC_NOT_MAPPED: u32 = 1360;
const ERROR_CANNOT_IMPERSONATE: u32 = 1368;

/// What a descriptor is kept on.
enum Target {
    Object(Arc<Object>),
    Section(Arc<Section>),
    /// A file or folder, by its full Windows path in lower case.
    File(String),
    Key(u64),
    Token(Arc<Token>),
    /// A window station (true) or desktop, by its handle.
    User(u64, bool),
}

/// How a kernel object's generic rights map, by its kind.
pub(crate) fn kind_mapping(kind: &Kind) -> Mapping {
    match kind {
        Kind::Event { .. } => EVENT,
        Kind::Mutant => MUTANT,
        Kind::Semaphore { .. } => SEMAPHORE,
        Kind::Timer { .. } => TIMER,
        Kind::Process(_) => PROCESS,
        Kind::Thread(_) => THREAD,
    }
}

impl Target {
    fn mapping(&self) -> Mapping {
        match self {
            Target::Object(o) => kind_mapping(&o.kind),
            Target::Section(_) => SECTION,
            Target::File(_) => FILE,
            Target::Key(_) => KEY,
            Target::Token(_) => TOKEN,
            Target::User(_, true) => crate::security::WINDOW_STATION,
            Target::User(_, false) => crate::security::DESKTOP,
        }
    }
}

impl Task {
    /// The token this thread acts as: the one it's impersonating, else its process's.
    pub(crate) fn effective_token(&self) -> Arc<Token> {
        let impersonating = lock(&self.control.token).clone();
        impersonating.unwrap_or_else(|| lock(&self.process.state).token.clone())
    }

    /// The descriptor in a SECURITY_ATTRIBUTES, if it has one.
    pub(crate) fn attributes_descriptor(&mut self, cx: &mut Cx<'_>, attributes: u64) -> Option<Descriptor> {
        if attributes == 0 {
            return None;
        }
        // SECURITY_ATTRIBUTES: nLength, then the pointer, aligned to its size.
        let at = self.get_ptr(cx, attributes + self.abi().width).ok().filter(|&p| p != 0)?;
        self.read_descriptor(cx, at).ok()
    }

    /// Whether this thread may have `access` to what a kept descriptor guards (anything, with none kept).
    pub(crate) fn allows(&self, kept: &std::sync::Mutex<Option<Descriptor>>, access: u32, mapping: &Mapping) -> bool {
        let Some(d) = lock(kept).clone() else { return true };
        access_check(&d, &lock(&self.effective_token().state), access, mapping).is_ok()
    }

    pub(crate) fn key_allows(&self, key: u64, access: u32) -> bool {
        let Some(d) = lock(&self.k.key_security).get(&key).cloned() else { return true };
        access_check(&d, &lock(&self.effective_token().state), access, &KEY).is_ok()
    }

    /// A path's key in the files' descriptors: its full Windows path, in lower case.
    pub(crate) fn security_path(&self, name: &str) -> Option<String> {
        crate::files::resolve(&self.cwd(), name).ok().map(|(full, _)| full.to_ascii_lowercase())
    }

    /// Forgets what was kept for a path that's gone.
    pub(crate) fn forget_security(&self, path: Option<&str>, done: Reply) {
        if done == Reply::Value(TRUE) && let Some(p) = path.and_then(|p| self.security_path(p)) {
            lock(&self.k.file_security).remove(&p);
        }
    }

    fn target_of(&mut self, cx: &mut Cx<'_>, handle: u64) -> Result<Target, u32> {
        if handle == CURRENT_PROCESS {
            return Ok(Target::Object(self.process.object.clone()));
        }
        if handle == CURRENT_THREAD {
            return Ok(Target::Object(self.control.object.clone()));
        }
        if let Some(root) = crate::registry::Root::from_handle(handle) {
            return Ok(Target::Key(lock(&self.k.registry).root(root)));
        }
        let user = self.win32k().stations.get(handle).map(|o| o.is_station());
        if let Some(station) = user {
            return Ok(Target::User(handle, station));
        }
        let found = lock(&self.process.state).handles.get(handle);
        Ok(match found {
            Some(Handle::Object(o)) => Target::Object(o),
            Some(Handle::Section(s, _)) => Target::Section(s),
            Some(Handle::File(f)) => Target::File(f.path.to_ascii_lowercase()),
            Some(Handle::Key(id)) => Target::Key(id),
            Some(Handle::Token(t)) => Target::Token(t),
            _ => {
                let _ = cx;
                return Err(ERROR_INVALID_HANDLE);
            }
        })
    }

    /// What's guarding a target: what was kept, or the default for its kind.
    fn descriptor_of(&self, target: &Target) -> Descriptor {
        let kept = match target {
            Target::Object(o) => lock(&o.security).clone(),
            Target::Section(s) => lock(&s.security).clone(),
            Target::Token(t) => lock(&t.security).clone(),
            Target::File(p) => lock(&self.k.file_security).get(p).cloned(),
            Target::Key(id) => lock(&self.k.key_security).get(id).cloned(),
            Target::User(handle, _) => lock(&self.k.user_security).get(handle).cloned(),
        };
        kept.unwrap_or_else(|| match target {
            Target::File(_) => Descriptor::from_sddl(FILE_DEFAULT).expect("the files' default"),
            Target::Key(_) => Descriptor::from_sddl(KEY_DEFAULT).expect("the keys' default"),
            _ => Descriptor::defaults(&lock(&self.effective_token().state)),
        })
    }

    fn keep(&self, target: &Target, d: Descriptor) {
        match target {
            Target::Object(o) => *lock(&o.security) = Some(d),
            Target::Section(s) => *lock(&s.security) = Some(d),
            Target::Token(t) => *lock(&t.security) = Some(d),
            Target::File(p) => {
                lock(&self.k.file_security).insert(p.clone(), d);
            }
            Target::Key(id) => {
                lock(&self.k.key_security).insert(*id, d);
            }
            Target::User(handle, _) => {
                lock(&self.k.user_security).insert(*handle, d);
            }
        }
    }

    /// A target's descriptor, the parts `info` asks for, self-relative. The SACL needs SeSecurityPrivilege.
    fn read_security(&self, target: &Target, info: u32) -> Result<Vec<u8>, u32> {
        let token = lock(&self.effective_token().state).clone();
        if info & SACL_INFO != 0 && !token.privileges.iter().any(|p| p.0 == 8 && p.1 & SE_PRIVILEGE_ENABLED != 0) {
            return Err(ERROR_PRIVILEGE_NOT_HELD);
        }
        Ok(self.descriptor_of(target).only(info).to_relative())
    }

    /// Sets the parts `info` names, if this thread may: `WRITE_OWNER` for the owner and group, `WRITE_DAC` for the
    /// DACL and label, `ACCESS_SYSTEM_SECURITY` for the SACL. An owner must be one the token may give.
    fn write_security(&self, target: &Target, info: u32, from: &Descriptor) -> Result<(), u32> {
        let token = lock(&self.effective_token().state).clone();
        let mut needed = 0;
        if info & (OWNER_INFO | GROUP_INFO) != 0 {
            needed |= WRITE_OWNER;
        }
        if info & (DACL_INFO | LABEL_INFO | PROTECTED_DACL | UNPROTECTED_DACL) != 0 {
            needed |= WRITE_DAC;
        }
        if info & (SACL_INFO | PROTECTED_SACL | UNPROTECTED_SACL) != 0 {
            needed |= ACCESS_SYSTEM_SECURITY;
        }
        let mut d = self.descriptor_of(target);
        if access_check(&d, &token, needed, &target.mapping()).is_err() {
            return Err(if needed & ACCESS_SYSTEM_SECURITY != 0 { ERROR_PRIVILEGE_NOT_HELD } else { ERROR_ACCESS_DENIED });
        }
        if info & OWNER_INFO != 0 && let Some(owner) = &from.owner {
            let restore = token.privileges.iter().any(|p| p.0 == 18 && p.1 & SE_PRIVILEGE_ENABLED != 0);
            let may = *owner == token.user || token.groups.iter().any(|(g, at)| g == owner && at & SE_GROUP_OWNER != 0);
            if !may && !restore {
                return Err(ERROR_INVALID_OWNER);
            }
        }
        d.set(info, from);
        self.keep(target, d.mapped(&target.mapping()));
        Ok(())
    }

    /// A descriptor from the parts `SetSecurityInfo` is given: pointers to an owner, group, DACL and SACL.
    fn parts_descriptor(&mut self, cx: &mut Cx<'_>, info: u32, parts: [u64; 4]) -> Result<Descriptor, u64> {
        let mut d = Descriptor::default();
        if info & OWNER_INFO != 0 && parts[0] != 0 {
            d.owner = Some(self.read_sid(cx, parts[0])?);
        }
        if info & GROUP_INFO != 0 && parts[1] != 0 {
            d.group = Some(self.read_sid(cx, parts[1])?);
        }
        if info & DACL_INFO != 0 {
            d.control |= SE_DACL_PRESENT;
            if parts[2] != 0 {
                d.dacl = Some(self.read_acl(cx, parts[2])?);
            }
        }
        if info & (SACL_INFO | LABEL_INFO) != 0 && parts[3] != 0 {
            d.control |= SE_SACL_PRESENT;
            d.sacl = Some(self.read_acl(cx, parts[3])?);
        }
        Ok(d)
    }

    /// `GetSecurityInfo`'s and `GetNamedSecurityInfo`'s answer: a `LocalAlloc` block, and pointers into it.
    fn give_security(&mut self, cx: &mut Cx<'_>, target: &Target, info: u32, out: [u64; 5]) -> u32 {
        let bytes = match self.read_security(target, info) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let block = global_new(self, cx, &bytes);
        if block == 0 {
            return 8;
        }
        // Each part's offset in the block's header: owner, group, SACL, DACL.
        let offset = |at: usize| u64::from(u32_at(&bytes, at));
        let parts = [(out[0], offset(4)), (out[1], offset(8)), (out[2], offset(16)), (out[3], offset(12))];
        for (to, off) in parts {
            if to != 0 && self.put_ptr(cx, to, if off == 0 { 0 } else { block + off }).is_err() {
                return 998;
            }
        }
        if out[4] != 0 {
            if self.put_ptr(cx, out[4], block).is_err() {
                return 998;
            }
        } else {
            global_release(self, cx, block);
        }
        0
    }

    /// What `GetNamedSecurityInfo` and its kin name: a file or folder, a registry key (`MACHINE\…`,
    /// `CURRENT_USER\…`, `USERS\…`, `CLASSES_ROOT\…`), or a named kernel object.
    fn named_target(&mut self, cx: &mut Cx<'_>, name: &str, kind: u32) -> Result<Target, u32> {
        match kind {
            1 => {
                // FILE_FLAG_BACKUP_SEMANTICS opens folders too.
                let (file, _) = self.open_file(name, 0, crate::files::FILE_SHARE_ALL, 3, 0x0200_0000)?;
                Ok(Target::File(file.path.to_ascii_lowercase()))
            }
            4 => {
                let (root, rest) = name.split_once('\\').unwrap_or((name, ""));
                let root = match root.to_ascii_uppercase().as_str() {
                    "MACHINE" => crate::registry::Root::LocalMachine,
                    "CURRENT_USER" => crate::registry::Root::CurrentUser,
                    "USERS" => crate::registry::Root::Users,
                    "CLASSES_ROOT" => crate::registry::Root::ClassesRoot,
                    "CURRENT_CONFIG" => crate::registry::Root::CurrentConfig,
                    _ => return Err(ERROR_FILE_NOT_FOUND),
                };
                let mut registry = lock(&self.k.registry);
                let base = registry.root(root);
                registry.open(base, rest).map(Target::Key)
            }
            6 => {
                let _ = cx;
                if let Some(o) = self.k.objects.named(name) {
                    return Ok(Target::Object(o));
                }
                let section = lock(&self.k.sections).get(name).and_then(std::sync::Weak::upgrade);
                section.map(Target::Section).ok_or(ERROR_FILE_NOT_FOUND)
            }
            _ => Err(ERROR_INVALID_PARAMETER),
        }
    }

    fn read_name(&mut self, cx: &mut Cx<'_>, at: u64, unicode: bool) -> Result<String, u32> {
        self.mem(cx, false, |t| read_text(&t.space(), at, 32768, unicode)).map_err(|_| 998)
    }
}

/// `GetSecurityInfo(handle, type, info, &owner, &group, &dacl, &sacl, &sd)`: an error code.
pub fn get_security_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let code = match task.target_of(cx, a[0]) {
        Ok(target) => task.give_security(cx, &target, a[2] as u32, [a[3], a[4], a[5], a[6], a[7]]),
        Err(e) => e,
    };
    Reply::Value(u64::from(code))
}

/// `SetSecurityInfo(handle, type, info, owner, group, dacl, sacl)`: an error code.
pub fn set_security_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let info = a[2] as u32;
    let code = (|| {
        let target = task.target_of(cx, a[0])?;
        let from = task.parts_descriptor(cx, info, [a[3], a[4], a[5], a[6]]).map_err(|_| ERROR_INVALID_PARAMETER)?;
        task.write_security(&target, info, &from)
    })();
    Reply::Value(u64::from(code.err().unwrap_or(0)))
}

/// `GetNamedSecurityInfoW(name, type, info, &owner, &group, &dacl, &sacl, &sd)`.
pub fn get_named_security_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_named_security_info(task, cx, a, true)
}

pub fn get_named_security_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_named_security_info(task, cx, a, false)
}

fn get_named_security_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let code = (|| {
        let name = task.read_name(cx, a[0], unicode)?;
        let target = task.named_target(cx, &name, a[1] as u32)?;
        match task.give_security(cx, &target, a[2] as u32, [a[3], a[4], a[5], a[6], a[7]]) {
            0 => Ok(()),
            e => Err(e),
        }
    })();
    Reply::Value(u64::from(code.err().unwrap_or(0)))
}

/// `SetNamedSecurityInfoW(name, type, info, owner, group, dacl, sacl)`.
pub fn set_named_security_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_named_security_info(task, cx, a, true)
}

pub fn set_named_security_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_named_security_info(task, cx, a, false)
}

fn set_named_security_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let info = a[2] as u32;
    let code = (|| {
        let name = task.read_name(cx, a[0], unicode)?;
        let target = task.named_target(cx, &name, a[1] as u32)?;
        let from = task.parts_descriptor(cx, info, [a[3], a[4], a[5], a[6]]).map_err(|_| ERROR_INVALID_PARAMETER)?;
        task.write_security(&target, info, &from)
    })();
    Reply::Value(u64::from(code.err().unwrap_or(0)))
}

/// Gives a target's descriptor into the program's buffer, as the `Get*Security` calls do: self-relative, with the
/// size it needs at `needed`. A BOOL, or (`status`) an error code.
fn give_into(task: &mut Task, cx: &mut Cx<'_>, target: Result<Target, u32>, info: u32, buffer: u64, length: u64, needed: u64) -> Result<(), u32> {
    (|| {
        let bytes = task.read_security(&target?, info)?;
        if needed != 0 {
            task.put_u32(cx, needed, bytes.len() as u32).map_err(|_| 998u32)?;
        }
        if (length as u32 as usize) < bytes.len() || buffer == 0 {
            return Err(ERROR_INSUFFICIENT_BUFFER);
        }
        task.put(cx, buffer, &bytes).map_err(|_| 998u32)
    })()
}

/// A result as the `Get*`/`Set*Security` calls give it: a BOOL with the last error, or (`status`) an error code.
fn finish(task: &mut Task, cx: &mut Cx<'_>, r: Result<(), u32>, status: bool) -> Reply {
    match (r, status) {
        (Ok(()), true) => Reply::Value(0),
        (Err(e), true) => Reply::Value(u64::from(e)),
        (Ok(()), false) => Reply::Value(TRUE),
        (Err(e), false) => Reply::Value(task.failed(cx, e)),
    }
}

/// Sets a target's parts from a descriptor in the program's memory: the `Set*Security` calls.
fn take_from(task: &mut Task, cx: &mut Cx<'_>, target: Result<Target, u32>, info: u32, sd: u64) -> Result<(), u32> {
    (|| {
        let target = target?;
        let from = task.read_descriptor(cx, sd).map_err(|_| ERROR_INVALID_SECURITY_DESCR)?;
        // A descriptor's DACL that's present but none is a NULL DACL.
        task.write_security(&target, info, &from)
    })()
}

/// `GetKernelObjectSecurity(handle, info, sd, length, &needed)` and `SetKernelObjectSecurity(handle, info, sd)`.
pub fn get_kernel_object_security(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let target = task.target_of(cx, a[0]);
    let r = give_into(task, cx, target, a[1] as u32, a[2], a[3], a[4]);
    finish(task, cx, r, false)
}

pub fn set_kernel_object_security(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let target = task.target_of(cx, a[0]);
    let r = take_from(task, cx, target, a[1] as u32, a[2]);
    finish(task, cx, r, false)
}

/// `GetFileSecurityW(name, info, sd, length, &needed)` and `SetFileSecurityW(name, info, sd)`.
pub fn get_file_security_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_file_security(task, cx, a, true)
}

pub fn get_file_security_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_file_security(task, cx, a, false)
}

fn get_file_security(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let target = task.read_name(cx, a[0], unicode).and_then(|n| task.named_target(cx, &n, 1));
    let r = give_into(task, cx, target, a[1] as u32, a[2], a[3], a[4]);
    finish(task, cx, r, false)
}

pub fn set_file_security_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_file_security(task, cx, a, true)
}

pub fn set_file_security_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_file_security(task, cx, a, false)
}

fn set_file_security(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let target = task.read_name(cx, a[0], unicode).and_then(|n| task.named_target(cx, &n, 1));
    let r = take_from(task, cx, target, a[1] as u32, a[2]);
    finish(task, cx, r, false)
}

/// `RegGetKeySecurity(key, info, sd, &length)` and `RegSetKeySecurity(key, info, sd)`: error codes.
pub fn reg_get_key_security(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let target = task.key_of(a[0]).map(Target::Key);
    let room = u64::from(task.get_u32(cx, a[3]).unwrap_or(0));
    let r = give_into(task, cx, target, a[1] as u32, a[2], room, a[3]);
    finish(task, cx, r, true)
}

pub fn reg_set_key_security(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let target = task.key_of(a[0]).map(Target::Key);
    if let Ok(Target::Key(id)) = &target {
        lock(&task.k.registry).changed(*id, crate::registry::CHANGE_SECURITY);
    }
    let r = take_from(task, cx, target, a[1] as u32, a[2]);
    finish(task, cx, r, true)
}

fn read_mapping(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Result<Mapping, u64> {
    let b = task.get(cx, at, 16)?;
    Ok(Mapping { read: u32_at(&b, 0), write: u32_at(&b, 4), execute: u32_at(&b, 8), all: u32_at(&b, 12) })
}

/// `MapGenericMask(&mask, &mapping)`.
pub fn map_generic_mask(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let (Ok(mask), Ok(m)) = (task.get_u32(cx, a[0]), read_mapping(task, cx, a[1])) {
        let _ = task.put_u32(cx, a[0], map_generic(mask, &m));
    }
    Reply::Value(0)
}

/// `AccessCheck(sd, token, desired, &mapping, privileges, &privileges length, &granted, &status)`: TRUE if it could
/// check; `status` says whether access is allowed (else ERROR_ACCESS_DENIED).
pub fn access_check_call(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let token = task.token_of(cx, a[1])?;
        let state = lock(&token.state).clone();
        if state.kind != 2 {
            return Err(task.failed(cx, ERROR_NO_IMPERSONATION_TOKEN));
        }
        let desired = a[2] as u32;
        if desired & (GENERIC_READ | GENERIC_WRITE | GENERIC_EXECUTE | GENERIC_ALL) != 0 {
            return Err(task.failed(cx, ERROR_GENERIC_NOT_MAPPED));
        }
        let d = task.read_descriptor(cx, a[0])?;
        if d.owner.is_none() || d.group.is_none() {
            return Err(task.failed(cx, ERROR_INVALID_SECURITY_DESCR));
        }
        let m = read_mapping(task, cx, a[3])?;
        let room = task.get_u32(cx, a[5])?;
        if room < 20 || a[4] == 0 {
            task.put_u32(cx, a[5], 20)?;
            return Err(task.failed(cx, ERROR_INSUFFICIENT_BUFFER));
        }
        task.put(cx, a[4], &[0; 20])?;
        match access_check(&d, &state, desired, &m) {
            Ok(granted) => {
                task.put_u32(cx, a[6], granted)?;
                task.put_u32(cx, a[7], 1)?;
            }
            Err(()) => {
                task.put_u32(cx, a[6], 0)?;
                task.put_u32(cx, a[7], 0)?;
                task.set_last_error(cx, ERROR_ACCESS_DENIED);
            }
        }
        Ok(TRUE)
    })();
    reply(r)
}

// --- Impersonation ---

/// `ImpersonateSelf(level)`: this thread acts as an impersonation copy of its process's token.
pub fn impersonate_self(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let level = a[0] as u32;
    if level > 3 {
        return Reply::Value(task.failed(cx, ERROR_INVALID_PARAMETER));
    }
    let token = lock(&task.process.state).token.clone();
    *lock(&task.control.token) = Some(Arc::new(token.duplicate(2, level)));
    Reply::Value(TRUE)
}

/// `RevertToSelf()`: back to the process's token.
pub fn revert_to_self(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    *lock(&task.control.token) = None;
    Reply::Value(TRUE)
}

/// `SetThreadToken(&thread or NULL, token or NULL)`: a thread acts as an impersonation token, or stops.
pub fn set_thread_token(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let thread = if a[0] == 0 { Some(task.control.clone()) } else { task.get_ptr(cx, a[0]).ok().and_then(|h| task.thread_of(h)) };
        let Some(thread) = thread else { return Err(task.failed(cx, ERROR_INVALID_HANDLE)) };
        let token = if a[1] == 0 { None } else { Some(task.token_of(cx, a[1])?) };
        if token.as_ref().is_some_and(|t| lock(&t.state).kind != 2) {
            return Err(task.failed(cx, ERROR_BAD_TOKEN_TYPE));
        }
        *lock(&thread.token) = token;
        Ok(TRUE)
    })();
    reply(r)
}

/// `ImpersonateLoggedOnUser(token)`: an impersonation token as it is, or a primary one's impersonation copy.
pub fn impersonate_logged_on_user(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let token = task.token_of(cx, a[0])?;
        let token = if lock(&token.state).kind == 2 { token } else { Arc::new(token.duplicate(2, 2)) };
        *lock(&task.control.token) = Some(token);
        Ok(TRUE)
    })();
    reply(r)
}

/// `ImpersonateNamedPipeClient(pipe)`: this thread acts as the client's process's token.
pub fn impersonate_named_pipe_client(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let Some(file) = task.pipe_end(a[0]) else { return Err(task.failed(cx, ERROR_INVALID_HANDLE)) };
        let crate::files::Node::NamedPipe(end) = &file.node else { return Err(task.failed(cx, ERROR_INVALID_HANDLE)) };
        let pid = lock(&end.instance.state).client_pid;
        let Some(client) = task.k.process(pid) else { return Err(task.failed(cx, ERROR_CANNOT_IMPERSONATE)) };
        let token = lock(&client.state).token.clone();
        *lock(&task.control.token) = Some(Arc::new(token.duplicate(2, 2)));
        Ok(TRUE)
    })();
    reply(r)
}

// --- Trustees, and ACLs built from EXPLICIT_ACCESS entries ---

const ERROR_INVALID_PARAMETER: u32 = 87;
/// TRUSTEE_IS_SID and TRUSTEE_IS_NAME: what a TRUSTEE's name field is.
const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_NAME: u32 = 1;
/// ACCESS_MODE: what an EXPLICIT_ACCESS entry does to the ACL.
const GRANT_ACCESS: u32 = 1;
const SET_ACCESS: u32 = 2;
const DENY_ACCESS: u32 = 3;
const REVOKE_ACCESS: u32 = 4;
/// The ACE flags an entry's inheritance can have: OBJECT_INHERIT_ACE to INHERIT_ONLY_ACE.
const INHERITANCE: u32 = 0xf;
const INHERITED_ACE: u8 = 0x10;

impl Task {
    /// A TRUSTEE's size, and where its form and its name are: { multiple, operation, form, type, name }, the name
    /// pointer-aligned.
    fn trustee_layout(&self) -> (usize, usize, usize) {
        if self.abi().width == 8 { (32, 12, 24) } else { (20, 8, 16) }
    }

    /// Fills a TRUSTEE: no multiple trustee, its form, TRUSTEE_IS_UNKNOWN, and `name` (a SID or a string).
    fn put_trustee(&mut self, cx: &mut Cx<'_>, at: u64, form: u32, name: u64) -> Result<(), u64> {
        let (size, form_at, name_at) = self.trustee_layout();
        let mut b = vec![0u8; size];
        b[form_at..form_at + 4].copy_from_slice(&form.to_le_bytes());
        let ptr = self.abi().ptr(name);
        b[name_at..].copy_from_slice(&ptr);
        self.put(cx, at, &b)
    }

    /// The SID a TRUSTEE names: by its SID, or by an account's name (`CURRENT_USER` is the caller's user).
    fn trustee_sid(&mut self, cx: &mut Cx<'_>, at: u64, unicode: bool) -> Result<Sid, u32> {
        let (size, form_at, name_at) = self.trustee_layout();
        let b = self.get(cx, at, size).map_err(|_| ERROR_INVALID_PARAMETER)?;
        let form = u32_at(&b, form_at);
        let name = self.abi().ptr_at(&b, name_at);
        let multiple = self.abi().ptr_at(&b, 0);
        if multiple != 0 || name == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        match form {
            TRUSTEE_IS_SID => self.read_sid(cx, name).map_err(|_| ERROR_INVALID_PARAMETER),
            TRUSTEE_IS_NAME => {
                let text = self.mem(cx, false, |t| read_text(&t.space(), name, 512, unicode)).map_err(|_| ERROR_INVALID_PARAMETER)?;
                if text.eq_ignore_ascii_case("CURRENT_USER") {
                    return Ok(lock(&self.effective_token().state).user.clone());
                }
                let (domain, account) = text.rsplit_once('\\').map_or((None, text.as_str()), |(d, n)| (Some(d), n));
                accounts()
                    .into_iter()
                    .find(|(_, d, n, _)| n.eq_ignore_ascii_case(account) && domain.is_none_or(|w| w.eq_ignore_ascii_case(d)))
                    .map(|(sid, ..)| sid)
                    .ok_or(ERROR_NONE_MAPPED)
            }
            _ => Err(ERROR_INVALID_PARAMETER),
        }
    }
}

/// `BuildTrusteeWithSidW(&trustee, sid)` and `A`.
pub fn build_trustee_with_sid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = task.put_trustee(cx, a[0], TRUSTEE_IS_SID, a[1]);
    Reply::Value(0)
}

/// `BuildTrusteeWithNameW(&trustee, name)` and `A`.
pub fn build_trustee_with_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = task.put_trustee(cx, a[0], TRUSTEE_IS_NAME, a[1]);
    Reply::Value(0)
}

/// `BuildExplicitAccessWithNameW(&entry, name, permissions, mode, inheritance)` and `A`: an EXPLICIT_ACCESS whose
/// trustee is named.
fn build_explicit_access_with_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let trustee_at = if task.abi().width == 8 { 16 } else { 12 };
    let mut head = Vec::with_capacity(12);
    for v in [a[2] as u32, a[3] as u32, a[4] as u32] {
        head.extend(v.to_le_bytes());
    }
    if task.put(cx, a[0], &head).is_ok() {
        let _ = task.put_trustee(cx, a[0] + trustee_at, TRUSTEE_IS_NAME, a[1]);
    }
    Reply::Value(0)
}

pub fn build_explicit_access_with_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    build_explicit_access_with_name(task, cx, a)
}

pub fn build_explicit_access_with_name_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    build_explicit_access_with_name(task, cx, a)
}

/// The ACL `entries` make of `old`'s ACEs: GRANT_ACCESS adds its rights to the trustee's explicit allowed ACE (or adds
/// one), SET_ACCESS replaces the trustee's explicit ACEs with one allowing just those rights, DENY_ACCESS adds a
/// denied ACE, REVOKE_ACCESS takes the trustee's explicit ACEs away. The result is in canonical order: explicit denied
/// ACEs, explicit allowed ones, then inherited ones as they were.
pub fn apply_entries(old: &[Ace], entries: &[(u32, u32, u32, Sid)]) -> Result<Vec<Ace>, u32> {
    let mut aces = old.to_vec();
    for (mask, mode, inheritance, sid) in entries {
        let flags = (*inheritance & INHERITANCE) as u8;
        let explicit_for = |a: &Ace| a.raw.is_none() && a.flags & INHERITED_ACE == 0 && a.sid == *sid;
        match *mode {
            0 => {}
            GRANT_ACCESS => {
                match aces.iter_mut().find(|a| explicit_for(a) && a.kind == ACCESS_ALLOWED && a.flags == flags) {
                    Some(ace) => ace.mask |= mask,
                    None => aces.push(Ace::new(ACCESS_ALLOWED, flags, *mask, sid.clone())),
                }
            }
            SET_ACCESS => {
                aces.retain(|a| !(explicit_for(a) && matches!(a.kind, ACCESS_ALLOWED | ACCESS_DENIED)));
                aces.push(Ace::new(ACCESS_ALLOWED, flags, *mask, sid.clone()));
            }
            DENY_ACCESS => aces.push(Ace::new(ACCESS_DENIED, flags, *mask, sid.clone())),
            REVOKE_ACCESS => aces.retain(|a| !explicit_for(a)),
            _ => return Err(ERROR_INVALID_PARAMETER),
        }
    }
    let rank = |a: &Ace| match (a.flags & INHERITED_ACE != 0, a.kind) {
        (false, ACCESS_DENIED) => 0,
        (false, _) => 1,
        (true, _) => 2,
    };
    aces.sort_by_key(rank);
    Ok(aces)
}

/// `SetEntriesInAclW(count, entries, old, &new)` and `A`: an error code; the new ACL is `LocalFree`d.
fn set_entries_in_acl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], unicode: bool) -> Reply {
    let code = (|| {
        if a[3] == 0 || (a[0] != 0 && a[1] == 0) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let (entry_size, trustee_at) = if task.abi().width == 8 { (48, 16) } else { (32, 12) };
        let mut entries = Vec::new();
        for i in 0..a[0].min(4096) {
            let at = a[1] + i * entry_size;
            let head = task.get(cx, at, 12).map_err(|_| ERROR_INVALID_PARAMETER)?;
            let sid = task.trustee_sid(cx, at + trustee_at, unicode)?;
            entries.push((u32_at(&head, 0), u32_at(&head, 4), u32_at(&head, 8), sid));
        }
        let old = if a[2] == 0 { Vec::new() } else { task.read_acl(cx, a[2]).map_err(|_| ERROR_INVALID_ACL)?.aces };
        let acl = Acl::new(apply_entries(&old, &entries)?);
        let block = global_new(task, cx, &acl.to_bytes());
        if block == 0 {
            return Err(8);
        }
        task.put_ptr(cx, a[3], block).map_err(|_| ERROR_INVALID_PARAMETER)
    })();
    Reply::Value(u64::from(code.err().unwrap_or(0)))
}

pub fn set_entries_in_acl_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_entries_in_acl(task, cx, a, true)
}

pub fn set_entries_in_acl_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_entries_in_acl(task, cx, a, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_merge_into_an_acl_in_canonical_order() {
        let user = Sid::parse("S-1-5-21-0-0-0-1000").unwrap();
        let everyone = Sid::parse("S-1-1-0").unwrap();
        let system = Sid::parse("S-1-5-18").unwrap();
        let old = vec![Ace::new(ACCESS_ALLOWED, 0, 0x1, user.clone()), Ace::new(ACCESS_ALLOWED, INHERITED_ACE, 0x10, system.clone())];
        let aces = apply_entries(&old, &[(0x2, GRANT_ACCESS, 0, user.clone()), (0x4, DENY_ACCESS, 0, everyone.clone()), (0x8, SET_ACCESS, 3, system.clone())]).unwrap();
        // The deny first; the user's grant merged; the inherited ACE untouched, last.
        let shown: Vec<(u8, u8, u32, String)> = aces.iter().map(|a| (a.kind, a.flags, a.mask, a.sid.to_string())).collect();
        assert_eq!(shown, vec![
            (ACCESS_DENIED, 0, 0x4, "S-1-1-0".into()),
            (ACCESS_ALLOWED, 0, 0x3, "S-1-5-21-0-0-0-1000".into()),
            (ACCESS_ALLOWED, 3, 0x8, "S-1-5-18".into()),
            (ACCESS_ALLOWED, INHERITED_ACE, 0x10, "S-1-5-18".into()),
        ]);
        let revoked = apply_entries(&aces, &[(0, REVOKE_ACCESS, 0, user)]).unwrap();
        assert_eq!(revoked.len(), 3);
        assert_eq!(apply_entries(&[], &[(0, 9, 0, everyone)]), Err(ERROR_INVALID_PARAMETER));
    }
}
