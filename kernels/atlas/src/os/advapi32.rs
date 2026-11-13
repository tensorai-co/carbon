//! advapi32.dll: the registry, in host Rust over `registry`. Registry functions return their error, rather than setting
//! the last error.

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::secapi::*;
use super::{Reply, dll};
use crate::handles::Handle;
use crate::process::Task;
use crate::registry::{self, REG_EXPAND_SZ, REG_MULTI_SZ, REG_SZ, Root};

dll! {
    "ADVAPI32.dll";
    host RegOpenKeyExW(5) => reg_open_key_ex_w;
    host RegOpenKeyExA(5) => reg_open_key_ex_a;
    host RegOpenKeyW(3) => reg_open_key_w;
    host RegOpenKeyA(3) => reg_open_key_a;
    host RegCreateKeyExW(9) => reg_create_key_ex_w;
    host RegCreateKeyExA(9) => reg_create_key_ex_a;
    host RegCreateKeyW(3) => reg_create_key_w;
    host RegCreateKeyA(3) => reg_create_key_a;
    host RegCloseKey(1) => reg_close_key;
    host RegQueryValueExW(6) => reg_query_value_ex_w;
    host RegQueryValueExA(6) => reg_query_value_ex_a;
    host RegSetValueExW(6) => reg_set_value_ex_w;
    host RegSetValueExA(6) => reg_set_value_ex_a;
    host RegGetValueW(7) => reg_get_value_w;
    host RegGetValueA(7) => reg_get_value_a;
    host RegDeleteValueW(2) => reg_delete_value_w;
    host RegDeleteValueA(2) => reg_delete_value_a;
    host RegDeleteKeyW(2) => reg_delete_key_w;
    host RegDeleteKeyA(2) => reg_delete_key_a;
    host RegDeleteKeyExW(4) => reg_delete_key_ex_w;
    host RegDeleteKeyExA(4) => reg_delete_key_ex_a;
    host RegDeleteTreeW(2) => reg_delete_tree_w;
    host RegDeleteTreeA(2) => reg_delete_tree_a;
    host RegEnumKeyExW(8) => reg_enum_key_ex_w;
    host RegEnumKeyExA(8) => reg_enum_key_ex_a;
    host RegEnumKeyW(4) => reg_enum_key_w;
    host RegEnumKeyA(4) => reg_enum_key_a;
    host RegEnumValueW(8) => reg_enum_value_w;
    host RegEnumValueA(8) => reg_enum_value_a;
    host RegQueryInfoKeyW(12) => reg_query_info_key_w;
    host RegQueryInfoKeyA(12) => reg_query_info_key_a;
    host RegFlushKey(1) => reg_flush_key;
    host RegOpenCurrentUser(2) => reg_open_current_user;
    host GetUserNameW(2) => get_user_name_w;
    host GetUserNameA(2) => get_user_name_a;
    host GetSecurityInfo(8) => get_security_info;
    host SetSecurityInfo(7) => set_security_info;
    host GetNamedSecurityInfoW(8) => get_named_security_info_w;
    host GetNamedSecurityInfoA(8) => get_named_security_info_a;
    host SetNamedSecurityInfoW(7) => set_named_security_info_w;
    host SetNamedSecurityInfoA(7) => set_named_security_info_a;
    host GetKernelObjectSecurity(5) => get_kernel_object_security;
    host SetKernelObjectSecurity(3) => set_kernel_object_security;
    host GetFileSecurityW(5) => get_file_security_w;
    host GetFileSecurityA(5) => get_file_security_a;
    host SetFileSecurityW(3) => set_file_security_w;
    host SetFileSecurityA(3) => set_file_security_a;
    host RegGetKeySecurity(4) => reg_get_key_security;
    host RegSetKeySecurity(3) => reg_set_key_security;
    host AccessCheck(8) => access_check_call;
    host MapGenericMask(2) => map_generic_mask;
    host ImpersonateSelf(1) => impersonate_self;
    host RevertToSelf(0) => revert_to_self;
    host SetThreadToken(2) => set_thread_token;
    host ImpersonateLoggedOnUser(1) => impersonate_logged_on_user;
    host ImpersonateNamedPipeClient(1) => impersonate_named_pipe_client;
    host OpenProcessToken(3) => open_process_token;
    host OpenThreadToken(4) => open_thread_token;
    host GetTokenInformation(5) => get_token_information;
    host SetTokenInformation(4) => set_token_information;
    host AdjustTokenPrivileges(6) => adjust_token_privileges;
    host PrivilegeCheck(3) => privilege_check;
    host DuplicateTokenEx(6) => duplicate_token_ex;
    host DuplicateToken(3) => duplicate_token;
    host CheckTokenMembership(3) => check_token_membership;
    host LookupPrivilegeValueW(3) => lookup_privilege_value_w;
    host LookupPrivilegeValueA(3) => lookup_privilege_value_a;
    host LookupPrivilegeNameW(4) => lookup_privilege_name_w;
    host LookupPrivilegeNameA(4) => lookup_privilege_name_a;
    host AllocateAndInitializeSid(11) => allocate_and_initialize_sid;
    host FreeSid(1) => free_sid;
    host InitializeSid(3) => initialize_sid;
    host GetSidLengthRequired(1) => get_sid_length_required;
    host GetSidSubAuthority(2) => get_sid_sub_authority;
    host GetSidSubAuthorityCount(1) => get_sid_sub_authority_count;
    host GetSidIdentifierAuthority(1) => get_sid_identifier_authority;
    host GetLengthSid(1) => get_length_sid;
    host IsValidSid(1) => is_valid_sid;
    host EqualSid(2) => equal_sid;
    host EqualPrefixSid(2) => equal_prefix_sid;
    host CopySid(3) => copy_sid;
    host CreateWellKnownSid(4) => create_well_known_sid;
    host IsWellKnownSid(2) => is_well_known_sid;
    host ConvertSidToStringSidW(2) => convert_sid_to_string_sid_w;
    host ConvertSidToStringSidA(2) => convert_sid_to_string_sid_a;
    host ConvertStringSidToSidW(2) => convert_string_sid_to_sid_w;
    host ConvertStringSidToSidA(2) => convert_string_sid_to_sid_a;
    host LookupAccountSidW(7) => lookup_account_sid_w;
    host LookupAccountSidA(7) => lookup_account_sid_a;
    host LookupAccountNameW(7) => lookup_account_name_w;
    host LookupAccountNameA(7) => lookup_account_name_a;
    host InitializeSecurityDescriptor(2) => initialize_security_descriptor;
    host IsValidSecurityDescriptor(1) => is_valid_security_descriptor;
    host GetSecurityDescriptorLength(1) => get_security_descriptor_length;
    host GetSecurityDescriptorControl(3) => get_security_descriptor_control;
    host SetSecurityDescriptorControl(3) => set_security_descriptor_control;
    host SetSecurityDescriptorOwner(3) => set_security_descriptor_owner;
    host SetSecurityDescriptorGroup(3) => set_security_descriptor_group;
    host SetSecurityDescriptorDacl(4) => set_security_descriptor_dacl;
    host SetSecurityDescriptorSacl(4) => set_security_descriptor_sacl;
    host GetSecurityDescriptorOwner(3) => get_security_descriptor_owner;
    host GetSecurityDescriptorGroup(3) => get_security_descriptor_group;
    host GetSecurityDescriptorDacl(4) => get_security_descriptor_dacl;
    host GetSecurityDescriptorSacl(4) => get_security_descriptor_sacl;
    host MakeSelfRelativeSD(3) => make_self_relative_sd;
    host InitializeAcl(3) => initialize_acl;
    host IsValidAcl(1) => is_valid_acl;
    host AddAccessAllowedAce(4) => add_access_allowed_ace;
    host AddAccessAllowedAceEx(5) => add_access_allowed_ace_ex;
    host AddAccessDeniedAce(4) => add_access_denied_ace;
    host AddAccessDeniedAceEx(5) => add_access_denied_ace_ex;
    host AddMandatoryAce(5) => add_mandatory_ace;
    host AddAce(5) => add_ace_list;
    host DeleteAce(2) => delete_ace;
    host GetAce(3) => get_ace;
    host GetAclInformation(4) => get_acl_information;
    host ConvertStringSecurityDescriptorToSecurityDescriptorW(4) => convert_string_security_descriptor_to_security_descriptor_w;
    host ConvertStringSecurityDescriptorToSecurityDescriptorA(4) => convert_string_security_descriptor_to_security_descriptor_a;
    host ConvertSecurityDescriptorToStringSecurityDescriptorW(5) => convert_security_descriptor_to_string_security_descriptor_w;
    host ConvertSecurityDescriptorToStringSecurityDescriptorA(5) => convert_security_descriptor_to_string_security_descriptor_a;
    host RegNotifyChangeKeyValue(5) => reg_notify_change_key_value;
    host RegSaveKeyW(3) => reg_save_key_w;
    host RegSaveKeyA(3) => reg_save_key_a;
    host RegSaveKeyExW(4) => reg_save_key_w;
    host RegSaveKeyExA(4) => reg_save_key_a;
    host RegLoadKeyW(3) => reg_load_key_w;
    host RegLoadKeyA(3) => reg_load_key_a;
    host RegUnLoadKeyW(2) => reg_unload_key_w;
    host RegUnLoadKeyA(2) => reg_unload_key_a;
    host RegRestoreKeyW(3) => reg_restore_key_w;
    host RegRestoreKeyA(3) => reg_restore_key_a;
    missing CryptAcquireContextW;
}

