# host

What kernels need from the host they run on, as one `Host` trait: the standard streams, clocks, randomness, CPUs, the
memory the host could give, the terminal, and whether the host trusts a certificate chain. A kernel never calls the
host's OS itself; it asks a `Host`, so the same kernel runs on any host, and tests give it a fake one
(`Std::capturing`).

**Built**, on all three hosts. Sockets and DNS aren't here: they're [`net`](../net/README.md)'s.

## What it has

- **Standard streams.** stdin is read by a thread of its own, so a kernel can wait on it and be woken
  (`watch_input`), as it waits on anything else. lambda's console feeds it through its tty's line discipline.
- **Clocks, randomness, CPUs, memory.** The memory the host could give a machine, from Mach's VM statistics (macOS),
  `/proc/meminfo` (Linux) or `GlobalMemoryStatusEx` (Windows); randomness from `getentropy` or `BCryptGenRandom`.
- **The terminal**: raw mode and window size, for lambda's console (macOS and Linux, in `unix.rs`).
- **Certificate trust.** Whether the host trusts a chain, for any use, a TLS server (with its name) or a client. It's
  checked offline: nothing is fetched to complete a chain or check revocation. Every failure found is reported at
  once, so an expired self-signed certificate is `untrusted root, not valid at this time`. `images` pulls through it,
  and atlas's `crypt32`, Schannel, WinHTTP and WinINet will check through it: Linux programs bring their own CA bundle
  in their image, but Windows programs expect the OS to have one.

## Each host

The shared part is `src/lib.rs`. What only one host can do is in its folder, with `unix.rs` for what macOS and Linux
do alike:

| Host | Memory | Randomness | Trust |
| --- | --- | --- | --- |
| macOS (`macos/`) | Mach's VM statistics | `getentropy` | Security.framework |
| Linux (`linux/`) | `/proc/meminfo` | `getentropy` | webpki over the system's CA bundle (`/etc/ssl/certs`) |
| Windows (`windows/`) | `GlobalMemoryStatusEx` | `BCryptGenRandom` | CryptoAPI's chain engine |

Windows keeps few roots on disk and adds the rest from Microsoft's root program as a chain first needs one. That's its
store working as designed, so it's allowed.

## Testing

```sh
cargo test -p host
```

A real chain (registry-1.docker.io's, at a fixed time) trusted for its names, and refused for another name, time or
use; test certificates from `tests/certs/make.sh`: an untrusted root, a chain that stops short, every failure at once,
rubbish. They pass on all three hosts, and Linux's checks also run on every host against test roots.

## To do

- [ ] A console in raw mode on Windows hosts (raw mode is Unix-only today), checked with a full-screen editor in a
      machine.
- [ ] Waiting on host events beyond stdin, as kernels need them.
