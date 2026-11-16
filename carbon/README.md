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
| [`Carbon`](src/lib.rs) | Carbon on this host: its folder of machines and images. `machine(image)`, `program(path)`, `again(name)` make a `MachineSpec`; `get(name)`, `machines()`, `images()`, `pull`, `import`, `remove_image`; `on_notice` for what machines fall back to |
| [`MachineSpec`](src/spec.rs) | A builder: `name`, `isolation`, `platform`, `env`, `workdir`, `memory`, `network`, `publish`, `display`, `gpu`, `audio`, `stdin`/`stdout`/`stderr` ([`Stdio`](src/stdio.rs): inherit, null, piped), `tty`, `relay_signals`, `rm`, and `hooks`, `rules`, `on_exec`, `on_output`, `on_connect`, `on_lookup`. Then `run(argv)` on this thread, `output(argv)` to its end with what it wrote, or `start(argv)` on a thread of its own |
| [`Running`](src/spec.rs) | A machine this process runs: `take_stdin`, `take_stdout`, `take_stderr` ([`Chunks`](src/stdio.rs), also an `io::Read`), `hooks()`, `machine()`, `stop`, `kill`, `wait` |
| [`Machine`](src/machine.rs) | A machine by name, in this process or another: `info`, `exec(argv)`, `stop`, `kill`, `wait`, `ports`, `screen`, `watch`, `freeze`, `copy_in`, `copy_out`, `remove` |
| [`ExecSpec`, `Process`](src/machine.rs) | A process in a running machine, as `docker exec`: `env`, `workdir`, `tty`; then `spawn` (stdin, output as it comes, `resize`, `signal`, `wait`), `output`, `detach`, or `attach` to this terminal |
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

## Seen working

On macOS, through hvf: `output` of a command in `debian:13`; an `apt-get` answered without running, `uname`
rewritten, a password redacted from output, `chmod 777` refused by a rule, a connection to an allowed host made and
one to any other refused; a machine started on a thread, `exec`ed into at once, observed, stopped (143) and removed;
three machines in a row in one process, each with its own VM. From the CLI: `--deny-exec` in a Linux machine (EACCES,
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

- [ ] **Workers**: a machine in a process of its own (this binary, run again by `init`), so one program runs several
      at once on macOS, where hvf allows one VM per process (a second falls back to the sandbox today, and says so),
      and a machine that crashes doesn't take the program with it. Hook frames over the control socket, under the same
      API.
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