const SUCCESS: u64 = 0;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_MORE_DATA: u32 = 234;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_UNSUPPORTED_TYPE: u32 = 1630;
const REG_OPTION_VOLATILE: u32 = 1;

fn error(code: u32) -> Reply {
    Reply::Value(u64::from(code))
}

fn is_string(kind: u32) -> bool {
    matches!(kind, REG_SZ | REG_EXPAND_SZ | REG_MULTI_SZ)
}

/// A string value's UTF-16 bytes as ANSI, with its NULs: what the `A` functions give.
fn to_ansi(data: &[u8]) -> Vec<u8> {
    let units: Vec<u16> = data.chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
    crate::ansi::encode(crate::ansi::ACP, &units, b'?', false).map(|(b, _)| b).unwrap_or_default()
}

/// ANSI bytes as UTF-16: what the `A` functions store.
fn from_ansi(data: &[u8]) -> Vec<u8> {
    crate::ansi::decode(crate::ansi::ACP, data, false).unwrap_or_default().iter().flat_map(|u| u.to_le_bytes()).collect()
}

impl Task {
    /// The registry key a handle stands for: one of the predefined keys, or one this process opened.
    pub(crate) fn key_of(&self, handle: u64) -> Result<u64, u32> {
        if let Some(root) = Root::from_handle(handle) {
            return Ok(lock(&self.k.registry).root(root));
        }
        match lock(&self.process.state).handles.get(handle) {
            Some(Handle::Key(id)) => Ok(id),
            _ => Err(ERROR_INVALID_HANDLE),
        }
    }

