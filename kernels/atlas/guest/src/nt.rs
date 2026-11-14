//! ntdll's Rtl string and run-once helpers and the Nt calls browsers make directly, over kernel32's host exports where
//! one does the work. The rest (files and devices by NT path, objects, the registry by NT path) say they aren't
//! written yet and end the process, as a host export's stub does.

use core::ffi::c_void;

use crate::strings::Message;
use crate::sys::host;

const STATUS_SUCCESS: u32 = 0;
const STATUS_UNSUCCESSFUL: u32 = 0xc000_0001;
const STATUS_INVALID_HANDLE: u32 = 0xc000_0008;
const STATUS_NOT_IMPLEMENTED: u32 = 0xc000_0002;

host!(close_handle, b"CloseHandle", (h));
host!(write_file, b"WriteFile", (h, buffer, length, written, overlapped));
host!(virtual_query, b"VirtualQuery", (address, info, length));
host!(reg_open_key_ex_w, b"RegOpenKeyExW", (key, subkey, options, access, out));
host!(reg_create_key_ex_w, b"RegCreateKeyExW", (key, subkey, reserved, class, options, access, security, out, disposition));
host!(reg_query_value_ex_w, b"RegQueryValueExW", (key, name, reserved, kind, data, size));
host!(reg_set_value_ex_w, b"RegSetValueExW", (key, name, reserved, kind, data, size));
host!(reg_delete_key_ex_w, b"RegDeleteKeyExW", (key, subkey, access, reserved));

/// A UNICODE_STRING: lengths in bytes, then (pointer-aligned) the buffer.
#[repr(C)]
pub struct UnicodeString {
    length: u16,
    maximum: u16,
    buffer: *mut u16,
}

/// `RtlInitUnicodeString(&string, text)`: the string over the text (no copy).
#[unsafe(no_mangle)]
pub extern "system" fn RtlInitUnicodeString(s: *mut UnicodeString, text: *const u16) {
    let mut n = 0usize;
    if !text.is_null() {
        // SAFETY: the program's NUL-terminated text.
        while unsafe { text.add(n).read() } != 0 {
            n += 1;
        }
    }
    let bytes = (n * 2).min(0xfffc) as u16;
    // SAFETY: the program's UNICODE_STRING.
    unsafe { s.write(UnicodeString { length: bytes, maximum: if text.is_null() { 0 } else { bytes + 2 }, buffer: text.cast_mut() }) }
}

/// `RtlFreeUnicodeString(&string)`: a buffer an Rtl call made, back to the heap.
#[unsafe(no_mangle)]
pub extern "system" fn RtlFreeUnicodeString(s: *mut UnicodeString) {
    // SAFETY: the program's UNICODE_STRING.
    unsafe {
        crate::heap::free((*s).buffer.cast());
        s.write(UnicodeString { length: 0, maximum: 0, buffer: core::ptr::null_mut() });
    }
}

/// The user's key under `\REGISTRY\USER`: the machine's one account, `atlas::registry::USER` on the host's side.
const CURRENT_USER_KEY: &[u8] = b"\\REGISTRY\\USER\\S-1-5-21-0-0-0-1000";

/// `RtlFormatCurrentUserKeyPath(&string)`: the user's key as an NT path, in a buffer `RtlFreeUnicodeString` frees.
#[unsafe(no_mangle)]
pub extern "system" fn RtlFormatCurrentUserKeyPath(s: *mut UnicodeString) -> u32 {
    const STATUS_NO_MEMORY: u32 = 0xc000_0017;
    let n = CURRENT_USER_KEY.len();
    let buffer = crate::heap::allocate((n + 1) * 2, false).cast::<u16>();
    if buffer.is_null() {
        return STATUS_NO_MEMORY;
    }
    for (i, &b) in CURRENT_USER_KEY.iter().chain(&[0]).enumerate() {
        // SAFETY: the buffer just allocated, n + 1 characters.
        unsafe { buffer.add(i).write(u16::from(b)) };
    }
    // SAFETY: the program's UNICODE_STRING.
    unsafe { s.write(UnicodeString { length: (n * 2) as u16, maximum: (n * 2 + 2) as u16, buffer }) };
    STATUS_SUCCESS
}

