//! Reading PE images where they're mapped: headers, data directories, exports, imports and TLS. An image is PE32+ on
//! the 64-bit ISAs and PE32 on x86, where its pointers (thunks, the TLS directory's, the image base) are 4 bytes.

/// The size of a pointer, a handle or a SIZE_T in this ISA, in bytes.
pub const PTR: u64 = size_of::<usize>() as u64;
/// INVALID_HANDLE_VALUE (-1), as a handle the program or atlas gives here reads: pointer-sized, so 0xffffffff on x86.
pub const INVALID_HANDLE: u64 = usize::MAX as u64;
/// An import thunk's bit that says it's by ordinal: the top one.
pub const ORDINAL: u64 = 1 << (PTR * 8 - 1);

pub const DIR_EXPORT: usize = 0;
pub const DIR_IMPORT: usize = 1;
#[cfg_attr(target_arch = "x86", allow(dead_code))]
pub const DIR_EXCEPTION: usize = 3;
pub const DIR_TLS: usize = 9;
pub const DIR_IAT: usize = 12;

/// # Safety
///
/// `at` is readable.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub unsafe fn u8_at(at: u64) -> u8 {
    // SAFETY: as the caller says.
    unsafe { core::ptr::read_volatile(at as *const u8) }
}

/// # Safety
///
/// `at` is readable.
pub unsafe fn u16_at(at: u64) -> u16 {
    // SAFETY: as the caller says.
    unsafe { core::ptr::read_unaligned(at as *const u16) }
}

/// # Safety
///
/// `at` is readable.
pub unsafe fn u32_at(at: u64) -> u32 {
    // SAFETY: as the caller says.
    unsafe { core::ptr::read_unaligned(at as *const u32) }
}

/// # Safety
///
/// `at` is readable.
pub unsafe fn u64_at(at: u64) -> u64 {
    // SAFETY: as the caller says.
    unsafe { core::ptr::read_unaligned(at as *const u64) }
}

/// A pointer-sized value: a pointer, a handle, a SIZE_T.
///
/// # Safety
///
/// `at` is readable.
pub unsafe fn ptr_at(at: u64) -> u64 {
    // SAFETY: as the caller says.
    unsafe { core::ptr::read_unaligned(at as usize as *const usize) as u64 }
}

/// Writes a pointer-sized value.
///
/// # Safety
///
/// `at` is writable.
pub unsafe fn put_ptr(at: u64, v: u64) {
    // SAFETY: as the caller says.
    unsafe { core::ptr::write_unaligned(at as usize as *mut usize, v as usize) }
}

/// A NUL-terminated string in an image.
///
/// # Safety
///
/// `at` is a readable NUL-terminated string.
pub unsafe fn cstr<'a>(at: u64) -> &'a [u8] {
    let mut len = 0;
    // SAFETY: as the caller says.
    while unsafe { *((at + len) as *const u8) } != 0 {
        len += 1;
    }
    // SAFETY: as above.
    unsafe { core::slice::from_raw_parts(at as *const u8, len as usize) }
}

/// A mapped image.
#[derive(Clone, Copy)]
pub struct Image(pub u64);

impl Image {
    fn optional(self) -> u64 {
        // SAFETY: a mapped image's headers.
        self.0 + u64::from(unsafe { u32_at(self.0 + 0x3c) }) + 24
    }

    pub fn is_dll(self) -> bool {
        // SAFETY: as above.
        (unsafe { u16_at(self.optional() - 2) } & 0x2000) != 0
    }

    pub fn entry(self) -> Option<u64> {
        // SAFETY: as above.
        let rva = unsafe { u32_at(self.optional() + 16) };
        (rva != 0).then(|| self.0 + u64::from(rva))
    }

    pub fn size(self) -> u64 {
        // SAFETY: as above.
        u64::from(unsafe { u32_at(self.optional() + 56) })
    }

    /// The linker's timestamp, from the file header.
    pub fn time_date_stamp(self) -> u32 {
        // SAFETY: as above.
        unsafe { u32_at(self.optional() - 16) }
    }

    /// The base it was linked for.
    pub fn preferred_base(self) -> u64 {
        // SAFETY: as above.
        unsafe { if PTR == 8 { u64_at(self.optional() + 24) } else { u64::from(u32_at(self.optional() + 28)) } }
    }

    /// A data directory, as an address and size, if the image has it.
    pub fn dir(self, index: usize) -> Option<(u64, u64)> {
        let opt = self.optional();
        // The directories follow the optional header's fixed part, which PE32's 4-byte fields make 16 bytes shorter.
        let count_at = if PTR == 8 { opt + 108 } else { opt + 92 };
        // SAFETY: as above.
        unsafe {
            if u32_at(count_at) as usize <= index {
                return None;
            }
            let at = count_at + 4 + index as u64 * 8;
            let (rva, size) = (u32_at(at), u32_at(at + 4));
            (rva != 0 && size != 0).then(|| (self.0 + u64::from(rva), u64::from(size)))
        }
    }
}

/// What an import or export names: a name, or an ordinal.
#[derive(Clone, Copy)]
pub enum Wanted<'a> {
    Name(&'a [u8]),
    Ordinal(u16),
}

/// What an export is.
pub enum Export<'a> {
    Address(u64),
    /// Another DLL's: `dll.name`, or `dll.#ordinal`.
    Forward(&'a [u8]),
}

/// Finds an export of a mapped image.
pub fn export(image: Image, wanted: Wanted<'_>) -> Option<Export<'static>> {
    let (dir, size) = image.dir(DIR_EXPORT)?;
    let base = image.0;
    // SAFETY: the export directory and its tables, in a mapped image.
    unsafe {
        let ordinal_base = u32_at(dir + 16);
        let (functions, names) = (u32_at(dir + 20), u32_at(dir + 24));
        let (function_table, name_table, ordinal_table) = (base + u64::from(u32_at(dir + 28)), base + u64::from(u32_at(dir + 32)), base + u64::from(u32_at(dir + 36)));
        let index = match wanted {
            Wanted::Ordinal(ordinal) => u32::from(ordinal).checked_sub(ordinal_base)?,
            Wanted::Name(name) => {
                // Names are sorted: a binary search.
                let (mut lo, mut hi) = (0, names);
                let mut found = None;
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let candidate = cstr(base + u64::from(u32_at(name_table + u64::from(mid) * 4)));
                    match candidate.cmp(name) {
                        core::cmp::Ordering::Equal => {
                            found = Some(u32::from(u16_at(ordinal_table + u64::from(mid) * 2)));
                            break;
                        }
                        core::cmp::Ordering::Less => lo = mid + 1,
                        core::cmp::Ordering::Greater => hi = mid,
                    }
                }
                found?
            }
        };
        if index >= functions {
            return None;
        }
        let address = base + u64::from(u32_at(function_table + u64::from(index) * 4));
        if address == base {
            return None;
        }
        Some(if address >= dir && address < dir + size { Export::Forward(cstr(address)) } else { Export::Address(address) })
    }
}
