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

/// Where system DLLs go: each in a slot of its own, below the top of every ISA's space.
pub const SYSTEM_BASE: u64 = 0x6ff0_0000_0000;
const SLOT: u64 = 0x100_0000;
/// The page of stubs every process has: where its first thread returns to, and where upcalls return to.
pub const ATLAS_PAGE: u64 = atlas_abi::call::ATLAS_PAGE;
pub const THREAD_RETURN_AT: u64 = ATLAS_PAGE;
pub const UPCALL_RETURN_AT: u64 = ATLAS_PAGE + 32;
const _: () = assert!(ATLAS_PAGE == SYSTEM_BASE - 0x1_0000);

/// `atlas-guest`'s image for `isa`, as `build.rs` built it, or None where it couldn't be built. Without it, guest
/// exports trap like missing ones and say so.
pub fn guest(isa: Isa) -> Option<&'static [u8]> {
    static ARM64: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ntdll-aarch64.dll"));
    static X86_64: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ntdll-x86_64.dll"));
    let image = match isa {
        Isa::Arm64 => ARM64,
        Isa::X86_64 => X86_64,
        Isa::X86_32 => &[],
    };
    (!image.is_empty()).then_some(image)
}

/// Where system DLL `dll` goes.
pub fn base(dll: usize) -> u64 {
    SYSTEM_BASE + dll as u64 * SLOT
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
                _ => Body::Trap(number(dll, i)),
            };
            (e.name, body)
        })
        .collect();
    make::dll(isa, d.name, base(dll), &exports, gate)
}

/// The atlas page's code: two stubs, 32 bytes apart, that come into atlas with the program's return value kept (in
/// `x0`, or moved to `r10`, since x86's stub needs `eax` for the number); then, at `TRAP_AT`, x86-64's trampoline into
/// atlas: `syscall; ret`, or a jump to the backend's `gate`.
pub fn atlas_page(isa: Isa, gate: Option<u64>) -> Vec<u8> {
    let mut page = Vec::new();
    for nr in [THREAD_RETURN, UPCALL_RETURN] {
        let mut stub = Vec::new();
        match isa {
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
    if isa != Isa::Arm64 {
        page.resize((atlas_abi::call::TRAP_AT - ATLAS_PAGE) as usize, 0xcc);
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
