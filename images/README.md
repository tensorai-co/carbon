# images

What machines start from, kept on the host's disk. An image is an index (its layers, merged: see
[`layers`](../layers/README.md)), its platform, what its config says to run, and its lineage: what it was made from.
Images are pulled from OCI registries over HTTPS and pushed back to them, saved to and loaded from archives as
`docker save` writes them, imported from a tar of a root file system, or committed from a machine's disk. Every chunk of every
image and machine is in one store, by its SHA-256, so what images share is kept once.

**Built.** `machine`, the CLI and the SDK use it; no kernel ever does. Nothing in it knows a kernel: a kernel's `M` is
made from each tar entry by the caller.

## On the host's disk

Everything is under `CARBON_HOME`, or `~/.carbon`:

```text
chunks/ab/cdef…                                 every image's and machine's file contents and indexes, by SHA-256, each kept once
images/docker.io/library/debian/13.json         an image: its index, its platform, its whole config, its history, its lineage
auth.json                                       logins to registries, readable by their owner alone
machines/dev/machine.json                       a machine: its kernel, its image and that image's index (pinned), its process
machines/dev/log                                its writable layer: every change to its disk since its image
```

- **The store** (`store.rs`, `DiskStore`): chunks in files named by their SHA-256, checked as they're read, written by
  rename; a machine's log, synced with the chunks it names.
- **Registries** (`registry.rs`): the distribution API (Docker Hub, GHCR, Quay, any), with a token from each
  registry's challenge (asked for with a login if there is one) or a login on each request if it wants that; HTTPS,
  or plain HTTP to a registry on this host (`localhost:5000`), as Docker treats those.
- **Logins** (`auth.rs`): `<home>/auth.json`, mode 600, in Docker's form; a registry it has none for is looked for in
  Docker's own `config.json`, through its credential helper (`docker-credential-osxkeychain` and the like) or its
  `auths`, so a host that has run `docker login` needs no `carbon login`.
- **Pulling** (`pull.rs`): the platform chosen from its index, every blob checked against its digest, and the layers
  stacked with their whiteouts and frozen into one index. What the registry had is kept as the image's lineage: the
  repository and each layer's digest, size, media type and diff ID.
- **Lineage** (`Lineage` in `lib.rs`): where an image's first disk came from (a repository and its layers there, for
  a pulled one), and each disk it was made through since, with where in its history each was made. A machine committed
  into an image adds the disk it began from; a build's steps are commits, so a built image has a disk per step.
- **Pushing** (`push.rs`): a layer per disk. The first is the registry's own layers when it was pulled: already there,
  mounted from the repository it came from on the same registry (nothing sent), or fetched from that registry and
  sent to another. Each disk after it is a layer of what differs from the disk before (`layers::tar::export`, gzipped),
  made into a file as it's written and hashed both ways, so nothing is held in memory; a step that changed nothing
  makes none. The history is made to say which steps made layers, as OCI wants, and `squash` makes one layer of it
  all. The same disks always make the same bytes, so a second push sends nothing and gets the same digest.
- **Archives** (`archive.rs`): `save` writes an OCI image layout with Docker's `manifest.json` beside it, as Docker 25
  and later do, a layer per disk as plain tars, and several images at once with what they share once. `load` reads
  that, an OCI layout alone, or Docker's older archive, gzipped or not, by seeking to what its index names; each
  layer's disk is kept as the loaded image's lineage, so it pushes and saves again as the layers it came as.
- **HTTPS** (`tls.rs`): rustls, with a verifier that asks [`host`](../host/README.md), so the host's own store and
  rules decide whom to trust.
- **Importing** a tar, and **saving** a committed machine as an image.
- **Config** (`config.rs`): OCI's whole image config (environment, entrypoint, command, user, working folder, exposed
  ports, volumes, labels, stop signal, health check, shell, `ONBUILD`) and its history, kept from a pull, in one JSON
  form for images and machine records. **Tagging** names an image again, sharing its index.

`debian:13` pulls in about 15 s, with a 2.2 MB index.

## Testing

```sh
cargo test -p images
cargo run --release -p images --example pull -- debian:13 linux/arm64   # pulled and looked into, without a machine
```

Names in full, references, digests and platforms; an image imported, saved and found again, with its lineage; base64,
registries' names, and a login kept private, found and forgotten; history made to say which steps made layers; an
image saved a layer per step, loaded back with its files, config and history, and saved again as the same layers.

## To do

As images ask:

- [ ] zstd layers (gzip only today).
- [x] Registries that want a login; pushing; saving and loading archives.
- [ ] Streaming a layer being pulled or loaded rather than holding it in memory (a pushed or saved one is streamed).
- [ ] Pushing several platforms as one index; pushing and saving Windows images (their layers have `Files/` and
      `Hives/`); keeping a layer made for a push, so the next push of the same disks needn't make it again.
- [ ] A registry's chunked uploads (`PATCH`), for ones that refuse a whole blob in one request; identity tokens from
      a credential helper (Docker Desktop's `<token>` user).
- [ ] Clearing chunks nothing uses any more.
