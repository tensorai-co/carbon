//! `carbon`: run Linux and Windows programs in machines of their own.
//!
//! ```sh
//! carbon run debian:13 -- bash              # a Linux image, with lambda (pulled if it isn't here)
//! carbon run --name dev debian:13           # named, to start again later with `carbon run --name dev`
//! carbon run --isolation vm debian:13 -- ./untrusted.sh
//! carbon run setup.exe                      # a Windows program, with atlas
//! carbon run --gpu host debian:13 -- vulkaninfo --summary   # the Mac's GPU, as a Carbon GPU
//! carbon gpu                                # the devices a machine's GPU can be
//! carbon ls | stop NAME | rm NAME | freeze NAME IMAGE
//! carbon display screenshot NAME -o screen.png
//! carbon display windows NAME
//! carbon display click NAME 640 400 && carbon display type NAME hello && carbon display key NAME ctrl+s
//! carbon pull IMAGE | images | rmi IMAGE
//! carbon atlas imports setup.exe            # what a Windows program imports that atlas doesn't have yet
//! ```
//!
//! On macOS the binary is signed with `carbon.entitlements` to use the hypervisor:
//! `codesign --force --sign - --entitlements carbon.entitlements target/release/carbon`.

mod imports;

use std::path::{Path, PathBuf};
use std::process::exit;

use images::Platform;
use machine::{Display, Gpu, Isolation, Machines, Run, Source};

const USAGE: &str = "\
usage: carbon <command> ...

  run [options] IMAGE|PROGRAM [-- ARGS...]   run a new machine from an image or a program
  run --name NAME [-- ARGS...]               start a stopped machine again, with its files as it left them
      --isolation vm|sandbox|auto            a hardware VM, the process sandbox, or the best there is (the default)
      --name NAME                            the machine's name (else one from its image or program)
      --platform OS/ARCH                     which build of an image to pull (linux for this CPU by default)
      --display headless=WxH[@SCALE]|host[=WxH]|host-windows|none
                                             where its windows go: a screen in memory (headless=1280x800 by
                                             default; @2 for 2×), the whole desktop in a window on this Mac's
                                             screen, or each of its windows in a Mac window of its own
      --gpu auto|host|software|none          its GPU: the host's, else the CPU device (auto, the default; said if
                                             it's the CPU device), the host's or fail, the CPU device, or none
      --audio none                           where its sound goes (only none, until audio is there)
      -e, --env NAME=VALUE                   an environment variable, over the image's
      --memory SIZE                          how much memory it may use, as 512m or 4g (4g by default)
  ls                                         the machines, and which are running
  gpu                                        the devices a machine's GPU can be: this host's, and the CPU device
  stop NAME                                  stop a running machine (SIGTERM, then everything after 10s)
  rm NAME                                    remove a stopped machine and its files
  freeze NAME IMAGE                          make an image of a stopped machine's disk
  cp NAME:PATH DIR                           copy a stopped machine's file or folder into DIR on this host
  cp PATH NAME:DIR                           copy this host's file or folder into a stopped machine's DIR (PATH/.
                                             for what's in a folder)
  display screenshot NAME [-o FILE]          a running machine's screen, as a PNG (screen.png by default)
  display windows NAME                       a running machine's windows, top first: ID, place, size, state, title
  display move NAME X Y                      moves its pointer
  display click NAME X Y [BUTTON] [COUNT]    clicks there (left by default; COUNT 2 for a double click)
  display drag NAME X1 Y1 X2 Y2              drags with the left button
  display scroll NAME X Y NOTCHES [SIDEWAYS] scrolls there (up and right positive)
  display type NAME TEXT...                  types text, as keys on a US keyboard
  display key NAME COMBO...                  presses keys: enter, esc, ctrl+s, alt+f4, ...
  display resize NAME WxH[@SCALE]            changes its screen's size and scale (1, 1.5, 2, ...)
  display clipboard NAME [TEXT...]           prints its clipboard's text, or sets it, as copied on the host
  display drop NAME X Y TEXT...              drops text there, as dragged in from the host
  display drop NAME X Y --files PATH...      drops files there (paths as the machine has them: C:\\..., /home/...)
  atlas imports PATH... [--all]              what Windows programs import, against what atlas has: for a file,
                                             each DLL and what of it isn't there; for several or a folder, what's
                                             wanted most (--all lists everything)
  pull IMAGE [--platform OS/ARCH]            pull an image
  images                                     the images here
  rmi IMAGE                                  forget an image
";

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("carbon: {message}");
    exit(1)
}

