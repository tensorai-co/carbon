//! whoami.exe: who's running, as `computer\user` (lowercase, as Windows prints it), or with `/upn`, `/fqdn` refused as
//! a machine with no domain refuses them.

#![no_std]
#![no_main]

extern crate alloc;
extern crate atlas_programs as programs;

use alloc::format;
use programs::*;

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    let name = format!("{}\\{}", computer_name(), user_name()).to_ascii_lowercase();
    let args = arguments();
    match args.first().map(|a| a.to_ascii_lowercase()) {
        None => out(&format!("{name}\r\n")),
        Some(a) if a == "/user" => {
            out(&format!("\r\nUSER INFORMATION\r\n----------------\r\n\r\nUser Name {:<w$} SID\r\n{} {}\r\n{name} S-1-5-21-0-0-0-1000\r\n", "", "=".repeat(name.len().max(9)), "=".repeat(20), w = name.len().max(9) - 9));
        }
        Some(a) if a == "/upn" || a == "/fqdn" => {
            err("ERROR: Unable to get User Principal Name (UPN) as the current logged-on user\r\n       is not a domain user.\r\n");
            exit(1);
        }
        Some(a) if a == "/?" => out("WhoAmI has three ways of working: whoami, whoami /user, whoami /upn.\r\n"),
        Some(other) => {
            err(&format!("ERROR: Invalid argument/option - '{other}'.\r\nType \"WHOAMI /?\" for usage.\r\n"));
            exit(1);
        }
    }
    exit(0)
}
