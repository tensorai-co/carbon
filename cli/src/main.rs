//! `carbon`: run Linux and Windows programs in machines of their own.
//!
//! ```sh
//! carbon run -it debian:13 bash             # a shell in a Linux image, with lambda (pulled if it isn't here)
//! carbon run debian:13 -- ls /              # a command, its output here and no stdin, as `docker run` has it
//! carbon run --name dev debian:13           # named, to start again later with `carbon run --name dev`
//! carbon run --isolation vm debian:13 -- ./untrusted.sh
//! carbon run setup.exe                      # a Windows program, with atlas
//! carbon run --gpu host debian:13 -- vulkaninfo --summary   # the Mac's GPU, as a Carbon GPU
//! carbon run -p 8080:80 debian:13 -- python3 -m http.server 80   # the host's port 8080 reaches the machine's 80
//! carbon run --network host debian:13 -- python3 -m http.server 80   # the host's port 80 itself
//! carbon port NAME                          # what a machine publishes: 80/tcp -> 0.0.0.0:8080
//! carbon exec -it NAME bash                 # a shell in a running machine
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
      -i, --interactive                      keep stdin: the machine's stdin is this one's (without it, it's empty)
      -t, --tty                              with -i, this terminal is the machine's console: a shell's prompt and
                                             line editing, Ctrl-C, full-screen programs (-it for a shell, as Docker)
      --isolation vm|sandbox|auto            a hardware VM, the process sandbox, or the best there is (the default)
      --name NAME                            the machine's name (else one from its image or program)
      --platform OS/ARCH                     which build of an image to pull (linux for this CPU by default)
      --display headless=WxH[@SCALE]|host[=WxH]|host-windows|none
                                             where its windows go: a screen in memory (headless=1280x800 by
                                             default; @2 for 2×), the whole desktop in a window on this Mac's
                                             screen, or each of its windows in a Mac window of its own
      --gpu auto|host|software|none          its GPU: the host's, else the CPU device (auto, the default; said if
                                             it's the CPU device), the host's or fail, the CPU device, or none
      --audio auto|host|none|FILE.wav        where its sound goes: the host's speakers when it's on the host's
                                             screen (auto, the default), the host's speakers or fail, nowhere, or a
                                             WAV file of everything played
      -e, --env NAME=VALUE                   an environment variable, over the image's
      --memory SIZE                          how much memory it may use, as 512m or 4g (4g by default)
      -p, --publish [IP:][HOST_PORT:]PORT[/tcp|udp]
                                             a host port that reaches a port in the machine, as Docker's: 2222:22,
                                             127.0.0.1:8080:80, [::1]:8080:80, 5353:53/udp, 8000-8009:8000-8009;
                                             no HOST_PORT takes a free one; no IP is every address, IPv4 and IPv6
      --network nat|host|none, --net         what its network reaches: its own addresses, out through the host and
                                             in only through --publish (nat, the default); the host's ports, with
                                             no publishing (host: a program listening on 80 has the host's 80); or
                                             loopback only (none). A machine run again keeps its last --network and
                                             --publish unless they're given again
  exec [options] NAME COMMAND [ARGS...]      run a command in a running machine, as docker exec
      -i, --interactive                      keep stdin
      -t, --tty                              a terminal of its own, this one (-it for a shell)
      -d, --detach                           start it and leave it running
      -e, --env NAME=VALUE                   an environment variable, over the machine's
      -w, --workdir DIR                      where it starts (the image's working directory by default)
  ls                                         the machines, and which are running
  port NAME [PORT[/tcp|udp]]                 what a running machine publishes: 22/tcp -> 0.0.0.0:2222
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
  import [options] FILE|- [NAME]             an image from a tar of a root file system, as docker import
      --platform OS/ARCH                     its platform (a tar of C:\\ for Windows, as atlas-base's, is found)
      -c, --change INSTRUCTION               its config: CMD, ENTRYPOINT, ENV, WORKDIR or USER, as a Dockerfile
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
        // `--flag=value` as well as `--flag value`, and a short flag's `-p=value` and `-pvalue`, as Docker takes them.
        let (arg, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") && source.is_none() => (flag.to_string(), Some(value.to_string())),
            _ if source.is_none() && (arg.starts_with("-p") || arg.starts_with("-e")) && arg.len() > 2 && !arg.starts_with("--") => {
                (arg[..2].to_string(), Some(arg[2..].trim_start_matches('=').to_string()))
            }
            _ => (arg, None),
        };
        let mut value = |what: &str| inline.clone().or_else(|| args.next()).unwrap_or_else(|| fail(format!("{what} needs a value")));
        match arg.as_str() {
            "--" => {
                spec.args = args.by_ref().collect();
                break;
            }
            "-i" | "--interactive" => spec.interactive = true,
            "-t" | "--tty" => spec.tty = true,
            // Short flags together, as `-it`.
            flag if source.is_none() && flag.len() > 2 && !flag.starts_with("--") && flag.starts_with('-') && flag[1..].chars().all(|c| c == 'i' || c == 't') => {
                spec.interactive |= flag.contains('i');
                spec.tty |= flag.contains('t');
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
            "--audio" => {
                spec.audio = match value("--audio").as_str() {
                    "auto" => machine::Audio::Auto,
                    "host" => machine::Audio::Host,
                    "none" => machine::Audio::None,
                    file if file.ends_with(".wav") => machine::Audio::Wav(file.into()),
                    other => fail(format!("--audio is auto, host, none or a .wav file, not {other}")),
                }
            }
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
            "-p" | "--publish" => {
                let text = value("--publish");
                if let Err(why) = machine::Publish::parse(&text) {
                    fail(format!("--publish {why}"));
                }
                spec.publish.get_or_insert_with(Vec::new).push(text);
            }
            "--network" | "--net" => {
                let text = value("--network");
                spec.network = Some(machine::parse_network(&text).unwrap_or_else(|| fail(format!("--network is nat, host or none, not {text}"))));
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
    // As Docker says it: a console needs a terminal to read from.
    if spec.interactive && spec.tty && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        fail("the input device is not a TTY: -t needs this terminal as stdin (leave out -t to pipe stdin in with -i)");
    }
    if spec.source.is_none() && spec.name.is_none() {
        usage();
    }
    match machines.run(spec, &mut |what| eprintln!("carbon: {what}")) {
        Ok(code) => exit(code),
        Err(why) => fail(why),
    }
}

/// `carbon exec [options] NAME COMMAND [ARGS...]`, as `docker exec`.
fn exec(machines: &Machines, mut args: impl Iterator<Item = String>) -> ! {
    let mut request = machine::ExecRequest::default();
    let (mut interactive, mut tty) = (false, false);
    let mut name = None;
    while let Some(arg) = args.next() {
        let (arg, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_string(), Some(value.to_string())),
            _ if (arg.starts_with("-e") || arg.starts_with("-w")) && arg.len() > 2 => (arg[..2].to_string(), Some(arg[2..].trim_start_matches('=').to_string())),
            _ => (arg, None),
        };
        let mut value = |what: &str| inline.clone().or_else(|| args.next()).unwrap_or_else(|| fail(format!("{what} needs a value")));
        match arg.as_str() {
            "-i" | "--interactive" => interactive = true,
            "-t" | "--tty" => tty = true,
            "-d" | "--detach" => request.detach = true,
            "-e" | "--env" => {
                let var = value("--env");
                if !var.contains('=') {
                    fail(format!("{var}: an environment variable is NAME=VALUE"));
                }
                request.env.push(var);
            }
            "-w" | "--workdir" => request.workdir = Some(value("--workdir")),
            flag if flag.len() > 1 && flag.starts_with('-') && !flag.starts_with("--") && flag[1..].chars().all(|c| "itd".contains(c)) => {
                interactive |= flag.contains('i');
                tty |= flag.contains('t');
                request.detach |= flag.contains('d');
            }
            flag if flag.starts_with('-') => fail(format!("exec doesn't know {flag}")),
            _ => {
                name = Some(arg);
                request.args = args.by_ref().collect();
                break;
            }
        }
    }
    let Some(name) = name else { fail("exec needs a machine's name and a command: carbon exec -it NAME bash") };
    if request.args.is_empty() {
        fail("exec needs a command: carbon exec -it NAME bash");
    }
    if tty && interactive && !request.detach && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        fail("the input device is not a TTY: -t needs this terminal as stdin (leave out -t to pipe stdin in with -i)");
    }
    request.tty = tty.then_some((24, 80));
    match machines.exec(&name, request, interactive) {
        Ok(code) => exit(code),
        Err(why) => fail(why),
    }
}

fn ls(machines: &Machines) {
    let rows: Vec<[String; 5]> = machines
        .list()
        .into_iter()
        .map(|m| {
            let from = m.image.as_deref().map(|i| i.strip_prefix("docker.io/library/").unwrap_or(i).to_string()).or(m.program.clone()).unwrap_or_default();
            let running = m.running();
            let status = if running { format!("running ({})", m.pid.unwrap_or(0)) } else { "stopped".to_string() };
            let ports = if !running {
                String::new()
            } else if m.network == machine::Network::Nat {
                m.ports.join(", ")
            } else {
                format!("--network {}", machine::network_name(m.network))
            };
            [m.name, from, m.backend, status, ports]
        })
        .collect();
    table(&["NAME", "FROM", "ISOLATION", "STATUS", "PORTS"], &rows);
}

/// `carbon port NAME [PORT[/PROTO]]`, as `docker port`: each published port and where on the host it is, or for one
/// port, just where.
fn port(machines: &Machines, name: &str, which: Option<&str>) {
    let record = machines.record(name).unwrap_or_else(|| fail(format!("there's no machine called {name}")));
    // `0.0.0.0:2222->22/tcp` as `22/tcp -> 0.0.0.0:2222`.
    let ports: Vec<(String, String)> = if record.running() { record.ports.iter().filter_map(|p| p.split_once("->")).map(|(host, machine)| (machine.to_string(), host.to_string())).collect() } else { Vec::new() };
    match which {
        None => {
            for (machine, host) in &ports {
                println!("{machine} -> {host}");
            }
        }
        Some(which) => {
            let which = if which.contains('/') { which.to_string() } else { format!("{which}/tcp") };
            let found: Vec<&String> = ports.iter().filter(|(machine, _)| *machine == which).map(|(_, host)| host).collect();
            if found.is_empty() {
                fail(format!("no public port {which} published for {name}"));
            }
            for host in found {
                println!("{host}");
            }
        }
    }
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
        "exec" => exec(&machines, args),
        "ls" | "ps" => ls(&machines),
        "port" => {
            let name = one("a machine's name");
            port(&machines, &name, args.next().as_deref());
        }
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
        "import" => {
            let (mut platform, mut changes, mut rest) = (None, Vec::new(), Vec::new());
            while let Some(arg) = args.next() {
                let (flag, inline) = match arg.split_once('=') {
                    Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
                    _ => (arg.clone(), None),
                };
                let mut value = |what: &str| inline.clone().or_else(|| args.next()).unwrap_or_else(|| fail(format!("{what} needs a value")));
                match flag.as_str() {
                    "--platform" => {
                        let text = value("--platform");
                        platform = Some(Platform::parse(&text).unwrap_or_else(|| fail(format!("{text} isn't a platform: windows/arm64, linux/amd64, ..."))));
                    }
                    "-c" | "--change" => changes.push(value("--change")),
                    _ => rest.push(arg),
                }
            }
            let Some(file) = rest.first() else { fail("import needs a tar: carbon import atlas-base-arm64.tar atlas") };
            let tar = if file == "-" {
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin(), &mut data).unwrap_or_else(|e| fail(format!("reading stdin: {e}")));
                data
            } else {
                std::fs::read(file).unwrap_or_else(|e| fail(format!("{file}: {e}")))
            };
            let name = rest.get(1).cloned().unwrap_or_else(|| {
                let stem = Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or("imported");
                format!("{stem}:latest")
            });
            let image = machines.import(&name, &tar, platform, &changes).unwrap_or_else(|e| fail(e));
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
