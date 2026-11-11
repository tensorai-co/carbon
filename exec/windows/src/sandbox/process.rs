//! The stub process: starting it locked down, keeping its memory matching guest memory, and its debug events.
//!
//! Windows ties debugging to the thread that started the process, which is the thread that opens the sandbox and owns
//! its one CPU: every debug call happens there. Mapping memory and moving registers need only the handles.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;
use std::ops::Range;
use std::sync::{Mutex, PoisonError};

use exec_core::{PAGE, Prot};

use super::ram::SLOT;
use super::stub;
use super::sys::*;

/// Programs live below this: Windows puts its own DLLs above it, and the stub is above it too.
pub const SPACE_TOP: u64 = 0x7000_0000_0000;

pub struct Process {
    pub process: Handle,
    pub thread: Handle,
    /// The stub's process ID: this thread debugs every stub it starts, and their events all come here.
    pub pid: u32,
    pub tid: u32,
    /// The stub thread's TEB, which Windows places and owns, and where a program's thread block has to be.
    pub teb: u64,
    /// What Windows has in the stub below `SPACE_TOP`, in whole 64 KiB blocks, which programs must stay out of.
    pub kept: Vec<Range<u64>>,
    job: Handle,
    section: Handle,
    /// Carbon's view of guest memory, for pages that have to be copied.
    ram: u64,
    state: Mutex<State>,
}

#[derive(Default)]
pub struct State {
    /// A debug event the stub is stopped at, to continue before it runs again.
    pub pending: Option<(u32, u32)>,
    /// The thread was suspended to preempt it, rather than stopped at an event.
    pub suspended: bool,
    /// Guest pages mapped as views, by address.
    views: BTreeSet<u64>,
    /// 64 KiB blocks reserved as placeholders for views.
    placeholders: BTreeSet<u64>,
    /// Guest pages that land in Windows' own memory (the TEB), by address, with their offset in guest memory: copied
    /// in before every run and out after every stop.
    copied: BTreeMap<u64, u64>,
}

fn check(ok: BOOL, what: &str) -> Result<(), String> {
    if ok == 0 { Err(format!("{what}: {}", last_error())) } else { Ok(()) }
}

/// The calling user's token, without its privileges and at low integrity, for the stub.
fn restricted_token() -> Result<Handle, String> {
    let mut own: HANDLE = std::ptr::null_mut();
    let access = TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT;
    // SAFETY: this process's own token, into an out-parameter.
    check(unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut own) }, "opening Carbon's token")?;
    let mut restricted: HANDLE = std::ptr::null_mut();
    let null = std::ptr::null();
    // SAFETY: the token just opened, and an out-parameter.
    let made = unsafe { CreateRestrictedToken(own, DISABLE_MAX_PRIVILEGE | LUA_TOKEN, 0, null, 0, null, 0, null, &mut restricted) };
    // SAFETY: the token opened above.
    unsafe { CloseHandle(own) };
    check(made, "restricting the stub's token")?;

    // Low integrity, S-1-16-4096: it can't write anything at medium integrity, which is nearly everything.
    let sid: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 16, 0x00, 0x10, 0, 0];
    #[repr(C)]
    struct Label {
        sid: *const u8,
        attributes: u32,
    }
    let label = Label { sid: sid.as_ptr(), attributes: SE_GROUP_INTEGRITY };
    // SAFETY: a TOKEN_MANDATORY_LABEL whose SID outlives the call.
    let set = unsafe { SetTokenInformation(restricted, TOKEN_INTEGRITY_LEVEL, (&raw const label).cast(), (size_of::<Label>() + sid.len()) as u32) };
    if set == 0 {
        // SAFETY: the token made above.
        unsafe { CloseHandle(restricted) };
        return Err(format!("lowering the stub's integrity: {}", last_error()));
    }
    Ok(Handle(restricted))
}

/// A job that holds the stub alone: no other process, no UI, and the stub dies when Carbon lets go of it.
fn job() -> Result<Handle, String> {
    // SAFETY: an anonymous job.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(format!("making the stub's job: {}", last_error()));
    }
    let limits = JobLimits {
        flags: JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
        active_processes: 1,
        ..JobLimits::default()
    };
    let ui = JOB_OBJECT_UILIMIT_ALL;
    // SAFETY: the job just made, and structures of the sizes given.
    let ok = unsafe {
        SetInformationJobObject(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION, (&raw const limits).cast(), size_of::<JobLimits>() as u32) != 0
            && SetInformationJobObject(job, JOB_OBJECT_BASIC_UI_RESTRICTIONS, (&raw const ui).cast(), 4) != 0
    };
    if !ok {
        let error = last_error();
        // SAFETY: as above.
        unsafe { CloseHandle(job) };
        return Err(format!("limiting the stub's job: {error}"));
    }
    Ok(Handle(job))
}

