# Carbon as a library

Status: **built**, in its first three steps of seven (below): the crate `carbon`, the CLI over it, hooks on programs,
output, connections and lookups in both kernels, and rules. It replaced `sdk/README.md` and the crate `machine`, both
gone. What's here is what was built, and where it went its own way from the proposal, why.

## In short

1. **No "sdk" crate.** Carbon's library is the crate `carbon`, in a top-level folder `carbon/`. It's what Carbon *is*;
   "SDK" is only a word for how it looks from outside. Everything a program can do with Carbon, it does through
   `carbon`, and it's used from GitHub as any git dependency is:

   ```toml
   [dependencies]
   carbon = { git = "https://github.com/tensorai-co/carbon" }
   ```

2. **`machine` folded into `carbon`.** Its `Machines`, `Run` and `Record` became `Carbon`, `MachineSpec` and
   `MachineInfo`; its control socket became `carbon::wire`.
3. **The CLI is a thin client of `carbon`**, and depends on nothing else. Every command and flag it had, it still
   has, and each is one or two calls into the library.
4. **Hooks are the point.** Carbon owns the whole kernel, so every program a machine starts, every byte it writes to
   its terminal, every connection it makes and every name it looks up passes through Carbon's own code. The library
   gives those to the program using it as real-time hooks: it sees each one as it happens and can **allow it, block
   it, rewrite it, or answer it itself**, while the machine's program waits.
5. **A small crate, `hooks`, holds the hook types**, with no dependencies of its own: the kernels call into it, and
   `carbon` re-exports it as `carbon::hooks`.
6. **Hooks across processes** come with workers (step 4). Until then, a machine's hooks run in the process that runs
   it; what they decided is seen from any process, with `Machine::watch` and `carbon watch`.

## What changed

| Crate | Before | Now |
| --- | --- | --- |
| `machine` | the top: `Machines`, `Run`, `Record`, booting each kernel, `control.rs`, `signals.rs` | folded into `carbon` |
| `cli` | parsed arguments and called `machine` and `images` directly | a thin layer over `carbon` |
| `sdk` | a README; nothing built | removed |
| `hooks` | didn't exist | events and decisions, the `Hooks` trait, filters, and the `Hub` (observers, subscriptions) |
| lambda, atlas | | a `hooks::Hub` in their `Options`, asked at `execve` and `CreateProcess`; `Stop::is_running`; they let go of everything they held before `boot` returns |
| `kernels-core` | | `thread::let_go`, which waits for a kernel's other threads to let it go |
| `images`, `layers`, `exec`, `net`, `display`, `audio`, `gpu`, `host` | the parts | unchanged |

```
carbon/                 crate `carbon`
  src/
    lib.rs              Carbon, Notice, init, and what the parts re-export
    spec.rs             MachineSpec, Running, Exit, Output, Display, Audio
    machine.rs          Machine, MachineInfo, ExecSpec, Process, Screen
    boot.rs             making a machine and running it (from machine/)
    store.rs            machines on the host's disk: records, freeze, copy, import, remove (from machine/)
    stdio.rs            a machine's streams, and the host::Host its kernel writes through: output's hook point
    network.rs          the net::Host its network runs on: connect's and lookup's hook point
    rules.rs            Rules
    stop.rs             stopping from outside the kernel
    wire.rs             the control protocol, both sides (from machine/control.rs)
    signals.rs          this process's signals, for a machine that is the process (from machine/)
  examples/agent.rs     the library, as an agent uses it
hooks/                  crate `hooks`
cli/                    the `carbon` command, over the crate `carbon`
```

The library package `carbon` and the binary `carbon` (built by `cli`) share the name, as `cargo` does; the binary's
docs are off, so `cargo doc` gives the library's.

**Output, connections and lookups are hooked without touching a kernel or `net`.** Both kernels write the machine's
stdout and stderr through the `host::Host` they're given, and a machine's network does everything through the
`net::Host` it's made with, so `carbon` gives them hosts of its own that ask the hooks first. Only programs starting
needed hook points inside the kernels.

## The library

