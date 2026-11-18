# carbon

Carbon as a library: the crate `carbon`. Everything the `carbon` command does, a Rust program does through it, and
more: it sees, in real time, every program a machine starts, everything it writes, every connection it makes and
every name it looks up, and can **allow, block, rewrite or answer** each one while the machine's program waits.

It's an ordinary Rust crate, used from GitHub as any git dependency is:

```toml
[dependencies]
carbon = { git = "https://github.com/tensorai-co/carbon" }
```

```rust
use carbon::hooks::{Decision, Filter, Reply};
use carbon::{Carbon, Network, Rules};

fn main() -> carbon::Result<()> {
    carbon::init(); // first in main: the process sandbox's stub is this binary, run again
    let carbon = Carbon::open()?; // CARBON_HOME, or ~/.carbon

    // A command in a machine of its own, thrown away after, and what it wrote.
    let out = carbon.machine("debian:13").network(Network::None).memory("512m").rm(true).output(["uname", "-a"])?;
    print!("{}", out.stdout_lossy());

    // Its programs and network decided as they happen.
    let out = carbon
        .machine("debian:13")
        .rm(true)
        .on_exec(Filter::program("apt-get"), |_| Decision::answer(Reply::new().stdout("Done\n").exit(0)))
        .on_output(Filter::any(), |o| o.replace(o.text().replace("hunter2", "*******")))
        .rules(Rules::new().deny_exec("chmod 777 *").deny("0.0.0.0/0")?.allow("pypi.org:443")?)
        .output(["bash", "-c", "apt-get install -y curl; echo password=hunter2"])?;
    print!("{}", out.stdout_lossy());

    // A machine kept running: `exec` into it, watch it, stop it.
    let dev = carbon.machine("debian:13").name("dev").publish("2222:22").start(["sleep", "infinity"])?;
    let ls = dev.machine().exec(["ls", "/"]).output()?;
    print!("{}", ls.stdout_lossy());
    dev.stop();
    dev.wait()?;

    // One in a process of its own, which outlives this program: its files, its logs, a command with a time limit.
    let web = carbon.machine("nginx:alpine").name("web").publish_all().launch(Vec::<String>::new())?;
    web.files().write("/usr/share/nginx/html/index.html", "hello\n", carbon::Write::new().mode(0o644))?;
    web.exec(["nginx", "-t"]).timeout(std::time::Duration::from_secs(30)).output()?;
    for line in web.logs(carbon::Logs::new().tail(10))? {
        print!("{}", line.text());
    }
    Ok(())
}
```

[`examples/agent.rs`](examples/agent.rs) is all of that, runnable:

```sh
cargo build --release -p carbon --example agent
codesign --force --sign - --entitlements carbon.entitlements target/release/examples/agent   # macOS
target/release/examples/agent
```

## What a program using it needs

| | |
| --- | --- |
| **`carbon::init()`** | First in `main`. The process sandbox runs a program in a stub that is this same binary, run again; `init` is where the stub takes over (and returns at once in every other process). |
| **The hypervisor** | On macOS, the binary signed with [`carbon.entitlements`](../carbon.entitlements) (`com.apple.security.hypervisor`, `cs.debugger`, `get-task-allow`): `codesign --force --sign - --entitlements carbon.entitlements target/release/yourapp`. On Linux, `/dev/kvm` (the `kvm` group). On Windows, `winkvm.sys`. Without one, machines run in the sandbox, and say so. |
| **Rust 1.85 or later**, with `rust-src` | `rustup component add rust-src`. atlas's and lambda's build scripts build Windows' guest code (ntdll) and the guest GPU drivers from source, for their own targets, with `-Zbuild-std`. |
| **LLVM's `lld-link` and `ld.lld`** | Homebrew's `lld` or `llvm`, Debian's `lld`, or LLVM on Windows (`ATLAS_GUEST_LINKER` and `LAMBDA_GUEST_LINKER` name them if they aren't found). Without them everything still builds, with a warning, and Linux machines run; Windows machines and the guest GPU drivers say what's missing. |
| **Images** | Pulled from any OCI registry (`debian:13`), or imported: Windows' own programs come from [atlas-base](https://github.com/tensorai-co/atlas-base), built locally and `carbon import`ed as `atlas`. |