fn usage() -> ! {
    eprint!("{USAGE}");
    exit(2)
}

fn size(text: &str) -> Option<u64> {
    let text = text.to_ascii_lowercase();
    let (number, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => (&text[..at], &text[at..]),
        None => (text.as_str(), ""),
    };
    let shift = match unit.trim_end_matches('b').trim_end_matches('i') {
        "" => 0,
        "k" => 10,
        "m" => 20,
        "g" => 30,
        _ => return None,
    };
    number.parse::<u64>().ok().map(|n| n << shift)
}

fn display(text: &str) -> Option<Display> {
    match text {
        "none" => Some(Display::None),
        "host" => Some(Display::Host(1280, 800)),
        "host-windows" => Some(Display::HostWindows),
        _ if text.starts_with("host=") => {
            let (w, h, scale) = machine::parse_display_size(&text["host=".len()..])?;
            // The window's scale is the Mac's own: no @SCALE (120ths: 120 is 1×).
            (scale == 120).then_some(Display::Host(w, h))
        }
        _ => {
            let (w, h, scale) = machine::parse_display_size(text.strip_prefix("headless=")?)?;
            Some(Display::Headless(w, h, scale))
        }
    }
}

fn run(machines: &Machines, mut args: impl Iterator<Item = String>) -> ! {
    let mut spec = Run::default();
    let mut source = None;
    while let Some(arg) = args.next() {
        let mut value = |what: &str| args.next().unwrap_or_else(|| fail(format!("{what} needs a value")));
        match arg.as_str() {
            "--" => {
                spec.args = args.by_ref().collect();
                break;
            }
            "--isolation" => {
                spec.isolation = match value("--isolation").as_str() {
                    "vm" => Isolation::Vm,
                    "sandbox" => Isolation::Sandbox,
                    "auto" => Isolation::Auto,
                    other => fail(format!("--isolation is vm, sandbox or auto, not {other}")),
                }
            }
            "--name" => spec.name = Some(value("--name")),
            "--platform" => {
                let text = value("--platform");
                spec.platform = Some(Platform::parse(&text).unwrap_or_else(|| fail(format!("{text} isn't a platform: linux/arm64, linux/amd64, windows/amd64"))));
            }
            "--display" => {
                let text = value("--display");
                spec.display = display(&text).unwrap_or_else(|| fail(format!("--display is headless=WxH, headless=WxH@SCALE, host, host=WxH, host-windows or none, not {text}")));
            }
            "--gpu" => {
                spec.gpu = match value("--gpu").as_str() {
                    "auto" => Gpu::Auto,
                    "host" => Gpu::Host,
                    "software" => Gpu::Software,
                    "none" => Gpu::None,
                    other => fail(format!("--gpu is auto, host, software or none, not {other}")),
                }
            }
            "--audio" => match value("--audio").as_str() {
                "none" => {}
                other => fail(format!("--audio {other} needs audio, which isn't there yet: use none")),
            },
            "-e" | "--env" => {
                let var = value("--env");
                if !var.contains('=') {
                    fail(format!("{var}: an environment variable is NAME=VALUE"));
                }
                spec.env.push(var);
            }
            "--memory" => {
                let text = value("--memory");
                spec.memory = size(&text).unwrap_or_else(|| fail(format!("{text} isn't a size: 512m, 4g")));
            }
            flag if flag.starts_with('-') => fail(format!("run doesn't know {flag}")),
            _ if source.is_none() => source = Some(arg),
            _ => {
                // Everything after what to run is its arguments, `--` or not.
                spec.args = std::iter::once(arg).chain(args.by_ref()).collect();
                break;
            }
        }
    }
    spec.source = source.map(|what| if Path::new(&what).is_file() { Source::Program(PathBuf::from(what)) } else { Source::Image(what) });
    if spec.source.is_none() && spec.name.is_none() {
        usage();
    }
    match machines.run(spec, &mut |what| eprintln!("carbon: {what}")) {
        Ok(code) => exit(code),
        Err(why) => fail(why),
    }
}