/// `RtlRunOnceExecuteOnce(&once, function, parameter, &context)`: `InitOnceExecuteOnce`, as an NTSTATUS.
#[unsafe(no_mangle)]
pub extern "system" fn RtlRunOnceExecuteOnce(once: &core::sync::atomic::AtomicUsize, function: extern "system" fn(*const core::sync::atomic::AtomicUsize, usize, *mut usize) -> i32, parameter: usize, context: *mut usize) -> u32 {
    if crate::sync::AtlasInitOnceExecuteOnce(once, function, parameter, context) != 0 { STATUS_SUCCESS } else { STATUS_UNSUCCESSFUL }
}

/// `RtlGetLastNtStatus()`: success (the NTSTATUS behind the last error isn't kept apart).
#[unsafe(no_mangle)]
pub extern "system" fn RtlGetLastNtStatus() -> u32 {
    STATUS_SUCCESS
}

/// `RtlAcquirePrivilege(privileges, count, flags, &state)`: the user is elevated and has them; the state is nothing
/// to release.
#[unsafe(no_mangle)]
pub extern "system" fn RtlAcquirePrivilege(_privileges: *const u32, _count: u32, _flags: u32, state: *mut *mut c_void) -> u32 {
    if !state.is_null() {
        // SAFETY: the program's pointer.
        unsafe { state.write(core::ptr::null_mut()) };
    }
    STATUS_SUCCESS
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlReleasePrivilege(_state: *mut c_void) {}

service! {
/// `NtClose(handle)`.
pub extern "system" fn NtClose(handle: usize) -> u32 {
    if close_handle(handle as u64) != 0 { STATUS_SUCCESS } else { STATUS_INVALID_HANDLE }
}
}

service! {
/// `NtWriteFile(handle, event, apc, its context, &status, buffer, length, &offset, key)`: written as `WriteFile`
/// writes, where the handle is, its IO_STATUS_BLOCK the status and the count.
#[allow(clippy::too_many_arguments)]
pub extern "system" fn NtWriteFile(handle: usize, _event: usize, _apc: usize, _context: usize, status: *mut usize, buffer: *const u8, length: u32, _offset: *const i64, _key: *const u32) -> u32 {
    let mut written = 0u32;
    let ok = write_file(handle as u64, buffer as usize as u64, u64::from(length), (&raw mut written) as usize as u64, 0) != 0;
    let code = if ok { STATUS_SUCCESS } else { STATUS_UNSUCCESSFUL };
    if !status.is_null() {
        // SAFETY: the program's IO_STATUS_BLOCK.
        unsafe {
            status.write(code as usize);
            status.add(1).write(written as usize);
        }
    }
    code
}
}

service! {
/// `NtQueryVirtualMemory(process, address, class, &info, length, &returned)`: MemoryBasicInformation (0) of this
/// process, as `VirtualQuery` gives it.
pub extern "system" fn NtQueryVirtualMemory(process: usize, address: usize, class: u32, info: *mut c_void, length: usize, returned: *mut usize) -> u32 {
    if process != usize::MAX || class != 0 {
        return STATUS_NOT_IMPLEMENTED;
    }
    let n = virtual_query(address as u64, info as usize as u64, length as u64);
    if n == 0 {
        return 0xc000_0004; // STATUS_INFO_LENGTH_MISMATCH
    }
    if !returned.is_null() {
        // SAFETY: the program's SIZE_T.
        unsafe { returned.write(n as usize) };
    }
    STATUS_SUCCESS
}
}

/// An ntdll export not written yet: it says so, and the process ends, as a host export's stub does. (On x86-64 these
/// are services, reached through their stubs: `services`.)
pub(crate) fn unwritten(name: &[u8]) -> ! {
    Message::new().text(b"atlas-guest: ntdll.dll!").text(name).text(b" isn't written yet\n").print();
    crate::sys::terminate(STATUS_NOT_IMPLEMENTED)
}

macro_rules! unwritten {
    ($($name:ident),*) => {$(
        #[cfg_attr(target_arch = "x86_64", unsafe(export_name = concat!("AtlasImpl", stringify!($name))))]
        #[cfg_attr(not(target_arch = "x86_64"), unsafe(no_mangle))]
        pub extern "system" fn $name() -> u32 {
            unwritten(stringify!($name).as_bytes())
        }
    )*};
}

unwritten!(NtCreateFile, NtDeviceIoControlFile, NtGetNextThread);

host!(query_object, b"AtlasQueryObject", (h, class, out, length, returned));

service! {
/// `NtQueryObject(handle, class, out, length, &returned)`: basic information (0), the object's name (1) and its type
/// (2), as the host knows them.
pub extern "system" fn NtQueryObject(handle: usize, class: u32, out: *mut c_void, length: u32, returned: *mut u32) -> u32 {
    query_object(handle as u64, u64::from(class), out as usize as u64, u64::from(length), returned as usize as u64) as u32
}
}

host!(query_section, b"AtlasQuerySection", (h, out));

service! {
/// `NtQuerySection(section, class, out, length, &returned)`: SectionBasicInformation (0): no base address, the
/// allocation attributes and the size. SectionImageInformation (1) is for image sections, which atlas doesn't make.
pub extern "system" fn NtQuerySection(section: usize, class: u32, out: *mut u8, length: u32, returned: *mut u32) -> u32 {
    const STATUS_INVALID_INFO_CLASS: u32 = 0xc000_0003;
    const STATUS_INFO_LENGTH_MISMATCH: u32 = 0xc000_0004;
    const STATUS_SECTION_NOT_IMAGE: u32 = 0xc000_0049;
    let w = core::mem::size_of::<usize>();
    // BaseAddress, then AllocationAttributes, then MaximumSize on eight bytes.
    let size = w + 4 + if w == 8 { 4 } else { 0 } + 8;
    match class {
        0 => {}
        1 => return STATUS_SECTION_NOT_IMAGE,
        _ => return STATUS_INVALID_INFO_CLASS,
    }
    if (length as usize) < size {
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    let mut info = [0u64; 2];
    let status = query_section(section as u64, info.as_mut_ptr() as u64) as u32;
    if status != STATUS_SUCCESS {
        return status;
    }
    let mut bytes = [0u8; 24];
    bytes[w..w + 4].copy_from_slice(&(info[1] as u32).to_le_bytes());
    bytes[size - 8..size].copy_from_slice(&info[0].to_le_bytes());
    // SAFETY: the program's buffer, at least `size` long.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, size) };
    if !returned.is_null() {
        // SAFETY: the program's ULONG.
        unsafe { returned.write_unaligned(size as u32) };
    }
    STATUS_SUCCESS
}
}

// ---- The registry by NT path ----

/// OBJECT_ATTRIBUTES: its length, the directory a relative name is in, and the name.
#[repr(C)]
pub struct ObjectAttributes {
    length: u32,
    root: usize,
    name: *const UnicodeString,
    attributes: u32,
    security: *const c_void,
    quality: *const c_void,
}

const STATUS_BUFFER_OVERFLOW: u32 = 0x8000_0005;
const STATUS_ACCESS_DENIED: u32 = 0xc000_0022;
const STATUS_BUFFER_TOO_SMALL: u32 = 0xc000_0023;
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xc000_0034;
const STATUS_INVALID_PARAMETER: u32 = 0xc000_000d;
const STATUS_NO_MEMORY: u32 = 0xc000_0017;

/// The NTSTATUS for a registry function's Win32 error.
fn status_of(error: u64) -> u32 {
    match error as u32 {
        0 => STATUS_SUCCESS,
        2 | 3 => STATUS_OBJECT_NAME_NOT_FOUND,
        5 => STATUS_ACCESS_DENIED,
        6 => STATUS_INVALID_HANDLE,
        87 => STATUS_INVALID_PARAMETER,
        _ => STATUS_UNSUCCESSFUL,
    }
}

/// A counted string's characters.
fn chars(s: *const UnicodeString) -> &'static [u16] {
    if s.is_null() {
        return &[];
    }
    // SAFETY: the program's UNICODE_STRING over its buffer.
    unsafe {
        let s = &*s;
        if s.buffer.is_null() { &[] } else { core::slice::from_raw_parts(s.buffer, usize::from(s.length / 2)) }
    }
}

/// A NUL-terminated copy of `text`, on the heap, for a host export; `free`d by the caller.
fn terminated(text: &[u16]) -> Option<*mut u16> {
    let copy = crate::heap::allocate((text.len() + 1) * 2, false).cast::<u16>();
    if copy.is_null() {
        return None;
    }
    // SAFETY: the copy has room for the text and its NUL.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), copy, text.len());
        copy.add(text.len()).write(0);
    }
    Some(copy)
}

