//! Unwinding one frame: finding a function's `.pdata` entry, and undoing its prolog from its unwind data to get its
//! caller's registers, as Windows' documentation for each ISA describes them.

use crate::arch::Context;
use crate::loader;
use crate::pe::{self, Image, u32_at};
#[cfg(target_arch = "aarch64")]
use crate::pe::u64_at;

/// Which handler is wanted: one for looking (dispatch), or one for unwinding.
pub const EHANDLER: u32 = 1;
pub const UHANDLER: u32 = 2;

/// What unwinding a frame found.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unwound {
    /// Its language handler, if it has one and the frame's in its body, and that handler's data.
    pub handler: u64,
    pub handler_data: u64,
    /// The frame its handler and unwinding are relative to.
    pub establisher: u64,
}

/// The `.pdata` entry for `pc`: its image's base, and the entry's address.
pub fn lookup(pc: u64) -> Option<(u64, u64)> {
    let (base, _) = loader::containing(pc)?;
    let (table, size) = Image(base).dir(pe::DIR_EXCEPTION)?;
    let rva = (pc - base) as u32;
    let count = size / ENTRY;
    // The last entry starting at or before `pc`.
    let (mut lo, mut hi) = (0, count);
    while lo < hi {
        let mid = (lo + hi) / 2;
        // SAFETY: entries in the image's exception directory.
        if unsafe { u32_at(table + mid * ENTRY) } <= rva {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let entry = table + lo.checked_sub(1)? * ENTRY;
    // SAFETY: as above.
    let (begin, length) = unsafe { (u32_at(entry), function_length(base, entry)) };
    (rva >= begin && u64::from(rva - begin) < length).then_some((base, entry))
}

#[cfg(target_arch = "x86_64")]
pub use x64::*;

#[cfg(target_arch = "aarch64")]
pub use arm64::*;

#[cfg(target_arch = "x86_64")]
mod x64 {
    use super::*;
    use crate::pe::{u8_at, u16_at, u64_at};

    /// RUNTIME_FUNCTION: begin, end, unwind info.
    pub const ENTRY: u64 = 12;

    /// # Safety
    ///
    /// `entry` is a `.pdata` entry.
    pub unsafe fn function_length(_base: u64, entry: u64) -> u64 {
        // SAFETY: as the caller says.
        unsafe { u64::from(u32_at(entry + 4).saturating_sub(u32_at(entry))) }
    }

    const FLAG_CHAININFO: u8 = 4;
    const PUSH_NONVOL: u8 = 0;
    const ALLOC_LARGE: u8 = 1;
    const ALLOC_SMALL: u8 = 2;
    const SET_FPREG: u8 = 3;
    const SAVE_NONVOL: u8 = 4;
    const SAVE_NONVOL_FAR: u8 = 5;
    const SAVE_XMM128: u8 = 8;
    const SAVE_XMM128_FAR: u8 = 9;
    const PUSH_MACHFRAME: u8 = 10;
    const RSP: usize = 4;

    /// How many 2-byte slots an unwind code takes.
    fn slots(op: u8, info: u8) -> u64 {
        match op {
            ALLOC_LARGE if info == 0 => 2,
            ALLOC_LARGE => 3,
            SAVE_NONVOL | SAVE_XMM128 => 2,
            SAVE_NONVOL_FAR | SAVE_XMM128_FAR => 3,
            _ => 1,
        }
    }

    /// If `pc` is in an epilog, finishes it: `add rsp` or `lea rsp`, pops, then `ret` or a tail jump. True if it was.
    ///
    /// # Safety
    ///
    /// The code at `pc` and the stack are readable.
    unsafe fn finish_epilog(c: &mut Context, frame_reg: usize) -> bool {
        // SAFETY: as the caller says, for each read.
        unsafe {
            let mut at = c.pc;
            let mut rsp = c.x[RSP];
            if u8_at(at) & 0xf8 == 0x48 {
                match (u8_at(at + 1), u8_at(at + 2)) {
                    (0x83, 0xc4) => {
                        rsp = rsp.wrapping_add(u8_at(at + 3) as i8 as u64);
                        at += 4;
                    }
                    (0x81, 0xc4) => {
                        rsp = rsp.wrapping_add(u32_at(at + 3) as i32 as u64);
                        at += 7;
                    }
                    (0x8d, modrm) if modrm >> 3 & 7 == 4 && frame_reg != 0 => {
                        let base = c.x[frame_reg];
                        match modrm >> 6 {
                            1 => {
                                rsp = base.wrapping_add(u8_at(at + 3) as i8 as u64);
                                at += 4;
                            }
                            2 => {
                                rsp = base.wrapping_add(u32_at(at + 3) as i32 as u64);
                                at += 7;
                            }
                            _ => return false,
                        }
                    }
                    _ => {}
                }
            }
            // Pops, then a return or a jump: only then is it an epilog.
            let pops = at;
            loop {
                let rex = u8_at(at) & 0xf0 == 0x40;
                let op = u8_at(at + u64::from(rex));
                if op & 0xf8 != 0x58 {
                    break;
                }
                at += 1 + u64::from(rex);
            }
            let last = u8_at(at);
            let ends = matches!(last, 0xc3 | 0xc2 | 0xe9 | 0xeb) || last == 0xf3 && u8_at(at + 1) == 0xc3 || last == 0x48 && u8_at(at + 1) == 0xff;
            if !ends {
                return false;
            }
            let mut at = pops;
            loop {
                let rex = u8_at(at) & 0xf0 == 0x40;
                let op = u8_at(at + u64::from(rex));
                if op & 0xf8 != 0x58 {
                    break;
                }
                let reg = usize::from(op & 7) + if rex && u8_at(at) & 1 != 0 { 8 } else { 0 };
                c.x[reg] = u64_at(rsp);
                rsp += 8;
                at += 1 + u64::from(rex);
            }
            c.pc = u64_at(rsp);
            c.x[RSP] = rsp + 8;
        }
        true
    }

    /// Unwinds one frame: `c` is the function's on the way in, its caller's on the way out.
    ///
    /// # Safety
    ///
    /// `entry` is the function's `.pdata` entry in the image at `base`, and its stack is readable.
    pub unsafe fn virtual_unwind(base: u64, entry: u64, c: &mut Context, handler_type: u32) -> Result<Unwound, ()> {
        let mut unwound = Unwound::default();
        let mut function = entry;
        // SAFETY: as the caller says, for every read of unwind data and the stack.
        unsafe {
            let offset = c.pc.wrapping_sub(base + u64::from(u32_at(function)));
            let mut first = true;
            for _ in 0..32 {
                let info = base + u64::from(u32_at(function + 8));
                let (flags, prolog, count, frame) = (u8_at(info) >> 3, u64::from(u8_at(info + 1)), u64::from(u8_at(info + 2)), u8_at(info + 3));
                let frame_reg = usize::from(frame & 0xf);
                let frame_offset = u64::from(frame >> 4) * 16;
                let code = |i: u64| info + 4 + i * 2;
                let in_prolog = first && offset < prolog;
                if first {
                    // The frame register counts once its prolog has set it.
                    let mut set = !in_prolog;
                    let mut i = 0;
                    while in_prolog && frame_reg != 0 && i < count {
                        let (at, byte) = (u64::from(u8_at(code(i))), u8_at(code(i) + 1));
                        set |= byte & 0xf == SET_FPREG && at <= offset;
                        i += slots(byte & 0xf, byte >> 4);
                    }
                    unwound.establisher = if frame_reg != 0 && set { c.x[frame_reg].wrapping_sub(frame_offset) } else { c.x[RSP] };
                    if !in_prolog && finish_epilog(c, frame_reg) {
                        return Ok(unwound);
                    }
                }
                let est = unwound.establisher;
                let mut i = 0;
                while i < count {
                    let (at, byte) = (u64::from(u8_at(code(i))), u8_at(code(i) + 1));
                    let (op, op_info) = (byte & 0xf, byte >> 4);
                    // A prolog's codes are last first, and those it hasn't reached haven't happened.
                    if !in_prolog || at <= offset {
                        match op {
                            PUSH_NONVOL => {
                                c.x[usize::from(op_info)] = u64_at(c.x[RSP]);
                                c.x[RSP] += 8;
                            }
                            ALLOC_LARGE if op_info == 0 => c.x[RSP] += u64::from(u16_at(code(i + 1))) * 8,
                            ALLOC_LARGE => c.x[RSP] += u64::from(u32_at(code(i + 1))),
                            ALLOC_SMALL => c.x[RSP] += u64::from(op_info) * 8 + 8,
                            SET_FPREG => c.x[RSP] = c.x[frame_reg].wrapping_sub(frame_offset),
                            SAVE_NONVOL => c.x[usize::from(op_info)] = u64_at(est + u64::from(u16_at(code(i + 1))) * 8),
                            SAVE_NONVOL_FAR => c.x[usize::from(op_info)] = u64_at(est + u64::from(u32_at(code(i + 1)))),
                            SAVE_XMM128 => c.xmm[usize::from(op_info)] = core::ptr::read_unaligned((est + u64::from(u16_at(code(i + 1))) * 16) as *const u128),
                            SAVE_XMM128_FAR => c.xmm[usize::from(op_info)] = core::ptr::read_unaligned((est + u64::from(u32_at(code(i + 1)))) as *const u128),
                            PUSH_MACHFRAME => {
                                let top = c.x[RSP] + if op_info != 0 { 8 } else { 0 };
                                c.pc = u64_at(top);
                                c.x[RSP] = u64_at(top + 24);
                                return Ok(unwound);
                            }
                            _ => return Err(()),
                        }
                    }
                    i += slots(op, op_info);
                }
                let after = code((count + 1) & !1);
                if flags & FLAG_CHAININFO != 0 {
                    // The chained entry is this function's too, and its prolog has all happened.
                    function = after;
                    first = false;
                    continue;
                }
                if !in_prolog && u32::from(flags & 3) & handler_type != 0 {
                    unwound.handler = base + u64::from(u32_at(after));
                    unwound.handler_data = after + 4;
                }
                c.pc = u64_at(c.x[RSP]);
                c.x[RSP] += 8;
                return Ok(unwound);
            }
        }
        Err(())
    }

    /// A function with no `.pdata` entry is a leaf: its return address is on top of the stack.
    pub fn unwind_leaf(c: &mut Context) {
        // SAFETY: the stack, as the caller holds it.
        c.pc = unsafe { u64_at(c.x[RSP]) };
        c.x[RSP] += 8;
    }

    /// How far back from a return address its call is, for finding the right function.
    pub const CALL_BACK: u64 = 1;
}

#[cfg(target_arch = "aarch64")]
mod arm64 {
    use super::*;
    use crate::arch::{FP, LR};

    /// RUNTIME_FUNCTION: begin, then unwind data or packed unwind data.
    pub const ENTRY: u64 = 8;

    /// # Safety
    ///
    /// `entry` is a `.pdata` entry in the image at `base`.
    pub unsafe fn function_length(base: u64, entry: u64) -> u64 {
        // SAFETY: as the caller says.
        unsafe {
            let data = u32_at(entry + 4);
            if data & 3 != 0 { u64::from(data >> 2 & 0x7ff) * 4 } else { u64::from(u32_at(base + u64::from(data)) & 0x3_ffff) * 4 }
        }
    }

    /// Unwinds one frame: `c` is the function's on the way in, its caller's on the way out. The establisher frame is
    /// what funclets reach the function's locals by: its frame pointer if its prolog set one and it's in its body, else
    /// its stack pointer (as LLVM's `eh.recoverfp` and MSVC's funclets both take it).
    ///
    /// # Safety
    ///
    /// `entry` is the function's `.pdata` entry in the image at `base`, and its stack is readable.
    pub unsafe fn virtual_unwind(base: u64, entry: u64, c: &mut Context, _handler_type: u32) -> Result<Unwound, ()> {
        // SAFETY: as the caller says.
        let (begin, data) = unsafe { (u32_at(entry), u32_at(entry + 4)) };
        let offset = (c.pc.wrapping_sub(base + u64::from(begin)) / 4) as u32;
        let unwound = if data & 3 != 0 {
            let establisher = if data >> 21 & 3 >= 2 { c.x[FP] } else { c.sp };
            // SAFETY: as above.
            unsafe { unwind_packed(data, c)? };
            Unwound { establisher, ..Unwound::default() }
        } else {
            // SAFETY: as above.
            unsafe { unwind_full(base, base + u64::from(data), offset, c)? }
        };
        c.pc = c.x[LR];
        Ok(unwound)
    }

    /// Packed unwind data, for a function in its body.
    ///
    /// # Safety
    ///
    /// The stack is readable.
    unsafe fn unwind_packed(data: u32, c: &mut Context) -> Result<(), ()> {
        let reg_f = data >> 13 & 7;
        let reg_i = (data >> 16 & 0xf) as u64;
        let homed = u64::from(data >> 20 & 1);
        let cr = data >> 21 & 3;
        let frame_size = u64::from(data >> 23 & 0x1ff) * 16;
        let int_size = reg_i * 8 + if cr == 1 { 8 } else { 0 };
        let fp_count = if reg_f > 0 { u64::from(reg_f) + 1 } else { 0 };
        let save_size = (int_size + fp_count * 8 + homed * 64).next_multiple_of(16);
        let locals = frame_size.checked_sub(save_size).ok_or(())?;
        // SAFETY: as the caller says.
        unsafe {
            if cr >= 2 {
                // A frame record at the bottom, which x29 points at.
                c.sp = c.x[FP];
                c.x[LR] = u64_at(c.sp + 8);
                c.x[FP] = u64_at(c.sp);
            }
            c.sp += locals;
            for i in 0..reg_i {
                c.x[19 + i as usize] = u64_at(c.sp + i * 8);
            }
            if cr == 1 {
                c.x[LR] = u64_at(c.sp + reg_i * 8);
            }
            for i in 0..fp_count {
                c.v[8 + i as usize] = u128::from(u64_at(c.sp + int_size + i * 8));
            }
        }
        c.sp += save_size;
        Ok(())
    }

    fn code_size(code: u8) -> usize {
        match code {
            0xc0..=0xdf | 0xe2 => 2,
            0xe0 => 4,
            0xe7 => 3,
            _ => 1,
        }
    }

    /// How many instructions the codes from `at` stand for, up to `end`.
    fn instructions(codes: &[u8], mut at: usize) -> usize {
        let mut count = 0;
        while let Some(&code) = codes.get(at) {
            if code == 0xe4 || code == 0xe5 {
                break;
            }
            count += 1;
            at += code_size(code);
        }
        count
    }

    fn skip(codes: &[u8], mut at: usize, count: usize) -> usize {
        for _ in 0..count {
            match codes.get(at) {
                Some(&code) if code != 0xe4 && code != 0xe5 => at += code_size(code),
                _ => break,
            }
        }
        at
    }

    /// Full unwind data: a header, epilog scopes, unwind codes and perhaps a handler.
    ///
    /// # Safety
    ///
    /// `xdata` is in the image, and the stack is readable.
    unsafe fn unwind_full(base: u64, xdata: u64, offset: u32, c: &mut Context) -> Result<Unwound, ()> {
        // SAFETY: as the caller says, for each read.
        unsafe {
            let header = u32_at(xdata);
            let length = header & 0x3_ffff;
            let has_handler = header & 1 << 20 != 0;
            let packed_epilog = header & 1 << 21 != 0;
            let (mut epilogs, mut words) = (header >> 22 & 0x1f, header >> 27 & 0x1f);
            let mut at = xdata + 4;
            if epilogs == 0 && words == 0 {
                let extended = u32_at(at);
                epilogs = extended & 0xffff;
                words = extended >> 16 & 0xff;
                at += 4;
            }
            let scopes = at;
            if !packed_epilog {
                at += u64::from(epilogs) * 4;
            }
            let codes = core::slice::from_raw_parts(at as *const u8, words as usize * 4);
            at += u64::from(words) * 4;
            let prolog = instructions(codes, 0);
            let mut sets_fp = false;
            let mut i = 0;
            while let Some(&code) = codes.get(i) {
                if code == 0xe4 || code == 0xe5 {
                    break;
                }
                sets_fp |= code == 0xe1 || code == 0xe2;
                i += code_size(code);
            }
            // Where to start running codes: none run if the function's at its epilog's `ret`.
            let mut start = Some(0);
            let mut in_body = true;
            if (offset as usize) < prolog {
                start = Some(skip(codes, 0, prolog - offset as usize));
                in_body = false;
            } else {
                let count = if packed_epilog { 1 } else { epilogs };
                for i in 0..count {
                    let (epilog_start, index) = if packed_epilog {
                        let index = epilogs as usize;
                        (length.saturating_sub(instructions(codes, index) as u32 + 1), index)
                    } else {
                        let word = u32_at(scopes + u64::from(i) * 4);
                        (word & 0x3_ffff, (word >> 22) as usize)
                    };
                    let n = instructions(codes, index) as u32;
                    if offset >= epilog_start && offset <= epilog_start + n {
                        let done = (offset - epilog_start) as usize;
                        start = if done as u32 == n { None } else { Some(skip(codes, index, done)) };
                        in_body = false;
                        break;
                    }
                }
            }
            let establisher = if sets_fp && in_body { c.x[FP] } else { c.sp };
            if let Some(start) = start {
                run_codes(codes, start, c)?;
            }
            let mut unwound = Unwound { establisher, ..Unwound::default() };
            if has_handler && in_body {
                unwound.handler = base + u64::from(u32_at(at));
                unwound.handler_data = at + 4;
            }
            Ok(unwound)
        }
    }

    /// Runs unwind codes from `start` to `end`, undoing what the prolog did.
    ///
    /// # Safety
    ///
    /// The stack is readable.
    unsafe fn run_codes(codes: &[u8], start: usize, c: &mut Context) -> Result<(), ()> {
        let byte = |at: usize| codes.get(at).copied().ok_or(());
        // How many more pairs the next pair-saving code covers (`save_next`).
        let mut next = 0usize;
        let mut i = start;
        // SAFETY: as the caller says, for each read of the stack.
        unsafe {
            let int_pairs = |c: &mut Context, first: usize, at: u64, pairs: usize| -> Result<(), ()> {
                for r in 0..pairs * 2 {
                    *c.x.get_mut(first + r).filter(|_| first + r < 31).ok_or(())? = u64_at(at + r as u64 * 8);
                }
                Ok(())
            };
            let fp_pairs = |c: &mut Context, first: usize, at: u64, pairs: usize| -> Result<(), ()> {
                for r in 0..pairs * 2 {
                    *c.v.get_mut(first + r).ok_or(())? = u128::from(u64_at(at + r as u64 * 8));
                }
                Ok(())
            };
            loop {
                let code = byte(i)?;
                match code {
                    // alloc_s
                    0x00..=0x1f => c.sp += u64::from(code) * 16,
                    // save_r19r20_x
                    0x20..=0x3f => {
                        int_pairs(c, 19, c.sp, 1 + next)?;
                        next = 0;
                        c.sp += u64::from(code & 0x1f) * 8;
                    }
                    // save_fplr
                    0x40..=0x7f => {
                        let at = c.sp + u64::from(code & 0x3f) * 8;
                        c.x[FP] = u64_at(at);
                        c.x[LR] = u64_at(at + 8);
                    }
                    // save_fplr_x
                    0x80..=0xbf => {
                        c.x[FP] = u64_at(c.sp);
                        c.x[LR] = u64_at(c.sp + 8);
                        c.sp += (u64::from(code & 0x3f) + 1) * 8;
                    }
                    // alloc_m
                    0xc0..=0xc7 => c.sp += (u64::from(code & 7) << 8 | u64::from(byte(i + 1)?)) * 16,
                    // save_regp, save_regp_x
                    0xc8..=0xcf => {
                        let b = byte(i + 1)?;
                        let reg = 19 + usize::from((code & 3) << 2 | b >> 6);
                        let z = u64::from(b & 0x3f);
                        if code < 0xcc {
                            int_pairs(c, reg, c.sp + z * 8, 1 + next)?;
                        } else {
                            int_pairs(c, reg, c.sp, 1 + next)?;
                            c.sp += (z + 1) * 8;
                        }
                        next = 0;
                    }
                    // save_reg
                    0xd0..=0xd3 => {
                        let b = byte(i + 1)?;
                        let reg = 19 + usize::from((code & 3) << 2 | b >> 6);
                        *c.x.get_mut(reg).filter(|_| reg < 31).ok_or(())? = u64_at(c.sp + u64::from(b & 0x3f) * 8);
                    }
                    // save_reg_x
                    0xd4 | 0xd5 => {
                        let b = byte(i + 1)?;
                        let reg = 19 + usize::from((code & 1) << 3 | b >> 5);
                        *c.x.get_mut(reg).filter(|_| reg < 31).ok_or(())? = u64_at(c.sp);
                        c.sp += (u64::from(b & 0x1f) + 1) * 8;
                    }
                    // save_lrpair
                    0xd6 | 0xd7 => {
                        let b = byte(i + 1)?;
                        let reg = 19 + 2 * usize::from((code & 1) << 2 | b >> 6);
                        let at = c.sp + u64::from(b & 0x3f) * 8;
                        *c.x.get_mut(reg).filter(|_| reg < 31).ok_or(())? = u64_at(at);
                        c.x[LR] = u64_at(at + 8);
                    }
                    // save_fregp, save_fregp_x
                    0xd8..=0xdb => {
                        let b = byte(i + 1)?;
                        let reg = 8 + usize::from((code & 1) << 2 | b >> 6);
                        let z = u64::from(b & 0x3f);
                        if code < 0xda {
                            fp_pairs(c, reg, c.sp + z * 8, 1 + next)?;
                        } else {
                            fp_pairs(c, reg, c.sp, 1 + next)?;
                            c.sp += (z + 1) * 8;
                        }
                        next = 0;
                    }
                    // save_freg
                    0xdc | 0xdd => {
                        let b = byte(i + 1)?;
                        let reg = 8 + usize::from((code & 1) << 2 | b >> 6);
                        *c.v.get_mut(reg).ok_or(())? = u128::from(u64_at(c.sp + u64::from(b & 0x3f) * 8));
                    }
                    // save_freg_x
                    0xde => {
                        let b = byte(i + 1)?;
                        c.v[8 + usize::from(b >> 5)] = u128::from(u64_at(c.sp));
                        c.sp += (u64::from(b & 0x1f) + 1) * 8;
                    }
                    // alloc_l
                    0xe0 => c.sp += (u64::from(byte(i + 1)?) << 16 | u64::from(byte(i + 2)?) << 8 | u64::from(byte(i + 3)?)) * 16,
                    // set_fp, add_fp
                    0xe1 => c.sp = c.x[FP],
                    0xe2 => c.sp = c.x[FP].wrapping_sub(u64::from(byte(i + 1)?) * 8),
                    // nop, pac_sign_lr, clear_unwound_to_call
                    0xe3 | 0xfc | 0xeb => {}
                    // end, end_c
                    0xe4 | 0xe5 => return Ok(()),
                    // save_next
                    0xe6 => next += 1,
                    _ => return Err(()),
                }
                i += code_size(code);
            }
        }
    }

    /// A function with no `.pdata` entry is a leaf: it returns to its link register.
    pub fn unwind_leaf(c: &mut Context) {
        c.pc = c.x[LR];
    }

    /// How far back from a return address its call is.
    pub const CALL_BACK: u64 = 4;
}