fn ls(machines: &Machines) {
    let rows: Vec<[String; 4]> = machines
        .list()
        .into_iter()
        .map(|m| {
            let from = m.image.as_deref().map(|i| i.strip_prefix("docker.io/library/").unwrap_or(i).to_string()).or(m.program.clone()).unwrap_or_default();
            let status = if m.running() { format!("running ({})", m.pid.unwrap_or(0)) } else { "stopped".to_string() };
            [m.name, from, m.backend, status]
        })
        .collect();
    table(&["NAME", "FROM", "ISOLATION", "STATUS"], &rows);
}

fn table<const N: usize>(head: &[&str; N], rows: &[[String; N]]) {
    let mut widths: [usize; N] = head.map(str::len);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }
    let line = |cells: Vec<&str>| {
        let text: Vec<String> = cells.iter().zip(widths).map(|(cell, width)| format!("{cell:<width$}")).collect();
        println!("{}", text.join("  ").trim_end());
    };
    line(head.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
}

fn main() {
    // On macOS the sandbox's stub is this binary, run again.
    exec::run_if_stub();
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| usage());
    let machines = Machines::open(None).unwrap_or_else(|e| fail(e));
    let mut one = |what: &str| args.next().unwrap_or_else(|| fail(format!("{command} needs {what}")));
    match command.as_str() {
        "run" => run(&machines, args),
        "ls" | "ps" => ls(&machines),
        "gpu" => {
            println!("{:<10} {:<24} {:<10} SUBGROUP", "KIND", "NAME", "BACKEND");
            for (kind, device) in ["host", "software"].into_iter().zip(machine::gpus()) {
                match device {
                    Ok(info) => println!("{kind:<10} {:<24} {:<10} {}", info.name, info.backend, info.subgroup_size),
                    Err(why) => println!("{kind:<10} -- ({why})"),
                }
            }
        }
        "stop" => machines.stop(&one("a machine's name")).unwrap_or_else(|e| fail(e)),
        "rm" => machines.remove(&one("a machine's name")).unwrap_or_else(|e| fail(e)),
        "cp" => {
            let (from, to) = (one("NAME:PATH or a path on this host"), one("a folder on this host or NAME:PATH"));
            // Out of a machine when the first names one, as web:/etc/hosts; else into the machine the second names.
            let machine = |arg: &str| arg.split_once(':').filter(|(name, _)| machines.record(name).is_some()).map(|(n, p)| (n.to_string(), p.to_string()));
            let n = match (machine(&from), machine(&to)) {
                (Some((name, path)), _) => machines.copy_out(&name, &path, std::path::Path::new(&to)),
                (None, Some((name, path))) => machines.copy_in(&name, std::path::Path::new(&from), &path),
                (None, None) => fail("cp copies NAME:PATH DIR out of a machine, or PATH NAME:DIR into one, as web:/etc/hosts".to_string()),
            }
            .unwrap_or_else(|e| fail(e));
            println!("{n} files");
        }
        "freeze" => {
            let (name, image) = (one("a machine's name"), one("a name for the image"));
            let image = machines.freeze(&name, &image).unwrap_or_else(|e| fail(e));
            println!("{}", image.short_name());
        }
        "display" => match one("what to do").as_str() {
            "screenshot" => {
                let name = one("a machine's name");
                let out = match (args.next().as_deref(), args.next()) {
                    (Some("-o"), Some(file)) => file,
                    (None, _) => "screen.png".to_string(),
                    _ => usage(),
                };
                let png = machines.screenshot(&name).unwrap_or_else(|e| fail(e));
                std::fs::write(&out, png).unwrap_or_else(|e| fail(format!("{out}: {e}")));
                println!("{out}");
            }
            "windows" => {
                let name = one("a machine's name");
                print!("{}", machines.display_windows(&name).unwrap_or_else(|e| fail(e)));
            }
            "clipboard" => {
                let name = one("a machine's name");
                let text: Vec<String> = args.by_ref().collect();
                if text.is_empty() {
                    print!("{}", machines.display_clipboard(&name, None).unwrap_or_else(|e| fail(e)));
                } else {
                    machines.display_clipboard(&name, Some(&text.join(" "))).unwrap_or_else(|e| fail(e));
                }
            }
            "drop" => {
                let name = one("a machine's name");
                let mut point = |what| one(what).parse::<i32>().unwrap_or_else(|_| fail("a drop needs a point: X Y"));
                let (x, y) = (point("X"), point("Y"));
                let mut rest: Vec<String> = args.by_ref().collect();
                let files = rest.first().is_some_and(|f| f == "--files");
                if files {
                    rest.remove(0);
                }
                if rest.is_empty() {
                    fail(if files { "drop --files needs a path" } else { "drop needs text" });
                }
                println!("{}", machines.display_drop(&name, x, y, files, &rest).unwrap_or_else(|e| fail(e)));
            }
            "resize" => {
                let name = one("a machine's name");
                let size = one("a size: WIDTHxHEIGHT, or WIDTHxHEIGHT@SCALE");
                machines.display_resize(&name, &size).unwrap_or_else(|e| fail(e));
            }
            verb @ ("move" | "click" | "drag" | "scroll" | "type" | "key") => {
                let name = one("a machine's name");
                let rest: Vec<String> = args.by_ref().collect();
                machines.display_input(&name, verb, &rest).unwrap_or_else(|e| fail(e));
            }
            other => fail(format!("display doesn't {other} yet: screenshot, windows, move, click, drag, scroll, type, key")),
        },
        "atlas" => match one("what to do").as_str() {
            "imports" => imports::run(args.by_ref().collect()),
            other => fail(format!("atlas doesn't {other}: imports")),
        },
        "pull" => {
            let name = one("an image");
            let platform = match (args.next().as_deref(), args.next()) {
                (Some("--platform"), Some(text)) => Platform::parse(&text).unwrap_or_else(|| fail(format!("{text} isn't a platform"))),
                (None, _) => Platform::linux_here(),
                _ => usage(),
            };
            let images = machines.images();
            let progress = &mut |what: &str| eprintln!("carbon: {what}");
            let image = if platform.os == "windows" {
                images.pull(&name, &platform, |_| [0u32; 4], [0x10, 0, 0, 0], progress)
            } else {
                images.pull(&name, &platform, |e: images::Entry| [e.mode, e.uid, e.gid, e.rdev as u32], [0o755, 0, 0, 0], progress)
            }
            .unwrap_or_else(|e| fail(e));
            println!("{} ({})", image.short_name(), image.platform);
        }
        "images" => {
            let rows: Vec<[String; 3]> = machines.images().images().into_iter().map(|i| [i.short_name().to_string(), i.platform.to_string(), i.source]).collect();
            table(&["IMAGE", "PLATFORM", "FROM"], &rows);
        }
        "rmi" => machines.images().remove(&one("an image")).unwrap_or_else(|e| fail(e)),
        "help" | "--help" | "-h" => print!("{USAGE}"),
        other => fail(format!("{other} isn't a command: carbon help")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_displays() {
        assert_eq!(size("512m"), Some(512 << 20));
        assert_eq!(size("4G"), Some(4 << 30));
        assert_eq!(size("2gib"), Some(2 << 30));
        assert_eq!(size("100"), Some(100));
        assert_eq!(size("lots"), None);
        assert_eq!(display("headless=800x600"), Some(Display::Headless(800, 600, 120)));
        assert_eq!(display("headless=1440x900@2"), Some(Display::Headless(1440, 900, 240)));
        assert_eq!(display("none"), Some(Display::None));
        assert_eq!(display("host"), Some(Display::Host(1280, 800)));
        assert_eq!(display("host=1024x700"), Some(Display::Host(1024, 700)));
        assert_eq!(display("host=1024x700@2"), None);
        assert_eq!(display("host-windows"), Some(Display::HostWindows));
        assert_eq!(display("big"), None);
    }
}
