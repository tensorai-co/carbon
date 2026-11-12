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
carbon display windows app                  # its windows, top first: ID, place, size, state, title
carbon display click app 640 400            # input, as a user's: move, click, drag, scroll, type, key
carbon display type app 'Hello, world!'
carbon display key app ctrl+s
carbon display resize app 1600x900@2        # its screen's size and scale, changed while it runs
carbon display clipboard app                # its clipboard's text (with text after: set, as copied on the host)
carbon display drop app 300 200 --files 'C:\notes.txt'  # drops files there, as dragged in from the host (or text)
carbon pull debian:13 [--platform OS/ARCH] | images | rmi IMAGE
carbon atlas imports setup.exe              # what a Windows program imports that atlas lacks (--all: everything)
carbon atlas imports ~/installers/          # a folder of them: which load, and what's wanted most
```

| `run` option | Meaning |
| --- | --- |
| `--isolation vm\|sandbox\|auto` | A hardware VM, the process sandbox, or the best there is (the default, which says so when it falls back) |
| `--name NAME` | The machine's name; else one from its image or program |
| `--platform OS/ARCH` | Which build of an image to pull; Linux for this CPU by default |
| `--display headless=WxH[@SCALE]\|host[=WxH]\|host-windows\|none` | Where its windows go: a screen in memory (`headless=1280x800` by default), the whole desktop in a window on this Mac's screen (which closing stops the machine), or each window in a Mac window of its own |
| `--audio none` | Where its sound goes: only `none` until `audio` is built |
| `-e`, `--env NAME=VALUE` | An environment variable, over the image's |
| `--memory SIZE` | How much memory it may use, as `512m` or `4g` (`4g` by default; only what's touched is taken) |

## What's coming

Each module's README has its part; together:

```sh
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
