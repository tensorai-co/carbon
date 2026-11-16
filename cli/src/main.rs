//! `carbon`: run Linux and Windows programs in machines of their own. A thin layer over the `carbon` library: every
//! command here is something a program can do with it.
//!
//! ```sh
//! carbon run -it debian:13 bash             # a shell in a Linux image, with lambda (pulled if it isn't here)
//! carbon run debian:13 -- ls /              # a command, its output here and no stdin, as `docker run` has it
//! carbon run --name dev debian:13           # named, to start again later with `carbon run --name dev`
//! carbon run --isolation vm debian:13 -- ./untrusted.sh
//! carbon run setup.exe                      # a Windows program, with atlas
//! carbon run -it atlas                      # atlas-base, Windows' userland, imported
//! carbon run --deny 0.0.0.0/0 --allow pypi.org:443 debian:13 -- pip install requests
//! carbon run --deny-exec 'rm -rf *' debian:13 -- ./agent-step.sh
//! carbon watch dev                          # what a running machine does: programs, output, connections, lookups
//! carbon exec -it NAME bash                 # a shell in a running machine
//! carbon ls | stop NAME | rm NAME | freeze NAME IMAGE
//! carbon display screenshot NAME -o screen.png
//! carbon pull IMAGE | import TAR NAME | images | rmi IMAGE
//! ```
//!
//! On macOS the binary is signed with `carbon.entitlements` to use the hypervisor:
//! `codesign --force --sign - --entitlements carbon.entitlements target/release/carbon`.

mod imports;

use std::path::Path;
use std::process::exit;

use carbon::hooks::{Interest, Kind, Verdict};
use carbon::{Audio, Carbon, Display, Gpu, Isolation, Machine, Platform, Rules, Stdio};

