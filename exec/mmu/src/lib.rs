//! Guest page tables, for x86-64 and ARM64, which both walk 4-level tables of 4 KiB pages with 48-bit addresses.
//!
//! Guest memory is one host mapping. A frame is named by its guest physical address, from `base`; its bytes are in the
//! host at `ram + index * stride`, where `stride` is a page unless the host can only map larger pieces (a sandbox on
//! Windows spaces frames 64 KiB apart). What an entry's bits mean is the `Format`'s. Making CPUs forget old entries is
//! the caller's: anything that takes access away sets `needs_flush`.

pub mod arm64;
mod frames;
pub mod x86;

use std::marker::PhantomData;
use std::ops::{BitOr, Range};
use std::sync::atomic::{Ordering, fence};

pub use frames::{Frames, Run};

pub const PAGE: u64 = 4096;
const ENTRIES: u64 = 512;
const LEVELS: u32 = 4;

/// What a program may do with a page.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Prot(u8);

impl Prot {
    pub const NONE: Prot = Prot(0);
    pub const R: Prot = Prot(1);
    pub const W: Prot = Prot(2);
    pub const X: Prot = Prot(4);
    pub const RW: Prot = Prot(3);
    pub const RX: Prot = Prot(5);
    pub const RWX: Prot = Prot(7);

    pub const fn read(self) -> bool {
        self.0 & 1 != 0
    }
    pub const fn write(self) -> bool {
        self.0 & 2 != 0
    }
    pub const fn exec(self) -> bool {
        self.0 & 4 != 0
    }
    pub const fn contains(self, other: Prot) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn bits(self) -> u8 {
        self.0
    }
}

impl BitOr for Prot {
    type Output = Prot;
    fn bitor(self, other: Prot) -> Prot {
        Prot(self.0 | other.0)
    }
}

impl std::fmt::Debug for Prot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let flag = |on: bool, c: char| if on { c } else { '-' };
        write!(f, "{}{}{}", flag(self.read(), 'r'), flag(self.write(), 'w'), flag(self.exec(), 'x'))
    }
}

/// A frame of guest memory, by its guest physical address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Frame(pub u64);

/// There are no free frames left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfMemory;

/// Nothing usable is mapped at this address, so a copy stopped there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unmapped(pub u64);

/// What an entry's bits mean on one CPU type.
pub trait Format: 'static {
    /// The frame an entry points to.
    const ADDRESS: u64;
    /// Software bit: the page is shared copy-on-write, so read-only until it's copied.
    const SHARED: u64;
    /// Software bit: the frame is the kernel's (`MAP_SHARED`), so a fork keeps it shared and writable.
    const KERNEL_FRAME: u64;
    /// How many of a root's entries are the program's. The rest are the shim's, the same in every space.
    const USER_ENTRIES: u64;

    fn is_valid(entry: u64) -> bool;
    /// An entry for a table at the next level.
    fn table(frame: u64) -> u64;
    /// An entry for a program's page.
    fn page(frame: u64, prot: Prot, shared: bool) -> u64;
    /// What the hardware lets the program do with it now.
    fn prot(entry: u64) -> Prot;
}

/// The format of the CPU this was built for.
#[cfg(target_arch = "aarch64")]
pub type Native = arm64::Arm64;
#[cfg(not(target_arch = "aarch64"))]
pub type Native = x86::X86;

/// Guest memory and every space's tables in it.
pub struct Mmu<F: Format> {
    ram: *mut u8,
    base: u64,
    size: u64,
    stride: u64,
    pub frames: Frames,
    /// Root entries every new space starts with: the shim's.
    kernel: Vec<(u64, u64)>,
    /// Something lost access, or went, since the last flush.
    pub needs_flush: bool,
    _format: PhantomData<F>,
}

// SAFETY: the memory is owned by whoever owns the Mmu, and only used through it.
unsafe impl<F: Format> Send for Mmu<F> {}

