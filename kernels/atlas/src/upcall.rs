//! Upcalls: host Rust calling the program back, as Windows' `win32k` calls a `WndProc` (`KeUserModeCallback`). A host
//! export runs on the thread that owns the program's CPU, so it can run the program again, nested, and carry on
//! when the callback returns.

use atlas_abi::call::UPCALL_RETURN;
use atlas_abi::status;
use exec::Stop;
use kernels_core::thread::{Cx, Flow};
use kernels_core::user;

use crate::images::UPCALL_RETURN_AT;
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
        let (frame_at, frame) = self.convention().prepare(&mut regs, target, &list, UPCALL_RETURN_AT);
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
