//! `execve`: loading a program, its interpreter, and its first stack into a new space.

use std::sync::Arc;

use exec::{Isa, PAGE, Prot, Turn};
use kernels_core::frame::StackWriter;
use kernels_core::lock;
use kernels_core::mm::Regions;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::elf::{self, ET_DYN, Elf};
use super::{Outcome, ProcState, Task};
use crate::Kernel;
use crate::abi::*;
use crate::mm::{Area, Backing, page_down, page_up};

/// How deep `#!` scripts may nest.
const MOST_SCRIPTS: usize = 4;
/// The stack's size, all of which is the program's to fault in.
const STACK: u64 = 8 << 20;
/// How much of the stack is mapped up front, for the arguments and the first frames.
const STACK_MAPPED: u64 = 256 << 10;
/// Where `mmap` stops, below the stack.
const STACK_GAP: u64 = 1 << 30;
/// The lowest address `mmap` gives out: above the macOS sandbox's empty bottom 4 GiB.
const MMAP_BOTTOM: u64 = 1 << 32;
/// Where a position-independent program goes, if the space reaches that high.
const PIE_BASE: u64 = 0x5555_5555_0000;

impl Task {
    pub(super) fn execve(&mut self, cx: &mut Cx<'_>, a: [u64; 6]) -> Outcome {
        let read = self.mem(cx, false, |t| {
            let space = t.space();
            let width = 8;
            let path = user::read_cstr(&space, a[0], 4096)?;
            let strings = |list: u64| -> Result<Vec<String>, user::Fault> {
                user::read_ptr_list(&space, list, width, 1 << 16)?
                    .into_iter()
                    .map(|p| Ok(String::from_utf8_lossy(&user::read_cstr(&space, p, user::MOST_STRING)?).into_owned()))
                    .collect()
            };
            Ok((String::from_utf8_lossy(&path).into_owned(), strings(a[1])?, strings(a[2])?))
        });
        let (path, argv, env) = match read {
            Ok(r) => r,
            Err(e) => return Err(e).into(),
        };
        let mut state = lock(&self.process.state);
        if state.threads.len() > 1 {
            // Ending the other threads first isn't done yet.
            return Err(EAGAIN).into();
        }
        match exec_into(&self.k, cx, &mut state, &path, argv, env) {
            Ok(()) => {
                self.isa = state.isa;
                self.clear_tid = 0;
                self.altstack = None;
                self.saved_fp.clear();
                Outcome::Keep
            }
            Err(e) => Err(e).into(),
        }
    }
}