const USAGE: &str = "\
usage: carbon <command> ...

  run [options] IMAGE|PROGRAM [-- ARGS...]   run a new machine from an image or a program
  run --name NAME [-- ARGS...]               start a stopped machine again, with its files as it left them
      -i, --interactive                      keep stdin: the machine's stdin is this one's (without it, it's empty)
      -t, --tty                              with -i, this terminal is the machine's console: a shell's prompt and
                                             line editing, Ctrl-C, full-screen programs (-it for a shell, as Docker)
      --isolation vm|sandbox|auto            a hardware VM, the process sandbox, or the best there is (the default)
      --name NAME                            the machine's name (else one from its image or program)
      --rm                                   remove it, and its files, once it ends
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
      -w, --workdir DIR                      where its program starts (the image's working directory by default)
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
      --deny-exec PATTERN                    refuse to start programs whose arguments match: 'rm -rf *' (the first
                                             word a program's name, each after it one argument, * and ? as a
                                             shell's; a last * takes any arguments left)
      --deny HOST|ADDRESS|NETWORK[:PORT]     refuse connections there: 0.0.0.0/0 and ::/0 for everything
      --allow HOST|ADDRESS|NETWORK[:PORT]    let connections there through, whatever --deny says: pypi.org:443
                                             (a host takes its subdomains), 10.0.0.0/8, [::1]:53
  exec [options] NAME COMMAND [ARGS...]      run a command in a running machine, as docker exec
      -i, --interactive                      keep stdin
      -t, --tty                              a terminal of its own, this one (-it for a shell)
      -d, --detach                           start it and leave it running
      -e, --env NAME=VALUE                   an environment variable, over the machine's
      -w, --workdir DIR                      where it starts (the image's working directory by default)
  watch NAME [--exec] [--output] [--net] [--json]
                                             what a running machine does, as it does it: the programs it starts,
                                             what they write, its connections and lookups, and what its rules and
                                             hooks decided (everything unless some are named; --json: a line each)
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

fn run(carbon: &Carbon, mut args: impl Iterator<Item = String>) -> ! {
    let (mut name, mut source, mut argv) = (None, None, Vec::new());
    let (mut interactive, mut tty) = (false, false);
    let mut options: Vec<Box<dyn FnOnce(carbon::MachineSpec) -> carbon::MachineSpec>> = Vec::new();
    let mut rules = Rules::new();
    while let Some(arg) = args.next() {
        // `--flag=value` as well as `--flag value`, and a short flag's `-p=value` and `-pvalue`, as Docker takes them.
        let (arg, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") && source.is_none() => (flag.to_string(), Some(value.to_string())),
            _ if source.is_none() && (arg.starts_with("-p") || arg.starts_with("-e") || arg.starts_with("-w")) && arg.len() > 2 && !arg.starts_with("--") => {
                (arg[..2].to_string(), Some(arg[2..].trim_start_matches('=').to_string()))
            }
            _ => (arg, None),
        };
        let mut value = |what: &str| inline.clone().or_else(|| args.next()).unwrap_or_else(|| fail(format!("{what} needs a value")));
        match arg.as_str() {
            "--" => {
                argv = args.by_ref().collect();
                break;
            }
            "-i" | "--interactive" => interactive = true,
            "-t" | "--tty" => tty = true,
            "--rm" => options.push(Box::new(|s| s.rm(true))),
            // Short flags together, as `-it`.
            flag if source.is_none() && flag.len() > 2 && !flag.starts_with("--") && flag.starts_with('-') && flag[1..].chars().all(|c| c == 'i' || c == 't') => {
                interactive |= flag.contains('i');
                tty |= flag.contains('t');
            }
            "--isolation" => {
                let isolation = match value("--isolation").as_str() {
                    "vm" => Isolation::Vm,
                    "sandbox" => Isolation::Sandbox,
                    "auto" => Isolation::Auto,
                    other => fail(format!("--isolation is vm, sandbox or auto, not {other}")),
                };
                options.push(Box::new(move |s| s.isolation(isolation)));
            }
            "--name" => name = Some(value("--name")),
            "--platform" => {
                let text = value("--platform");
                let platform = Platform::parse(&text).unwrap_or_else(|| fail(format!("{text} isn't a platform: linux/arm64, linux/amd64, windows/amd64")));
                options.push(Box::new(move |s| s.platform(platform)));
            }
            "--display" => {
                let text = value("--display");
                let display = Display::parse(&text).unwrap_or_else(|| fail(format!("--display is headless=WxH, headless=WxH@SCALE, host, host=WxH, host-windows or none, not {text}")));
                options.push(Box::new(move |s| s.display(display)));
            }
            "--gpu" => {
                let gpu = match value("--gpu").as_str() {
                    "auto" => Gpu::Auto,
                    "host" => Gpu::Host,
                    "software" => Gpu::Software,
                    "none" => Gpu::None,
                    other => fail(format!("--gpu is auto, host, software or none, not {other}")),
                };
                options.push(Box::new(move |s| s.gpu(gpu)));
            }
            "--audio" => {
                let audio = match value("--audio").as_str() {
                    "auto" => Audio::Auto,
                    "host" => Audio::Host,
                    "none" => Audio::None,
                    file if file.ends_with(".wav") => Audio::Wav(file.into()),
                    other => fail(format!("--audio is auto, host, none or a .wav file, not {other}")),
                };
                options.push(Box::new(move |s| s.audio(audio)));
            }
            "-e" | "--env" => {
                let var = value("--env");
                let Some((key, val)) = var.split_once('=') else { fail(format!("{var}: an environment variable is NAME=VALUE")) };
                let (key, val) = (key.to_string(), val.to_string());
                options.push(Box::new(move |s| s.env(&key, &val)));
            }
            "-w" | "--workdir" => {
                let dir = value("--workdir");
                options.push(Box::new(move |s| s.workdir(dir)));
            }
            "--memory" => {
                let text = value("--memory");
                if carbon::parse_size(&text).is_none() {
                    fail(format!("{text} isn't a size: 512m, 4g"));
                }
                options.push(Box::new(move |s| s.memory(text)));
            }
            "-p" | "--publish" => {
                let text = value("--publish");
                if let Err(why) = carbon::Publish::parse(&text) {
                    fail(format!("--publish {why}"));
                }
                options.push(Box::new(move |s| s.publish(&text)));
            }
            "--network" | "--net" => {
                let text = value("--network");
                let network = carbon::parse_network(&text).unwrap_or_else(|| fail(format!("--network is nat, host or none, not {text}")));
                options.push(Box::new(move |s| s.network(network)));
            }
            "--deny-exec" => rules = rules.deny_exec(&value("--deny-exec")),
            "--deny" => rules = rules.deny(&value("--deny")).unwrap_or_else(|e| fail(format!("--deny {e}"))),
            "--allow" => rules = rules.allow(&value("--allow")).unwrap_or_else(|e| fail(format!("--allow {e}"))),
            flag if flag.starts_with('-') => fail(format!("run doesn't know {flag}")),
            _ if source.is_none() => source = Some(arg),
            _ => {
                // Everything after what to run is its arguments, `--` or not.
                argv = std::iter::once(arg).chain(args.by_ref()).collect();
                break;
            }
        }
    }
    // As Docker says it: a console needs a terminal to read from.
    if interactive && tty && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        fail("the input device is not a TTY: -t needs this terminal as stdin (leave out -t to pipe stdin in with -i)");
    }
    let spec = match (source, name) {
        (Some(what), name) => {
            let spec = if Path::new(&what).is_file() { carbon.program(what) } else { carbon.machine(what) };
            match name {
                Some(name) => spec.name(name),
                None => spec,
            }
        }
        (None, Some(name)) => carbon.again(name),
        (None, None) => usage(),
    };
    let mut spec = spec.stdin(if interactive { Stdio::Inherit } else { Stdio::Null }).tty(interactive && tty).relay_signals(true).rules(rules);
    for option in options {
        spec = option(spec);
    }
    match spec.run(argv) {
        Ok(exit_code) => exit(exit_code.code()),
        Err(why) => fail(why),
    }
}

/// `carbon exec [options] NAME COMMAND [ARGS...]`, as `docker exec`.
fn exec(carbon: &Carbon, mut args: impl Iterator<Item = String>) -> ! {
    let (mut interactive, mut tty, mut detach) = (false, false, false);
    let (mut env, mut workdir) = (Vec::new(), None);
    let mut target = None;
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
            "-d" | "--detach" => detach = true,
            "-e" | "--env" => {
                let var = value("--env");
                let Some((key, val)) = var.split_once('=') else { fail(format!("{var}: an environment variable is NAME=VALUE")) };
                env.push((key.to_string(), val.to_string()));
            }
            "-w" | "--workdir" => workdir = Some(value("--workdir")),
            flag if flag.len() > 1 && flag.starts_with('-') && !flag.starts_with("--") && flag[1..].chars().all(|c| "itd".contains(c)) => {
                interactive |= flag.contains('i');
                tty |= flag.contains('t');
                detach |= flag.contains('d');
            }
            flag if flag.starts_with('-') => fail(format!("exec doesn't know {flag}")),
            _ => {
                target = Some((arg, args.by_ref().collect::<Vec<_>>()));
                break;
            }
        }
    }
    let Some((name, command)) = target else { fail("exec needs a machine's name and a command: carbon exec -it NAME bash") };
    if command.is_empty() {
        fail("exec needs a command: carbon exec -it NAME bash");
    }
    if tty && interactive && !detach && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        fail("the input device is not a TTY: -t needs this terminal as stdin (leave out -t to pipe stdin in with -i)");
    }
    let machine = carbon.get(&name).unwrap_or_else(|e| fail(e));
    let mut spec = machine.exec(command);
    for (key, val) in &env {
        spec = spec.env(key, val);
    }
    if let Some(dir) = workdir {
        spec = spec.workdir(dir);
    }
    if tty {
        // Its real size is this terminal's, taken as it starts.
        spec = spec.tty(24, 80);
    }
    if detach {
        spec.detach().unwrap_or_else(|e| fail(e));
        exit(0);
    }
    match spec.attach(interactive) {
        Ok(exit_code) => exit(exit_code.code()),
        Err(why) => fail(why),
    }
}

