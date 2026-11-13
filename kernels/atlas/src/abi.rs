//! Where things are in a process's own structures, by its ISA: the TEB, PEB, process parameters and exception records
//! are 64-bit for ARM64 and x86-64 processes and 32-bit for x86 ones (WoW64), with `atlas-abi` saying both. Host code
//! that writes one asks its process's `Abi` rather than naming `atlas_abi::teb` (which is always the 64-bit one here).

use exec::{Isa, Regs};

/// A struct of offsets, filled from `atlas-abi`'s 64-bit or 32-bit module of the same constants.
macro_rules! layout {
    ($name:ident from $m64:ident, $m32:ident { $($field:ident: $c:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Copy)]
        pub(crate) struct $name {
            $(pub $field: u64,)*
        }

        impl $name {
            pub fn of(isa: Isa) -> $name {
                if isa == Isa::X86_32 {
                    $name { $($field: atlas_abi::$m32::$c,)* }
                } else {
                    $name { $($field: atlas_abi::$m64::$c,)* }
                }
            }
        }
    };
}

layout!(Teb from teb64, teb32 {
    stack_base: STACK_BASE,
    stack_limit: STACK_LIMIT,
    self_at: SELF,
    client_id: CLIENT_ID,
    peb: PEB,
    last_error: LAST_ERROR,
    deallocation_stack: DEALLOCATION_STACK,
});

layout!(Peb from peb64, peb32 {
    image_base: IMAGE_BASE,
    process_parameters: PROCESS_PARAMETERS,
    number_of_processors: NUMBER_OF_PROCESSORS,
    os_major_version: OS_MAJOR_VERSION,
    os_minor_version: OS_MINOR_VERSION,
    os_build_number: OS_BUILD_NUMBER,
    os_platform_id: OS_PLATFORM_ID,
    image_subsystem: IMAGE_SUBSYSTEM,
    image_subsystem_major: IMAGE_SUBSYSTEM_MAJOR,
    image_subsystem_minor: IMAGE_SUBSYSTEM_MINOR,
});

layout!(Params from params64, params32 {
    standard_input: STANDARD_INPUT,
    standard_output: STANDARD_OUTPUT,
    standard_error: STANDARD_ERROR,
    image_path_name: IMAGE_PATH_NAME,
    command_line: COMMAND_LINE,
    environment: ENVIRONMENT,
    environment_size: ENVIRONMENT_SIZE,
    size: SIZE,
});

layout!(Record from record64, record32 {
    size: SIZE,
    address: ADDRESS,
    parameter_count: PARAMETER_COUNT,
    parameters: PARAMETERS,
});

/// A process's structures, as its ISA lays them out.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Abi {
    pub isa: Isa,
    /// A pointer's size, a handle's and a SIZE_T's: 8 or 4.
    pub width: u64,
    pub teb: Teb,
    pub peb: Peb,
    pub params: Params,
    pub record: Record,
}

impl Abi {
    pub fn of(isa: Isa) -> Abi {
        Abi { isa, width: if isa == Isa::X86_32 { 4 } else { 8 }, teb: Teb::of(isa), peb: Peb::of(isa), params: Params::of(isa), record: Record::of(isa) }
    }

    /// A pointer-sized value's bytes.
    pub fn ptr(&self, value: u64) -> Vec<u8> {
        value.to_le_bytes()[..self.width as usize].to_vec()
    }

    /// A UNICODE_STRING of `bytes` bytes with its NUL (which its length leaves out), at `at`: two lengths, then the
    /// pointer, aligned to its size.
    pub fn unicode_string(&self, bytes: usize, at: u64) -> Vec<u8> {
        let mut s = ((bytes - 2) as u16).to_le_bytes().to_vec();
        s.extend((bytes as u16).to_le_bytes());
        s.resize(self.width as usize, 0);
        s.extend(self.ptr(at));
        s
    }

    /// Reads a pointer-sized value from bytes a program gave, at `at`.
    pub fn ptr_at(&self, bytes: &[u8], at: usize) -> u64 {
        let mut word = [0u8; 8];
        let n = self.width as usize;
        if let Some(b) = bytes.get(at..at + n) {
            word[..n].copy_from_slice(b);
        }
        u64::from_le_bytes(word)
    }

    /// OVERLAPPED: `Internal` (an NTSTATUS) and `InternalHigh` (the count) are pointer-sized, then `Offset` and
    /// `OffsetHigh`, then `hEvent`.
    pub fn overlapped_offset(&self) -> u64 {
        2 * self.width
    }

    pub fn overlapped_event(&self) -> u64 {
        2 * self.width + 8
    }

    /// An OVERLAPPED's first two fields: a status and a count.
    pub fn overlapped_result(&self, status: u32, count: u64) -> Vec<u8> {
        [self.ptr(u64::from(status)), self.ptr(count)].concat()
    }

    /// A new thread's TEB fields: its stack (top, the bottom of what's mapped, and the bottom of the reservation),
    /// itself, its IDs and the PEB. A 32-bit TEB's chain of SEH frames starts empty (-1).
    #[allow(clippy::too_many_arguments)]
    pub fn teb_fields(&self, teb_at: u64, top: u64, limit: u64, bottom: u64, pid: u64, tid: u64, peb: u64) -> Vec<(u64, Vec<u8>)> {
        let t = &self.teb;
        let mut fields = vec![
            (t.stack_base, self.ptr(top)),
            (t.stack_limit, self.ptr(limit)),
            (t.deallocation_stack, self.ptr(bottom)),
            (t.self_at, self.ptr(teb_at)),
            (t.client_id, self.ptr(pid)),
            (t.client_id + self.width, self.ptr(tid)),
            (t.peb, self.ptr(peb)),
        ];
        if self.isa == Isa::X86_32 {
            fields.push((atlas_abi::teb32::EXCEPTION_LIST, self.ptr(u64::from(u32::MAX))));
        }
        fields
    }
}

/// Points a thread's registers at its TEB, where Windows keeps it: x18 on ARM64, gs on x86-64, fs on x86.
pub(crate) fn set_teb(regs: &mut Regs, isa: Isa, teb_at: u64) {
    match regs {
        Regs::Arm64(r) => r.x[18] = teb_at,
        Regs::X86(r) if isa == Isa::X86_32 => r.fs_base = teb_at,
        Regs::X86(r) => r.gs_base = teb_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_by_isa() {
        let (a64, a32) = (Abi::of(Isa::X86_64), Abi::of(Isa::X86_32));
        assert_eq!((a64.width, a64.teb.peb, a64.peb.process_parameters), (8, 0x60, 0x20));
        assert_eq!((a32.width, a32.teb.peb, a32.peb.process_parameters), (4, 0x30, 0x10));
        assert_eq!(a32.unicode_string(6, 0x1234), [4, 0, 6, 0, 0x34, 0x12, 0, 0]);
        assert_eq!(a64.unicode_string(6, 0x1234).len(), 16);
    }
}