impl Process {
    /// Starts the stub, locked down, and runs its loader to the first breakpoint, where it stops for Carbon.
    pub fn spawn(section: HANDLE, ram: u64) -> Result<Process, String> {
        let path = stub::path()?;
        let token = restricted_token()?;
        let job = match job() {
            Ok(job) => job,
            Err(why) => {
                // SAFETY: the token made above.
                unsafe { CloseHandle(token.0) };
                return Err(why);
            }
        };
        let path = wide(&path.to_string_lossy());
        let si = StartupInfo { cb: size_of::<StartupInfo>() as u32, rest: [0; 100] };
        let mut pi = ProcessInformation { process: std::ptr::null_mut(), thread: std::ptr::null_mut(), pid: 0, tid: 0 };
        let flags = CREATE_SUSPENDED | DEBUG_ONLY_THIS_PROCESS | CREATE_NO_WINDOW;
        let null = std::ptr::null::<c_void>();
        // SAFETY: a valid path, token and out-parameters. This thread becomes the stub's debugger.
        let made = unsafe { CreateProcessAsUserW(token.0, path.as_ptr(), std::ptr::null_mut(), null, null, 0, flags, null, std::ptr::null(), &si, &mut pi) };
        // SAFETY: the token, which the process has its own copy of now.
        unsafe { CloseHandle(token.0) };
        if made == 0 {
            let error = last_error();
            // SAFETY: the job made above.
            unsafe { CloseHandle(job.0) };
            return Err(format!("starting the stub: {error}"));
        }
        let mut process = Process {
            process: Handle(pi.process),
            thread: Handle(pi.thread),
            pid: pi.pid,
            tid: pi.tid,
            teb: 0,
            kept: Vec::new(),
            job,
            section: Handle(section),
            ram,
            state: Mutex::new(State::default()),
        };
        // SAFETY: the stub, suspended, and the job; the stub must never outlive Carbon's thread.
        unsafe {
            check(AssignProcessToJobObject(job.0, pi.process), "putting the stub in its job")?;
            DebugSetProcessKillOnExit(1);
            ResumeThread(pi.thread);
        }
        process.run_loader()?;
        process.teb = process.thread_teb()?;
        process.kept = process.windows_memory();
        Ok(process)
    }

    /// Pumps the loader's events until its breakpoint, where the process is whole and its main thread is stopped.
    fn run_loader(&mut self) -> Result<(), String> {
        let mut event = DebugEvent::new();
        loop {
            // SAFETY: an out-parameter for the next event.
            check(unsafe { WaitForDebugEventEx(&mut event, INFINITE) }, "waiting for the stub to start")?;
            if event.pid != self.pid {
                // An earlier stub's, still on its way out.
                // SAFETY: continuing it, so it can go.
                unsafe { ContinueDebugEvent(event.pid, event.tid, DBG_CONTINUE) };
                continue;
            }
            match event.code {
                CREATE_PROCESS_DEBUG_EVENT => {
                    // SAFETY: the image's file handle, which the debugger closes.
                    unsafe { CloseHandle(event.handle()) };
                    if event.image_base() != stub::BASE {
                        return Err(format!("the stub loaded at {:#x}, not {:#x}", event.image_base(), stub::BASE));
                    }
                }
                LOAD_DLL_DEBUG_EVENT => {
                    // SAFETY: as above.
                    unsafe { CloseHandle(event.handle()) };
                }
                EXCEPTION_DEBUG_EVENT if event.exception_code() == STATUS_BREAKPOINT && event.tid == self.tid => {
                    self.lock().pending = Some((event.pid, event.tid));
                    return Ok(());
                }
                EXIT_PROCESS_DEBUG_EVENT => return Err("the stub ended before it was ready".to_string()),
                _ => {}
            }
            // SAFETY: continuing the event just handled.
            unsafe { ContinueDebugEvent(event.pid, event.tid, DBG_CONTINUE) };
        }
    }

    fn thread_teb(&self) -> Result<u64, String> {
        let mut info = [0u64; 6];
        let mut returned = 0;
        // SAFETY: THREAD_BASIC_INFORMATION (48 bytes), into room for it.
        let status = unsafe { NtQueryInformationThread(self.thread.0, 0, info.as_mut_ptr().cast(), 48, &mut returned) };
        if status < 0 {
            return Err(format!("finding the stub's TEB: status {status:#x}"));
        }
        Ok(info[1])
    }

