//! x86's flags ⇄ ARM's NZCV, for translated code, where x86's SF, ZF, CF and OF live in ARM's N, Z, C and V.

const fn bit(value: u64, from: u32, to: u32) -> u64 {
    (value >> from & 1) << to
}

/// x86's SF, ZF, CF and OF as ARM's N, Z, C and V.
pub const fn eflags_to_nzcv(eflags: u64) -> u64 {
    bit(eflags, 7, 31) | bit(eflags, 6, 30) | bit(eflags, 0, 29) | bit(eflags, 11, 28)
}

/// The other way, into `eflags`, keeping the flags NZCV doesn't carry.
pub const fn merge_nzcv(eflags: u64, nzcv: u64) -> u64 {
    let carried = 1 << 7 | 1 << 6 | 1 << 0 | 1 << 11;
    (eflags & !carried) | bit(nzcv, 31, 7) | bit(nzcv, 30, 6) | bit(nzcv, 29, 0) | bit(nzcv, 28, 11)
}