/// Whether `text` starts with ASCII `prefix`, ignoring case: the rest if so.
fn strip_prefix<'a>(text: &'a [u16], prefix: &[u8]) -> Option<&'a [u16]> {
    if text.len() < prefix.len() {
        return None;
    }
    let (head, rest) = text.split_at(prefix.len());
    head.iter().zip(prefix).all(|(&c, &p)| c < 0x80 && (c as u8).eq_ignore_ascii_case(&p)).then_some(rest)
}

/// Where the attributes' name is: the key handle it's from (a predefined key for `\REGISTRY\MACHINE` and
/// `\REGISTRY\USER`) and the rest of the path, NUL-terminated on the heap.
fn key_path(attributes: *const ObjectAttributes) -> Result<(u64, *mut u16), u32> {
    const HKEY_LOCAL_MACHINE: u64 = 0x8000_0002;
    const HKEY_USERS: u64 = 0x8000_0003;
    if attributes.is_null() {
        return Err(STATUS_INVALID_PARAMETER);
    }
    // SAFETY: the program's OBJECT_ATTRIBUTES.
    let attributes = unsafe { &*attributes };
    let name = chars(attributes.name);
    let (base, rest) = if attributes.root != 0 {
        (attributes.root as u64, name)
    } else if let Some(rest) = strip_prefix(name, b"\\REGISTRY\\MACHINE") {
        (HKEY_LOCAL_MACHINE, rest)
    } else if let Some(rest) = strip_prefix(name, b"\\REGISTRY\\USER") {
        (HKEY_USERS, rest)
    } else {
        return Err(STATUS_OBJECT_NAME_NOT_FOUND);
    };
    let rest = rest.strip_prefix(&[u16::from(b'\\')]).unwrap_or(rest);
    terminated(rest).map(|path| (base, path)).ok_or(STATUS_NO_MEMORY)
}