```rust
use carbon::hooks::{Decision, Filter, Reply};
use carbon::{Carbon, Display, Isolation, Network};

fn main() -> carbon::Result<()> {
    carbon::init(); // first: the sandbox's stub is this same binary, run again (workers will be too)
    let carbon = Carbon::open()?; // CARBON_HOME, or ~/.carbon

    // A command in a fresh machine, thrown away after (Docker's `run --rm`).
    let out = carbon
        .machine("python:3.13-slim")
        .isolation(Isolation::Vm)
        .network(Network::None)
        .memory("512m")
        .rm(true)
        .output(["python3", "-c", "print(6 * 7)"])?; // exit code, stdout, stderr
    println!("{}", out.stdout_lossy());

    // A named machine that's kept (Docker's `run -d --name dev -p 2222:22`).
    let dev = carbon.machine("debian:13").name("dev").publish("2222:22").start(["/usr/sbin/sshd", "-D"])?;

    // `exec`: a process in the running machine, with a terminal or with pipes.
    let mut p = dev.machine().exec(["bash", "-lc", "apt-get update"]).spawn()?;
    for chunk in p.stdout() {
        print!("{}", String::from_utf8_lossy(&chunk));
    }
    println!("{}", p.wait());

    // A Windows program from the host's disk, and its screen, for computer use.
    let app = carbon.program("installers/setup.exe").display(Display::headless(1280, 800)).start(Vec::<String>::new())?;
    app.machine().screen().click(500, 300)?;
    std::fs::write("screen.png", app.machine().screen().screenshot()?)?;

    dev.stop();
    dev.wait()?;
    carbon.get("dev")?.freeze("dev:1")?; // an image of its disk, which new machines can start from
    Ok(())
}
```

| Type | What it is |
| --- | --- |
| `Carbon` | the store on the host's disk, opened once and shared by cloning: `machine(image)`, `program(path)` and `again(name)` make a `MachineSpec`; `get(name)`, `machines()`, `images()`, `pull`, `import`, `remove_image`; `on_notice` for what machines fall back to and warn of (stderr by default, since Carbon never falls back silently) |
| `MachineSpec` | a builder: name, isolation, memory, platform, env, workdir, network and publish, display, gpu, audio, `stdin`/`stdout`/`stderr` (`Stdio::Inherit`, `Null`, `Piped`), tty, `relay_signals`, `rm`, hooks and rules; `run(argv)` on this thread to the end, `output(argv)` with what it wrote, `start(argv)` on a thread of its own |
| `Running` | what `start` gives, once the machine's kernel is up: its piped streams, `hooks()`, `machine()`, `stop`, `kill`, `wait` |
| `Machine` | a machine by name, running or not, in this process or another: `info`, `exec`, `stop`, `kill`, `wait`, `ports`, `screen`, `watch`, `freeze`, `copy_in`, `copy_out`, `remove` |
| `ExecSpec`, `Process` | what `exec` makes and starts: `env`, `workdir`, `tty`; `spawn` (stdin, output as it comes, `resize`, `signal`, `wait`), `output`, `detach`, `attach` (to this terminal, as `carbon exec`) |
| `Screen` | a running machine's desktop: screenshot, windows, click, drag, scroll, type, key, clipboard, drop, resize |
| `Image` | pulled, frozen, or imported: its name, platform and config |
| `Error` | a kind to match on (`NotFound`, `AlreadyExists`, `Running`, `NotRunning`, `InvalidInput`, `Unsupported`, `Denied`, `Image`, `Io`, `Failed`) and a message in words |

Every CLI command maps onto these: `run` is `machine(...)` or `program(...)` or `again(...)`, then `.run`; `exec` is
`Machine::exec(...).attach`; `port` is `Machine::ports`; `display ...` is `Screen`; `cp` is `copy_in` and `copy_out`;
`freeze` is `Machine::freeze`; `watch` is `Machine::watch`.

**Where it differs from the sketch, and why**: `run(...).output()` became `output(...)`, since a run has nothing to
configure after it's named; `publish` takes Docker's text and checks it, as `memory` does, with errors held until the
machine runs, so a builder chain needs no `?`; `freeze`, `copy_in` and the rest are `Machine`'s, not `Running`'s,
since they need a stopped machine; and `rm` was added, since a program running many commands would otherwise keep a
machine for each.

## Hooks

### The model

