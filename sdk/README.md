# sdk

**Not started.** Carbon as a Rust library: the crate `carbon`, which the `carbon` command will become a thin layer
over. Everything the CLI does, a program can do, without a shell in between.

```rust
use carbon::{Carbon, Display, Isolation, Network};

fn main() -> carbon::Result<()> {
    // On macOS a machine runs in a worker process: this same binary, if it calls this first.
    carbon::worker::run_if_worker();
    let carbon = Carbon::open()?; // CARBON_HOME, or ~/.carbon

    // One command in a fresh machine, thrown away after.
    let output = carbon
        .command("python:3.13-slim", "python3")
        .args(["untrusted.py"])
        .isolation(Isolation::Vm)
        .network(Network::None)
        .memory_mib(512)
        .output()?;
    println!("{}", output.stdout_lossy());

    // A named machine keeps its disk; freeze it into an image.
    let dev = carbon.machine("dev")?;
    dev.freeze("checkpoint:1")?;

    // A desktop to drive.
    let app = carbon.command("setup.exe", "").display(Display::headless(1280, 800)).spawn()?;
    std::fs::write("screen.png", app.screen()?.screenshot()?)?;
    app.screen()?.click(500, 300)?;
    Ok(())
}
```

## What it is

- **`Carbon`**: the store on the host's disk (images, machines), opened once.
- **`Command`**: an image (or a program on the host) and what to run in it, with the machine's options:
  isolation, memory, CPUs, environment, network (`--net`, `--publish`), display, audio. `output()` runs it to the end
  and gives its exit code and output; `status()` only the code; `spawn()` gives a running `Machine`.
- **`Machine`**: a machine, running or not, by name: its stdin and stdout, signals, `wait`, `stop`, `freeze`,
  `remove`, and its `screen()`.
- **`Screen`**: `screenshot`, `click`, `type`, `key`, and its windows, for computer use by agents.
- **Images:** `pull`, `import`, `list`, `remove`.

It's a layer over [`machine`](../machine/README.md) and [`images`](../images/README.md), as `cli` is; the CLI's
commands become calls into it. A program using it on macOS is signed with `carbon.entitlements`, like `carbon` itself.

## For agents

What agents need from it: a fresh machine for every step, thrown away afterwards, fast enough
to feel like a local command. Clean state from the same image each time; checkpoints with `freeze`; limits on memory,
CPUs and network per task; and screens to look at and click, headless.

## To do

- [ ] The crate, over `machine` and `images`: `Carbon`, `Command` (`output`, `status`, `spawn`), `Machine`, images.
- [ ] `Screen`, once `display` has the commands `carbon display` will (screenshot is there; click, type, key).
- [ ] The CLI rewritten over it.
- [ ] Worker processes (`run_if_worker`), once `machine` runs machines in workers of their own.
- [ ] Forking a machine (`fork`: a second machine sharing what it can), once `layers` and `exec` can share
      a running machine's disk and memory.
