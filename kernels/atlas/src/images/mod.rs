//! Images: reading PE files, and making each system DLL's image from its declaration, once per ISA, shared by every
//! process that maps it.

pub mod make;
pub mod pe;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use atlas_abi::call::{THREAD_RETURN, UPCALL_RETURN};
use exec::Isa;
use kernels_core::lock;

use crate::os::{DLLS, Kind, number};
use make::Body;

/// Where things go in a process of one ISA: the program's allocations, atlas's page of stubs, and the system DLLs above
/// it, each in a slot of its own. A 32-bit process has them all below 2 GiB, as WoW64 has: the program's from 64 KiB,
/// then atlas's page and the system DLLs from 0x70000000, under the shared data page at 0x7ffe0000.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Where `VirtualAlloc`, stacks and images find room, bottom up as on Windows.
    pub alloc: std::ops::Range<u64>,
    /// The page of stubs every process has: where its first thread returns to, and where upcalls return to.
    pub atlas_page: u64,
    pub system_base: u64,
    slot: u64,
}

impl Layout {
    pub fn thread_return(&self) -> u64 {
        self.atlas_page
    }

    pub fn upcall_return(&self) -> u64 {
        self.atlas_page + 32
    }

    /// Where system DLL `dll` goes.
    pub fn dll_base(&self, dll: usize) -> u64 {
        self.system_base + dll as u64 * self.slot
    }
}

pub fn layout(isa: Isa) -> Layout {
    match isa {
        // The 64-bit ISAs' allocations start above 4 GiB, which the macOS sandbox keeps empty.
        Isa::Arm64 | Isa::X86_64 => {
            let system_base = 0x6ff0_0000_0000;
            Layout { alloc: 0x1_0000_0000..system_base - (1 << 30), atlas_page: atlas_abi::call::ATLAS_PAGE64, system_base, slot: 0x100_0000 }
        }
        Isa::X86_32 => {
            let system_base = 0x7000_0000;
            Layout { alloc: 0x1_0000..0x6000_0000, atlas_page: atlas_abi::call::ATLAS_PAGE32, system_base, slot: 0x40_0000 }
        }
    }
}

/// `atlas-guest`'s image for `isa`, as `build.rs` built it, or None where it couldn't be built. Without it, guest
/// exports trap like missing ones and say so.
pub fn guest(isa: Isa) -> Option<&'static [u8]> {
    static ARM64: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ntdll-aarch64.dll"));
    static X86_64: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ntdll-x86_64.dll"));
    static X86: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ntdll-i686.dll"));
    let image = match isa {
        Isa::Arm64 => ARM64,
        Isa::X86_64 => X86_64,
        Isa::X86_32 => X86,
    };
    (!image.is_empty()).then_some(image)
}

/// Images made so far, by ISA and DLL.
type Made = HashMap<(Isa, usize, bool), Arc<Vec<u8>>>;

#[derive(Default)]
pub struct Images {
    made: Mutex<Made>,
    /// Programs reach atlas through the backend's gate, not `syscall` (the Windows sandbox).
    gate: AtomicBool,
}

impl Images {
    /// System DLL `dll`'s image for `isa`, made the first time it's wanted.
    pub fn get(&self, isa: Isa, dll: usize) -> Arc<Vec<u8>> {
        let gate = self.gate.load(Ordering::Relaxed);
        lock(&self.made).entry((isa, dll, gate)).or_insert_with(|| Arc::new(make_dll(isa, dll, gate))).clone()
    }

    /// Says whether this machine's programs reach atlas through a gate.
    pub fn use_gate(&self, gate: bool) {
        self.gate.store(gate, Ordering::Relaxed);
    }
}

fn make_dll(isa: Isa, dll: usize, gate: bool) -> Vec<u8> {
    let d = DLLS[dll];
    let exports: Vec<(&str, Body)> = d
        .exports
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let body = match e.kind {
                Kind::Guest(target) if guest(isa).is_some() => Body::Forward(format!("ntdll.{target}")),
                // x86's stubs pop a stdcall function's arguments: four bytes each, as atlas reads them. The C
                // runtime's, and the functions that take a variable number, are cdecl: their caller pops.
                Kind::Host { args, .. } if !cdecl(d.name, e.name) => {
                    let slots = x86_wide(e.name).map_or(args, |(slots, _)| slots);
                    Body::Trap { nr: number(dll, i), pop: 4 * slots as u16 }
                }
                _ => Body::Trap { nr: number(dll, i), pop: 0 },
            };
            (e.name, body)
        })
        .collect();
    make::dll(isa, d.name, layout(isa).dll_base(dll), &d.ordinals(), &exports, gate)
}