    /// Everything Windows has below `SPACE_TOP`, widened to whole 64 KiB blocks.
    fn windows_memory(&self) -> Vec<Range<u64>> {
        let mut kept: Vec<Range<u64>> = Vec::new();
        let mut at = 0u64;
        while at < SPACE_TOP {
            let mut info = MemoryBasicInformation { base: 0, allocation_base: 0, allocation_protect: 0, partition: 0, size: 0, state: 0, protect: 0, kind: 0 };
            // SAFETY: an out-parameter of the right size.
            let got = unsafe { VirtualQueryEx(self.process.0, at, &mut info, size_of::<MemoryBasicInformation>()) };
            if got == 0 || info.size == 0 {
                break;
            }
            if info.state != MEM_FREE {
                let range = (info.base & !(SLOT - 1))..(info.base + info.size).next_multiple_of(SLOT).min(SPACE_TOP);
                match kept.last_mut() {
                    Some(last) if last.end >= range.start => last.end = last.end.max(range.end),
                    _ => kept.push(range),
                }
            }
            at = info.base + info.size;
        }
        kept
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_windows(&self, page: u64) -> bool {
        self.kept.iter().any(|range| range.contains(&page))
    }

    /// Makes the stub's page at `page` show guest memory at `offset`, with `prot`, or nothing.
    pub fn mirror(&self, page: u64, offset: Option<u64>, prot: Prot) -> Result<(), String> {
        if page >= SPACE_TOP {
            return Err(format!("{page:#x} is above where programs may be"));
        }
        let mut state = self.lock();
        if self.is_windows(page) {
            match offset {
                Some(offset) => state.copied.insert(page, offset),
                None => state.copied.remove(&page),
            };
            return Ok(());
        }
        if state.views.remove(&page) {
            // SAFETY: a view this mapped, back to a placeholder.
            check(unsafe { UnmapViewOfFile2(self.process.0, page, MEM_PRESERVE_PLACEHOLDER) }, "unmapping a guest page")?;
        }
        let Some(offset) = offset else { return Ok(()) };
        let block = page & !(SLOT - 1);
        if state.placeholders.insert(block) {
            // SAFETY: reserving a free block of the stub's address space as a placeholder.
            let at = unsafe { VirtualAlloc2(self.process.0, block, SLOT as usize, MEM_RESERVE | MEM_RESERVE_PLACEHOLDER, PAGE_NOACCESS, std::ptr::null_mut(), 0) };
            if at != block {
                state.placeholders.remove(&block);
                return Err(format!("reserving {block:#x} in the stub: {}", last_error()));
            }
        }
        // A placeholder of its own for this page, split out of its block's. If it already is one, this does nothing.
        // SAFETY: within a placeholder this reserved.
        unsafe { VirtualFreeEx(self.process.0, page, PAGE as usize, MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER) };
        // SAFETY: one page of the guest section, at its frame's 64 KiB-aligned offset, over that placeholder.
        let at = unsafe { MapViewOfFile3(self.section.0, self.process.0, page, offset, PAGE as usize, MEM_REPLACE_PLACEHOLDER, protection(prot), std::ptr::null_mut(), 0) };
        if at != page {
            return Err(format!("mapping a guest page at {page:#x}: {}", last_error()));
        }
        state.views.insert(page);
        Ok(())
    }

    /// Guest pages in Windows' memory, from guest memory into the stub, before it runs.
    pub fn copy_in(&self) {
        for (&page, &offset) in &self.lock().copied {
            let mut done = 0;
            // SAFETY: a page of Carbon's view of guest memory, into a page of the stub's that Windows keeps writable.
            unsafe { WriteProcessMemory(self.process.0, page, (self.ram + offset) as *const c_void, PAGE as usize, &mut done) };
        }
    }

    /// The other way, after it stops.
    pub fn copy_out(&self) {
        for (&page, &offset) in &self.lock().copied {
            let mut done = 0;
            // SAFETY: as above, the other way.
            unsafe { ReadProcessMemory(self.process.0, page, (self.ram + offset) as *mut c_void, PAGE as usize, &mut done) };
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // Let go of it as its debugger first: otherwise its last events stay queued for this thread, where the next
        // stub it starts would find them.
        let pending = self.state.lock().unwrap_or_else(|p| p.into_inner()).pending.take();
        // SAFETY: the stub and its handles, which nothing uses any more. Closing the job would end it anyway.
        unsafe {
            if let Some((pid, tid)) = pending {
                ContinueDebugEvent(pid, tid, DBG_CONTINUE);
            }
            DebugActiveProcessStop(self.pid);
            TerminateProcess(self.process.0, 0);
            CloseHandle(self.thread.0);
            CloseHandle(self.process.0);
            CloseHandle(self.job.0);
        }
    }
}

/// Windows' protection for what a program may do with a page.
fn protection(prot: Prot) -> u32 {
    match (prot.read() || prot.write(), prot.write(), prot.exec()) {
        (_, true, true) => PAGE_EXECUTE_READWRITE,
        (_, true, false) => PAGE_READWRITE,
        (true, false, true) => PAGE_EXECUTE_READ,
        (false, false, true) => PAGE_EXECUTE,
        (true, false, false) => PAGE_READONLY,
        (false, false, false) => PAGE_NOACCESS,
    }
}