Its dependencies are the rest of this repository (each crate a member of its workspace, found by cargo in the
checkout) and `serde_json`; below them, `libc`, `rustls` for pulling images over HTTPS, and each host's system
libraries.

## The types

| Type | What it is |
| --- | --- |
| [`Carbon`](src/lib.rs) | Carbon on this host: its folder of machines and images. `machine(image)`, `program(path)`, `again(name)` make a `MachineSpec`; `build(context)` a [`Build`](#building); `get(name)`, `machines()`, `images()`, `image(name)` (or an unnamed one's `sha256:…`), `pull`, `push`, `push_with` ([`Push`](#registries)), `login`, `logout`, `save`, `load`, `import`, `tag`, `remove_image`, `prune`; `on_notice` for what machines fall back to |
| [`MachineSpec`](src/spec.rs) | A builder: `name`, `isolation`, `platform`, `user`, `env`, `workdir`, `memory`, `network`, `publish`, `display`, `gpu`, `audio`, `stdin`/`stdout`/`stderr` ([`Stdio`](src/stdio.rs): inherit, null, piped), `tty`, `relay_signals`, `rm`, `publish_all`, `timeout`, `mount`, and `hooks`, `rules`, `on_exec`, `on_output`, `on_connect`, `on_lookup`. Then `run(argv)` on this thread, `output(argv)` to its end with what it wrote, `start(argv)` on a thread of its own, `launch(argv)` in a process of its own ([workers](#workers)), or `create()` to make it without running it, as `docker create` |
| [`Running`](src/spec.rs) | A machine this process runs: `take_stdin`, `take_stdout`, `take_stderr` ([`Chunks`](src/stdio.rs), also an `io::Read`), `hooks()`, `machine()`, `stop`, `kill`, `wait` |
| [`Machine`](src/machine.rs) | A machine by name, in this process or another: `info`, `exec(argv)`, `stop`, `kill`, `wait`, `ports`, `screen`, `watch`, `logs`, `files`, `copy_in`, `copy_out`, `freeze`, `freeze_with` ([`Freeze`](src/machine.rs): its name or none, config, step, history), `remove` |
| [`Files`](src/files.rs) | A machine's files, running or stopped: `stat`, `list`, `read`, `read_to_string`, `write` ([`Write`](src/files.rs): mode, owner, parents, append, time), `mkdir`, `symlink`, `rename`, `remove`, and `copy_in`, `copy_out`, `copy_from` (another machine's disk) ([`Copy`](src/files.rs): `chown`, `chmod`, `exclude` with an ignore file's patterns), which keep kinds, modes, owners, times and links |
| [`Mount`, `Volume`](src/mounts.rs) | A host's folder (or one file) seen live in a machine (`Mount::bind`), a volume (`Mount::volume`: a folder Carbon keeps that outlives its machines, its files' owners kept), a tmpfs; `read_only`; `Mount::parse` takes Docker's `-v` and `--mount`. `Carbon::volume(name)`: `create`, `path`, `info`; `volumes()`, `remove_volume` |
| [`Logs`, `LogLine`](src/logs.rs) | What a launched machine wrote, as lines on its stream: `tail(n)`, `follow()` |
| [`Image`, `ImageConfig`, `History`](../images/src/config.rs) | An image: its whole OCI config (exposed ports, volumes, labels, stop signal, health check, shell, `ONBUILD`), each step that made it, and its [`Lineage`](../images/src/lib.rs): the repository and [`Layer`](../images/src/lib.rs)s its first disk was pulled as, and each [`Disk`](../images/src/lib.rs) it was made through |
| [`Push`, `Pushed`, `PushProgress`](src/lib.rs) | How to push (as another name, squashed), what a push did (the name, the manifest's digest, layers sent), and where one is |
| [`ExecSpec`, `Process`](src/machine.rs) | A process in a running machine, as `docker exec`: `user`, `env`, `workdir`, `tty`, `timeout` (killed after it, exit 124); then `spawn` (stdin, output as it comes, `resize`, `signal`, `wait`), `output`, `detach`, or `attach` to this terminal |
| [`Screen`](src/machine.rs) | A running machine's desktop, for computer use: `screenshot` (PNG), `windows`, `click`, `drag`, `scroll`, `type_text`, `key`, `resize`, `clipboard`, `drop_text`, `drop_files` |
| [`hooks`](../hooks/README.md) | Events, decisions, the `Hooks` trait, filters, and the `Hub` each machine's hook points ask, with observers and subscriptions |
| [`Rules`](src/rules.rs) | Decisions with no code: programs that may not start (`deny_exec("rm -rf *")`), where the network may and may not go (`deny("0.0.0.0/0")`, `allow("pypi.org:443")`) |
| [`Watch`](src/wire.rs) | A running machine's hooked events, read from its control socket, from any process |
| [`Error`](src/error.rs) | A kind to match on (`NotFound`, `Running`, `NotRunning`, `InvalidInput`, `Unsupported`, `Denied`, `Image`, `Io`, `Failed`) and a message in words |

## Hooks

A hook point is a place in Carbon's own code a program can't get around. Each makes an event and asks the machine's
`Hub` on the thread that asked, which waits: so a decision really decides.

| Event | Hook point | Decisions |
| --- | --- | --- |
| **`Exec`**: a program about to start (path, arguments, environment, folder, pid, parent) | lambda's `execve`; atlas's `CreateProcess`; a machine's first program; `exec` from outside | allow; deny (EACCES, ERROR_ACCESS_DENIED, or an error before the machine boots); rewrite the program, arguments, environment; answer with output and an exit code, running nothing (lambda's `execve`, the first program, and `exec`; atlas's `CreateProcess` refuses an answer for now) |
| **`Output`**: what a process writes to the machine's stdout or stderr, or an `exec`'s | the `host::Host` the machine's kernel writes through ([`stdio.rs`](src/stdio.rs)), and each `exec` session | pass; drop; replace the bytes |
| **`Connect`**: a TCP connection out, or a datagram to a new UDP destination, with the name a lookup gave its address | the `net::Host` the machine's network runs on ([`network.rs`](src/network.rs)) | allow; refuse (ECONNREFUSED, and EPERM for UDP); redirect to another address |
| **`Lookup`**: a name the machine looks up | the same: its DNS server, and atlas's `getaddrinfo` | allow; NXDOMAIN; answer with addresses; rewrite the name |

- **Nothing hooked costs nothing.** Each hook point checks one atomic flag for its kind before it makes an event; a
  closure's `Filter` (`Filter::program("curl")`, `Filter::port(443)`, `Filter::host("pypi.org")`) is checked before
  it's called, and a `Hooks` type says which kinds it wants with `interest()`.
- **The first that decides, decides**: hooks, rules and closures are one chain, in the order they're added; anything
  but `Decision::Pass` ends it. Then subscribers; then it's allowed.
- **Subscribers** decide from anywhere, in their own time, for decisions that take a while (a model, a person):
  `running.hooks().subscribe(Interest::exec(), Timeout::secs(30).default_deny())` gives each event as a `Pending` to
  `allow()`, `deny()` or `decide(...)`; one not decided in time gets the timeout's default.
- **Observers** see what was decided: `running.hooks().observe(Interest::all(), 1024)`, in this process, or
  `machine.watch(...)` and `carbon watch NAME` from any other. An observer that falls behind loses events, and is told
  how many.
- **Where they can't see**: a program the image hasn't got isn't asked about, because shells look along `PATH`
  themselves before starting anything; TLS traffic is opaque (there's no `Data` hook yet); output is what a program
  wrote to the machine's own stdio, not what passes between its processes.

## Inside

| File | What it is |
| --- | --- |
| [`lib.rs`](src/lib.rs) | `Carbon`, `Notice`, `init`, and what's re-exported from the parts below |
| [`spec.rs`](src/spec.rs) | `MachineSpec`, `Running`, `Exit`, `Output`, `Display`, `Audio` |
| [`worker.rs`](src/worker.rs) | Workers: `launch`, a machine in this binary run again, and its hook link back to the launcher |
| [`files.rs`](src/files.rs) | `Files` and copies: each operation one `Op`, done on a stopped disk directly or by a running machine's kernel |
| [`mounts.rs`](src/mounts.rs) | `Mount`, volumes, and a host folder as an external tree of the machine's disk (`HostTree`) |
| [`users.rs`](src/users.rs) | Who a machine's programs run as: `name[:group]` or `uid[:gid]`, read from the machine's own `/etc/passwd` and `/etc/group` as Docker reads them |
| [`logs.rs`](src/logs.rs) | A launched machine's output, kept as records of each write, and read back as lines |
| [`build/`](src/build/mod.rs) | Building: `parse` (the Dockerfile's grammar), `lex` (substitution and quoting), `ignore` (ignore files), `archive` (`ADD`'s tars), `run` (the steps, the cache) |
| [`machine.rs`](src/machine.rs) | `Machine`, `MachineInfo`, `ExecSpec`, `Process`, `Screen` |
| [`boot.rs`](src/boot.rs) | Making a machine and running it: the kernel, exec with its isolation, the disk, its desktop, sound and network, its control socket, and the hooks on its first program and its `exec`s |
| [`store.rs`](src/store.rs) | Machines on the host's disk: records, names, and what's done to a stopped one (freeze, copy, import, remove) |
| [`stdio.rs`](src/stdio.rs) | A machine's standard streams, and the `host::Host` its kernel writes through |
| [`network.rs`](src/network.rs) | The `net::Host` a machine's network runs on, asking its hooks |
| [`rules.rs`](src/rules.rs) | `Rules` |
| [`stop.rs`](src/stop.rs) | Stopping a machine from outside its kernel: ask, then ten seconds, then end |
| [`wire.rs`](src/wire.rs) | The control protocol, both sides: requests, `exec`'s frames, `watch`'s JSON lines |
| [`signals.rs`](src/signals.rs) | This process's signals, for a machine that is the process (`relay_signals`, as `carbon run`) |

A machine runs on the thread that starts it (hvf ties a VM's CPUs to the thread that made them) until its first
process ends; each kernel lets go of everything it held before `boot` returns, so one process can run machine after
machine. Its record and log are in `<home>/machines/<name>/`, and while it runs it answers on a Unix socket named
there ([`wire`](src/wire.rs)), which is how another process `exec`s into it, watches it, takes its screen or stops it.

## Building

`carbon.build(context)` builds an image from a Carbonfile, which is a Dockerfile, exactly: the same grammar, as
BuildKit's `docker/dockerfile:1` reads it. A build is the library's own calls: each `RUN` a launched machine frozen
after (`launch`, `freeze_with`), each `COPY` and `ADD` a copy into a machine made but never run (`create`,
`copy_in`, `copy_from`), every other instruction a change to the image's config. So rules and hooks hold for every
`RUN`, and `carbon watch` sees each step's machine (`build-<id>-<n>`) as it runs.

```rust
let built = carbon
    .build("./app")                                        // ./Carbonfile, else ./Dockerfile; .file(path) for another
    .tag("app:2")
    .arg("VERSION", "2")
    .target("runtime")
    .rules(Rules::new().deny("0.0.0.0/0")?.allow("pypi.org:443")?)
    .run_with(|step| eprintln!("{step}"))?;                // each stage, step, cache hit and line a RUN writes
let file = carbon::build::Carbonfile::parse(&text)?;      // the grammar alone
```

| Instruction | What a build does |
| --- | --- |
| `FROM`, `--platform` | The image, pulled if it isn't here, or an earlier stage; `scratch` is an empty disk. Its `ONBUILD` triggers run first |
| `RUN` | A machine launched from the step's image with its `ENV`, `ARG`s, `WORKDIR`, `SHELL` and `USER`, run to its end, frozen with the config and the step; a non-zero exit fails the build. `--network`; here-documents as scripts (a `#!` one run as a file) |
| `RUN --mount` | `type=cache`: a volume kept between builds (`build-cache-…`, by its `id` or target), left out of the step's key; `bind`: part of the context (a folder or a file), read-only, its contents in the key; `tmpfs`; `secret`: a file the build was given (`Build::secret`, `--secret id=ID,src=FILE`) at `/run/secrets/ID` for that step, or a variable (`env=`), never in the image or the key |
| `COPY`, `ADD` | Into a machine made but never run: the context's files (the ignore file applied, wildcards matched), a here-document, or another stage's or image's (`--from`, disk to disk). `--chown`, octal `--chmod`, `--exclude`; kinds, modes, times and links kept. `ADD` unpacks a local tar, plain or gzip, found by its contents |
| `ENV`, `ARG`, `WORKDIR`, `USER` (who the next `RUN`s and the image's machines run as), `CMD`, `ENTRYPOINT`, `SHELL`, `LABEL`, `EXPOSE`, `VOLUME`, `STOPSIGNAL`, `HEALTHCHECK`, `ONBUILD`, `MAINTAINER` | The config and a history entry, nothing on the disk (a `WORKDIR`'s folder is made by the next step that changes the disk) |

**The cache.** Each step's key is a SHA-256 of what it starts from (the stage's key so far, which every instruction
before it fed), the instruction as resolved, the `ARG`s a `RUN` gets, its network and platform, and what a `COPY`
reads (each file's path, kind, mode and contents). It names an unnamed image (`sha256:…`, by an id made from what
the image is) in `<home>/build/cache/<key>.json`, so a step done before is found at once. Keys carry a version
(`CACHE_VERSION` in [`run.rs`](src/build/run.rs)), changed whenever what a step does changes, so a fix never finds
what an older build cached. `Carbon::prune` forgets unnamed images unused for `BUILD_CACHE_KEEP` (a week).

**Held to Docker's.** The grammar (`parse.rs`, `lex.rs`, `ignore.rs`) is tested against BuildKit's behavior case by
case: directives, continuations and comments, flags, every instruction's forms, here-documents, stages, every
`${…}` modifier, ignore patterns.

## Mounts and volumes

```rust
let dev = carbon
    .machine("python:3.13-slim")
    .mount(Mount::bind("/Users/me/work", "/work"))           // a host folder, live both ways (or one file)
    .mount(Mount::bind("./config.toml", "/etc/app.toml").read_only())
    .mount(Mount::volume("pip-cache", "/root/.cache/pip"))   // made if it isn't there
    .mount(Mount::tmpfs("/scratch"))
    .launch(["sleep", "infinity"])?;
```

A bound folder and a volume are both host folders, attached to the machine's disk as an external tree
(`layers::External`, over `host::folder`) and mounted where the machine wants them by its kernel's mount table: every
call a program makes on a file there is made on the host's file as it happens. A volume is a folder under
`<home>/volumes/<name>/data` whose files keep their owners and modes (in an extended attribute), so a database's
files are its user's when it's mounted again; a new volume is filled from what the image has at its target, as
Docker's is. A machine keeps its mounts for when it runs again. Linux machines have them; Windows machines don't yet.

## Users

```rust
let app = carbon.machine("node:22-slim").user("node").launch(["sleep", "infinity"])?;  // or "1000:1000", "node:staff"
app.exec(["id"]).output()?;                         // node, as the machine runs
app.exec(["apt-get", "update"]).user("root").output()?;
```

A machine's programs run as its `user`, else its image's `USER`, else root, as Docker's do: a name or a number, and
optionally a group, read from the machine's own `/etc/passwd` and `/etc/group` (a number it hasn't got is taken as it
is, with group 0), with that user's home as `HOME` and the groups that list it as its supplementary groups. An `exec`
runs as the machine does unless it's given a user. A build's `RUN` runs as the `USER` before it.

lambda then checks what they do as Linux does: each file's owner, group and other bits (and folders' search bits on
the way), `CAP_DAC_OVERRIDE` and `CAP_DAC_READ_SEARCH`, sticky folders, the setgid bit on new files' folders, who may
`chmod`, `chown` and set times, `access`'s real IDs, and setuid and setgid programs at `execve` (not on a `nosuid`
mount, nor traced), so `su` works, and a program that drops root to its own user (nginx's
workers, `apt`'s `_apt`) can't reach root's files. Root, every machine's user unless it's given another, is checked
for nothing but running a file with no execute bit, so it pays nothing. A bound folder's files are owned by the
machine's user, so whoever it runs as can use them, as on Docker Desktop; a volume's keep their own owners.
Windows machines don't take a user yet.

## Registries

```rust
carbon.login("docker.io", "someone", &token)?;                    // checked, and kept in <home>/auth.json (mode 600)
let pushed = carbon.push("someone/app:1")?;                       // a layer per step
println!("{}@{}", pushed.name, pushed.digest);
carbon.push_with("app:1", &Push::new().to("ghcr.io/team/app:1").squash(true), |p| eprintln!("{p}"))?;
carbon.save(&["app:1", "debian:13"], std::fs::File::create("images.tar")?)?;   // as docker save writes
let loaded = carbon.load("images.tar")?;                          // docker save's, or an OCI layout
```

An image keeps its **lineage**: the layers a pulled base came as, and each disk a step made since (a build's every
`RUN`, `COPY` and `ADD`, a `carbon freeze`). A push sends a layer per disk: the base's layers as the registry has
them (taken from the base's own repository on the same registry with nothing sent, or fetched from another), then a
layer of what each step changed, with whiteouts for what it removed, made from the two disks as it's sent. A step
that changed nothing makes no layer, and the image's history says which steps made one, as OCI's config wants. The
same disks make the same bytes, so a second push sends nothing and gets the same digest; `squash` sends one layer.
Logins are Carbon's own, else Docker's (`docker login`, and its credential helpers). Prune keeps the disks a named
image was made through, so it can still be pushed a layer per step.

`save` writes the same layers, as plain tars, into an OCI image layout with Docker's `manifest.json` (as Docker 25
and later write: `docker load` reads it); `load` reads that, an OCI layout alone, or Docker's older archive, gzipped
or not, and keeps each layer's disk as the image's lineage. Windows images don't push or save yet.

## Workers

`launch` runs a machine in a **worker**: this program's own binary, run again, which `carbon::init()` takes over. It
outlives the program that launched it, a crash in it takes nothing else down, and it has a VM of its own (on macOS a
process has one, so this is how one program runs several machines on hvf at once). The launcher gets a `Machine`,
reached over its control socket as any other.

- **Rules** go to the worker as data, and are decided there.
- **Hooks with code** stay in the launcher. The worker asks it about each event they want, over a hook link (a Unix
  socket it connects to before its kernel boots), and waits for the decision. Once the launcher is gone, an event a
  hook with code wanted is **refused** (output passes), so a policy written in code never quietly stops holding.
- **Logs**: what it writes is kept in `<home>/machines/<name>/output` (8 MiB, then rotated once), read with
  `Machine::logs`, from any process.

## Seen working

On macOS, through hvf: `output` of a command in `debian:13`; an `apt-get` answered without running, `uname`
rewritten, a password redacted from output, `chmod 777` refused by a rule, a connection to an allowed host made and
one to any other refused; a machine started on a thread, `exec`ed into at once, observed, stopped (143) and removed;
three machines in a row in one process, each with its own VM. Two launched machines running at once, each in its own
worker and VM; a launched machine's `whoami` answered and `uname` rewritten by the launcher's hooks, and refused once
the launcher had exited. A script copied into a running machine, run at once with its mode kept. `nginx:alpine`
launched with `-P`: port 80 published from its config, its page served, its logs read, and stopped by its SIGQUIT.
A host folder bound into a machine: written, linked, renamed and removed on both sides, each seen by the other at
once while it ran; read-only refusing writes (EROFS); `npm install` of Express into a bound project (67 packages and
their links, on the host). A volume's file chowned to 999 and chmod 600, the same for the next machine; a new volume
filled from the image's `/etc/apt`. As users: `-u node` refused `/root`, `/etc/shadow` and `/usr/local` and given its own home; an image's `USER` taken by default and by `exec`; `exec -u nobody` and `-u 1234:1234`; a setuid root copy of `id` (`euid=0`) and a setgid one; `/tmp`'s sticky bit refusing another user's `rm`; `chown` refused to a user; `su` as root, and refused without a password; nginx's workers as `nginx`; a build that `useradd`s, then `USER app`, `COPY --chown` and a `RUN` as `app`. With Docker Hub: a login refused and one kept; an image built on `alpine:3.20` pushed with alpine's layer mounted from `library/alpine` and two step layers sent (its `RUN`'s removed `/etc/motd` a whiteout), pulled into an empty home and run as it was built, its history as built; pushed again with nothing sent and the same digest; pushed squashed; `quay.io`'s alpine pushed to Docker Hub, its layer fetched and sent; a push without a login refused with what to do. Two images saved as one archive and loaded into an empty home, plain, gzipped from stdin, and as an OCI layout alone; `debian` saved (139 MB) and loaded with its setuid, setgid, sticky bits, groups and hard links as they were; a loaded image pushed as the three layers it came as; `build --output type=oci`. A build with every `RUN --mount`: `pip`'s cache reused by the next build, a bound
`requirements.txt`, a secret as a file and as a variable, a tmpfs, and none of it in the image. From the CLI: `--deny-exec` in a Linux machine (EACCES,
126) and a Windows one (`where` refused when cmd starts it), a refused first program, `--deny` with `--allow`, and
`carbon watch` showing another process's `carbon exec`, its output, its lookups and its refusals. A program outside
this repository built with `carbon` as a git dependency, and ran a machine.

## Testing

`cargo test -p carbon -p hooks`: the hooks' chain, filters, observers (and what they lose), subscriptions and
their timeouts; rules' patterns and networks; events as JSON and back; piped streams through the output hooks; a
refused connection and an answered lookup through the network's host; records, names and copies. The rest end to end,
with [`examples/agent.rs`](examples/agent.rs) and `carbon`.

## To do

In [the proposal](../docs/carbon_lib.md)'s order:

- [x] **Workers** (`launch`), logs and timeouts; **files**, and copies that keep metadata, running or stopped;
      **images'** whole config, `tag` and history.
- [x] **Building** from a Carbonfile (the Dockerfile's grammar), with its cache.
- [x] **Mounts and volumes**, and `RUN --mount` (`cache`, `bind`, `tmpfs`, `secret`).
- [ ] **The rest of mounts:** in Windows machines (a drive or a folder); an image's `VOLUME`s made volumes at
      `launch`; `RUN --mount=type=bind,from=` and `rw`, a secret's target outside `/run/secrets`, `type=ssh`;
      volumes' metadata kept on Windows hosts; seeing a bound folder's changes through inotify.
- [x] **Users**: `MachineSpec::user`, `ExecSpec::user`, the image's `USER` (by `RUN` too), and lambda checking
      files' permissions, setuid and setgid against them.
- [ ] **The rest of users:** in Windows machines (refused today); `--group-add`; `user` in a `Files` operation (they
      are root's); file capabilities (`security.capability`) at `execve`.
- [x] **Registries**: `login`, `push`, `save`, `load`; a built image pushed a layer per step (each its disk's
      difference from the step before, as a tar with whiteouts), `--squash` for one; `build --output type=docker|oci`.
- [ ] **The rest of registries:** Windows images pushed and saved; several platforms pushed as one index;
      `build --output type=tar` (the files alone); `carbon export` of a machine's disk as a tar.
- [ ] **Policy and budget** (step 11), held by every build too; **checkpoints** of a running machine (step 12).
- [ ] **Windows images built**: `FROM atlas`, `RUN` by `cmd.exe` and `powershell`, `COPY` into `C:\`.
- [ ] **The rest of building**: `ADD` of a URL (`--checksum`) or a git repository, and a tar or git URL as the
      context; `COPY --parents`; symbolic `--chmod`; bzip2, xz and zstd archives; stages that don't need each
      other built at once; `HEALTHCHECK` run, its state in `MachineInfo`; BuildKit's own parser test files
      (`frontend/dockerfile/parser/testfiles`) and substitution cases vendored into the tests; comparing a build's
      files with `docker build`'s.
- [ ] **Open, for building:** labels of Carbon's an image could carry (`carbon.network=none`), *recommended* none
      until images want their own rules; a push's shape, *recommended* a layer per step; `ADD` of a URL fetched on
      the host or in a machine, *recommended* in a machine (by the build's own network, so `--deny` holds) and git
      cloned in a small helper image; `RUN` in a worker or on a thread, *recommended* a worker until start-up shows
      in builds of many short steps.
- [ ] Workers on Windows hosts: the control socket and hook link are Unix sockets today.
- [ ] **More hook points**: files (open, create, write, remove, rename), process lifecycle and signals, accepts on
      published ports, connection data, the registry, desktop dialogs, time and randomness.
- [ ] **Answers**: a Windows program's start answered (a process that ends at once with the answer's output); a
      connection answered by a peer in memory.
- [ ] **A rules file** (`--policy`) and read-only paths (`--read-only`), once files are hooked.
- [ ] **For agents**: checkpoints and rollback of a running machine's disk, forks, record and replay, limits,
      virtual time, and `carbon mcp`.
- [ ] **Async**: `Subscription` and `Process` as streams, behind a feature, so no runtime is chosen for its users.
- [ ] **Windows hosts**: the control socket (exec, watch, stop from another process) is Unix-only today.
- [ ] **Other languages**, as clients of [the control protocol](src/wire.rs).
