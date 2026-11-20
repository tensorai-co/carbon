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
//! carbon ps | stop NAME | rm NAME | commit NAME [IMAGE]
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
      -d, --detach                           run it in a process of its own, which outlives this one, and print its
                                             name; what it writes is kept (carbon logs NAME)
      --timeout DURATION                     stop it if it's still running this long after it starts: 90s, 5m, 2h
      --isolation vm|sandbox|auto            a hardware VM, the process sandbox, or the best there is (the default)
      --name NAME                            the machine's name (else one from its image or program)
      -h, --hostname NAME                    the machine's hostname (its name by default)
      -l, --label KEY[=VALUE]                a label, kept with it over its image's (and in an image committed from
                                             it); carbon ps --filter label=KEY finds it
      --label-file FILE                      labels from a file, a KEY[=VALUE] line each
      --add-host NAME:IP                     a line for its /etc/hosts (NAME=IP too; IPv6 as it is, or in brackets)
      --rm                                   remove it, and its files, once it ends
      -v, --volume SOURCE:TARGET[:ro]        mount a host folder (/path, ./path, ~/path) or a volume (a name) at TARGET:
                                             each side sees the other's changes at once; a new volume is filled from
                                             the image's files at TARGET
      --mount type=bind|volume|tmpfs,source=...,target=...[,readonly]
                                             the same, as Docker's --mount
      --tmpfs TARGET                         a folder in memory only
      -P, --publish-all                      publish every port its image exposes, each on a free host port
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
      --env-file FILE                        environment variables from a file, as Docker reads one: a NAME=VALUE
                                             line each (NAME alone takes this one's), # for comments
      -w, --workdir DIR                      where its program starts (the image's working directory by default),
                                             made if it isn't there
      -u, --user USER[:GROUP]                who it runs as, by name or number from the image's /etc/passwd (the
                                             image's USER by default, else root)
      --memory SIZE                          how much memory it may use, as 512m or 4g (4g, less in a smaller container)
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
      --registries apt,npm,pypi,git          its package managers and git answered by Carbon at registry.carbon, and each
                                             request sent to --upstream as one checked call (POST /v1/apt/deb, …);
                                             nothing the machine sends goes anywhere, and it works under --deny
                                             0.0.0.0/0 and --network none (npm's NPM_CONFIG_REGISTRY, pip's
                                             PIP_INDEX_URL and git's insteadOf, for each host the server names,
                                             are set)
      --upstream URL|SERVICE=URL             the package server: https://…, or http:// to this host's loopback
      --upstream-token-file FILE             the token sent with every call (as a bearer), never in the machine
      --registry-meta KEY=VALUE              a field of meta on every call, for the server: sid=8f3a
      --deny-registry SERVICE[:OP] [FIELD=PATTERN ...]
                                             refuse registry calls: npm, git:push, 'apt:deb name=nmap*'
  exec [options] NAME COMMAND [ARGS...]      run a command in a running machine, as docker exec
      -i, --interactive                      keep stdin
      -t, --tty                              a terminal of its own, this one (-it for a shell)
      -d, --detach                           start it and leave it running
      -e, --env NAME=VALUE                   an environment variable, over the machine's
      --env-file FILE                        environment variables from a file, a NAME=VALUE line each
      -w, --workdir DIR                      where it starts (the image's working directory by default)
      -u, --user USER[:GROUP]                who it runs as (the machine's first program's user by default)
      --timeout DURATION                     kill it if it hasn't ended this long after it starts (exit 124)
  build [options] CONTEXT                    build an image from CONTEXT's Carbonfile (else its Dockerfile)
      -t, --tag NAME                         name it (as many as wanted)
      -f, --file FILE                        the file to build, or - for stdin (a Dockerfile, exactly)
      --build-arg NAME=VALUE                 a value for an ARG the file declares
      --target STAGE                         the stage to build (the last by default)
      --platform OS/ARCH                     what to build for: linux/amd64 on Apple silicon runs translated
      --no-cache                             run every step, finding none in the cache
      --secret id=ID,src=FILE|env=VAR        a secret for RUN --mount=type=secret,id=ID: at /run/secrets/ID for
                                             that step only, never in the image
      -o, --output DIR                       also copy what's built to this folder
      --output type=docker|oci,dest=FILE     also write the image as an archive, as carbon save does
      --isolation, --network, --memory, --timeout, --deny, --allow, --deny-exec
                                             as run's, for every RUN
      --squash                               what the steps did as one layer, on the base's layers as they are
      -q, --quiet                            print only the image's name
  volume create|ls|inspect|rm [NAME]         volumes: folders kept under CARBON_HOME that machines mount and outlive
  logs [-f] [--tail N] NAME                  what a machine run with -d wrote: -f keeps following until it stops
  watch NAME [--exec] [--output] [--net] [--registry] [--json]
                                             what a running machine does, as it does it: the programs it starts,
                                             what they write, its connections, lookups and registry calls, and what its rules and
                                             hooks decided (everything unless some are named; --json: a line each)
  ps [--filter label=KEY[=VALUE]]            the machines, and which are running (ls too); with a filter, only
                                             those with that label (as many filters as wanted, all of them)
  port NAME [PORT[/tcp|udp]]                 what a running machine publishes: 22/tcp -> 0.0.0.0:2222
  gpu                                        the devices a machine's GPU can be: this host's, and the CPU device
  stop NAME                                  stop a running machine (SIGTERM, then everything after 10s)
  rm NAME                                    remove a stopped machine and its files
  commit [options] NAME [IMAGE]              an image of a machine's disk, running or not, as docker commit: its
                                             image and one more layer of what it changed (unnamed without IMAGE)
      -m, --message TEXT                     its history entry's comment
      -c, --change INSTRUCTION               its config: CMD, ENTRYPOINT, ENV, EXPOSE, LABEL, ONBUILD, USER,
                                             VOLUME or WORKDIR, as a Dockerfile
  cp NAME:PATH DIR                           copy a machine's file or folder into DIR on this host, running or not:
                                             modes, times and links kept
  cp PATH NAME:DIR                           copy this host's file or folder into a machine's DIR, running or not
                                             (PATH/. for what's in a folder); root's, as docker cp makes it
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
  mcp [options]                              Carbon's front door: an MCP server for agents, on stdin and stdout by
                                             default (claude mcp add carbon -- carbon mcp), or on the network (-H)
      --machine NAME                         serve one machine alone: its tools, with no machine argument
      --host-dir DIR                         a host folder tools may name, for binds, copies and archives (as many
                                             as wanted; none by default, and then no host paths at all)
      --deny-exec, --deny, --allow, --deny-registry
                                             rules on every machine it starts, as carbon run's
      --network, --memory, --timeout         defaults for every machine it starts, as carbon run's
      --registries, --upstream, --upstream-token-file, --registry-meta
                                             registries for every machine it starts, as carbon run's
      --tools NAME,...                       list only these tools
      -H, --host ADDR                        listen on host:port, or a Unix socket's path (no scheme), instead of
                                             stdio: MCP at POST /mcp, and a WebSocket carrying everything, on the
                                             same address; as many as wanted
      --engine                               also serve the Engine API, the control plane: engine/... methods, and
                                             REST under /v1 (off by default)
      --max-sessions N                       serve N sessions in all, then exit (not how many at once)
      --tls, --tlscert FILE, --tlskey FILE   TLS 1.3, with a certificate and key in PEM files
      --token-file FILE                      tokens that may connect, a line each: SCOPES SHA256 NAME, scopes
                                             carbon:mcp and carbon:engine (required beyond loopback)
      --allow-origin ORIGIN                  a browser origin that may call it (none by default)
      --no-auth                              no tokens beyond loopback: something in front of it does auth
  mcp token [--scope SCOPES] [--name NAME]   a new token, and its line for --token-file (scopes carbon:mcp by
                                             default; carbon:mcp,carbon:engine for both)
  push [--squash] IMAGE                      push an image to its registry, a layer per step of what made it
      --squash                               its whole disk as one layer, base included (build --squash keeps the
                                             base's layers and makes one more)
  login [options] [REGISTRY]                 log in to a registry (docker.io by default), for pushes and private pulls
      -u, --username USER                    who to be (asked for if not given)
      --password-stdin                       the password or token on stdin (else asked for, unseen)
  logout [REGISTRY]                          forget the login for a registry
  save [-o FILE] IMAGE...                    images as one archive, as docker save writes it (to stdout without -o)
  load [-i FILE]                             the images an archive has, docker save's or an OCI layout's (stdin
                                             without -i)
  import [options] FILE|- [NAME]             an image from a tar of a root file system, as docker import
      --platform OS/ARCH                     its platform (a tar of C:\\ for Windows, as atlas-base's, is found)
      -c, --change INSTRUCTION               its config: CMD, ENTRYPOINT, ENV, EXPOSE, LABEL, ONBUILD, USER,
                                             VOLUME or WORKDIR, as a Dockerfile
  images                                     the images here
  inspect IMAGE                              an image's config, history and layers, as JSON
  history IMAGE                              the steps that made an image, newest first
  tag IMAGE NAME                             another name for an image: the same disk and config
  rmi IMAGE                                  forget an image
";

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("carbon: {message}");
    exit(1)
}

/// Asks on the terminal, and reads a line: seen as it's typed, or not (a password: the terminal's echo off meanwhile).
fn ask(prompt: &str, seen: bool) -> String {
    use std::io::Write;
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let stty = |arg: &str| {
        let _ = std::process::Command::new("stty").arg(arg).stdin(std::process::Stdio::inherit()).status();
    };
    if !seen && cfg!(unix) {
        stty("-echo");
    }
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    if !seen && cfg!(unix) {
        stty("echo");
        eprintln!();
    }
    read.unwrap_or_else(|e| fail(format!("reading the terminal: {e}")));
    line.trim_end_matches(['\r', '\n']).to_string()
}

fn usage() -> ! {
    eprint!("{USAGE}");
    exit(2)
}

/// A length of time as `--timeout` takes it: `90` or `90s`, `500ms`, `5m`, `2h`.
/// `--env-file`, as Docker reads one: a `NAME=VALUE` line each, the value as it is (quotes and all); `NAME` alone takes
/// this process's, and is left out if it has none; blank lines and `#` lines skipped.
fn env_file(path: &str) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("--env-file {path}: {e}")));
    let mut vars = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((key, value)) => (key, Some(value.to_string())),
            None => (line.trim_end(), None),
        };
        if key.is_empty() || key.contains(char::is_whitespace) {
            fail(format!("--env-file {path}: line {}: {key:?} isn't a variable's name", i + 1));
        }
        match value.or_else(|| std::env::var(key).ok()) {
            Some(value) => vars.push((key.to_string(), value)),
            None => continue,
        }
    }
    vars
}

