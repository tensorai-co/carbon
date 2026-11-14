//! imm32.dll: input method editors, of which the machine has none (its keyboard is the US layout, which has no IME),
//! so IMM is off: there are no input contexts, and everything that asks about one says there's nothing, as Windows
//! does for a window without one.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "IMM32.dll";
    host ImmAssociateContext(2) => zero;
    host ImmCreateContext(0) => zero;
    host ImmDestroyContext(1) => zero;
    host ImmGetCandidateListCountW(2) => zero;
    host ImmGetCandidateListW(4) => zero;
    host ImmGetCompositionFontW(2) => zero;
    host ImmGetCompositionStringW(4) => no_data;
    host ImmGetContext(1) => zero;
    host ImmGetConversionStatus(3) => zero;
    host ImmGetIMEFileNameA(3) => zero;
    host ImmGetOpenStatus(1) => zero;
    host ImmGetProperty(2) => zero;
    host ImmNotifyIME(4) => zero;
    host ImmReleaseContext(2) => zero;
    host ImmSetCandidateWindow(2) => zero;
    host ImmSetCompositionStringW(6) => zero;
    host ImmSetCompositionWindow(2) => zero;
    host ImmSetConversionStatus(3) => zero;
    host ImmSetOpenStatus(2) => zero;
    host ImmGetIMEFileNameW(3) => zero;
}

/// No context, no IME: NULL, FALSE, or nothing written. `ImmAssociateContext` had none before; `ImmReleaseContext`
/// of none fails, as there was none to release.
fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `ImmGetCompositionStringW(context, index, buffer, length)` without a context: IMM_ERROR_GENERAL (-2).
fn no_data(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(if task.abi().width == 8 { (-2i64) as u64 } else { u64::from((-2i32) as u32) })
}
