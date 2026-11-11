# images

What machines start from, kept on the host's disk. An image is an index (its layers, merged: see
[`layers`](../layers/README.md)), its platform, and what its config says to run. Images are pulled from OCI registries
over HTTPS, imported from a tar of a root file system, or saved from a machine's frozen disk. Every chunk of every
image and machine is in one store, by its SHA-256, so what images share is kept once.

**Built.** `machine`, the CLI and the SDK use it; no kernel ever does. Nothing in it knows a kernel: a kernel's `M` is
made from each tar entry by the caller.

## On the host's disk

Everything is under `CARBON_HOME`, or `~/.carbon`:

```text
chunks/ab/cdef…                                 every image's and machine's file contents and indexes, by SHA-256, each kept once
images/docker.io/library/debian/13.json         an image: its index, its platform, what its config says to run
machines/dev/machine.json                       a machine: its kernel, its image and that image's index (pinned), its process
machines/dev/log                                its writable layer: every change to its disk since its image
```

- **The store** (`store.rs`, `DiskStore`): chunks in files named by their SHA-256, checked as they're read, written by
  rename; a machine's log, synced with the chunks it names.
- **Pulling** (`pull.rs`): from any registry with the distribution API (Docker Hub, GHCR, …), with tokens from its
  challenge, the platform chosen from its index, every blob checked against its digest, and the layers stacked with
  their whiteouts and frozen into one index.
- **HTTPS** (`tls.rs`): rustls, with a verifier that asks [`host`](../host/README.md), so the host's own store and
  rules decide whom to trust.
- **Importing** a tar, and **saving** a frozen machine as an image.

`debian:13` pulls in about 15 s, with a 2.2 MB index.

## Testing

```sh
cargo test -p images
cargo run --release -p images --example pull -- debian:13 linux/arm64   # pulled and looked into, without a machine
```

Names in full, references, digests and platforms; an image imported, saved and found again.

## To do

As images ask:

- [ ] zstd layers (gzip only today).
- [ ] Registries that want a login.
- [ ] Streaming a layer rather than holding it in memory.
- [ ] Clearing chunks nothing uses any more.