/// `--label-file`: a `KEY[=VALUE]` line each, blank lines and `#` lines skipped.
fn label_file(path: &str) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("--label-file {path}: {e}")));
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (k, v) = l.split_once('=').unwrap_or((l, ""));
            (k.to_string(), v.to_string())
        })
        .collect()
}

fn duration(text: &str) -> std::time::Duration {
    let (number, unit) = text.find(|c: char| !c.is_ascii_digit()).map_or((text, ""), |at| text.split_at(at));
    let n: u64 = number.parse().unwrap_or_else(|_| fail(format!("{text} isn't a length of time: 90, 90s, 500ms, 5m, 2h")));
    std::time::Duration::from_millis(match unit {
        "" | "s" => n * 1000,
        "ms" => n,
        "m" => n * 60_000,
        "h" => n * 3_600_000,
        _ => fail(format!("{text} isn't a length of time: 90, 90s, 500ms, 5m, 2h")),
    })
}

/// `carbon build`: its arguments made into a `Build`, and its steps printed.
fn build(carbon: &Carbon, mut args: impl Iterator<Item = String>) {
    let (mut context, mut file, mut tags, mut quiet) = (None, None, Vec::new(), false);
    // `--output type=docker|oci,dest=FILE`: the image, as an archive, once it's built.
    let mut archive: Option<String> = None;
    let mut rules = Rules::new();
    let mut options: Vec<Box<dyn FnOnce(carbon::build::Build) -> carbon::build::Build>> = Vec::new();
    while let Some(arg) = args.next() {
        let (arg, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_string(), Some(value.to_string())),
            _ => (arg, None),
        };
        let mut value = |what: &str| inline.clone().or_else(|| args.next()).unwrap_or_else(|| fail(format!("{what} needs a value")));
        match arg.as_str() {
            "-t" | "--tag" => tags.push(value("--tag")),
            "-f" | "--file" => file = Some(value("--file")),
            "--build-arg" => {
                let pair = value("--build-arg");
                let (k, v) = match pair.split_once('=') {
                    Some((k, v)) => (k.to_string(), v.to_string()),
                    // `--build-arg NAME` takes this process's value, as Docker does.
                    None => (pair.clone(), std::env::var(&pair).unwrap_or_default()),
                };
                options.push(Box::new(move |b| b.arg(k, v)));
            }
            "--target" => {
                let t = value("--target");
                options.push(Box::new(move |b| b.target(t)));
            }
            "--platform" => {
                let text = value("--platform");
                let p = Platform::parse(&text).unwrap_or_else(|| fail(format!("--platform {text}: linux/arm64, linux/amd64, windows/amd64…")));
                options.push(Box::new(move |b| b.platform(p)));
            }
            "--isolation" => {
                let isolation = match value("--isolation").as_str() {
                    "vm" => Isolation::Vm,
                    "sandbox" => Isolation::Sandbox,
                    "auto" => Isolation::Auto,
                    other => fail(format!("--isolation {other}: vm, sandbox or auto")),
                };
                options.push(Box::new(move |b| b.isolation(isolation)));
            }
            "--network" | "--net" => {
                let text = value("--network");
                let n = carbon::parse_network(&text).unwrap_or_else(|| fail(format!("--network {text}: default, none or host")));
                options.push(Box::new(move |b| b.network(n)));
            }
            "--memory" | "-m" => {
                let m = value("--memory");
                options.push(Box::new(move |b| b.memory(m)));
            }
            "--timeout" => {
                let t = duration(&value("--timeout"));
                options.push(Box::new(move |b| b.timeout(t)));
            }
            "--no-cache" => options.push(Box::new(|b| b.no_cache(true))),
            "--squash" => options.push(Box::new(|b| b.squash(true))),
            "--secret" => {
                // id=ID,src=FILE (or source=), or id=ID,env=VAR: a host variable's value, as Docker takes it.
                let text = value("--secret");
                let field = |k: &str| text.split(',').find_map(|p| p.strip_prefix(k).and_then(|r| r.strip_prefix('='))).map(String::from);
                let id = field("id").unwrap_or_else(|| fail(format!("--secret {text}: id=ID,src=FILE")));
                let file = match (field("src").or_else(|| field("source")), field("env")) {
                    (Some(src), _) => std::path::PathBuf::from(src),
                    (None, Some(var)) => {
                        let value = std::env::var(&var).unwrap_or_else(|_| fail(format!("--secret {text}: {var} isn't set")));
                        let temp = std::env::temp_dir().join(format!("carbon-secret-{}-{id}", std::process::id()));
                        std::fs::write(&temp, value).unwrap_or_else(|e| fail(e));
                        temp
                    }
                    (None, None) => fail(format!("--secret {text}: src=FILE or env=VAR")),
                };
                options.push(Box::new(move |b| b.secret(id, file)));
            }
            "-o" | "--output" => {
                let text = value("--output");
                let fields: Vec<(&str, &str)> = text.split(',').filter_map(|f| f.split_once('=')).collect();
                let get = |k: &str| fields.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string());
                match (get("type").as_deref(), get("dest")) {
                    (Some("docker" | "oci" | "image-archive"), Some(dest)) => archive = Some(dest),
                    (Some("docker" | "oci" | "image-archive"), None) => fail(format!("--output {text}: dest=FILE")),
                    (Some("local") | None, dest) => {
                        let dir = dest.unwrap_or_else(|| text.clone());
                        options.push(Box::new(move |b| b.output(dir)));
                    }
                    (Some(other), _) => fail(format!("--output type={other} isn't there: local, docker or oci")),
                }
            }
            // Accepted as Docker's: one line a step is all there is.
            "--progress" => {
                let _ = value("--progress");
            }
            "-q" | "--quiet" => quiet = true,
            "--deny-exec" => rules = rules.deny_exec(&value("--deny-exec")),
            "--deny" => rules = rules.deny(&value("--deny")).unwrap_or_else(|e| fail(format!("--deny {e}"))),
            "--allow" => rules = rules.allow(&value("--allow")).unwrap_or_else(|e| fail(format!("--allow {e}"))),
            flag if flag.starts_with('-') && flag.len() > 1 => fail(format!("build: unknown option {flag}")),
            path if context.is_none() => context = Some(path.to_string()),
            extra => fail(format!("build takes one context, not {extra} too")),
        }
    }
    let context = context.unwrap_or_else(|| fail("build needs a context: a folder (.)"));
    let mut b = carbon.build(&context).rules(rules);
    if let Some(file) = file {
        // `-f -`: the file from stdin.
        let path = if file == "-" {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).unwrap_or_else(|e| fail(format!("reading the file from stdin: {e}")));
            let temp = std::env::temp_dir().join(format!("carbon-build-{}.Carbonfile", std::process::id()));
            std::fs::write(&temp, text).unwrap_or_else(|e| fail(e));
            temp
        } else {
            std::path::PathBuf::from(file)
        };
        b = b.file(path);
    }
    for tag in &tags {
        b = b.tag(tag);
    }
    for option in options {
        b = option(b);
    }
    let built = b
        .run_with(|step| {
            if quiet {
                return;
            }
            match step {
                carbon::build::Step::Output { line, .. } => eprint!("    {}", line.text()),
                carbon::build::Step::Done { .. } => {}
                other => eprintln!("{other}"),
            }
        })
        .unwrap_or_else(|e| fail(e));
    if !quiet {
        eprintln!("built in {:.1}s: {} ran, {} cached", built.took.as_secs_f64(), built.ran, built.cached);
    }
    if let Some(dest) = archive {
        let names: Vec<&str> = if tags.is_empty() { vec![built.image.name.as_str()] } else { tags.iter().map(String::as_str).collect() };
        let out = std::fs::File::create(&dest).unwrap_or_else(|e| fail(format!("{dest}: {e}")));
        carbon.save(&names, std::io::BufWriter::with_capacity(1 << 20, out)).unwrap_or_else(|e| fail(e));
    }
    println!("{}", built.image.short_name());
}