/// `carbon watch NAME [--exec] [--output] [--net] [--json]`: a running machine's hooked events, as they're decided.
fn watch(machine: &Machine, args: impl Iterator<Item = String>) {
    let mut interest = Interest::none();
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--exec" => interest |= Interest::exec(),
            "--output" => interest |= Interest::output(),
            "--net" | "--network" => interest |= Interest::network(),
            "--json" => json = true,
            other => fail(format!("watch doesn't know {other}: --exec, --output, --net, --json")),
        }
    }
    if interest.is_empty() {
        interest = Interest::all();
    }
    let mut feed = machine.watch(interest).unwrap_or_else(|e| fail(e));
    let mut lost = 0;
    while let Some(line) = feed.next_line() {
        if json {
            println!("{line}");
            continue;
        }
        let Some(observed) = carbon::wire::parse_observed(&line) else { continue };
        // A shell's tries along its PATH that found nothing, and were let fail as they would, aren't news.
        if matches!((&observed.event, &observed.verdict), (carbon::hooks::Event::Exec(e), Verdict::Allow) if !e.found) {
            continue;
        }
        if feed.lost() != lost {
            eprintln!("carbon: {} events lost: this fell behind", feed.lost() - lost);
            lost = feed.lost();
        }
        let secs = observed.at.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
        let verdict = match &observed.verdict {
            Verdict::Allow => String::new(),
            Verdict::Deny => " [denied]".to_string(),
            Verdict::Rewrite(e) => format!(" [rewritten: {}]", e.describe()),
            Verdict::Answer(_) => " [answered]".to_string(),
        };
        let kind = observed.event.kind();
        let mut what = observed.event.describe();
        // Output's own words are enough; long output is cut.
        if kind == Kind::Output && what.len() > 200 {
            what.truncate(200);
            what.push_str("...");
        }
        println!("{h:02}:{m:02}:{s:02} UTC  {what}{verdict}");
    }
}