fn index_at(va: u64, level: u32) -> u64 {
    (va >> (12 + 9 * (LEVELS - 1 - level))) & (ENTRIES - 1)
}

fn span(level: u32) -> u64 {
    1 << (12 + 9 * (LEVELS - 1 - level))
}

impl<F: Format> Mmu<F> {
    /// `size` bytes of guest memory from guest physical `base`, at `ram` in the host, a frame every `stride` bytes,
    /// backed by the host in granules of `granule` bytes.
    pub fn new(ram: *mut u8, base: u64, size: u64, stride: u64, granule: u64) -> Mmu<F> {
        let stride = stride.max(PAGE);
        let pages = size / stride;
        // How many frames share one host granule, as guest bytes.
        let granule_frames = (granule.max(stride) / stride).max(1);
        Mmu {
            ram,
            base,
            size: pages * PAGE,
            stride,
            frames: Frames::new(base, pages * PAGE, granule_frames * PAGE),
            kernel: Vec::new(),
            needs_flush: false,
            _format: PhantomData,
        }
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    /// Guest memory in bytes, as frames.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Where a frame's bytes are in the host.
    pub fn host(&self, frame: Frame) -> *mut u8 {
        debug_assert!(self.frames.contains(frame));
        // SAFETY: within guest memory, as frames are.
        unsafe { self.ram.add(((frame.0 - self.base) / PAGE * self.stride) as usize) }
    }

    /// Where a frame is in guest memory's host mapping, in bytes.
    pub fn offset(&self, frame: Frame) -> u64 {
        (frame.0 - self.base) / PAGE * self.stride
    }

    /// A zeroed frame, with one holder.
    pub fn new_frame(&mut self) -> Result<Frame, OutOfMemory> {
        let frame = self.frames.take()?;
        // SAFETY: the whole of a frame just taken.
        unsafe { self.host(frame).write_bytes(0, PAGE as usize) };
        Ok(frame)
    }

    pub fn hold(&mut self, frame: Frame) {
        self.frames.hold(frame);
    }

    pub fn release(&mut self, frame: Frame) -> bool {
        self.frames.release(frame)
    }

    pub fn refs(&self, frame: Frame) -> u32 {
        self.frames.refs(frame)
    }

    fn slot(&self, table: u64, index: u64) -> *mut u64 {
        // SAFETY: a table is a frame of 512 entries.
        unsafe { self.host(Frame(table)).cast::<u64>().add(index as usize) }
    }

    fn read_slot(&self, table: u64, index: u64) -> u64 {
        // SAFETY: an entry in a table.
        unsafe { self.slot(table, index).read_volatile() }
    }

    fn write_slot(&self, table: u64, index: u64, entry: u64) {
        // SAFETY: an entry in a table.
        unsafe { self.slot(table, index).write_volatile(entry) };
    }

    /// Sets a root entry that every space made from now on gets, for the shim's own pages.
    pub fn set_kernel_entry(&mut self, index: u64, entry: u64) {
        self.kernel.retain(|(i, _)| *i != index);
        self.kernel.push((index, entry));
    }

    /// A new, empty space.
    pub fn new_root(&mut self) -> Result<u64, OutOfMemory> {
        let root = self.new_frame()?.0;
        for &(index, entry) in &self.kernel {
            self.write_slot(root, index, entry);
        }
        Ok(root)
    }

    /// The entry for `va`, making tables on the way if `create`.
    pub fn entry(&mut self, root: u64, va: u64, create: bool) -> Result<Option<*mut u64>, OutOfMemory> {
        let mut table = root;
        for level in 0..LEVELS - 1 {
            let index = index_at(va, level);
            let entry = self.read_slot(table, index);
            if F::is_valid(entry) {
                table = entry & F::ADDRESS;
            } else if create {
                let next = self.new_frame()?.0;
                // The table must be zeroed before anything can walk into it.
                fence(Ordering::Release);
                self.write_slot(table, index, F::table(next));
                table = next;
            } else {
                return Ok(None);
            }
        }
        Ok(Some(self.slot(table, index_at(va, LEVELS - 1))))
    }

    /// The entry for `va`, or 0.
    pub fn descriptor(&self, root: u64, va: u64) -> u64 {
        let mut table = root;
        for level in 0..LEVELS - 1 {
            let entry = self.read_slot(table, index_at(va, level));
            if !F::is_valid(entry) {
                return 0;
            }
            table = entry & F::ADDRESS;
        }
        self.read_slot(table, index_at(va, LEVELS - 1))
    }

    /// Calls `f` with each valid page entry in `range`, skipping over tables that aren't there.
    pub fn for_each(&mut self, root: u64, range: Range<u64>, f: &mut dyn FnMut(&mut Self, u64, *mut u64)) {
        self.walk_range(root, 0, 0, range.start, range.end, f);
    }

    fn walk_range(&mut self, table: u64, level: u32, base: u64, start: u64, end: u64, f: &mut dyn FnMut(&mut Self, u64, *mut u64)) {
        let size = span(level);
        let entries = if level == 0 { F::USER_ENTRIES } else { ENTRIES };
        for index in 0..entries {
            let at = base + index * size;
            if at + size <= start || at >= end {
                continue;
            }
            let entry = self.read_slot(table, index);
            if !F::is_valid(entry) {
                continue;
            }
            if level == LEVELS - 1 {
                let slot = self.slot(table, index);
                f(self, at, slot);
            } else {
                self.walk_range(entry & F::ADDRESS, level + 1, at, start, end, f);
            }
        }
    }

    /// Calls `each(va, entry)` for each valid page entry in `range`, without changing anything.
    pub fn visit(&self, root: u64, range: Range<u64>, each: &mut dyn FnMut(u64, u64)) {
        self.visit_range(root, 0, 0, range.start, range.end, each);
    }

    fn visit_range(&self, table: u64, level: u32, base: u64, start: u64, end: u64, each: &mut dyn FnMut(u64, u64)) {
        let size = span(level);
        let entries = if level == 0 { F::USER_ENTRIES } else { ENTRIES };
        for index in 0..entries {
            let at = base + index * size;
            if at + size <= start || at >= end {
                continue;
            }
            let entry = self.read_slot(table, index);
            if !F::is_valid(entry) {
                continue;
            }
            if level == LEVELS - 1 {
                each(at, entry);
            } else {
                self.visit_range(entry & F::ADDRESS, level + 1, at, start, end, each);
            }
        }
    }

    /// Calls `each(va, frame, entry)` for every page in a space, and counts its tables into `tables`.
    pub fn walk(&self, root: u64, each: &mut dyn FnMut(u64, Frame, u64), tables: &mut u64) {
        self.walk_table(root, 0, 0, each, tables);
    }

    fn walk_table(&self, table: u64, level: u32, base: u64, each: &mut dyn FnMut(u64, Frame, u64), tables: &mut u64) {
        *tables += 1;
        let entries = if level == 0 { F::USER_ENTRIES } else { ENTRIES };
        for index in 0..entries {
            let entry = self.read_slot(table, index);
            if !F::is_valid(entry) {
                continue;
            }
            let at = base + index * span(level);
            if level == LEVELS - 1 {
                each(at, Frame(entry & F::ADDRESS), entry);
            } else {
                self.walk_table(entry & F::ADDRESS, level + 1, at, each, tables);
            }
        }
    }

    /// Maps one page to `frame`, which gains a holder. Whatever was there is dropped.
    pub fn map(&mut self, root: u64, va: u64, frame: Frame, prot: Prot, shared: bool, kernel_frame: bool) -> Result<(), OutOfMemory> {
        let slot = self.entry(root, va, true)?.ok_or(OutOfMemory)?;
        // SAFETY: an entry in a table.
        let old = unsafe { slot.read_volatile() };
        let mut entry = F::page(frame.0, prot, shared);
        if kernel_frame {
            entry |= F::KERNEL_FRAME;
        }
        // The frame's contents must be visible before the entry is.
        fence(Ordering::Release);
        // SAFETY: as above.
        unsafe { slot.write_volatile(entry) };
        if F::is_valid(old) {
            self.release(Frame(old & F::ADDRESS));
            self.needs_flush = true;
        }
        Ok(())
    }

    /// Drops every page in `range`.
    pub fn unmap(&mut self, root: u64, range: Range<u64>) {
        self.for_each(root, range, &mut |mmu, _, slot| {
            // SAFETY: for_each gives entries in tables.
            let entry = unsafe { slot.read_volatile() };
            // SAFETY: as above.
            unsafe { slot.write_volatile(0) };
            mmu.release(Frame(entry & F::ADDRESS));
            mmu.needs_flush = true;
        });
    }

    /// Changes what the program may do with each page in `range`, keeping shared pages read-only.
    pub fn protect(&mut self, root: u64, range: Range<u64>, prot: Prot) {
        self.for_each(root, range, &mut |mmu, _, slot| {
            // SAFETY: for_each gives entries in tables.
            let entry = unsafe { slot.read_volatile() };
            let updated = F::page(entry & F::ADDRESS, prot, entry & F::SHARED != 0) | (entry & F::KERNEL_FRAME);
            // SAFETY: as above.
            unsafe { slot.write_volatile(updated) };
            mmu.needs_flush = true;
        });
    }

    /// A copy of a space, sharing every page until either side writes to it.
    pub fn fork(&mut self, root: u64) -> Result<u64, OutOfMemory> {
        let copy = self.copy_table(root, 0)?;
        for &(index, entry) in &self.kernel {
            self.write_slot(copy, index, entry);
        }
        Ok(copy)
    }

    fn copy_table(&mut self, table: u64, level: u32) -> Result<u64, OutOfMemory> {
        let copy = self.new_frame()?.0;
        let entries = if level == 0 { F::USER_ENTRIES } else { ENTRIES };
        for index in 0..entries {
            let entry = self.read_slot(table, index);
            if !F::is_valid(entry) {
                continue;
            }
            if level < LEVELS - 1 {
                let child = self.copy_table(entry & F::ADDRESS, level + 1)?;
                self.write_slot(copy, index, F::table(child));
                continue;
            }
            let frame = Frame(entry & F::ADDRESS);
            self.hold(frame);
            let shared = if entry & F::KERNEL_FRAME != 0 || entry & F::SHARED != 0 {
                entry
            } else {
                // Both sides lose write access until one of them copies the page.
                let shared = F::page(frame.0, F::prot(entry), true);
                self.write_slot(table, index, shared);
                self.needs_flush = true;
                shared
            };
            self.write_slot(copy, index, shared);
        }
        Ok(copy)
    }

    /// Frees a space: its tables, and its hold on every page.
    pub fn free_root(&mut self, root: u64) {
        self.free_table(root, 0);
        self.needs_flush = true;
    }

    fn free_table(&mut self, table: u64, level: u32) {
        let entries = if level == 0 { F::USER_ENTRIES } else { ENTRIES };
        for index in 0..entries {
            let entry = self.read_slot(table, index);
            if !F::is_valid(entry) {
                continue;
            }
            if level < LEVELS - 1 {
                self.free_table(entry & F::ADDRESS, level + 1);
            } else {
                self.release(Frame(entry & F::ADDRESS));
            }
        }
        self.release(Frame(table));
    }

    /// Gives this space its own copy of a shared page, writable if its access allows. False if it wasn't shared.
    pub fn make_writable(&mut self, root: u64, page: u64, prot: Prot) -> Result<bool, OutOfMemory> {
        let Some(slot) = self.entry(root, page, false)? else { return Ok(false) };
        // SAFETY: an entry in a table.
        let entry = unsafe { slot.read_volatile() };
        if !F::is_valid(entry) || entry & F::SHARED == 0 {
            return Ok(false);
        }
        let frame = Frame(entry & F::ADDRESS);
        let mine = if self.refs(frame) == 1 {
            frame
        } else {
            let copy = self.new_frame()?;
            // SAFETY: two distinct frames, a page each.
            unsafe { std::ptr::copy_nonoverlapping(self.host(frame), self.host(copy), PAGE as usize) };
            self.release(frame);
            copy
        };
        fence(Ordering::Release);
        // SAFETY: as above.
        unsafe { slot.write_volatile(F::page(mine.0, prot, false)) };
        self.needs_flush = true;
        Ok(true)
    }

    /// Moves the pages in `[from, from + len)` to the same offsets from `to`, dropping what was there.
    pub fn move_pages(&mut self, root: u64, from: u64, to: u64, len: u64) -> Result<(), OutOfMemory> {
        let mut moving = Vec::new();
        self.for_each(root, from..from + len, &mut |_, va, slot| {
            // SAFETY: for_each gives entries in tables.
            moving.push((va - from, unsafe { slot.read_volatile() }));
            // SAFETY: as above.
            unsafe { slot.write_volatile(0) };
        });
        self.unmap(root, to..to + len);
        for (offset, entry) in moving {
            let slot = self.entry(root, to + offset, true)?.ok_or(OutOfMemory)?;
            // SAFETY: an entry in a table.
            unsafe { slot.write_volatile(entry) };
        }
        self.needs_flush = true;
        Ok(())
    }

    /// Calls `f(host, len, done)` for each piece of `[addr, addr + len)`. A write stops at a shared page, as the
    /// caller has to decide whether to copy it first.
    pub fn copy(&self, root: u64, addr: u64, len: usize, writing: bool, f: &mut dyn FnMut(*mut u8, usize, usize)) -> Result<(), Unmapped> {
        let mut done = 0;
        while done < len {
            let at = addr + done as u64;
            let entry = self.descriptor(root, at);
            if !F::is_valid(entry) || (writing && entry & F::SHARED != 0) {
                return Err(Unmapped(at));
            }
            let offset = at % PAGE;
            let part = ((PAGE - offset) as usize).min(len - done);
            // SAFETY: within the page's frame.
            f(unsafe { self.host(Frame(entry & F::ADDRESS)).add(offset as usize) }, part, done);
            done += part;
        }
        Ok(())
    }

    /// Where an aligned value is in the host, for atomic access.
    pub fn value_at(&self, root: u64, addr: u64, writing: bool) -> Result<*mut u8, Unmapped> {
        let entry = self.descriptor(root, addr);
        if !F::is_valid(entry) || (writing && entry & F::SHARED != 0) {
            return Err(Unmapped(addr));
        }
        // SAFETY: within the page's frame.
        Ok(unsafe { self.host(Frame(entry & F::ADDRESS)).add((addr % PAGE) as usize) })
    }

    pub fn frame_read(&self, frame: Frame, offset: usize, buffer: &mut [u8]) {
        let len = buffer.len().min(PAGE as usize - offset.min(PAGE as usize));
        // SAFETY: within the frame.
        unsafe { std::ptr::copy_nonoverlapping(self.host(frame).add(offset), buffer.as_mut_ptr(), len) };
    }

    pub fn frame_write(&self, frame: Frame, offset: usize, data: &[u8]) {
        let len = data.len().min(PAGE as usize - offset.min(PAGE as usize));
        // SAFETY: within the frame.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), self.host(frame).add(offset), len) };
    }

    /// A hash of a frame's contents, 0 if it's all zeroes.
    pub fn hash_frame(&self, frame: Frame) -> u64 {
        let mut page = [0u8; PAGE as usize];
        self.frame_read(frame, 0, &mut page);
        hash_of_page(&page)
    }
}

/// FNV-1a over a page, 0 for a page of zeroes, so zero pages can be counted.
pub fn hash_of_page(page: &[u8]) -> u64 {
    if page.iter().all(|b| *b == 0) {
        return 0;
    }
    page.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, b| (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)).max(1)
}