/// The program at `path`'s absolute path, from the process's working directory.
pub(crate) fn exe_path(cwd: &str, path: &str) -> String {
    let joined = if path.starts_with('/') { path.to_string() } else { format!("{}/{path}", cwd.trim_end_matches('/')) };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Replaces the process's program with the one at `path`: a new space, its segments, its interpreter's, and a
/// stack with its arguments. The old space goes. On failure, the process is as it was.
pub(crate) fn exec_into(k: &Arc<Kernel>, cx: &mut Cx<'_>, state: &mut ProcState, path: &str, mut argv: Vec<String>, env: Vec<String>) -> Result<(), Errno> {
    let mut path = path.to_string();
    // Follow `#!` lines to the real program.
    let mut bytes = read_file(k, state, &path)?;
    for _ in 0..MOST_SCRIPTS {
        let Some(line) = bytes.strip_prefix(b"#!") else { break };
        let line = String::from_utf8_lossy(line.split(|&b| b == b'\n').next().unwrap_or_default()).trim().to_string();
        let mut words = line.splitn(2, char::is_whitespace);
        let interpreter = words.next().ok_or(ENOEXEC)?.to_string();
        let mut new_argv = vec![interpreter.clone()];
        if let Some(arg) = words.next().map(str::trim).filter(|a| !a.is_empty()) {
            new_argv.push(arg.to_string());
        }
        new_argv.push(path.clone());
        new_argv.extend(argv.into_iter().skip(1));
        argv = new_argv;
        path = interpreter;
        bytes = read_file(k, state, &path)?;
    }
    let program = elf::parse(&bytes)?;
    let interp_bytes = match &program.interp {
        Some(interp) => Some(read_file(k, state, interp)?),
        None => None,
    };
    let interp = interp_bytes.as_deref().map(elf::parse).transpose()?;
    if interp.as_ref().is_some_and(|i| i.isa != program.isa) {
        return Err(ENOEXEC);
    }
    let isa = program.isa;
    let exe = exe_path(&state.cwd.1, &path);
    let mut random = [0u8; 16];
    k.host.random(&mut random);

    let loaded = cx.turn(|t| -> Result<_, Errno> {
        let space = t.new_space(isa).map_err(|_| ENOEXEC)?;
        let old = state.space;
        t.use_space(space);
        let result = (|| {
            let layout = t.space().layout();
            let mut regions: Regions<Area> = Regions::default();
            for kept in &layout.kept {
                regions.insert(kept.clone(), Area { prot: Prot::NONE, backing: Backing::Reserved, name: String::new() });
            }
            let stack_top = page_down(layout.top.min(1 << 47) - (16 << 20));
            let stack_bottom = stack_top - STACK;
            let mmap = MMAP_BOTTOM..stack_bottom - STACK_GAP;
            regions.insert(stack_bottom..stack_top, Area { prot: Prot::RW, backing: Backing::Anon, name: "[stack]".to_string() });
            t.space().map(stack_top - STACK_MAPPED..stack_top, Prot::RW).zeroed().map_err(|_| ENOMEM)?;

            let (lo, hi) = program.span();
            let base = if program.kind == ET_DYN {
                let span = page_up(hi) - page_down(lo);
                if PIE_BASE + span < mmap.end && regions.is_free(PIE_BASE..PIE_BASE + span) {
                    PIE_BASE
                } else {
                    regions.free_above(mmap.clone(), span, 2 << 20).ok_or(ENOMEM)?
                }
            } else {
                0
            };
            map_image(t, &mut regions, &program, base, &exe)?;
            // The page handlers return to, between mmap's room and the stack.
            let trampoline = mmap.end;
            t.space().map(trampoline..trampoline + PAGE, Prot::RX).with(&crate::signal::trampoline(isa)).map_err(|_| ENOMEM)?;
            regions.insert(trampoline..trampoline + PAGE, Area { prot: Prot::RX, backing: Backing::Anon, name: "[vdso]".to_string() });
            let mut interp_base = 0;
            if let Some(interp) = &interp {
                let (ilo, ihi) = interp.span();
                let span = page_up(ihi) - page_down(ilo);
                interp_base = regions.free_below(mmap.clone(), span, 2 << 20).ok_or(ENOMEM)?;
                map_image(t, &mut regions, interp, interp_base, program.interp.as_deref().unwrap_or_default())?;
            }
            let entry = match &interp {
                Some(i) => interp_base + i.entry,
                None => base + program.entry,
            };
            let phdr = base + program.phdr.unwrap_or(program.phoff);
            let auxv = [
                (AT_PHDR, phdr),
                (AT_PHENT, 56),
                (AT_PHNUM, program.phnum),
                (AT_PAGESZ, PAGE),
                (AT_BASE, interp_base),
                (AT_FLAGS, 0),
                (AT_ENTRY, base + program.entry),
                (AT_UID, 0),
                (AT_EUID, 0),
                (AT_GID, 0),
                (AT_EGID, 0),
                (AT_HWCAP, if isa == Isa::Arm64 { 0b11 } else { 0 }),
                (AT_HWCAP2, 0),
                (AT_CLKTCK, 100),
                (AT_SECURE, 0),
            ];
            let sp = write_stack(t, stack_top, &argv, &env, &auxv, &exe, isa, &random)?;
            t.thread().start(entry, sp);
            let brk = page_up(base + hi);
            Ok((regions, mmap, brk, trampoline))
        })();
        match result {
            Ok(r) => {
                t.destroy_space(old);
                Ok((space, r))
            }
            Err(e) => {
                t.use_space(old);
                t.destroy_space(space);
                Err(e)
            }
        }
    })?;
    let (space, (regions, mmap, brk, trampoline)) = loaded;
    state.sigreturn_at = trampoline;
    state.space = space;
    state.isa = isa;
    state.regions = regions;
    state.mmap = mmap;
    state.brk = (brk, brk);
    state.exe = exe;
    state.files.close_on_exec();
    state.signals.reset_on_exec();
    state.pending = 0;
    Ok(())
}

/// The whole of a file on the disk, found from the process's working directory.
fn read_file(k: &Kernel, state: &ProcState, path: &str) -> Result<Vec<u8>, Errno> {
    let disk = lock(&k.disk);
    let ino = disk.resolve(state.cwd.0, path, true)?;
    let meta = disk.meta(ino)?;
    if meta.mode & 0o111 == 0 {
        return Err(EPERM);
    }
    Ok(disk.contents(ino)?.to_vec())
}

/// Maps an ELF's segments at `base`, page by page with each page's widest protection, and notes them as regions.
/// Gives the end of the highest one.
fn map_image(t: &mut Turn<'_>, regions: &mut Regions<Area>, elf: &Elf<'_>, base: u64, name: &str) -> Result<u64, Errno> {
    let (lo, hi) = elf.span();
    let (lo, hi) = (page_down(base + lo), page_up(base + hi));
    let mut image = vec![0u8; (hi - lo) as usize];
    let mut prots = vec![Prot::NONE; ((hi - lo) / PAGE) as usize];
    for s in &elf.segments {
        let at = (base + s.vaddr - lo) as usize;
        image[at..at + s.filesz as usize].copy_from_slice(&elf.bytes[s.offset as usize..(s.offset + s.filesz) as usize]);
        let first = (page_down(base + s.vaddr) - lo) / PAGE;
        let last = (page_up(base + s.vaddr + s.memsz) - lo) / PAGE;
        for p in &mut prots[first as usize..last as usize] {
            *p = *p | s.prot;
        }
    }
    // Map each run of pages with the same protection.
    let mut page = 0;
    while page < prots.len() {
        let prot = prots[page];
        let mut end = page + 1;
        while end < prots.len() && prots[end] == prot {
            end += 1;
        }
        if prot != Prot::NONE {
            let (from, to) = (page as u64 * PAGE, end as u64 * PAGE);
            t.space().map(lo + from..lo + to, prot).with(&image[from as usize..to as usize]).map_err(|_| ENOMEM)?;
            regions.insert(lo + from..lo + to, Area { prot, backing: Backing::Anon, name: name.to_string() });
        }
        page = end;
    }
    Ok(hi)
}

/// The first stack, as Linux lays it out: argc, argv, envp, the auxiliary vector, then the strings they point at.
#[allow(clippy::too_many_arguments)]
fn write_stack(t: &mut Turn<'_>, top: u64, argv: &[String], env: &[String], auxv: &[(u64, u64)], exe: &str, isa: Isa, random: &[u8; 16]) -> Result<u64, Errno> {
    let mut stack = StackWriter::new(top);
    stack.push_u64(0);
    let execfn = stack.push_cstr(exe.as_bytes());
    let env_ptrs: Vec<u64> = env.iter().map(|s| stack.push_cstr(s.as_bytes())).collect();
    let arg_ptrs: Vec<u64> = argv.iter().map(|s| stack.push_cstr(s.as_bytes())).collect();
    let platform = stack.push_cstr(if isa == Isa::Arm64 { b"aarch64" } else { b"x86_64" });
    stack.align(16);
    let random_at = stack.push(random);
    let mut aux: Vec<(u64, u64)> = auxv.to_vec();
    aux.extend([(AT_RANDOM, random_at), (AT_PLATFORM, platform), (AT_EXECFN, execfn), (AT_NULL, 0)]);
    let words = 1 + arg_ptrs.len() + 1 + env_ptrs.len() + 1 + aux.len() * 2;
    stack.align_for(words as u64 * 8, 16);
    for (key, value) in aux.iter().rev() {
        stack.push_u64(*value);
        stack.push_u64(*key);
    }
    stack.push_u64(0);
    for p in env_ptrs.iter().rev() {
        stack.push_u64(*p);
    }
    stack.push_u64(0);
    for p in arg_ptrs.iter().rev() {
        stack.push_u64(*p);
    }
    stack.push_u64(argv.len() as u64);
    let (sp, bytes) = stack.finish();
    if top - sp > STACK_MAPPED {
        return Err(EINVAL);
    }
    user::write(&mut t.space(), sp, &bytes)?;
    Ok(sp)
}