    fn reg_string(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> String {
        if at == 0 {
            return String::new();
        }
        if wide {
            self.mem(cx, false, |t| user::read_wstr(&t.space(), at, 32767)).unwrap_or_default()
        } else {
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 32767)).map(|b| crate::ansi::text(&b)).unwrap_or_default()
        }
    }

    fn new_key_handle(&self, id: u64) -> u64 {
        lock(&self.process.state).handles.add(Handle::Key(id))
    }

    /// Writes `data` to a buffer whose size is at `size_at`, as the registry's functions do: just the size if there's
    /// no buffer, ERROR_MORE_DATA and the size if it's too small.
    fn put_sized(&mut self, cx: &mut Cx<'_>, buffer: u64, size_at: u64, data: &[u8]) -> u32 {
        if size_at == 0 {
            return if buffer == 0 { 0 } else { registry::ERROR_INVALID_PARAMETER };
        }
        let room = self.mem(cx, false, |t| user::read_u32(&t.space(), size_at)).unwrap_or(0) as usize;
        let _ = self.mem(cx, true, |t| user::write_u32(&mut t.space(), size_at, data.len() as u32));
        if buffer == 0 {
            return 0;
        }
        if room < data.len() {
            return ERROR_MORE_DATA;
        }
        if self.mem(cx, true, |t| user::write(&mut t.space(), buffer, data)).is_err() { 998 } else { 0 }
    }

    /// Writes a name to a buffer whose size in characters is at `size_at`: the count without the NUL goes back.
    fn put_name(&mut self, cx: &mut Cx<'_>, buffer: u64, size_at: u64, name: &str, wide: bool) -> u32 {
        let units: Vec<u16> = name.encode_utf16().collect();
        let room = self.mem(cx, false, |t| user::read_u32(&t.space(), size_at)).unwrap_or(0) as usize;
        if room <= units.len() {
            return ERROR_MORE_DATA;
        }
        let bytes: Vec<u8> = if wide { units.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect() } else { crate::ansi::bytes(name).into_iter().chain([0]).collect() };
        let wrote = self.mem(cx, true, |t| {
            user::write(&mut t.space(), buffer, &bytes)?;
            user::write_u32(&mut t.space(), size_at, units.len() as u32)
        });
        if wrote.is_err() { 998 } else { 0 }
    }

    fn write_out_u32(&mut self, cx: &mut Cx<'_>, at: u64, value: u32) {
        if at != 0 {
            let _ = self.mem(cx, true, |t| user::write_u32(&mut t.space(), at, value));
        }
    }
}