fn run(carbon: &Carbon, mut args: impl Iterator<Item = String>) -> ! {
    let (mut name, mut source, mut argv) = (None, None, Vec::new());
    let (mut interactive, mut tty, mut detach) = (false, false, false);
    let mut options: Vec<Box<dyn FnOnce(carbon::MachineSpec) -> carbon::MachineSpec>> = Vec::new();
    let mut rules = Rules::new();
    let mut registries = carbon::Registries::default();
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
            "-d" | "--detach" => detach = true,
            "--rm" => options.push(Box::new(|s| s.rm(true))),
            "-v" | "--volume" | "--mount" | "--tmpfs" => {
                let text = value(&arg);
                let text = if arg == "--tmpfs" { format!("type=tmpfs,target={}", text.split(':').next().unwrap_or("")) } else { text };
                let mount = carbon::Mount::parse(&text).unwrap_or_else(|e| fail(format!("{arg} {e}")));
                options.push(Box::new(move |s| s.mount(mount)));
            }
            "-P" | "--publish-all" => options.push(Box::new(|s| s.publish_all())),
            "--timeout" => {
                let timeout = duration(&value("--timeout"));
                options.push(Box::new(move |s| s.timeout(timeout)));
            }
            // Short flags together, as `-it`.
            flag if source.is_none() && flag.len() > 2 && !flag.starts_with("--") && flag.starts_with('-') && flag[1..].chars().all(|c| "itd".contains(c)) => {
                interactive |= flag.contains('i');
                tty |= flag.contains('t');
                detach |= flag.contains('d');
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
            "-h" | "--hostname" => {
                let hostname = value("--hostname");
                options.push(Box::new(move |s| s.hostname(hostname)));
            }
            "-l" | "--label" | "--label-file" => {
                let text = value(&arg);
                let labels = if arg == "--label-file" {
                    label_file(&text)
                } else {
                    let (k, v) = text.split_once('=').unwrap_or((&text, ""));
                    vec![(k.to_string(), v.to_string())]
                };
                for (k, v) in labels {
                    options.push(Box::new(move |s| s.label(k, v)));
                }
            }
            "--add-host" => {
                let host = carbon::ExtraHost::parse(&value("--add-host")).unwrap_or_else(|e| fail(e));
                options.push(Box::new(move |s| s.add_host(host.name, host.ip)));
            }
            "--env-file" => {
                for (key, val) in env_file(&value("--env-file")) {
                    options.push(Box::new(move |s| s.env(&key, &val)));
                }
            }
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
            "-u" | "--user" => {
                let user = value("--user");
                options.push(Box::new(move |s| s.user(user)));
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
            "--deny-registry" => rules = rules.deny_registry(&value("--deny-registry")).unwrap_or_else(|e| fail(e)),
            flag if registry_flag(&mut registries, flag, &mut value) => {}
            flag if flag.starts_with('-') => fail(format!("run doesn't know {flag}")),
            _ if source.is_none() => source = Some(arg),
            _ => {
                // Everything after what to run is its arguments, `--` or not.
                argv = std::iter::once(arg).chain(args.by_ref()).collect();
                break;
            }
        }
    }
    if detach && (interactive || tty) {
        fail("-d leaves the machine running in a process of its own, with no stdin or terminal: carbon exec -it NAME bash for a shell in it");
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
    let mut spec = spec.stdin(if interactive { Stdio::Inherit } else { Stdio::Null }).tty(interactive && tty).relay_signals(!detach).rules(rules);
    for option in options {
        spec = option(spec);
    }
    if let Some(r) = registries_given(registries) {
        spec = spec.registries(r);
    }
    if detach {
        // In a process of its own, which outlives this one: its name, as Docker prints a container's id.
        let machine = spec.launch(argv).unwrap_or_else(|e| fail(e));
        println!("{}", machine.name());
        exit(0);
    }
    match spec.run(argv) {
        Ok(exit_code) => exit(exit_code.code()),
        Err(why) => fail(why),
    }
}

/// `carbon exec [options] NAME COMMAND [ARGS...]`, as `docker exec`.
fn exec(carbon: &Carbon, mut args: impl Iterator<Item = String>) -> ! {
    let (mut interactive, mut tty, mut detach) = (false, false, false);
    let (mut env, mut workdir, mut timeout, mut user) = (Vec::new(), None, None, None::<String>);
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
            "--env-file" => env.extend(env_file(&value("--env-file"))),
            "-w" | "--workdir" => workdir = Some(value("--workdir")),
            "-u" | "--user" => user = Some(value("--user")),
            "--timeout" => timeout = Some(duration(&value("--timeout"))),
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
    if let Some(user) = user {
        spec = spec.user(user);
    }
    if let Some(dir) = workdir {
        spec = spec.workdir(dir);
    }
    if tty {
        // Its real size is this terminal's, taken as it starts.
        spec = spec.tty(24, 80);
    }
    if let Some(timeout) = timeout {
        if detach || interactive || tty {
            fail("--timeout is for a command run to its end: not with -d, -i or -t");
        }
        let out = spec.timeout(timeout).output().unwrap_or_else(|e| fail(e));
        use std::io::Write;
        let _ = std::io::stdout().write_all(&out.stdout);
        let _ = std::io::stderr().write_all(&out.stderr);
        if out.timed_out {
            eprintln!("carbon: {name}: the command ran past its timeout, and was killed");
            exit(124);
        }
        exit(out.code());
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
/// `carbon mcp token [--scope SCOPES] [--name NAME]`: a new token, and the line that lets it in.
fn mcp_token(mut args: impl Iterator<Item = String>) {
    let (mut scopes, mut name) = (mcp::Scopes { mcp: true, engine: false }, String::from("token"));
    while let Some(arg) = args.next() {
        let (arg, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_string(), Some(value.to_string())),
            _ => (arg, None),
        };
        let mut value = |what: &str| inline.clone().or_else(|| args.next()).unwrap_or_else(|| fail(format!("{what} needs a value")));
        match arg.as_str() {
            "--scope" | "--scopes" => scopes = mcp::net::tokens::parse_scopes(&value("--scope")).unwrap_or_else(|e| fail(format!("--scope: {e}"))),
            "--name" => name = value("--name"),
            other => fail(format!("mcp token doesn't know {other}: --scope, --name")),
        }
    }
    let (token, line) = mcp::net::tokens::new_token(scopes, &name).unwrap_or_else(|e| fail(e));
    eprintln!("carbon mcp token: the token, shown once (clients send Authorization: Bearer <token>):");
    println!("{token}");
    eprintln!("its line, for --token-file:");
    println!("{line}");
}

/// `carbon mcp [options]`: Carbon's front door, an MCP server over this Carbon home: on stdin and stdout, for an agent's
/// client to start, or with `-H` on the network, and with `--engine` the Engine API too.
fn mcp(carbon: &Carbon, mut args: impl Iterator<Item = String>) {
    let mut options = mcp::Options::default();
    let mut net = mcp::net::NetOptions::default();
    let (mut tls, mut tlscert, mut tlskey) = (false, None, None);
    let mut rules = Rules::new();
    let mut registries = carbon::Registries::default();
    while let Some(arg) = args.next() {
        let (arg, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_string(), Some(value.to_string())),
            _ => (arg, None),
        };
        let mut value = |what: &str| inline.clone().or_else(|| args.next()).unwrap_or_else(|| fail(format!("{what} needs a value")));
        match arg.as_str() {
            "--machine" => options.machine = Some(value("--machine")),
            "--host-dir" => options.host_dirs.push(std::path::PathBuf::from(value("--host-dir"))),
            "--deny-exec" => rules = rules.deny_exec(&value("--deny-exec")),
            "--deny" => rules = rules.deny(&value("--deny")).unwrap_or_else(|e| fail(format!("--deny {e}"))),
            "--allow" => rules = rules.allow(&value("--allow")).unwrap_or_else(|e| fail(format!("--allow {e}"))),
            "--deny-registry" => rules = rules.deny_registry(&value("--deny-registry")).unwrap_or_else(|e| fail(e)),
            flag if registry_flag(&mut registries, flag, &mut value) => {}
            "--network" => {
                let text = value("--network");
                options.network = Some(carbon::parse_network(&text).unwrap_or_else(|| fail(format!("--network {text}: nat, none or host"))));
            }
            "--memory" | "-m" => options.memory = Some(value("--memory")),
            "--timeout" => options.timeout = Some(duration(&value("--timeout"))),
            "--tools" => options.tools = Some(value("--tools").split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()),
            "-H" | "--host" => net.hosts.push(value("--host")),
            "--engine" => options.engine = true,
            "--max-sessions" => {
                let text = value("--max-sessions");
                net.max_sessions = Some(text.parse().unwrap_or_else(|_| fail(format!("--max-sessions {text}: a number"))));
            }
            "--tls" => tls = true,
            "--tlscert" => tlscert = Some(std::path::PathBuf::from(value("--tlscert"))),
            "--tlskey" => tlskey = Some(std::path::PathBuf::from(value("--tlskey"))),
            "--token-file" => net.token_file = Some(std::path::PathBuf::from(value("--token-file"))),
            "--allow-origin" => net.allow_origins.push(value("--allow-origin")),
            "--no-auth" => net.no_auth = true,
            other => fail(format!(
                "mcp doesn't know {other}: --machine, --host-dir, --deny-exec, --deny, --allow, --deny-registry, --registries, --upstream, \
                 --upstream-token-file, --registry-meta, --network, --memory, --timeout, --tools, \
                 -H, --engine, --max-sessions, --tls, --tlscert, --tlskey, --token-file, --allow-origin, --no-auth"
            )),
        }
    }
    options.rules = rules;
    options.registries = registries_given(registries);
    // Docker's TLS flags: --tlscert and --tlskey alone imply --tls, and --tls needs both.
    match (tls || tlscert.is_some() || tlskey.is_some(), tlscert, tlskey) {
        (false, ..) => {}
        (true, Some(cert), Some(key)) => net.tls = Some((cert, key)),
        _ => fail("--tls needs --tlscert and --tlskey"),
    }
    if net.hosts.is_empty() {
        let network_only = net.tls.is_some() || net.token_file.is_some() || net.no_auth || net.max_sessions.is_some() || !net.allow_origins.is_empty();
        if network_only {
            fail("--tls, --token-file, --no-auth, --max-sessions and --allow-origin are for -H: on stdio, whoever started carbon mcp is its client");
        }
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("carbon mcp: an MCP server on stdin and stdout, for an MCP client to start (as `claude mcp add carbon -- carbon mcp`). Waiting for JSON-RPC.");
        }
        let server = mcp::Server::new(carbon.clone(), options).unwrap_or_else(|e| fail(e));
        server.serve_stdio().unwrap_or_else(|e| fail(format!("stdio: {e}")));
        return;
    }
    // Before anything starts: who may connect where.
    if let Err(e) = mcp::net::check(&net) {
        fail(format!("mcp: {e}"));
    }
    let server = mcp::Server::new(carbon.clone(), options).unwrap_or_else(|e| fail(e));
    mcp::net::serve(server, net).unwrap_or_else(|e| fail(format!("mcp: {e}")));
}

