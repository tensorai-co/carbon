//! The few Objective-C runtime entry points Metal needs: classes, selectors, messages, strings, pools.

use std::ffi::{CStr, CString, c_char, c_void};

pub(crate) type Id = *mut c_void;
pub(crate) type Sel = *const c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    pub(crate) fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

pub(crate) fn sel(name: &str) -> Sel {
    let name = CString::new(name).unwrap_or_default();
    // SAFETY: a NUL-terminated name; selectors live as long as the process.
    unsafe { sel_registerName(name.as_ptr()) }
}

pub(crate) fn class(name: &str) -> Id {
    let name = CString::new(name).unwrap_or_default();
    // SAFETY: a NUL-terminated name; nil if there's no such class.
    unsafe { objc_getClass(name.as_ptr()) }
}

/// Sends a message: `msg!(Ret; receiver, "selector:with:", arg => Type, ...)`, called as the method's own C signature.
macro_rules! msg {
    ($ret:ty; $obj:expr, $sel:expr $(, $arg:expr => $t:ty)* $(,)?) => {{
        let receiver: $crate::macos::objc::Id = $obj;
        let selector: $crate::macos::objc::Sel = $crate::macos::objc::sel($sel);
        // SAFETY: objc_msgSend is called through a pointer of the method's own signature, which the selector names,
        // on an object (or nil, which takes any message) that has it.
        unsafe {
            let f: unsafe extern "C" fn($crate::macos::objc::Id, $crate::macos::objc::Sel $(, $t)*) -> $ret =
                std::mem::transmute($crate::macos::objc::objc_msgSend as unsafe extern "C" fn());
            f(receiver, selector $(, $arg)*)
        }
    }};
}
pub(crate) use msg;

/// An `NSString` for the text, autoreleased.
pub(crate) fn ns_string(text: &str) -> Id {
    let text = CString::new(text.replace('\0', "")).unwrap_or_default();
    msg!(Id; class("NSString"), "stringWithUTF8String:", text.as_ptr() => *const c_char)
}

/// An `NSString`'s text; empty for nil.
pub(crate) fn rust_string(obj: Id) -> String {
    if obj.is_null() {
        return String::new();
    }
    let utf8 = msg!(*const c_char; obj, "UTF8String");
    if utf8.is_null() {
        return String::new();
    }
    // SAFETY: a NUL-terminated string the object owns, copied before anything can let it go.
    unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned()
}

/// An `NSError`'s description.
pub(crate) fn error_text(error: Id) -> String {
    if error.is_null() {
        return "no reason given".into();
    }
    rust_string(msg!(Id; error, "localizedDescription"))
}

pub(crate) fn release(obj: Id) {
    if !obj.is_null() {
        msg!((); obj, "release");
    }
}

pub(crate) fn retain(obj: Id) -> Id {
    if obj.is_null() { obj } else { msg!(Id; obj, "retain") }
}

/// Runs a block with an autorelease pool of its own.
pub(crate) fn pooled<R>(f: impl FnOnce() -> R) -> R {
    // SAFETY: a pool pushed and popped around the block, on one thread.
    let pool = unsafe { objc_autoreleasePoolPush() };
    let r = f();
    // SAFETY: the pool just pushed.
    unsafe { objc_autoreleasePoolPop(pool) };
    r
}