A hook point is a place in Carbon's own code that a program can't get around. When something there is hooked, Carbon
makes an **event** (who's asking, and for what) and calls the machine's hooks **on the thread that asked**. That
guest thread is stopped until the hooks answer, so a decision really decides: nothing has happened yet. The machine's
other threads run on.

| Level | What it means | Example |
| --- | --- | --- |
| **Observe** | see it; nothing changes | a live feed of every program started, with its arguments |
| **Decide** | allow it, or block it (the program gets the error its own system would give) | `rm -rf /` gets EACCES; a connection to a host that isn't on the list is refused |
| **Rewrite** | let it happen, differently | `npm install` run with `--ignore-scripts`; a connection sent to a mirror; a token redacted from stdout |
| **Answer** | do it in Carbon instead, and give the program the result | `apt-get install` "succeeds" with output of the hook's own; a name resolves to an address of its own |

- **Nothing hooked costs nothing.** Each hook point checks one atomic flag (per machine, per kind of event) before it
  makes an event. Filters (`Filter::program`, `Filter::port`, `Filter::host`, `Filter::stream`) are given with a
  closure and checked before it's called; a `Hooks` type says which kinds it wants with `interest()`.
- **Order.** Hooks, closures and rules are one chain, in the order they're added, and the first that decides (anything
  but `Decision::Pass`) decides. Then subscribers, in order. Then it's allowed. Observers see what was finally decided.
- **Time limits.** A subscriber that doesn't decide in time gets its timeout's default (allow, or deny), so a stuck
  agent can't hang a machine. Hooks and closures run on the asking thread and have no timeout: they're code.
- **Who.** An exec says its pid and parent where the kernel knows them, its path, arguments, environment and folder,
  and whether the program's file is there; observers are told which machine.

### What's hooked

| Area | Events | Decisions | Where |
| --- | --- | --- | --- |
| **Programs** | `Exec { os, origin, pid, parent, path, argv, env, cwd, found }`, before a program starts | allow; deny (EACCES; ERROR_ACCESS_DENIED; an error before the machine boots); rewrite the program, arguments, environment; **answer** with stdout, stderr and an exit code, running nothing | lambda `process/load.rs` (`execve`: `hook_exec`, `answer_exec`); atlas `os/procapi.rs` (`CreateProcess`); `carbon/src/boot.rs` (a first program, `hook_first`; an `exec`, `hooked_enter`) |
| **Terminal and stdio** | `Output { stream, source, bytes }`: what a process writes to the machine's stdout or stderr, or an `exec`'s | pass; drop; **replace the bytes** | `carbon/src/stdio.rs` (the `host::Host` both kernels write through), `wire::Hooked` (each `exec`) |
| **Network** | `Connect { proto, to, name }`, with the name a lookup gave that address | allow; refuse (ECONNREFUSED, EPERM for UDP); **redirect** to another address (UDP's answers come back from where the program sent) | `carbon/src/network.rs` (the `net::Host` both kernels' sockets go through) |
| | `Lookup { name }` | allow; NXDOMAIN; **answer** with addresses; rewrite the name | the same, for its DNS server and atlas's `getaddrinfo` |

Limits that come with where the hooks are, found as they were built:

- **A program the image hasn't got isn't asked about.** Shells (bash, and dash too) look along `PATH` with `stat`
  before they start anything, so a missing program never reaches `execve`. An exec that does reach it, of a file that
  isn't there, is asked about with `found: false` (`carbon watch` leaves those out when they're let fail).
- **Answering a Windows program's start isn't there yet**: `CreateProcess` gives its caller a process and a thread,
  and an answer needs one that ends at once with the answer's output; until then an answer is refused, as a deny.
- **Encrypted traffic is opaque**, and there's no `Data` hook yet. Reading TLS would need a Carbon CA trusted inside
  the machine (lambda: added to the image's store; atlas: its certificate checks are already Carbon's, through `host`).
- **`Output` sees what the program wrote** to the machine's own stdio or an `exec`'s, before a terminal's line
  discipline; writes between processes inside the machine (a pipe) aren't events.
- **A hook holds up only the thread that asked.** A multi-threaded program's other threads go on, and a slow hook
  makes the program slow.

### In Rust

A program's own decisions, as a type:

