//! Upcalls: host Rust calling the program back, as Windows' `win32k` calls a `WndProc` (`KeUserModeCallback`). A host
//! export runs on the thread that owns the program's CPU, so it can run the program again, nested, and carry on
//! when the callback returns.

use atlas_abi::call::UPCALL_RETURN;
use atlas_abi::status;
use exec::Stop;
use kernels_core::thread::{Cx, Flow};
use kernels_core::user;

use crate::os::Reply;
use crate::process::Task;

/// How deep upcalls nest before the process ends, so a runaway `SendMessage` loop can't overflow Carbon's stack.
const MOST_UPCALLS: usize = 64;
/// Room left below the stack pointer the program trapped with, past any red zone it may use.
const LEAVE: u64 = 256;

impl Task {
    /// Calls `target` in the program with `args`, and runs it until it returns. Err is the reply for the host export
    /// that made the upcall, if the process ended while the callback ran.
    pub fn upcall(&mut self, cx: &mut Cx<'_>, target: u64, args: &[u64]) -> Result<u64, Reply> {
        self.upcall_data(cx, target, &[], |_| args.to_vec())
    }

    /// Like `upcall`, first copying `data` onto the program's stack, for the callback to read: its address becomes the
    /// first argument.
    pub fn upcall_with_data(&mut self, cx: &mut Cx<'_>, target: u64, data: &[u8], args: &[u64]) -> Result<u64, Reply> {
        self.upcall_data(cx, target, data, |at| [at].into_iter().chain(args.iter().copied()).collect())
    }

    /// Runs `body` with `len` bytes of the program's stack as scratch, made by `build` from where they go: below the
    /// stack pointer the thread came in with, which is lowered meanwhile so nothing nested (an upcall, a message sent
    /// while it waits) overwrites them. How a host export hands the program a string it converted.
    pub fn with_scratch<R>(&mut self, cx: &mut Cx<'_>, len: usize, build: impl FnOnce(u64) -> Vec<u8>, body: impl FnOnce(&mut Task, &mut Cx<'_>, u64) -> Result<R, Reply>) -> Result<R, Reply> {
        let mut regs = cx.turn(|t| t.thread().regs());
        let old = regs.sp();
        let at = (((old - LEAVE) & !15) - len.max(16) as u64) & !15;
        let data = build(at);
        debug_assert!(data.len() <= len.max(16));
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &data)).map_err(|_| Reply::ExitProcess(status::ACCESS_VIOLATION))?;
        regs.set_sp(at);
        cx.turn(|t| t.thread().set_regs(&regs));
        let result = body(self, cx, at);
        let mut regs = cx.turn(|t| t.thread().regs());
        regs.set_sp(old);
        cx.turn(|t| t.thread().set_regs(&regs));
        result
    }

    /// Like `upcall`, first copying `data` onto the program's stack: `args` makes the arguments from where it went.
    pub fn upcall_data(&mut self, cx: &mut Cx<'_>, target: u64, data: &[u8], args: impl FnOnce(u64) -> Vec<u64>) -> Result<u64, Reply> {
        if self.depth >= MOST_UPCALLS {
            self.die(status::STACK_OVERFLOW, "upcalls nested too deep");
            return Err(Reply::Ended);
        }
        let saved = cx.turn(|t| t.thread().save());
        let mut regs = cx.turn(|t| t.thread().regs());
        let mut sp = (regs.sp() - LEAVE) & !15;
        if !data.is_empty() {
            sp = (sp - data.len() as u64) & !15;
            self.mem(cx, true, |t| user::write(&mut t.space(), sp, data)).map_err(|_| Reply::ExitProcess(status::ACCESS_VIOLATION))?;
        }
        let list = args(sp);
        regs.set_sp(sp);
        let (frame_at, frame) = self.convention().prepare(&mut regs, target, &list, self.layout().upcall_return());
        self.mem(cx, true, |t| user::write(&mut t.space(), frame_at, &frame)).map_err(|_| Reply::ExitProcess(status::ACCESS_VIOLATION))?;
        cx.turn(|t| t.thread().set_regs(&regs));

        self.depth += 1;
        let result = loop {
            let exit = cx.run();
            let stop = cx.turn(|t| t.stop(exit));
            if let Some(Stop::Syscall { nr: UPCALL_RETURN, .. }) = stop {
                break Ok(self.return_value(cx));
            }
            if self.handle(cx, stop) == Flow::End {
                break Err(Reply::Ended);
            }
        };
        self.depth -= 1;
        if result.is_ok() {
            // An exception never unwinds through host frames: the callback either returned here, or the process ended.
            cx.turn(|t| t.thread().restore(&saved));
        }
        result
    }
}
