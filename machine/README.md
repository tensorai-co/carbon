# machine

Puts one machine together and owns its life. A machine is a kernel, on exec with the isolation asked for, over a disk
that's its image's index plus its own writable layer, with a display, sound, a network and the host's services.
`cli` and `sdk` are front ends over it; nothing outside `machine/` depends on a kernel.

**Built** for what exists: Linux and Windows machines run from images or programs, are kept by name, and are
stopped, frozen and started again. Sound, published ports and the host's display come as those modules do.

`machine` doesn't run the kernel's threads: that run loop belongs to [`kernels/core`](../kernels/README.md), because
a kernel's locks and its run loop have to be designed together.

## What it does

1. **Picks the kernel** from the image or program: lambda for a Linux image or an ELF, atlas for a Windows image or a
   PE (and, later, an MSI).
2. **Opens exec** with `--isolation`, and says loudly when `auto` falls back to the sandbox, with why:
   `carbon: no hypervisor here (vm/kvm: /dev/kvm missing), running with --isolation sandbox`.
3. **Mounts the disk**: the image's index from `images`, pinned by hash when the machine was made (so pulling the name
   again doesn't move it), under the machine's own log.
4. **Finds the program**: one named without a `/` is found on the image's `PATH`; with none, the image's entrypoint
   and command run, with its environment and `--env` over it.
5. **Attaches a display, sound, a network and host services**: a headless desktop (atlas), the machine's own `net`
   network, `host::Std`. Later: the host's windows and speakers, `--net` and `--publish`, and `remote` when `--remote`
   asks for it.
6. **Runs it** in the calling process (hvf allows one VM per process, and a machine is one), relaying stdin, stdout
   and the exit code. SIGINT, SIGTERM and SIGHUP go to the first process; a second signal, or ten seconds, ends
   everything (`signals.rs`).
7. **Keeps its state** in `<home>/machines/<name>/`: its record says whether it's running (its process), so another
   `carbon` can stop it, screenshot it (over a Unix socket the machine answers on, `control.rs`), freeze it or remove
   it, and `carbon run --name NAME` starts it again with its files as it left them.

```rust
let (exec, cpu) = Exec::open(Options { memory, isolation, ..Options::default() })?;
if isolation == Isolation::Auto && exec.backend().isolation == Isolation::Sandbox {
    eprintln!("carbon: no hypervisor here, running with --isolation sandbox");   // OpenError says why, on failure
}
// The kernel's Options carry the rest: its network, environment, hostname, and a Stop for other threads.
let status = lambda::boot(exec, cpu, disk, host, &argv, options)?;   // or atlas::boot
```

## Seen working

On all three hosts (macOS through hvf; Linux through kvm and its sandbox; Windows through winkvm): `debian:13` pulled,
run, started again with a file it wrote, frozen into an image a new machine runs; exit codes; a Windows program from
the host's disk. On macOS and Linux also: an interactive `bash` through a terminal, `carbon stop` on a running
`sleep 30` (exit 143), and on macOS a screenshot of a Windows program's window. On Linux, with `/dev/kvm` out of
reach, `auto` falls back to the sandbox and says why, and `--isolation vm` refuses.

## Testing

`cargo test -p machine`: records round-trip, and programs are found on the image's `PATH`. The rest is checked end to
end through `carbon`.

## To do

- [ ] **Sound, networks and the host's display**, as [`audio`](../audio/README.md), [`net`](../net/README.md)'s
      phases 3 and 5, and [`display`](../display/README.md)'s host backends come: `--audio`, `--net`, `--publish`,
      `--display host`; and `--remote`, attaching a [`remote`](../remote/README.md) listener to the desktop and mixer.
- [ ] **The image's working directory and user** for its first process (lambda starts it in `/`, as root).
- [ ] **A worker process per machine**, once one `carbon` process runs several (a daemon, or the SDK): hvf allows one
      VM per process.
- [ ] **`msiexec` for an `.msi`**, once atlas has it.
- [ ] **On Windows hosts**: stopping a machine and taking its screenshot from another process (the control socket is
      Unix-only today).
- [ ] **Shutting down cleanly**: a way to ask a running machine to stop that commits its disk and exits, whatever
      signal reached it.
- [ ] **Out of memory says so**: a machine that runs out of guest memory should fail clearly, not with odd faults.
- [ ] **Forking a machine** (`carbon fork`), once `layers` and `exec` can share a running machine's disk and
      memory.