```rust
use carbon::hooks::{Connect, Decision, Exec, Hooks, Interest, Lookup, Output, Reply};

struct Agent;

impl Hooks for Agent {
    fn exec(&self, e: &Exec) -> Decision<Exec> {
        match e.program() {
            "rm" if e.argv().iter().any(|a| a == "-rf") => Decision::deny(),
            "npm" => Decision::rewrite(e.clone().arg("--ignore-scripts")),
            "apt-get" => Decision::answer(Reply::new().stdout("Done\n").exit(0)),
            _ => Decision::pass(),
        }
    }

    fn connect(&self, c: &Connect) -> Decision<Connect> {
        match c.name() {
            Some("pypi.org" | "files.pythonhosted.org") => Decision::allow(),
            _ => Decision::refuse(),
        }
    }

    fn lookup(&self, l: &Lookup) -> Decision<Lookup> {
        if l.name() == "api.internal" { Decision::answer(vec!["10.89.0.1".parse().unwrap()]) } else { Decision::pass() }
    }

    fn output(&self, o: &Output) -> Decision<Output> {
        o.replace(redact_secrets(o.bytes())) // the agent's own function
    }

    fn interest(&self) -> Interest {
        Interest::all()
    }
}

let m = carbon.machine("debian:13").hooks(Agent).start(["bash"])?;
```

`Decision<E>` is typed by its event, so an answer that doesn't fit doesn't compile: an exec's is a `Reply`, a
lookup's addresses, and output and connections have none (yet). Closures for one-offs, each with its filter:

```rust
use carbon::hooks::Filter;

let m = carbon
    .machine("debian:13")
    .on_exec(Filter::program("curl"), |e| {
        println!("curl {:?}", e.argv());
        Decision::pass()
    })
    .on_connect(Filter::port(443), |c| if allowed(c) { Decision::allow() } else { Decision::refuse() })
    .start(["bash"])?;
```

**Decisions that take a while** (a model, a person) don't hold a thread of the agent's: subscribe, and get each event
as a `Pending` to decide from anywhere before its time limit.

```rust
use carbon::hooks::{Interest, Timeout};

let events = m.hooks().subscribe(Interest::exec() | Interest::connect(), Timeout::secs(30).default_deny());
for pending in events {
    if ask_the_model(pending.event()).approved { pending.allow() } else { pending.deny() }
}
```

**Observers** get what happened, for dashboards and audit logs: `m.hooks().observe(Interest::all(), 1024)` in the same
process, `machine.watch(interest)` from any other (the machine's control socket streams them as JSON lines), and
`carbon watch NAME` on the command line. An observer that falls behind loses events and is told how many.

### Rules, for speed

Many decisions need no code. `carbon::Rules` is a `Hooks` like any other, checked on the asking thread, and it's what
the CLI's flags make:

```sh
carbon run --deny 0.0.0.0/0 --deny ::/0 --allow pypi.org:443 debian:13 -- pip install requests
carbon run --deny-exec 'rm -rf *' debian:13 -- ./agent-step.sh
```

```rust
let rules = Rules::new().deny_exec("rm -rf *").deny("0.0.0.0/0")?.allow("pypi.org:443")?;
```

An `--allow` beats a `--deny`; a host takes its subdomains and matches the name the machine looked up; an exec
pattern's first word is the program's name and each after it one argument, with `*` and `?`, and a last `*` takes
the rest.

## Machines in processes of their own

hvf allows one VM per process, so on macOS a program with several machines **at once** will run each in a
**worker**: the same binary, started again (`carbon::init()` already runs first in `main`, for the sandbox's stub,
and is where a worker will take over). Machines **one after another** in one process work today: each kernel waits,
before `boot` returns, for its own threads to let it go, so its VM is gone and the next machine gets one of its own
(a second at once falls back to the sandbox, and says so).

The library talks to a running machine over its control socket (`carbon::wire`):

- **Requests and answers**: screenshot, windows (as a table, or JSON), input, clipboard, drops, resize, and `stop`
  and `kill`, so stopping a machine never signals the process it runs in.
- **`exec` frames**: stdin, terminal sizes and signals in; stdout, stderr and the exit code out.
- **`watch`**: each hooked event, as it's decided, as a JSON line.
- **Hook frames**, with workers: the worker sends an event and the library answers with a decision, while the guest
  thread waits in the worker. Rules go to the worker when the machine starts, so they never cross.

The protocol is documented in `wire.rs` and kept stable, so other languages are clients of it: a Python or TypeScript
package is a thin layer over the socket, not a second implementation.

## The CLI on top

| Command | What it calls |
| --- | --- |
| `carbon run [-i] [-t] [--rm] [-p] [--network] [-w] IMAGE CMD` | `carbon.machine(image)`, `.program(path)` or `.again(name)`, then `.run` |
| `carbon exec [-i] [-t] [-d] NAME CMD` | `Machine::exec`, then `.attach` or `.detach` |
| `carbon ls`, `port`, `stop`, `rm`, `freeze`, `cp`, `display ...` | `Carbon::machines`; `Machine::ports`, `stop`, `remove`, `freeze`, `copy_in`, `copy_out`; `Screen` |
| `carbon watch NAME [--exec] [--output] [--net] [--json]` | `Machine::watch` |
| `carbon run --deny-exec`, `--allow`, `--deny` | `Rules` |
| `carbon mcp` | an MCP server over the library ([`mcp`](../mcp/README.md)): stdio built, HTTP to do |

## For agents, beyond hooks

What the parts already make cheap, which the library should name (step 6):

- **Checkpoints and rollback.** A machine's disk is layers, so a checkpoint is a layer frozen and rolling back is
  dropping what's above it. (Today a machine must be stopped to freeze; checkpointing a running one needs `layers` to
  freeze its top layer in place.)