/// Reads `--registries`, `--upstream`, `--upstream-token-file` and `--registry-meta` into `r`: whether `flag` is one.
fn registry_flag(r: &mut carbon::Registries, flag: &str, value: &mut dyn FnMut(&str) -> String) -> bool {
    let taken = std::mem::take(r);
    *r = match flag {
        "--registries" => taken.services_flag(&value("--registries")),
        "--upstream" => taken.upstream_flag(&value("--upstream")).unwrap_or_else(|e| fail(e)),
        "--upstream-token-file" => {
            let path = value("--upstream-token-file");
            let token = std::fs::read_to_string(&path).unwrap_or_else(|e| fail(format!("--upstream-token-file {path}: {e}")));
            let token = token.trim();
            if token.is_empty() || token.contains(char::is_whitespace) {
                fail(format!("--upstream-token-file {path}: one token, on its own"));
            }
            taken.token(token)
        }
        "--registry-meta" => taken.meta_flag(&value("--registry-meta")).unwrap_or_else(|e| fail(e)),
        _ => {
            *r = taken;
            return false;
        }
    };
    true
}

/// The registries flags gave, if any: a server or a token with no `--registries` is a mistake, said so.
fn registries_given(r: carbon::Registries) -> Option<carbon::Registries> {
    if r.is_empty() {
        if r != carbon::Registries::default() {
            fail("--upstream, --upstream-token-file and --registry-meta are for --registries: which ones (apt, npm, pypi, git)?");
        }
        return None;
    }
    Some(r)
}