/// Makes the arguments atlas reads from the stack slots x86 passed.
pub type Join = fn(&[u64]) -> Vec<u64>;

/// The exports that take a 64-bit value by value, which is two stack slots on x86 where it's one register on the
/// 64-bit ISAs: how many slots they take there, and how the slots make the arguments atlas reads (low half first).
pub fn x86_wide(name: &str) -> Option<(usize, Join)> {
    const LOW: u64 = 0xffff_ffff;
    let joined: (usize, Join) = match name {
        // (file, LARGE_INTEGER distance, &new, method)
        "SetFilePointerEx" => (5, |s| vec![s[0], s[1] & LOW | (s[2] & LOW) << 32, s[3], s[4]]),
        // (info, type mask, ULONGLONG condition mask)
        "VerifyVersionInfoW" | "VerifyVersionInfoA" => (4, |s| vec![s[0], s[1], s[2] & LOW | (s[3] & LOW) << 32]),
        // (ULONGLONG condition mask, type, condition)
        "VerSetConditionMask" => (4, |s| vec![s[0] & LOW | (s[1] & LOW) << 32, s[2], s[3]]),
        // (POINT): x and y, as one 64-bit value has them.
        "WindowFromPoint" => (2, |s| vec![s[0] & LOW | (s[1] & LOW) << 32]),
        // (POINT, flags)
        "MonitorFromPoint" => (3, |s| vec![s[0] & LOW | (s[1] & LOW) << 32, s[2]]),
        _ => return None,
    };
    Some(joined)
}

/// The exports that return 64 bits, which x86 returns in edx:eax.
pub fn x86_returns_wide(name: &str) -> bool {
    matches!(name, "GetTickCount64" | "VerSetConditionMask")
}

/// Whether an export is cdecl on x86: the C runtime's, and those with a variable number of arguments.
pub fn cdecl(dll: &str, name: &str) -> bool {
    dll.eq_ignore_ascii_case("ucrtbase.dll") || dll.eq_ignore_ascii_case("msvcrt.dll") || matches!(name, "wsprintfA" | "wsprintfW" | "DbgPrint" | "DbgPrintEx")
}

/// The atlas page's code: two stubs, 32 bytes apart, that come into atlas with the program's return value kept (in
/// `x0`, or moved to `r10`, or on x86 to `edx`, since x86's stub needs `eax` for the number); then, at `TRAP_AT`,
/// x86-64's trampoline into atlas: `syscall; ret`, or a jump to the backend's `gate`.
pub fn atlas_page(isa: Isa, gate: Option<u64>) -> Vec<u8> {
    let mut page = Vec::new();
    for nr in [THREAD_RETURN, UPCALL_RETURN] {
        let mut stub = Vec::new();
        match isa {
            // mov edx, eax; mov eax, nr; int 0x2e; int3
            Isa::X86_32 => {
                stub.extend([0x89, 0xc2, 0xb8]);
                stub.extend((nr as u32).to_le_bytes());
                stub.extend([0xcd, 0x2e, 0xcc]);
            }
            Isa::Arm64 => {
                // movz x8, #nr; svc #0; brk #0
                for word in [0xd280_0008 | (nr as u32) << 5, 0xd400_0001, 0xd420_0000] {
                    stub.extend(u32::to_le_bytes(word));
                }
            }
            _ => {
                // mov r10, rax; mov eax, nr; then syscall, or mov r11, TRAP_AT; call r11; then int3
                stub.extend([0x49, 0x89, 0xc2, 0xb8]);
                stub.extend((nr as u32).to_le_bytes());
                if gate.is_some() {
                    stub.extend([0x49, 0xbb]);
                    stub.extend(atlas_abi::call::TRAP_AT.to_le_bytes());
                    stub.extend([0x41, 0xff, 0xd3]);
                } else {
                    stub.extend([0x0f, 0x05]);
                }
                stub.push(0xcc);
            }
        }
        stub.resize(32, if isa == Isa::Arm64 { 0 } else { 0xcc });
        page.extend(stub);
    }
    if isa == Isa::X86_64 {
        page.resize((atlas_abi::call::TRAP_AT - atlas_abi::call::ATLAS_PAGE64) as usize, 0xcc);
        match gate {
            // mov r11, gate; jmp r11: the caller's return address stays, for the gate to return to.
            Some(gate) => {
                page.extend([0x49, 0xbb]);
                page.extend(gate.to_le_bytes());
                page.extend([0x41, 0xff, 0xe3]);
            }
            // syscall; ret
            None => page.extend([0x0f, 0x05, 0xc3]),
        }
    }
    page
}