/// KEY_WOW64_32KEY: the 32-bit view, as a 32-bit program sees the registry.
const KEY_WOW64_32KEY: u32 = 0x200;

/// Where `path` from `base` is, in the view `access` asks for.
fn in_view(task: &Task, base: u64, path: String, access: u32) -> Result<(u64, String), u32> {
    let base = task.key_of(base)?;
    Ok(if access & KEY_WOW64_32KEY != 0 { lock(&task.k.registry).view32(base, &path) } else { (base, path) })
}

fn open_key(task: &mut Task, cx: &mut Cx<'_>, base: u64, subkey: u64, out: u64, wide: bool, access: u32) -> Reply {
    let path = task.reg_string(cx, subkey, wide);
    let opened = in_view(task, base, path, access).and_then(|(base, path)| lock(&task.k.registry).open(base, &path));
    match opened {
        // A key given a descriptor of its own is opened only as it allows.
        Ok(id) if !task.key_allows(id, access) => error(5),
        Ok(id) => {
            let handle = task.new_key_handle(id);
            let _ = task.write_ptr(cx, out, handle);
            Reply::Value(SUCCESS)
        }
        Err(e) => error(e),
    }
}

pub fn reg_open_key_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_key(task, cx, a[0], a[1], a[4], true, a[3] as u32)
}

pub fn reg_open_key_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_key(task, cx, a[0], a[1], a[4], false, a[3] as u32)
}

pub fn reg_open_key_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_key(task, cx, a[0], a[1], a[2], true, crate::security::MAXIMUM_ALLOWED)
}

pub fn reg_open_key_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_key(task, cx, a[0], a[1], a[2], false, crate::security::MAXIMUM_ALLOWED)
}

pub fn reg_open_current_user(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_key(task, cx, 0x8000_0001, 0, a[1], true, a[0] as u32)
}

#[allow(clippy::too_many_arguments)]
fn create_key(task: &mut Task, cx: &mut Cx<'_>, base: u64, subkey: u64, options: u32, access: u32, out: u64, disposition: u64, wide: bool) -> Reply {
    let path = task.reg_string(cx, subkey, wide);
    let now = task.k.host.wall();
    let made = in_view(task, base, path, access).and_then(|(base, path)| lock(&task.k.registry).create(base, &path, options & REG_OPTION_VOLATILE != 0, now));
    match made {
        Ok((id, created)) => {
            let handle = task.new_key_handle(id);
            let _ = task.write_ptr(cx, out, handle);
            task.write_out_u32(cx, disposition, if created { 1 } else { 2 });
            Reply::Value(SUCCESS)
        }
        Err(e) => error(e),
    }
}

pub fn reg_create_key_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_key(task, cx, a[0], a[1], a[4] as u32, a[5] as u32, a[7], a[8], true)
}

pub fn reg_create_key_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_key(task, cx, a[0], a[1], a[4] as u32, a[5] as u32, a[7], a[8], false)
}

pub fn reg_create_key_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_key(task, cx, a[0], a[1], 0, 0, a[2], 0, true)
}

pub fn reg_create_key_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_key(task, cx, a[0], a[1], 0, 0, a[2], 0, false)
}