- **Forks.** A second machine from a checkpoint, to try two things at once.
- **Record and replay.** Every hooked event and its decision, as a log (`watch`'s lines are most of it): what an
  agent's step did, to audit, or to run again with the same answers.
- **Limits.** Memory (there today), CPUs, time, bytes out and processes started, per machine; a hook hears when one is
  reached.
- **Virtual time.** A clock that runs fast, for programs that wait, or one that's frozen, for a test that repeats.

## The order of work

1. **`carbon` from `machine`.** *Done:* `cli` depends only on `carbon`, and everything that worked still does
   (atlas's 33 test programs and lambda's, Debian and atlas-base machines, `exec`, `stop`, `import`).
2. **`hooks`, and the first hook points:** exec in both kernels, output, connect and lookup, in the same process; and
   `carbon watch`. *Done:* a program using the library blocks a command, rewrites one, answers one, redacts output
   and refuses a connection; tested in each kernel.
3. **Rules:** network allow and deny, exec patterns. *Done*, as `Rules`, `--deny-exec`, `--allow` and `--deny`.
4. **Workers**, and hook frames over the socket; several machines at once from one program on macOS.
5. **The other hook points:** files, process lifecycle and signals, input,
   accepts, data, the registry, desktop dialogs, time. And the answers not there yet: a Windows exec, a connection.
6. **For agents:** checkpoints and rollback, `carbon mcp`, record and replay, limits, virtual time.
7. **Other languages**, over the wire protocol; async streams for `Subscription` and `Process`, behind a feature.

## Decided

1. **One crate, or `carbon-core` under `carbon`?** One crate, `carbon`. Split it only if build times ask.
2. **A sync or an async API?** Sync: the kernels are threads, not futures, and hooks run on guest threads.
   Subscriptions and processes are iterators; async wrappers come later, behind a feature, so the library doesn't
   choose a runtime for its users.
3. **Where do the hook types live?** In `hooks`, with no dependencies, re-exported as `carbon::hooks`.
4. **How real is an answered exec?** A real process that writes the answer and exits, with no program loaded (in
   lambda, the process that called `execve`; for a first program or an `exec`, no kernel process at all). Rewriting
   is how to run something else instead.
5. **Hooks with `--isolation sandbox`.** The kernel is host code under either isolation, so every hook works the same
   in the sandbox as in a VM.
6. **Trust.** Hooks run on the host, with the host's rights. A rule is data, but a hook is code: the library never
   loads hooks from inside a machine.

## Open

1. **Answering a program the image hasn't got.** Shells never `execve` it. A hook could ask for a file to be made
   for it (a stub on the disk, in the system layer, that runs nothing), so `git push` can be answered in an image
   without git. Is that a decision a hook should be able to make?
2. **Workers' API**: always (one model everywhere, and a crash that doesn't take the program with it), or only when a
   second machine needs one? *Recommendation:* always for `start`, never for `run`, which is the CLI's case.
