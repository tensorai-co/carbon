# cli

`carbon`: the command. It parses its own arguments and calls [`machine`](../machine/README.md) and
[`images`](../images/README.md); once [`sdk`](../sdk/README.md) exists, it becomes a thin layer over it.

**Built**, on macOS, Linux and Windows. `carbon help` lists everything.

```sh
cargo build --release -p cli
codesign --force --sign - --entitlements carbon.entitlements target/release/carbon   # macOS: the hypervisor
```

## Commands

```sh
carbon run debian:13 -- bash                # pulled if it isn't here; a machine called debian
carbon run --name dev debian:13             # named; `carbon run --name dev` starts it again, files as they were
carbon run --platform linux/amd64 debian:13 -- uname -m     # x86_64, translated on Apple silicon
carbon run --isolation vm debian:13 -- ./untrusted.sh       # hardware VM or fail; sandbox forces the sandbox
carbon run setup.exe                        # a Windows program, with atlas, on a disk of its own
carbon ls                                   # NAME  FROM  ISOLATION  STATUS
carbon stop dev                             # SIGTERM to its first process; everything ends 10 s later
carbon rm dev                               # a stopped machine and its files
carbon freeze dev mine:1                    # an image of a stopped machine's disk
carbon display screenshot app -o screen.png # a running machine's screen
carbon pull debian:13 [--platform OS/ARCH] | images | rmi IMAGE
```

| `run` option | Meaning |
| --- | --- |
| `--isolation vm\|sandbox\|auto` | A hardware VM, the process sandbox, or the best there is (the default, which says so when it falls back) |
| `--name NAME` | The machine's name; else one from its image or program |
| `--platform OS/ARCH` | Which build of an image to pull; Linux for this CPU by default |
| `--display headless=WxH\|none` | Where its windows go; `headless=1280x800` by default |
| `--audio none` | Where its sound goes: only `none` until `audio` is built |
| `-e`, `--env NAME=VALUE` | An environment variable, over the image's |
| `--memory SIZE` | How much memory it may use, as `512m` or `4g` (`4g` by default; only what's touched is taken) |

## What's coming

Each module's README has its part; together:

```sh
# display: the host's windows, and driving a machine
carbon run --display host setup.exe
carbon display click app 300 200 | type app "hello" | key app Return

# audio
carbon run --audio host … | --audio out.wav …

# net: in, policy, and networks between machines
carbon run --publish 8080:80 debian:13 -- python3 -m http.server 80
carbon run --net none|host|lab --allow pypi.org:443 --deny 0.0.0.0/0 --dns 1.1.1.1 --hostname web …
carbon net ls | inspect lab | log web
carbon port web

# remote: serving a desktop, and connecting to one
carbon run --name app --remote 0.0.0.0:4433 setup.exe
carbon remote allow ~/.ssh/id_ed25519.pub --name laptop | keys | revoke laptop | start app | stop app
carbon connect box.example.com/app [--windows] [--view-only] [--headless out/]

# machines
carbon cp dev:/etc/os-release ./os-release
carbon fork dev dev-2
```

## To do

- [ ] The options and commands above, as their modules come.
- [ ] Rewritten over `sdk`, so the CLI and the library can't drift apart.
- [ ] An interactive console on Windows hosts (it works on macOS and Linux): see `host`.

## Open question

**The `auto` default.** Keep `auto`, with a loud notice when it falls back to the sandbox, or default to `vm` wherever
a hypervisor exists and require `--isolation sandbox` to use the sandbox? Today it's `auto`, and it never falls back
silently.