fn watch(machine: &Machine, args: impl Iterator<Item = String>) {
    let mut interest = Interest::none();
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--exec" => interest |= Interest::exec(),
            "--output" => interest |= Interest::output(),
            "--net" | "--network" => interest |= Interest::network(),
            "--registry" | "--registries" => interest |= Interest::registry(),
            "--json" => json = true,
            other => fail(format!("watch doesn't know {other}: --exec, --output, --net, --registry, --json")),
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

/// `carbon logs [-f] [--tail N] NAME`, as `docker logs`: what a launched machine wrote, each stream to this one's.
fn logs(carbon: &Carbon, args: impl Iterator<Item = String>) {
    let mut options = carbon::Logs::new();
    let (mut name, mut args) = (None, args.peekable());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-f" | "--follow" => options = options.follow(),
            "-n" | "--tail" => {
                let n = args.next().and_then(|n| n.parse().ok()).unwrap_or_else(|| fail("--tail needs a number of lines"));
                options = options.tail(n);
            }
            flag if flag.starts_with('-') => fail(format!("logs doesn't know {flag}: -f, --tail N")),
            _ => name = Some(arg),
        }
    }
    let name = name.unwrap_or_else(|| fail("logs needs a machine's name"));
    let reader = carbon.get(&name).and_then(|m| m.logs(options)).unwrap_or_else(|e| fail(e));
    use std::io::Write;
    for line in reader {
        let _ = match line.stream {
            carbon::LogStream::Stdout => std::io::stdout().write_all(&line.bytes),
            carbon::LogStream::Stderr => std::io::stderr().write_all(&line.bytes),
        };
    }
}

