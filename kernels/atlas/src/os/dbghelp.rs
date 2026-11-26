//! dbghelp.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use atlas_abi::context;
use exec::Isa;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::{Task, read_remote};

dll! {
    "DBGHELP.dll";
    host MiniDumpWriteDump(7) => mini_dump_write_dump;
    host StackWalk64(9) => zero;
    host SymCleanup(1) => zero;
    host SymFromAddr(4) => zero;
    host SymFunctionTableAccess64(2) => zero;
    host SymGetLineFromAddr64(4) => zero;
    host SymGetModuleBase64(2) => zero;
    host SymGetSearchPathW(3) => zero;
    host SymInitialize(3) => zero;
    host SymSetOptions(1) => zero;
    host SymSetSearchPathW(2) => zero;
    host ImageNtHeader(1) => image_nt_header;
    host SymGetModuleInfo64(3) => zero;
    host SymGetModuleInfoW64(3) => zero;
    host SymGetOptions(0) => zero;
    host SymInitializeW(3) => zero;
    host SymLoadModuleExW(7) => zero;
    host SymUnloadModule64(2) => zero;
}

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn image_nt_header(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let base = a[0];
    if base == 0 {
        return Reply::Value(0);
    }
    let Ok(dos) = task.mem(cx, false, |t| user::read_vec(&t.space(), base, 0x40)) else {
        return Reply::Value(0);
    };
    if dos.len() < 0x40 || &dos[0..2] != b"MZ" {
        return Reply::Value(0);
    }
    let lfanew = u32::from_le_bytes([dos[0x3c], dos[0x3d], dos[0x3e], dos[0x3f]]) as u64;
    Reply::Value(base.wrapping_add(lfanew))
}

/// Logs crash information passed to `MiniDumpWriteDump` and returns FALSE (`ERROR_CALL_NOT_IMPLEMENTED`).
fn mini_dump_write_dump(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (h_process, target_pid, h_file, dump_type, exc_param) = (a[0], a[1] as u32, a[2], a[3] as u32, a[4]);
    let (pc, sp) = cx.turn(|t| {
        let r = t.thread().regs();
        (r.pc(), r.sp())
    });
    let caller_process = task.process.clone();
    let caller_frames = task.stack_frames(cx, &caller_process, pc, sp);
    let mut msg = format!(
        "[{}] pid={} DBGHELP!MiniDumpWriteDump(hProc={h_process:#x}, pid={target_pid}, hFile={h_file:#x}, type={dump_type:#x}, exc={exc_param:#x}) caller=[{}]",
        task.tid,
        task.process.pid,
        caller_frames.join(", ")
    );
    if exc_param != 0 {
        let target = task.process_of(h_process).unwrap_or_else(|| caller_process.clone());
        let w = if task.isa == Isa::X86_32 { 4usize } else { 8usize };
        let info_len = if w == 4 { 12 } else { 20 };
        if let Ok(info) = task.mem(cx, false, |t| user::read_vec(&t.space(), exc_param, info_len)) {
            let tid = u32::from_le_bytes(info[0..4].try_into().unwrap_or_default());
            let exc_ptrs = if w == 4 {
                u64::from(u32::from_le_bytes(info[4..8].try_into().unwrap_or_default()))
            } else {
                u64::from_le_bytes(info[8..16].try_into().unwrap_or_default())
            };
            let client_ptrs = if w == 4 {
                u32::from_le_bytes(info[8..12].try_into().unwrap_or_default())
            } else {
                u32::from_le_bytes(info[16..20].try_into().unwrap_or_default())
            };
            msg.push_str(&format!(" | exc_tid={tid} exc_ptrs={exc_ptrs:#x} client={client_ptrs}"));
            let ptr_proc = if client_ptrs != 0 { &caller_process } else { &target };
            if exc_ptrs != 0
                && let Some(ep) = read_remote(&task.k, cx, ptr_proc, exc_ptrs, w * 2)
            {
                let (rec_at, ctx_at) = if w == 4 {
                    (u64::from(u32::from_le_bytes(ep[0..4].try_into().unwrap_or_default())), u64::from(u32::from_le_bytes(ep[4..8].try_into().unwrap_or_default())))
                } else {
                    (u64::from_le_bytes(ep[0..8].try_into().unwrap_or_default()), u64::from_le_bytes(ep[8..16].try_into().unwrap_or_default()))
                };
                if rec_at != 0
                    && let Some(rec) = read_remote(&task.k, cx, ptr_proc, rec_at, if w == 4 { 80 } else { 152 })
                {
                    let code = u32::from_le_bytes(rec[0..4].try_into().unwrap_or_default());
                    let flags = u32::from_le_bytes(rec[4..8].try_into().unwrap_or_default());
                    let addr = if w == 4 {
                        u64::from(u32::from_le_bytes(rec[12..16].try_into().unwrap_or_default()))
                    } else {
                        u64::from_le_bytes(rec[16..24].try_into().unwrap_or_default())
                    };
                    let count = u32::from_le_bytes(rec[if w == 4 { 16..20 } else { 24..28 }].try_into().unwrap_or_default()).min(15) as usize;
                    let base_off = if w == 4 { 20 } else { 32 };
                    let params: Vec<String> = (0..count)
                        .map(|i| {
                            let o = base_off + i * w;
                            let v = if w == 4 {
                                u64::from(u32::from_le_bytes(rec[o..o + 4].try_into().unwrap_or_default()))
                            } else {
                                u64::from_le_bytes(rec[o..o + 8].try_into().unwrap_or_default())
                            };
                            format!("{v:#x}")
                        })
                        .collect();
                    let place = {
                        let s = kernels_core::lock(&target.state);
                        Task::where_in(&s, addr)
                    };
                    msg.push_str(&format!(" | code={code:#010x} flags={flags:#x} addr={addr:#x} ({place}) params=[{}]", params.join(", ")));
                }
                if ctx_at != 0 {
                    let tw = if target.isa == Isa::X86_32 { 4 } else { 8 };
                    let ctx_len = if tw == 4 { context::X86_SIZE } else { context::X64_SIZE } as usize;
                    if let Some(ctx_bytes) = read_remote(&task.k, cx, ptr_proc, ctx_at, ctx_len) {
                        let (crash_pc, crash_sp) = if tw == 4 {
                            let eip = u64::from(u32::from_le_bytes(ctx_bytes[context::X86_EIP as usize..][..4].try_into().unwrap_or_default()));
                            let esp = u64::from(u32::from_le_bytes(ctx_bytes[context::X86_ESP as usize..][..4].try_into().unwrap_or_default()));
                            (eip, esp)
                        } else {
                            let rip = u64::from_le_bytes(ctx_bytes[context::X64_RIP as usize..][..8].try_into().unwrap_or_default());
                            let rsp_off = (context::X64_GPRS + 4 * 8) as usize;
                            let rsp = u64::from_le_bytes(ctx_bytes[rsp_off..][..8].try_into().unwrap_or_default());
                            (rip, rsp)
                        };
                        let crash_frames = task.stack_frames(cx, &target, crash_pc, crash_sp);
                        msg.push_str(&format!(" | crash_stack=[{}]", crash_frames.join(", ")));
                    }
                }
            }
        }
    }
    msg.push('\n');
    task.k.host.write(host::Stream::Err, msg.as_bytes());
    task.set_last_error(cx, 120); // ERROR_CALL_NOT_IMPLEMENTED
    Reply::Value(0)
}
