# hooks

What a machine does, as it does it, and what can be decided about it: the events Carbon's hook points make, the
decisions about them, and the `Hub` each machine's hook points ask. It has no dependencies, so the kernels can depend
on it; [`carbon`](../carbon/README.md) re-exports it as `carbon::hooks`.

| Event | Fields | Answer |
| --- | --- | --- |
| `Exec` | `os`, `origin` (`First`, `Program`, `Outside`), `pid`, `parent`, `path`, `argv`, `env`, `cwd`, `found` | `Reply`: stdout, stderr, an exit code |
| `Output` | `stream` (`Stdout`, `Stderr`), `source` (`Machine`, `Exec(id)`), `bytes` | none |
| `Connect` | `proto` (`Tcp`, `Udp`), `to`, `name` (the name a lookup gave that address) | none (yet) |
| `Lookup` | `name` | addresses |

A [`Decision<E>`](src/lib.rs) is `Pass`, `Allow`, `Deny`, `Rewrite(E)` or `Answer(E::Answer)`, typed by its event, so
an answer that doesn't fit doesn't compile. A [`Verdict`](src/lib.rs) is the same without the type, for observers and
subscribers; one for the wrong kind of event is a deny.

```rust
use hooks::{Decision, Exec, Filter, Hooks, Hub, Interest, Reply, Timeout};

struct Agent;

impl Hooks for Agent {
    fn exec(&self, e: &Exec) -> Decision<Exec> {
        match e.program() {
            "npm" => Decision::rewrite(e.clone().arg("--ignore-scripts")),
            "git" if e.args().first().map(String::as_str) == Some("push") => {
                Decision::answer(Reply::new().stdout("Everything up-to-date\n").exit(0))
            }
            _ => Decision::pass(),
        }
    }
    fn interest(&self) -> Interest {
        Interest::exec() // only execs are made into events for it
    }
}

let hub = Hub::new();
hub.add(Agent);
hub.on_connect(Filter::port(443), |c| if c.name() == Some("pypi.org") { Decision::allow() } else { Decision::refuse() });
let seen = hub.observe(Interest::all(), 1024); // what was decided, as it happens
let asked = hub.subscribe(Interest::lookup(), Timeout::secs(30).default_deny()); // decided from another thread
```

- **The chain**: hooks added with `add` and closures added with `on_exec`, `on_output`, `on_connect` and `on_lookup`
  are asked in order; the first that doesn't `Pass` decides. Then subscribers, in order; then it's allowed.
- **Nothing hooked costs nothing**: `Hub::wants(kind)` is one atomic load, which every hook point checks before it
  makes an event. It's the union of what hooks, closures, observers and subscribers want, kept as they come and go.
- **Observers** get `Observed { machine, event, verdict, at }` through a bounded channel; one that's full loses the
  event and counts it (`Observer::lost`).
- **Subscribers** get a `Pending` for each event, which the program's thread waits on until it's decided, dropped
  (then the next subscriber's), or its timeout's default.

Hook points, and what each kernel does with each decision, are in [`carbon`'s README](../carbon/README.md#hooks).

`cargo test -p hooks`: the chain and its order, filters, typed answers and verdicts of the wrong kind, observers
(and what they lose when they fall behind, and that a dropped one stops being wanted), and subscribers deciding in
time, too late, and not at all.