pub fn reg_close_key(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    if Root::from_handle(a[0]).is_some() {
        return Reply::Value(SUCCESS);
    }
    let (closed, id, others) = {
        let mut state = lock(&task.process.state);
        let id = match state.handles.get(a[0]) {
            Some(Handle::Key(id)) => Some(id),
            _ => None,
        };
        let closed = id.is_some() && state.handles.close(a[0]);
        let others = id.is_some_and(|id| state.handles.any(|h| matches!(h, Handle::Key(k) if *k == id)));
        (closed, id, others)
    };
    if !closed {
        return error(ERROR_INVALID_HANDLE);
    }
    if let Some(id) = id
        && !others
    {
        lock(&task.k.registry).forget(id);
    }
    Reply::Value(SUCCESS)
}

fn query_value(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = task.reg_string(cx, a[1], wide);
    let found = task.key_of(a[0]).and_then(|key| lock(&task.k.registry).value(key, &name).cloned());
    match found {
        Ok(value) => {
            task.write_out_u32(cx, a[3], value.kind);
            let data = if !wide && is_string(value.kind) { to_ansi(&value.data) } else { value.data };
            error(task.put_sized(cx, a[4], a[5], &data))
        }
        Err(e) => error(e),
    }
}

pub fn reg_query_value_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    query_value(task, cx, a, true)
}

pub fn reg_query_value_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    query_value(task, cx, a, false)
}

fn set_value(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = task.reg_string(cx, a[1], wide);
    let kind = a[3] as u32;
    let Ok(mut data) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[4], (a[5] as u32 as usize).min(1 << 20))) else { return error(998) };
    if !wide && is_string(kind) {
        data = from_ansi(&data);
    }
    let now = task.k.host.wall();
    let set = task.key_of(a[0]).and_then(|key| lock(&task.k.registry).set_value(key, &name, kind, data, now));
    match set {
        Ok(()) => Reply::Value(SUCCESS),
        Err(e) => error(e),
    }
}

pub fn reg_set_value_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_value(task, cx, a, true)
}

pub fn reg_set_value_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_value(task, cx, a, false)
}

/// `RegGetValueW`: a value from a key or a subkey, of the types `flags` allow, with strings NUL-terminated and
/// `REG_EXPAND_SZ` expanded unless RRF_NOEXPAND.
pub fn reg_get_value_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_value(task, cx, a, true)
}

pub fn reg_get_value_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_value(task, cx, a, false)
}

fn get_value(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    const RRF_NOEXPAND: u32 = 0x1000_0000;
    let flags = a[3] as u32;
    let (subkey, name) = (task.reg_string(cx, a[1], wide), task.reg_string(cx, a[2], wide));
    let found = task.key_of(a[0]).and_then(|key| {
        let registry = lock(&task.k.registry);
        let key = registry.open(key, &subkey)?;
        registry.value(key, &name).cloned()
    });
    let mut value = match found {
        Ok(value) => value,
        Err(e) => return error(e),
    };
    if value.kind == REG_EXPAND_SZ && flags & RRF_NOEXPAND == 0 {
        value.kind = REG_SZ;
        value.data = registry::expand(&registry::text(&value.data)).encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    }
    let allowed = match value.kind {
        registry::REG_NONE => flags & 1,
        REG_SZ => flags & 2,
        REG_EXPAND_SZ => flags & 4,
        registry::REG_BINARY => flags & 8,
        registry::REG_DWORD => flags & 0x10,
        REG_MULTI_SZ => flags & 0x20,
        registry::REG_QWORD => flags & 0x40,
        _ => 0,
    } != 0;
    if !allowed {
        return error(ERROR_UNSUPPORTED_TYPE);
    }
    if is_string(value.kind) && !value.data.ends_with(&[0, 0]) {
        value.data.extend([0, 0]);
    }
    if !wide && is_string(value.kind) {
        value.data = to_ansi(&value.data);
    }
    task.write_out_u32(cx, a[4], value.kind);
    error(task.put_sized(cx, a[5], a[6], &value.data))
}

fn delete_value(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = task.reg_string(cx, a[1], wide);
    let now = task.k.host.wall();
    match task.key_of(a[0]).and_then(|key| lock(&task.k.registry).delete_value(key, &name, now)) {
        Ok(()) => Reply::Value(SUCCESS),
        Err(e) => error(e),
    }
}

pub fn reg_delete_value_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_value(task, cx, a, true)
}

pub fn reg_delete_value_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_value(task, cx, a, false)
}