service! {
/// `NtOpenKey(&key, access, &attributes)`.
pub extern "system" fn NtOpenKey(key: *mut usize, access: u32, attributes: *const ObjectAttributes) -> u32 {
    NtOpenKeyEx(key, access, attributes, 0)
}
}

service! {
/// `NtOpenKeyEx(&key, access, &attributes, options)`: `RegOpenKeyExW`.
pub extern "system" fn NtOpenKeyEx(key: *mut usize, access: u32, attributes: *const ObjectAttributes, _options: u32) -> u32 {
    let (base, path) = match key_path(attributes) {
        Ok(found) => found,
        Err(status) => return status,
    };
    let mut handle = 0usize;
    let error = reg_open_key_ex_w(base, path as u64, 0, u64::from(access), &raw mut handle as u64);
    crate::heap::free(path.cast());
    if error == 0 {
        // SAFETY: the program's handle out.
        unsafe { key.write(handle) };
    }
    status_of(error)
}
}

service! {
/// `NtCreateKey(&key, access, &attributes, title index, class, options, &disposition)`: `RegCreateKeyExW`.
pub extern "system" fn NtCreateKey(key: *mut usize, access: u32, attributes: *const ObjectAttributes, _index: u32, class: *const UnicodeString, options: u32, disposition: *mut u32) -> u32 {
    let (base, path) = match key_path(attributes) {
        Ok(found) => found,
        Err(status) => return status,
    };
    let class = if class.is_null() { None } else { terminated(chars(class)) };
    let mut handle = 0usize;
    let mut created = 0u32;
    let class_at = class.map_or(0, |c| c as u64);
    let error = reg_create_key_ex_w(base, path as u64, 0, class_at, u64::from(options), u64::from(access), 0, &raw mut handle as u64, &raw mut created as u64);
    crate::heap::free(path.cast());
    if let Some(class) = class {
        crate::heap::free(class.cast());
    }
    if error == 0 {
        // SAFETY: the program's outs.
        unsafe {
            key.write(handle);
            if !disposition.is_null() {
                disposition.write(created);
            }
        }
    }
    status_of(error)
}
}