fn ls(carbon: &Carbon, mut args: impl Iterator<Item = String>) {
    // Docker's `--filter label=KEY` and `label=KEY=VALUE`: every one must match.
    let mut labels: Vec<(String, Option<String>)> = Vec::new();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f == "--filter" => (f.to_string(), Some(v.to_string())),
            _ => (arg, None),
        };
        match flag.as_str() {
            "-f" | "--filter" => {
                let text = inline.or_else(|| args.next()).unwrap_or_else(|| fail("--filter needs a value: label=KEY[=VALUE]"));
                let Some(label) = text.strip_prefix("label=") else { fail(format!("--filter {text}: label=KEY[=VALUE] is the filter there is")) };
                let (k, v) = match label.split_once('=') {
                    Some((k, v)) => (k.to_string(), Some(v.to_string())),
                    None => (label.to_string(), None),
                };
                labels.push((k, v));
            }
            "-a" | "--all" => {}
            other => fail(format!("ps doesn't know {other}: --filter label=KEY[=VALUE]")),
        }
    }
    let rows: Vec<[String; 5]> = carbon
        .machines()
        .into_iter()
        .filter(|m| labels.iter().all(|(k, v)| m.config.labels.get(k).is_some_and(|have| v.as_ref().is_none_or(|v| v == have))))
        .map(|m| {
            let status = match (m.running, m.exit) {
                (true, _) => format!("running ({})", m.pid.unwrap_or(0)),
                (false, Some(code)) => format!("exited ({code})"),
                (false, None) => "stopped".to_string(),
            };
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
    let mut args = args.peekable();
    // A token's made without a Carbon home.
    if command == "mcp" && args.peek().is_some_and(|a| a == "token") {
        args.next();
        mcp_token(args);
        return;
    }
    let carbon = Carbon::open().unwrap_or_else(|e| fail(e));
    let mut one = |what: &str| args.next().unwrap_or_else(|| fail(format!("{command} needs {what}")));
    let machine = |name: &str| carbon.get(name).unwrap_or_else(|e| fail(e));
    match command.as_str() {
        "run" => run(&carbon, args),
        "exec" => exec(&carbon, args),
        "mcp" => mcp(&carbon, args),
        "watch" => {
            let name = one("a machine's name");
            watch(&machine(&name), args);
        }
        "ls" | "ps" => ls(&carbon, args),
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
        "logs" => logs(&carbon, args),
        "build" => build(&carbon, args),
        "volume" => {
            let verb = one("what to do: create, ls, inspect or rm");
            match verb.as_str() {
                "create" => {
                    let name = one("a volume's name");
                    carbon.volume(&name).create().unwrap_or_else(|e| fail(e));
                    println!("{name}");
                }
                "ls" => {
                    let rows: Vec<[String; 2]> = carbon.volumes().into_iter().map(|v| [v.name, v.path.display().to_string()]).collect();
                    table(&["NAME", "FOLDER"], &rows);
                }
                "inspect" => {
                    let name = one("a volume's name");
                    let info = carbon.volume(&name).info().unwrap_or_else(|| fail(format!("there's no volume {name}")));
                    println!("{{\n  \"name\": {:?},\n  \"created\": {},\n  \"path\": {:?}\n}}", info.name, info.created, info.path.display().to_string());
                }
                "rm" => {
                    let name = one("a volume's name");
                    carbon.remove_volume(&name).unwrap_or_else(|e| fail(e));
                    println!("{name}");
                }
                other => fail(format!("volume {other}: create, ls, inspect or rm")),
            }
        }
        "tag" => {
            let (source, target) = (one("an image"), one("its new name"));
            let image = carbon.tag(&source, &target).unwrap_or_else(|e| fail(e));
            println!("{}", image.short_name());
        }
        "inspect" => {
            let name = one("an image");
            let image = carbon.image(&name).unwrap_or_else(|| fail(format!("there's no image {name}")));
            println!("{}", carbon::image_json(&image));
        }
        "history" => {
            let name = one("an image");
            let image = carbon.image(&name).unwrap_or_else(|| fail(format!("there's no image {name}")));
            let rows: Vec<[String; 3]> = image.history.iter().rev().map(|h| {
                let by: String = h.created_by.chars().take(90).collect();
                [h.created.clone(), if h.empty_layer { "config".to_string() } else { "files".to_string() }, by]
            }).collect();
            table(&["CREATED", "CHANGED", "CREATED BY"], &rows);
        }
        "stop" => machine(&one("a machine's name")).stop().unwrap_or_else(|e| fail(e)),
        "rm" => machine(&one("a machine's name")).remove().unwrap_or_else(|e| fail(e)),
        "cp" => {
            let (from, to) = (one("NAME:PATH or a path on this host"), one("a folder on this host or NAME:PATH"));
            // Out of a machine when the first names one, as web:/etc/hosts; else into the machine the second names.
            let named = |arg: &str| arg.split_once(':').and_then(|(name, path)| carbon.get(name).ok().map(|m| (m, path.to_string())));
            let how = carbon::Copy::new();
            let n = match (named(&from), named(&to)) {
                (Some((m, path)), _) => m.copy_out(&path, &to, &how),
                (None, Some((m, path))) => m.copy_in(&from, &path, &how),
                (None, None) => fail("cp copies NAME:PATH DIR out of a machine, or PATH NAME:DIR into one, as web:/etc/hosts".to_string()),
            }
            .unwrap_or_else(|e| fail(e));
            println!("{n} files");
        }
        "commit" => {
            let (mut how, mut rest) = (carbon::Commit::new(), Vec::new());
            while let Some(arg) = args.next() {
                let (flag, inline) = match arg.split_once('=') {
                    Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
                    _ => (arg.clone(), None),
                };
                let mut value = |what: &str| inline.clone().or_else(|| args.next()).unwrap_or_else(|| fail(format!("{what} needs a value")));
                match flag.as_str() {
                    "-m" | "--message" => how = how.comment(value("--message")),
                    "-c" | "--change" => how = how.change(value("--change")),
                    f if f.starts_with('-') => fail(format!("commit {f}: -m MESSAGE or -c INSTRUCTION")),
                    _ => rest.push(arg),
                }
            }
            let Some(name) = rest.first() else { fail("commit needs a machine: carbon commit dev mine:1") };
            if let Some(image) = rest.get(1) {
                how = how.name(image.clone());
            }
            let image = machine(name).commit_with(&how).unwrap_or_else(|e| fail(e));
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
            // On a terminal, one line that keeps up: what it's doing and how far, by bytes.
            let live = std::io::IsTerminal::is_terminal(&std::io::stderr());
            let image = carbon
                .pull_with(&name, platform, |p| {
                    if live {
                        eprint!("\r\x1b[K{name}: {p} ({:.0}%)", p.fraction() * 100.0);
                    }
                })
                .unwrap_or_else(|e| {
                    if live {
                        eprintln!();
                    }
                    fail(e)
                });
            if live {
                eprint!("\r\x1b[K");
            }
            println!("{} ({})", image.short_name(), image.platform);
        }
        "push" => {
            let (mut squash, mut name) = (false, None);
            for arg in args.by_ref() {
                match arg.as_str() {
                    "--squash" => squash = true,
                    _ if name.is_none() && !arg.starts_with('-') => name = Some(arg),
                    _ => usage(),
                }
            }
            let name = name.unwrap_or_else(|| fail("push needs an image: carbon push someone/app:1"));
            let live = std::io::IsTerminal::is_terminal(&std::io::stderr());
            let pushed = carbon
                .push_with(&name, &carbon::Push::new().squash(squash), |p| {
                    if live {
                        eprint!("\r\x1b[K{name}: {p}");
                    } else if p.stage != carbon::PushStage::Sending || p.done == 0 {
                        eprintln!("{name}: {p}");
                    }
                })
                .unwrap_or_else(|e| {
                    if live {
                        eprintln!();
                    }
                    fail(e)
                });
            if live {
                eprint!("\r\x1b[K");
            }
            eprintln!("{} layers, {} sent, {} already there", pushed.layers, pushed.sent, pushed.layers - pushed.sent);
            println!("{}@{}", pushed.name, pushed.digest);
        }
        "login" => {
            let (mut user, mut from_stdin, mut password, mut registry) = (None, false, None, None);
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "-u" | "--username" => user = Some(args.next().unwrap_or_else(|| fail("--username needs a value"))),
                    "--password-stdin" => from_stdin = true,
                    "-p" | "--password" => {
                        eprintln!("carbon: a password on the command line is seen by others on this host: use --password-stdin");
                        password = Some(args.next().unwrap_or_else(|| fail("--password needs a value")));
                    }
                    _ if registry.is_none() && !arg.starts_with('-') => registry = Some(arg),
                    _ => usage(),
                }
            }
            let registry = registry.unwrap_or_else(|| "docker.io".into());
            let terminal = std::io::IsTerminal::is_terminal(&std::io::stdin());
            let user = user.unwrap_or_else(|| {
                if from_stdin || !terminal {
                    fail("login needs a username: -u USER");
                }
                ask("Username: ", true)
            });
            let password = match password {
                Some(p) => p,
                None if from_stdin => {
                    let mut text = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).unwrap_or_else(|e| fail(format!("reading stdin: {e}")));
                    text.trim_end_matches(['\r', '\n']).to_string()
                }
                None if terminal => ask("Password: ", false),
                None => fail("login needs a password: --password-stdin"),
            };
            if password.is_empty() {
                fail("login needs a password");
            }
            carbon.login(&registry, &user, &password).unwrap_or_else(|e| fail(e));
            println!("Logged in to {registry} as {user}");
        }
        "logout" => {
            let registry = args.next().unwrap_or_else(|| "docker.io".into());
            match carbon.logout(&registry).unwrap_or_else(|e| fail(e)) {
                true => println!("Logged out of {registry}"),
                false => println!("Not logged in to {registry}"),
            }
        }
        "save" => {
            let (mut output, mut names) = (None, Vec::new());
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "-o" | "--output" => output = Some(args.next().unwrap_or_else(|| fail("--output needs a file"))),
                    _ if arg.starts_with("--output=") => output = Some(arg["--output=".len()..].to_string()),
                    _ if !arg.starts_with('-') => names.push(arg),
                    _ => usage(),
                }
            }
            if names.is_empty() {
                fail("save needs an image: carbon save -o app.tar app:1");
            }
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            match output {
                Some(path) => {
                    // Written beside where it goes, then moved there, so a failed save leaves nothing half written.
                    let partial = format!("{path}.partial");
                    let file = std::fs::File::create(&partial).unwrap_or_else(|e| fail(format!("{partial}: {e}")));
                    let saved = carbon.save(&names, std::io::BufWriter::with_capacity(1 << 20, file));
                    if let Err(e) = saved {
                        let _ = std::fs::remove_file(&partial);
                        fail(e);
                    }
                    std::fs::rename(&partial, &path).unwrap_or_else(|e| fail(format!("{path}: {e}")));
                }
                None => {
                    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                        fail("an archive isn't for a terminal: -o FILE, or send stdout elsewhere");
                    }
                    carbon.save(&names, std::io::BufWriter::with_capacity(1 << 20, std::io::stdout().lock())).unwrap_or_else(|e| fail(e));
                }
            }
        }
        "load" => {
            let input = match (args.next().as_deref(), args.next()) {
                (Some("-i" | "--input"), Some(file)) => Some(file),
                (None, _) => None,
                _ => usage(),
            };
            let loaded = match input {
                Some(file) => carbon.load(&file),
                None => {
                    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                        fail("load needs an archive: -i FILE, or one on stdin");
                    }
                    // Kept as a file first, as an archive is read in the order its index says, not as it comes.
                    let dir = carbon.home().join("tmp");
                    let _ = std::fs::create_dir_all(&dir);
                    let path = dir.join(format!("stdin-{}.tar", std::process::id()));
                    let mut file = std::fs::File::create(&path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
                    let copied = std::io::copy(&mut std::io::stdin().lock(), &mut file);
                    drop(file);
                    let loaded = copied.map_err(|e| format!("reading stdin: {e}")).and_then(|_| carbon.load(&path).map_err(|e| e.to_string()));
                    let _ = std::fs::remove_file(&path);
                    loaded.map_err(|e| fail(e))
                }
            };
            for image in loaded.unwrap_or_else(|e| fail(e)) {
                println!("Loaded image: {} ({})", image.short_name(), image.platform);
            }
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
            // The kernel a machine from it runs: atlas for a Windows image, lambda for a Linux one.
            let kernel = |i: &carbon::Image| if i.platform.os == "windows" { carbon::Kernel::Atlas } else { carbon::Kernel::Lambda }.name().to_string();
            let rows: Vec<[String; 4]> = carbon.images().into_iter().map(|i| [kernel(&i), i.short_name().to_string(), i.platform.to_string(), i.source]).collect();
            table(&["KERNEL", "IMAGE", "PLATFORM", "FROM"], &rows);
        }
        "rmi" => carbon.remove_image(&one("an image")).unwrap_or_else(|e| fail(e)),
        other => fail(format!("{other} isn't a command: carbon help")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_and_label_files_as_docker_reads_them() {
        let dir = std::env::temp_dir().join(format!("carbon-cli-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("env");
        // SAFETY: a variable only this test reads.
        unsafe { std::env::set_var("CARBON_TEST_FROM_HOST", "hosts") };
        std::fs::write(&file, "# a comment\n\nA=1\n  B=two words\nC=\"quoted\"\nD=x=y\nCARBON_TEST_FROM_HOST\nCARBON_TEST_UNSET_ANYWHERE\n").unwrap();
        let vars = env_file(file.to_str().unwrap());
        let want: Vec<(String, String)> = [("A", "1"), ("B", "two words"), ("C", "\"quoted\""), ("D", "x=y"), ("CARBON_TEST_FROM_HOST", "hosts")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(vars, want);
        std::fs::write(&file, "team=infra\n# no\ntier\n").unwrap();
        assert_eq!(label_file(file.to_str().unwrap()), [("team".to_string(), "infra".to_string()), ("tier".to_string(), String::new())]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