fn delete_key(task: &mut Task, cx: &mut Cx<'_>, base: u64, subkey: u64, wide: bool, tree: bool, access: u32) -> Reply {
    let path = task.reg_string(cx, subkey, wide);
    let deleted = in_view(task, base, path, access).and_then(|(base, path)| {
        let mut registry = lock(&task.k.registry);
        // RegDeleteTree with no subkey empties the key itself.
        if tree && path.is_empty() { registry.clear(base) } else { registry.delete_key(base, &path, tree) }
    });
    match deleted {
        Ok(()) => Reply::Value(SUCCESS),
        Err(e) => error(e),
    }
}

pub fn reg_delete_key_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_key(task, cx, a[0], a[1], true, false, 0)
}

pub fn reg_delete_key_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_key(task, cx, a[0], a[1], false, false, 0)
}

pub fn reg_delete_key_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_key(task, cx, a[0], a[1], true, false, a[2] as u32)
}

pub fn reg_delete_key_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_key(task, cx, a[0], a[1], false, false, a[2] as u32)
}

pub fn reg_delete_tree_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_key(task, cx, a[0], a[1], false, true, 0)
}

pub fn reg_delete_tree_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_key(task, cx, a[0], a[1], true, true, 0)
}

/// The `index`th subkey's name, in order.
fn subkey_name(task: &Task, key: u64, index: u64) -> Result<String, u32> {
    let registry = lock(&task.k.registry);
    let k = registry.key(key)?;
    let id = *k.subkeys.values().nth(index as usize).ok_or(ERROR_NO_MORE_ITEMS)?;
    Ok(registry.key(id)?.name.clone())
}

pub fn reg_enum_key_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_key_ex(task, cx, a, true)
}

pub fn reg_enum_key_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_key_ex(task, cx, a, false)
}

fn enum_key_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    match task.key_of(a[0]).and_then(|key| subkey_name(task, key, a[1])) {
        Ok(name) => error(task.put_name(cx, a[2], a[3], &name, wide)),
        Err(e) => error(e),
    }
}

pub fn reg_enum_key_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_key(task, cx, a, true)
}

pub fn reg_enum_key_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_key(task, cx, a, false)
}

/// The old `RegEnumKey`: its buffer's size is a value, not a pointer.
fn enum_key(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    match task.key_of(a[0]).and_then(|key| subkey_name(task, key, a[1])) {
        Ok(name) => {
            let bytes: Vec<u8> = if wide { name.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() } else { crate::ansi::bytes(&name).into_iter().chain([0]).collect() };
            if (bytes.len() / if wide { 2 } else { 1 }) as u64 > a[3] & 0xffff_ffff {
                return error(ERROR_MORE_DATA);
            }
            let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &bytes));
            Reply::Value(SUCCESS)
        }
        Err(e) => error(e),
    }
}

pub fn reg_enum_value_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_value(task, cx, a, true)
}

pub fn reg_enum_value_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_value(task, cx, a, false)
}

fn enum_value(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let found = task.key_of(a[0]).and_then(|key| {
        let registry = lock(&task.k.registry);
        registry.key(key)?.values.values().nth(a[1] as usize).cloned().ok_or(ERROR_NO_MORE_ITEMS)
    });
    let value = match found {
        Ok(value) => value,
        Err(e) => return error(e),
    };
    let named = task.put_name(cx, a[2], a[3], &value.name, wide);
    if named != 0 {
        return error(named);
    }
    task.write_out_u32(cx, a[5], value.kind);
    if a[7] != 0 {
        let data = if !wide && is_string(value.kind) { to_ansi(&value.data) } else { value.data };
        return error(task.put_sized(cx, a[6], a[7], &data));
    }
    Reply::Value(SUCCESS)
}

pub fn reg_query_info_key_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    query_info_key(task, cx, a, true)
}

pub fn reg_query_info_key_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    query_info_key(task, cx, a, false)
}