fn ls(carbon: &Carbon) {
    let rows: Vec<[String; 5]> = carbon
        .machines()
        .into_iter()
        .map(|m| {
            let status = if m.running { format!("running ({})", m.pid.unwrap_or(0)) } else { "stopped".to_string() };
            let ports = if !m.running {
                String::new()
            } else if m.network == carbon::Network::Nat {
                m.ports.iter().map(|p| format!("{}->{}/{}", p.host, p.port, p.protocol)).collect::<Vec<_>>().join(", ")
            } else {
                format!("--network {}", carbon::network_name(m.network))
            };
            [m.name.clone(), m.from(), m.backend.clone(), status, ports]
        })
        .collect();
    table(&["NAME", "FROM", "ISOLATION", "STATUS", "PORTS"], &rows);
}

/// `carbon port NAME [PORT[/PROTO]]`, as `docker port`: each published port and where on the host it is, or for one
/// port, just where.
fn port(carbon: &Carbon, name: &str, which: Option<&str>) {
    let ports = carbon.get(name).and_then(|m| m.ports()).unwrap_or_else(|e| fail(e));
    match which {
        None => {
            for p in &ports {
                println!("{p}");
            }
        }
        Some(which) => {
            let which = if which.contains('/') { which.to_string() } else { format!("{which}/tcp") };
            let found: Vec<_> = ports.iter().filter(|p| format!("{}/{}", p.port, p.protocol) == which).collect();
            if found.is_empty() {
                fail(format!("no public port {which} published for {name}"));
            }
            for p in found {
                println!("{}", p.host);
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
    carbon::init();
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| usage());
    if matches!(command.as_str(), "help" | "--help" | "-h") {
        print!("{USAGE}");
        return;
    }
    let carbon = Carbon::open().unwrap_or_else(|e| fail(e));
    let mut one = |what: &str| args.next().unwrap_or_else(|| fail(format!("{command} needs {what}")));
    let machine = |name: &str| carbon.get(name).unwrap_or_else(|e| fail(e));
    match command.as_str() {
        "run" => run(&carbon, args),
        "exec" => exec(&carbon, args),
        "watch" => {
            let name = one("a machine's name");
            watch(&machine(&name), args);
        }
        "ls" | "ps" => ls(&carbon),
        "port" => {
            let name = one("a machine's name");
            port(&carbon, &name, args.next().as_deref());
        }
        "gpu" => {
            println!("{:<10} {:<24} {:<10} SUBGROUP", "KIND", "NAME", "BACKEND");
            for (kind, device) in ["host", "software"].into_iter().zip(carbon::gpus()) {
                match device {
                    Ok(info) => println!("{kind:<10} {:<24} {:<10} {}", info.name, info.backend, info.subgroup_size),
                    Err(why) => println!("{kind:<10} -- ({why})"),
                }
            }
        }
        "stop" => machine(&one("a machine's name")).stop().unwrap_or_else(|e| fail(e)),
        "rm" => machine(&one("a machine's name")).remove().unwrap_or_else(|e| fail(e)),
        "cp" => {
            let (from, to) = (one("NAME:PATH or a path on this host"), one("a folder on this host or NAME:PATH"));
            // Out of a machine when the first names one, as web:/etc/hosts; else into the machine the second names.
            let named = |arg: &str| arg.split_once(':').and_then(|(name, path)| carbon.get(name).ok().map(|m| (m, path.to_string())));
            let n = match (named(&from), named(&to)) {
                (Some((m, path)), _) => m.copy_out(&path, &to),
                (None, Some((m, path))) => m.copy_in(&from, &path),
                (None, None) => fail("cp copies NAME:PATH DIR out of a machine, or PATH NAME:DIR into one, as web:/etc/hosts".to_string()),
            }
            .unwrap_or_else(|e| fail(e));
            println!("{n} files");
        }
        "freeze" => {
            let (name, image) = (one("a machine's name"), one("a name for the image"));
            let image = machine(&name).freeze(&image).unwrap_or_else(|e| fail(e));
            println!("{}", image.short_name());
        }
        "display" => {
            let verb = one("what to do");
            let name = one("a machine's name");
            let screen = machine(&name).screen();
            match verb.as_str() {
                "screenshot" => {
                    let out = match (args.next().as_deref(), args.next()) {
                        (Some("-o"), Some(file)) => file,
                        (None, _) => "screen.png".to_string(),
                        _ => usage(),
                    };
                    let png = screen.screenshot().unwrap_or_else(|e| fail(e));
                    std::fs::write(&out, png).unwrap_or_else(|e| fail(format!("{out}: {e}")));
                    println!("{out}");
                }
                "windows" => print!("{}", screen.windows_table().unwrap_or_else(|e| fail(e))),
                "clipboard" => {
                    let text: Vec<String> = args.by_ref().collect();
                    if text.is_empty() {
                        print!("{}", screen.clipboard().unwrap_or_else(|e| fail(e)));
                    } else {
                        screen.set_clipboard(&text.join(" ")).unwrap_or_else(|e| fail(e));
                    }
                }
                "drop" => {
                    let mut point = || args.next().and_then(|n| n.parse::<i32>().ok()).unwrap_or_else(|| fail("a drop needs a point: X Y"));
                    let (x, y) = (point(), point());
                    let mut rest: Vec<String> = args.by_ref().collect();
                    let files = rest.first().is_some_and(|f| f == "--files");
                    if files {
                        rest.remove(0);
                    }
                    if rest.is_empty() {
                        fail(if files { "drop --files needs a path" } else { "drop needs text" });
                    }
                    let done = if files { screen.drop_files(x, y, &rest.iter().map(String::as_str).collect::<Vec<_>>()) } else { screen.drop_text(x, y, &rest.join(" ")) };
                    println!("{}", done.unwrap_or_else(|e| fail(e)));
                }
                "resize" => {
                    let size = args.next().unwrap_or_else(|| fail("resize needs a size: WIDTHxHEIGHT, or WIDTHxHEIGHT@SCALE"));
                    screen.send("resize", &[size]).unwrap_or_else(|e| fail(e));
                }
                "move" | "click" | "drag" | "scroll" | "type" | "key" => {
                    let rest: Vec<String> = args.by_ref().collect();
                    screen.send(&verb, &rest).unwrap_or_else(|e| fail(e));
                }
                other => fail(format!("display doesn't {other} yet: screenshot, windows, move, click, drag, scroll, type, key, resize, clipboard, drop")),
            }
        }
        "atlas" => match one("what to do").as_str() {
            "imports" => imports::run(args.by_ref().collect()),
            other => fail(format!("atlas doesn't {other}: imports")),
        },
        "pull" => {
            let name = one("an image");
            let platform = match (args.next().as_deref(), args.next()) {
                (Some("--platform"), Some(text)) => Some(Platform::parse(&text).unwrap_or_else(|| fail(format!("{text} isn't a platform")))),
                (None, _) => None,
                _ => usage(),
            };
            let image = carbon.pull(&name, platform).unwrap_or_else(|e| fail(e));
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
            let changes: Vec<&str> = changes.iter().map(String::as_str).collect();
            let image = carbon.import(&name, &tar, platform, &changes).unwrap_or_else(|e| fail(e));
            println!("{} ({})", image.short_name(), image.platform);
        }
        "images" => {
            let rows: Vec<[String; 3]> = carbon.images().into_iter().map(|i| [i.short_name().to_string(), i.platform.to_string(), i.source]).collect();
            table(&["IMAGE", "PLATFORM", "FROM"], &rows);
        }
        "rmi" => carbon.remove_image(&one("an image")).unwrap_or_else(|e| fail(e)),
        other => fail(format!("{other} isn't a command: carbon help")),
    }
}
