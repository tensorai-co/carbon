//! timeout.exe: waits `/T` seconds, counting down, or until a key on the console (`/NOBREAK`: only Ctrl+C); `-1`
//! waits for a key.

#![no_std]
#![no_main]

extern crate alloc;
extern crate atlas_programs as programs;

use alloc::format;
use programs::*;

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    let args = arguments();
    let mut seconds: Option<i64> = None;
    let mut nobreak = false;
    let mut i = 0;
    while i < args.len() {
        let lower = args[i].to_ascii_lowercase();
        if lower == "/t" {
            i += 1;
            seconds = args.get(i).and_then(|s| s.parse().ok());
        } else if lower == "/nobreak" {
            nobreak = true;
        } else if let Ok(n) = lower.parse::<i64>() {
            seconds = Some(n);
        }
        i += 1;
    }
    let Some(seconds) = seconds.filter(|s| (-1..=99999).contains(s)) else {
        err("ERROR: Invalid syntax. Default option is not allowed more than '1' time(s).\r\nType \"TIMEOUT /?\" for usage.\r\n");
        exit(1);
    };
    let _ = nobreak;
    if seconds < 0 {
        out("\r\nPress any key to continue ...");
        let _ = Input::stdin().line();
        out("\r\n");
        exit(0);
    }
    for left in (1..=seconds).rev() {
        out(&format!("\rWaiting for {left:>2} seconds, press {} to continue ...", if nobreak { "CTRL+C" } else { "a key" }));
        unsafe { Sleep(1000) };
    }
    out(&format!("\rWaiting for {:>2} seconds, press {} to continue ...\r\n", 0, if nobreak { "CTRL+C" } else { "a key" }));
    exit(0)
}