/// `RegQueryInfoKey`: how many subkeys and values a key has, and the longest of each.
fn query_info_key(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let info = task.key_of(a[0]).and_then(|key| {
        let registry = lock(&task.k.registry);
        let k = registry.key(key)?;
        let longest_subkey = k.subkeys.values().filter_map(|id| registry.key(*id).ok()).map(|s| s.name.encode_utf16().count()).max().unwrap_or(0);
        let longest_name = k.values.values().map(|v| v.name.encode_utf16().count()).max().unwrap_or(0);
        let longest_data = k.values.values().map(|v| v.data.len()).max().unwrap_or(0);
        Ok((k.subkeys.len(), longest_subkey, k.values.len(), longest_name, longest_data, k.modified))
    });
    let (subkeys, longest_subkey, values, longest_name, longest_data, modified) = match info {
        Ok(info) => info,
        Err(e) => return error(e),
    };
    if a[1] != 0 && a[2] != 0 {
        let _ = task.put_name(cx, a[1], a[2], "", wide);
    }
    for (at, value) in [(a[4], subkeys), (a[5], longest_subkey), (a[6], 0), (a[7], values), (a[8], longest_name), (a[9], longest_data), (a[10], 0)] {
        task.write_out_u32(cx, at, value as u32);
    }
    if a[11] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[11], crate::files::filetime(modified)));
    }
    Reply::Value(SUCCESS)
}

/// `RegFlushKey`: saves the registry to the machine's disk now.
pub fn reg_flush_key(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let key = match task.key_of(a[0]) {
        Ok(key) => key,
        Err(e) => return error(e),
    };
    // A loaded hive goes back to its own file.
    let hive = {
        let registry = lock(&task.k.registry);
        registry.hive_of(key).and_then(|(root, file)| registry.save_tree(root).ok().map(|bytes| (bytes, file)))
    };
    if let Some((bytes, file)) = hive
        && let Err(e) = task.write_whole(&file, &bytes, false)
    {
        return error(e);
    }
    task.k.save_registry();
    Reply::Value(SUCCESS)
}

/// `RegNotifyChangeKeyValue(key, subtree, filter, event, asynchronous)`: the next change to the key (or, with
/// `subtree`, under it) of a kind `filter` asks for sets the event, once; not asynchronous, it waits for it here.
pub fn reg_notify_change_key_value(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const FILTERS: u32 = registry::CHANGE_NAME | registry::CHANGE_ATTRIBUTES | registry::CHANGE_LAST_SET | registry::CHANGE_SECURITY;
    const THREAD_AGNOSTIC: u32 = 0x1000_0000;
    let (filter, asynchronous) = (a[2] as u32, a[4] as u32 != 0);
    if filter & !(FILTERS | THREAD_AGNOSTIC) != 0 || filter & FILTERS == 0 {
        return error(registry::ERROR_INVALID_PARAMETER);
    }
    let key = match task.key_of(a[0]) {
        Ok(key) => key,
        Err(e) => return error(e),
    };
    let event = if asynchronous {
        match task.object(a[3]) {
            Some(event) if matches!(event.kind, crate::objects::Kind::Event { .. }) => event,
            _ => return error(ERROR_INVALID_HANDLE),
        }
    } else {
        crate::objects::Object::new(crate::objects::Kind::Event { manual: true }, crate::objects::State::default())
    };
    // Changes before this one aren't its to hear.
    task.k.registry_changed();
    lock(&task.k.reg_watches).push(registry::Watch { key, subtree: a[1] as u32 != 0, filter: filter & FILTERS, event: event.clone() });
    if asynchronous {
        return Reply::Value(SUCCESS);
    }
    match task.wait_for(cx, &[event], false, u32::MAX, false) {
        Ok(_) => Reply::Value(SUCCESS),
        Err(reply) => reply,
    }
}

/// Whether the thread's token has a privilege, enabled: what saving and loading hives ask for.
fn privileged(task: &Task, luid: u32) -> bool {
    const SE_PRIVILEGE_ENABLED: u32 = 2;
    lock(&task.effective_token().state).privileges.iter().any(|p| p.0 == luid && p.1 & SE_PRIVILEGE_ENABLED != 0)
}

const SE_BACKUP: u32 = 17;
const SE_RESTORE: u32 = 18;
const ERROR_PRIVILEGE_NOT_HELD: u32 = 1314;

