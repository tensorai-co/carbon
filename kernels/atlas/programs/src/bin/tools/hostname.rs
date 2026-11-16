//! hostname.exe: the computer's name.

#![no_std]
#![no_main]

extern crate alloc;
extern crate atlas_programs as programs;

use alloc::format;
use programs::*;

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    if !arguments().is_empty() {
        err("sethostname: Use the Network Control Panel Applet to set hostname.\r\nhostname -s is not supported.\r\n");
        exit(1);
    }
    out(&format!("{}\r\n", computer_name()));
    exit(0)
}
