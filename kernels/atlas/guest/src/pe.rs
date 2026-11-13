//! Reading PE images where they're mapped: headers, data directories, exports, imports and TLS.

pub const DIR_EXPORT: usize = 0;
pub const DIR_IMPORT: usize = 1;
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

    /// The base it was linked for (PE32+).
    pub fn preferred_base(self) -> u64 {
        // SAFETY: as above.
        unsafe { u64_at(self.optional() + 24) }
    }

    /// A data directory, as an address and size, if the image has it.
    pub fn dir(self, index: usize) -> Option<(u64, u64)> {
        let opt = self.optional();
        // SAFETY: as above; PE32+ only.
        unsafe {
            if u32_at(opt + 108) as usize <= index {
                return None;
            }
            let at = opt + 112 + index as u64 * 8;
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
