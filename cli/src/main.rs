//! `carbon`: run Linux and Windows programs in machines of their own.
//!
//! ```sh
//! carbon run debian:13 -- bash              # a Linux image, with lambda (pulled if it isn't here)
//! carbon run --name dev debian:13           # named, to start again later with `carbon run --name dev`
//! carbon run --isolation vm debian:13 -- ./untrusted.sh
//! carbon run setup.exe                      # a Windows program, with atlas
//! carbon ls | stop NAME | rm NAME | freeze NAME IMAGE
//! carbon display screenshot NAME -o screen.png
//! carbon pull IMAGE | images | rmi IMAGE
//! ```
//!
//! On macOS the binary is signed with `carbon.entitlements` to use the hypervisor:
//! `codesign --force --sign - --entitlements carbon.entitlements target/release/carbon`.

use std::path::{Path, PathBuf};
use std::process::exit;

use images::Platform;
use machine::{Display, Isolation, Machines, Run, Source};

const USAGE: &str = "\
usage: carbon <command> ...

  run [options] IMAGE|PROGRAM [-- ARGS...]   run a new machine from an image or a program
  run --name NAME [-- ARGS...]               start a stopped machine again, with its files as it left them
      --isolation vm|sandbox|auto            a hardware VM, the process sandbox, or the best there is (the default)
      --name NAME                            the machine's name (else one from its image or program)
      --platform OS/ARCH                     which build of an image to pull (linux for this CPU by default)
      --display headless=WxH|none            where its windows go (headless=1280x800 by default)
      --audio none                           where its sound goes (only none, until audio is there)
      -e, --env NAME=VALUE                   an environment variable, over the image's
      --memory SIZE                          how much memory it may use, as 512m or 4g (4g by default)
  ls                                         the machines, and which are running
  stop NAME                                  stop a running machine (SIGTERM, then everything after 10s)
  rm NAME                                    remove a stopped machine and its files
  freeze NAME IMAGE                          make an image of a stopped machine's disk
  display screenshot NAME [-o FILE]          a running machine's screen, as a PNG (screen.png by default)
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
        "host" => fail("--display host needs the desktop on the host, which isn't there yet: use headless=WxH"),
        _ => {
            let (w, h) = text.strip_prefix("headless=")?.split_once('x')?;
            Some(Display::Headless(w.parse().ok()?, h.parse().ok()?))
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
                spec.display = display(&text).unwrap_or_else(|| fail(format!("--display is headless=WxH or none, not {text}")));
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
        "stop" => machines.stop(&one("a machine's name")).unwrap_or_else(|e| fail(e)),
        "rm" => machines.remove(&one("a machine's name")).unwrap_or_else(|e| fail(e)),
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
            other => fail(format!("display doesn't {other} yet: screenshot")),
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
        assert_eq!(display("headless=800x600"), Some(Display::Headless(800, 600)));
        assert_eq!(display("none"), Some(Display::None));
        assert_eq!(display("big"), None);
    }
}