/// `RegSaveKey(key, file, attributes)`: the key and what's under it, in a new file. SeBackupPrivilege.
fn save_key(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if !privileged(task, SE_BACKUP) {
        return error(ERROR_PRIVILEGE_NOT_HELD);
    }
    let file = task.reg_string(cx, a[1], wide);
    let bytes = task.key_of(a[0]).and_then(|key| lock(&task.k.registry).save_tree(key));
    match bytes.and_then(|bytes| task.write_whole(&file, &bytes, true)) {
        Ok(()) => Reply::Value(SUCCESS),
        Err(crate::files::ERROR_FILE_EXISTS) => error(registry::ERROR_ALREADY_EXISTS),
        Err(e) => error(e),
    }
}

pub fn reg_save_key_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    save_key(task, cx, a, true)
}

pub fn reg_save_key_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    save_key(task, cx, a, false)
}

/// The root a hive's loaded under: `HKEY_LOCAL_MACHINE` or `HKEY_USERS`, and nothing else.
fn hive_root(task: &Task, handle: u64) -> Result<u64, u32> {
    let id = task.key_of(handle)?;
    if lock(&task.k.registry).is_root(id) { Ok(id) } else { Err(registry::ERROR_INVALID_PARAMETER) }
}

/// `RegLoadKey(root, name, file)`: the hive in `file`, as a new key under HKEY_LOCAL_MACHINE or HKEY_USERS, written
/// back to its file as it's flushed and unloaded. SeRestorePrivilege.
fn load_key(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if !privileged(task, SE_RESTORE) {
        return error(ERROR_PRIVILEGE_NOT_HELD);
    }
    let (name, file) = (task.reg_string(cx, a[1], wide), task.reg_string(cx, a[2], wide));
    let now = task.k.host.wall();
    let loaded = hive_root(task, a[0]).and_then(|root| {
        let bytes = task.read_whole(&file)?;
        let full = task.resolve(&file)?.0;
        lock(&task.k.registry).mount(root, &name, &bytes, &full, now)
    });
    match loaded {
        Ok(()) => Reply::Value(SUCCESS),
        Err(e) => error(e),
    }
}

pub fn reg_load_key_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    load_key(task, cx, a, true)
}

pub fn reg_load_key_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    load_key(task, cx, a, false)
}

/// `RegUnLoadKey(root, name)`: a loaded hive written back to its file, and taken away. SeRestorePrivilege.
fn unload_key(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if !privileged(task, SE_RESTORE) {
        return error(ERROR_PRIVILEGE_NOT_HELD);
    }
    let name = task.reg_string(cx, a[1], wide);
    let unloaded = hive_root(task, a[0]).and_then(|root| lock(&task.k.registry).unmount(root, &name));
    match unloaded.and_then(|(bytes, file)| task.write_whole(&file, &bytes, false)) {
        Ok(()) => Reply::Value(SUCCESS),
        Err(e) => error(e),
    }
}

pub fn reg_unload_key_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    unload_key(task, cx, a, true)
}

pub fn reg_unload_key_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    unload_key(task, cx, a, false)
}

/// `RegRestoreKey(key, file, flags)`: everything in the key replaced by what `RegSaveKey` wrote. SeRestorePrivilege.
fn restore_key(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if !privileged(task, SE_RESTORE) {
        return error(ERROR_PRIVILEGE_NOT_HELD);
    }
    let file = task.reg_string(cx, a[1], wide);
    let now = task.k.host.wall();
    let restored = task.key_of(a[0]).and_then(|key| {
        let bytes = task.read_whole(&file)?;
        lock(&task.k.registry).restore(key, &bytes, now)
    });
    match restored {
        Ok(()) => Reply::Value(SUCCESS),
        Err(e) => error(e),
    }
}

pub fn reg_restore_key_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    restore_key(task, cx, a, true)
}

pub fn reg_restore_key_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    restore_key(task, cx, a, false)
}

pub fn get_user_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_user_name(task, cx, a, true)
}

pub fn get_user_name_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_user_name(task, cx, a, false)
}

/// `GetUserName(buffer, &size)`: the size counts the NUL.
fn get_user_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let bytes: Vec<u8> = if wide { "carbon".encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() } else { b"carbon\0".to_vec() };
    let count = bytes.len() / if wide { 2 } else { 1 };
    let room = task.mem(cx, false, |t| user::read_u32(&t.space(), a[1])).unwrap_or(0) as usize;
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], count as u32));
    if room < count {
        task.set_last_error(cx, 122); // ERROR_INSUFFICIENT_BUFFER
        return Reply::Value(0);
    }
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &bytes));
    Reply::Value(1)
}