service! {
/// `NtDeleteKey(key)`: the key itself, `RegDeleteKeyExW` with no subkey.
pub extern "system" fn NtDeleteKey(key: usize) -> u32 {
    let empty = 0u16;
    status_of(reg_delete_key_ex_w(key as u64, &raw const empty as u64, 0, 0))
}
}

service! {
/// `NtSetValueKey(key, &name, title index, type, data, size)`: `RegSetValueExW`.
pub extern "system" fn NtSetValueKey(key: usize, name: *const UnicodeString, _index: u32, kind: u32, data: *const u8, size: u32) -> u32 {
    let Some(name) = terminated(chars(name)) else { return STATUS_NO_MEMORY };
    let error = reg_set_value_ex_w(key as u64, name as u64, 0, u64::from(kind), data as u64, u64::from(size));
    crate::heap::free(name.cast());
    status_of(error)
}
}

service! {
/// `NtQueryValueKey(key, &name, class, info, length, &needed)`: a value's KEY_VALUE_BASIC_INFORMATION (0),
/// KEY_VALUE_FULL_INFORMATION (1) or KEY_VALUE_PARTIAL_INFORMATION (2), or the last two's Align64 forms (3, 4).
pub extern "system" fn NtQueryValueKey(key: usize, name: *const UnicodeString, class: u32, info: *mut u8, length: u32, needed: *mut u32) -> u32 {
    let text = chars(name);
    let Some(name_z) = terminated(text) else { return STATUS_NO_MEMORY };
    let mut kind = 0u32;
    let mut size = 0u32;
    let mut error = reg_query_value_ex_w(key as u64, name_z as u64, 0, &raw mut kind as u64, 0, &raw mut size as u64);
    let mut data: *mut u8 = core::ptr::null_mut();
    if error == 0 && class != 0 {
        data = crate::heap::allocate(size.max(1) as usize, false);
        if data.is_null() {
            crate::heap::free(name_z.cast());
            return STATUS_NO_MEMORY;
        }
        error = reg_query_value_ex_w(key as u64, name_z as u64, 0, &raw mut kind as u64, data as u64, &raw mut size as u64);
    }
    crate::heap::free(name_z.cast());
    if error != 0 {
        crate::heap::free(data);
        return status_of(error);
    }
    let name_bytes = (text.len() * 2) as u32;
    // The fixed part's fields, then where the name and the data go.
    let (fields, name_at, data_at): (&[u32], Option<u32>, Option<u32>) = match class {
        0 => (&[0, kind, name_bytes], Some(12), None),
        1 | 3 => {
            let offset = (20 + name_bytes + 7) & !7;
            (&[0, kind, offset, size, name_bytes], Some(20), Some(offset))
        }
        2 => (&[0, kind, size], None, Some(12)),
        4 => (&[kind, size], None, Some(8)),
        _ => {
            crate::heap::free(data);
            return STATUS_INVALID_PARAMETER;
        }
    };
    let fixed = fields.len() as u32 * 4;
    let total = match (name_at, data_at) {
        (_, Some(at)) => at + size,
        (Some(at), None) => at + name_bytes,
        (None, None) => fixed,
    };
    if !needed.is_null() {
        // SAFETY: the program's out.
        unsafe { needed.write(total) };
    }
    let status = if length < fixed {
        STATUS_BUFFER_TOO_SMALL
    } else {
        // SAFETY: the program's buffer, `length` bytes; each part is written only where it fits.
        unsafe {
            for (i, &field) in fields.iter().enumerate() {
                info.add(i * 4).cast::<u32>().write_unaligned(field);
            }
            if length < total {
                STATUS_BUFFER_OVERFLOW
            } else {
                if let Some(at) = name_at {
                    core::ptr::copy_nonoverlapping(text.as_ptr().cast::<u8>(), info.add(at as usize), name_bytes as usize);
                }
                if let Some(at) = data_at {
                    core::ptr::copy_nonoverlapping(data, info.add(at as usize), size as usize);
                }
                STATUS_SUCCESS
            }
        }
    };
    crate::heap::free(data);
    status
}
}
