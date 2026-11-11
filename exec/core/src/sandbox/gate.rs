//! The gate protocol: what the registers mean when a program reaches the gate, and how it gets back.
//!
//! However a program gets there, the registers at the gate are the ones it had at its system call, by its own trap's
//! convention (`isa`). A program that *calls* the gate (`Reach::Breakpoint`) has its return address on its stack; that's
//! popped here, as `ret` would, so the program carries on after its call once the kernel has answered. A program sent
//! there by the host (`Dispatch`) or by rewritten code (`Rewrite`) carries on where it was.

use exec_mmu::{Mmu, Native};

use crate::Regs;
use crate::contract::Reach;

/// Called when a program arrives at the gate, before the kernel sees the call.
pub(crate) fn arrived(reach: Reach, regs: &mut Regs, mmu: &Mmu<Native>, root: u64) {
    if reach != Reach::Breakpoint {
        return;
    }
    let Regs::X86(r) = regs else { return };
    let sp = r.gpr[crate::isa::x86::RSP];
    let mut back = [0u8; 8];
    let _ = mmu.copy(root, sp, 8, false, &mut |host, len, done| {
        // SAFETY: `len` bytes of a frame, into the rest of an 8-byte buffer.
        unsafe { std::ptr::copy_nonoverlapping(host, back.as_mut_ptr().add(done), len) };
    });
    r.rip = u64::from_le_bytes(back);
    r.gpr[crate::isa::x86::RSP] = sp + 8;
}
