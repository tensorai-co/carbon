//! oleacc.dll: Microsoft Active Accessibility (MSAA). Atlas has no assistive-technology bridge, so requests for
//! standard or window-backed `IAccessible` objects report `E_NOINTERFACE` / `E_NOTIMPL` and zero their out-pointers.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "OLEACC.dll";
    host AccessibleChildren(5) => accessible_children;
    host AccessibleObjectFromWindow(4) => accessible_object_from_window;
    host CreateStdAccessibleObject(4) => create_std_accessible_object;
    host LresultFromObject(3) => lresult_from_object;
    host WindowFromAccessibleObject(2) => window_from_accessible_object;
}

const E_NOTIMPL: u32 = 0x8000_4001;
const E_NOINTERFACE: u32 = 0x8000_4002;
const E_POINTER: u32 = 0x8000_4003;
const E_INVALIDARG: u32 = 0x8007_0057;

fn hresult(hr: u32) -> Reply {
    Reply::Value(u64::from(hr))
}

/// `AccessibleChildren(paccContainer, iChildStart, cChildren, rgvarChildren, pcObtained)`: no accessible objects.
fn accessible_children(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[4] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[4], 0));
    }
    hresult(E_INVALIDARG)
}

/// `AccessibleObjectFromWindow(hwnd, dwId, riid, ppvObject)`: no MSAA object for the window.
fn accessible_object_from_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[3] == 0 {
        return hresult(E_POINTER);
    }
    let _ = task.write_ptr(cx, a[3], 0);
    hresult(E_NOINTERFACE)
}

/// `CreateStdAccessibleObject(hwnd, idObject, riid, ppvObject)`: no standard MSAA proxy available.
fn create_std_accessible_object(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[3] == 0 {
        return hresult(E_POINTER);
    }
    let _ = task.write_ptr(cx, a[3], 0);
    hresult(E_NOINTERFACE)
}

/// `LresultFromObject(riid, wParam, punk)`: returns an `LRESULT` carrying a negative `HRESULT` on failure.
fn lresult_from_object(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    let hr = E_NOTIMPL as i32;
    Reply::Value(if task.abi().width == 8 { i64::from(hr) as u64 } else { u64::from(E_NOTIMPL) })
}

/// `WindowFromAccessibleObject(pacc, phwnd)`: no window associated with an unknown accessible object.
fn window_from_accessible_object(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] == 0 {
        return hresult(E_POINTER);
    }
    let _ = task.write_ptr(cx, a[1], 0);
    hresult(E_INVALIDARG)
}
