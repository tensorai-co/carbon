# cli

`carbon`: the command. It parses its own arguments and calls the library [`carbon`](../carbon/README.md), and
nothing else: every command is something a Rust program can do with the crate.

**Built**, on macOS, Linux and Windows. `carbon help` lists everything.

```sh
cargo build --release -p cli
codesign --force --sign - --entitlements carbon.entitlements target/release/carbon   # macOS: the hypervisor
```

## Commands

```sh
carbon run -it debian:13 bash               # a shell (pulled if it isn't here); a machine called debian
carbon run debian:13 -- ls /                # a command: its output here, no stdin (-i for stdin, -t for a terminal)
carbon run --name dev debian:13             # named; `carbon run --name dev` starts it again, files as they were
carbon exec -it dev bash                    # a shell in the running machine dev, as docker exec
carbon run -d -P --name web nginx:alpine    # in a process of its own, outliving this one; every exposed port published
carbon logs -f --tail 20 web                # what it wrote (stdout here, stderr there), and what it writes
carbon run -v ~/work:/work -v pgdata:/var/lib/postgresql/data postgres:17   # a host folder, live; a volume
carbon run --mount type=bind,src=./config.toml,dst=/etc/app.toml,readonly --tmpfs /scratch app:1
carbon volume create pgdata | ls | inspect pgdata | rm pgdata
carbon exec --timeout 5m web ./check.sh     # killed after five minutes (exit 124); run --timeout for a whole machine
carbon run -it atlas                        # atlas-base, Carbon's Windows userland, at cmd.exe's prompt
carbon run -it atlas powershell             # or PowerShell's
carbon exec -it app cmd                     # a cmd in a running Windows machine
carbon run --platform linux/amd64 debian:13 -- uname -m     # x86_64, translated on Apple silicon
carbon run --isolation vm debian:13 -- ./untrusted.sh       # hardware VM or fail; sandbox forces the sandbox
carbon run setup.exe                        # a Windows program, with atlas, on a disk of its own
carbon run --rm debian:13 -- uname -a       # a machine for one command, removed after
carbon run --deny 0.0.0.0/0 --allow pypi.org:443 debian:13 -- pip install requests   # only there
carbon run --deny-exec 'rm -rf *' --deny-exec 'shutdown *' debian:13 -- ./agent-step.sh
carbon watch dev                            # what it does, as it does it: programs, output, connections, lookups
carbon watch dev --exec --net --json        # as JSON lines, for a program to read
carbon ls                                   # NAME  FROM  ISOLATION  STATUS
carbon stop dev                             # SIGTERM to its first process; everything ends 10 s later
carbon rm dev                               # a stopped machine and its files
carbon freeze dev mine:1                    # an image of a stopped machine's disk
carbon cp app:'C:\logs' out/               # a machine's file or folder, running or stopped, copied out to the host
carbon cp ./tool web:/usr/local/bin         # or in: modes, times, symlinks and hard links kept
carbon display screenshot app -o screen.png # a running machine's screen
carbon display windows app                  # its windows, top first: ID, place, size, state, title
carbon display click app 640 400            # input, as a user's: move, click, drag, scroll, type, key
carbon display type app 'Hello, world!'
carbon display key app ctrl+s
carbon display resize app 1600x900@2        # its screen's size and scale, changed while it runs
carbon display clipboard app                # its clipboard's text (with text after: set, as copied on the host)
carbon display drop app 300 200 --files 'C:\notes.txt'  # drops files there, as dragged in from the host (or text)
carbon pull debian:13 [--platform OS/ARCH] | rmi IMAGE
carbon login -u someone                     # a login for docker.io (or `carbon login ghcr.io`), asked for unseen
carbon push someone/app:1                   # a layer per step; a pulled base's layers taken from where they are
carbon save -o app.tar app:1 debian:13      # images as one archive, as docker save writes it; docker load reads it
carbon load -i app.tar                      # docker save's archives, or an OCI layout, gzipped or not
carbon images                               # KERNEL  IMAGE  PLATFORM  FROM: atlas for a Windows image, lambda for a Linux one
carbon inspect nginx:alpine                 # an image as JSON: its whole config (ports, stop signal, labels…), history, layers
carbon history nginx:alpine                 # CREATED  CHANGED  CREATED BY: each step that made it
carbon tag nginx:alpine web:1               # another name for the same image
carbon build -t app:1 .                     # an image from ./Carbonfile, else ./Dockerfile (a Dockerfile, exactly)
carbon build -f Dockerfile.prod --build-arg VERSION=2 --target runtime -t app:2 .
carbon build --deny 0.0.0.0/0 --allow pypi.org:443 --allow files.pythonhosted.org:443 -t locked:1 .
carbon import -c 'CMD ["cmd.exe"]' atlas-base-arm64.tar atlas   # an image from a tar of a root file system
carbon atlas imports setup.exe              # what a Windows program imports that atlas lacks (--all: everything)
carbon atlas imports ~/installers/          # a folder of them: which load, and what's wanted most
```

| `run` option | Meaning |
| --- | --- |
| `-i`, `--interactive` | Keep stdin: the machine's stdin is this one's. Without it, it's empty from the start, as Docker's is |
| `-t`, `--tty` | With `-i`, this terminal is the machine's console (raw, the kernel editing lines: lambda's line discipline, atlas's conhost-like console), `TERM=xterm`; `-it` for a shell. Without `-t`, a Windows machine's standard handles are pipes, not a console. `-it` with stdin not a terminal is refused, as Docker refuses it |
| `--isolation vm\|sandbox\|auto` | A hardware VM, the process sandbox, or the best there is (the default, which says so when it falls back) |
| `--name NAME` | The machine's name; else one from its image or program |
| `--platform OS/ARCH` | Which build of an image to pull; Linux for this CPU by default |
| `--display headless=WxH[@SCALE]\|host[=WxH]\|host-windows\|none` | Where its windows go: a screen in memory (`headless=1280x800` by default), the whole desktop in a window on this Mac's screen (which closing stops the machine), or each window in a Mac window of its own |
| `--gpu auto\|host\|software\|none` | Its GPU: the host's if there is one, else the CPU device (`auto`, the default, which says so when it takes the CPU device), the host's or fail, the CPU device, or none. Linux machines reach it through lambda's Vulkan driver; Windows machines don't yet |
| `--audio auto\|host\|none\|FILE.wav` | Where its sound goes: the host's speakers when it's on the host's screen (`auto`, the default), the host's speakers or fail, nowhere, or a WAV file of everything played |
| `-e`, `--env NAME=VALUE` | An environment variable, over the image's |
| `-w`, `--workdir DIR` | Where its program starts; the image's working directory by default |
| `-u`, `--user USER[:GROUP]` | Who its programs run as: a name or a number from the image's `/etc/passwd` and `/etc/group`; the image's `USER` by default, else root. Linux machines only, for now |
| `--memory SIZE` | How much memory it may use, as `512m` or `4g` (`4g` by default, or ¾ of a container's memory limit if that's less, as on Cloud Run; only what's touched is taken) |
| `--rm` | Removed, and its files, once it ends |
| `-v`, `--volume SOURCE:TARGET[:ro]` | Mounts a host folder or file (`/path`, `./path`, `~/path`) or a volume (a name, made if it isn't there) at `TARGET`: what either side changes, the other sees at once. A new volume is filled from the image's files at `TARGET`. A machine run again keeps its mounts unless given new ones |
| `--mount type=bind\|volume\|tmpfs,source=…,target=…[,readonly]` | The same, as Docker's `--mount` |
| `--tmpfs TARGET` | A folder in memory only |
| `-d`, `--detach` | In a process of its own (a worker: `carbon` run again), which outlives this one; prints its name. What it writes is kept, for `carbon logs`. Each worker has its own VM, so on macOS several run at once |
| `--timeout DURATION` | Stopped if it's still running this long after it starts: `90s`, `5m`, `2h` |
| `-P`, `--publish-all` | Every port its image exposes (`EXPOSE`), each on a free host port |
| `--deny-exec PATTERN` | Refuses to start programs whose arguments match: the first word is a program's name (`rm`, not `/bin/rm`), each after it one argument, with `*` and `?` as a shell has them, and a last `*` takes any arguments left. In a Linux machine the program gets EACCES (126 from a shell); in a Windows one, ERROR_ACCESS_DENIED; a first program refused is an error |
| `--deny HOST\|ADDRESS\|NETWORK[:PORT]` | Refuses connections there (TCP connects, and UDP to each new destination): `0.0.0.0/0` and `::/0` for everything |
| `--allow HOST\|ADDRESS\|NETWORK[:PORT]` | Lets connections there through, whatever `--deny` says: a host takes its subdomains, and matches the name the machine looked up |

`carbon exec [options] NAME COMMAND [ARGS...]` runs a command in a running machine, as `docker exec` does: a process
of its own (lambda: a session with no parent in the machine; atlas: a console of its own), with the machine's
environment, working directory and user unless `-e`, `-w` and `-u` say otherwise. `-i` relays stdin, `-t` gives it a terminal
of its own (a pty in lambda, a console in atlas) with this one raw and its size followed, `-d` starts it and leaves
it. Its exit code is `carbon exec`'s; without `-t`, Ctrl-C and other signals go to it. It goes over the machine's
control socket, so it's there on macOS and Linux hosts (not yet Windows hosts).

`carbon watch NAME [--exec] [--output] [--net] [--json]` shows what a running machine does as it does it, from any
process: each program it starts (with its arguments), what it writes, each connection and lookup, and what its rules
and hooks decided (`[denied]`, `[rewritten: ...]`, `[answered]`). With `--json`, a line each, as
[`carbon::wire`](../carbon/src/wire.rs) writes them. It reads the machine's control socket, as `carbon exec` does.

`carbon stop NAME` asks the machine over its control socket (a machine run by a program using the library is
stopped without signalling that program), and signals the process it's in only if it can't. Its first process gets
the image's stop signal (`STOPSIGNAL`: nginx's SIGQUIT), SIGTERM if it has none.

`carbon logs [-f] [--tail N] NAME` prints what a machine run with `-d` wrote, kept in its folder (8 MiB, then
rotated once): its stdout to stdout and its stderr to stderr, in the order written. `-f` keeps following until it
stops. `carbon ls` says how a stopped machine last ended (`exited (143)`).

`carbon cp NAME:PATH DIR` and `carbon cp PATH NAME:DIR` copy a file or folder out of a machine or into it, running
or stopped. A running machine's files are reached through its own kernel, over its control socket, so its programs
see a copy at once. Kinds, modes, times, symlinks and hard links are kept.

`carbon build [options] CONTEXT` builds an image from the context folder's `Carbonfile`, else its `Dockerfile`: any
Dockerfile, unchanged, as `docker build` reads it. `-t NAME` names it (as often as wanted), `-f FILE` (or `-f -` for
stdin) names the file, `--build-arg NAME=VALUE`, `--target STAGE`, `--platform OS/ARCH` (x86-64 on Apple silicon runs
translated), `--no-cache`, `-o DIR` copies what's built to a folder, `-q` prints only its name. `--isolation`,
`--network`, `--memory`, `--timeout`, `--deny`, `--allow` and `--deny-exec` are `run`'s, for every `RUN`, so a build
can reach only what it's allowed to. `--secret id=ID,src=FILE` (or `env=VAR`) gives a `RUN --mount=type=secret,id=ID`
its secret, never kept in the image; `RUN --mount=type=cache` keeps its folder (pip's, apt's caches) between builds.

`carbon volume create|ls|inspect|rm NAME` manages volumes: folders under `CARBON_HOME/volumes` that machines mount and
outlive, each file's owner and mode kept. Each step is printed as it starts, with what a `RUN` writes; a step done before
is found in the cache. `carbon watch build-<id>-<n>` shows a step's machine as it runs.

`carbon inspect IMAGE` prints an image as JSON: OCI's whole config (environment, entrypoint, command, user, working
folder, exposed ports, volumes, labels, stop signal, health check, shell, `ONBUILD`), its history, and its layers'
digests. `carbon history IMAGE` is the history as a table; `carbon tag SOURCE TARGET` names it again.

`carbon login [-u USER] [--password-stdin] [REGISTRY]` checks a login with a registry (`docker.io` by default) and
keeps it in `CARBON_HOME/auth.json`, readable by its owner alone; without `-u` and a password it asks for them, the
password unseen. A registry Carbon has no login for is looked for in Docker's (`~/.docker/config.json` and its
credential helper), so `docker login` is enough. `carbon logout [REGISTRY]` forgets Carbon's.

`carbon push [--squash] IMAGE` pushes an image to its registry, as `docker push` does, and prints `NAME@DIGEST`. Each
step that made it is a layer: the layers of a base it was built on, pulled from the same registry, are taken from the
base's repository with nothing sent; from another registry, they're fetched from there and sent. A step that changed
no files makes no layer. A second push of the same image sends nothing and gets the same digest. `--squash` pushes
it as one layer. Tag an image first to push it somewhere else (`carbon tag app:1 ghcr.io/team/app:1`).

`carbon save [-o FILE] IMAGE...` writes images as one archive (to stdout without `-o`): an OCI image layout with
Docker's `manifest.json`, as Docker 25 and later write, a layer per step, which `docker load` reads. `carbon load
[-i FILE]` loads one (from stdin without `-i`): Docker's, an OCI layout, or Docker's older kind, gzipped or not.
`carbon build --output type=docker,dest=FILE` (or `type=oci`) also writes the built image so.

`carbon import [--platform OS/ARCH] [-c INSTRUCTION]... FILE|- [NAME]` makes an image from a tar of a root file
system, as `docker import` does. `-c` sets its config with a Dockerfile instruction: `CMD`, `ENTRYPOINT`, `ENV`,
`WORKDIR` or `USER`. Without `--platform`, a tar whose first entry is `Windows/` is a Windows image for this host's CPU,
and anything else is Linux's. Without a name, it's named for the file.

Windows' programs come from an image, as Linux's do. **atlas-base**
([tensorai-co/atlas-base](https://github.com/tensorai-co/atlas-base)) is Carbon's Windows userland: `cmd.exe`,
`powershell.exe`, `where`, `findstr`, `find`, `more`, `sort`, `whoami`, `hostname` and `timeout`. Its `build.sh` makes
a tar for each ISA, which `carbon import` takes in as `atlas`. Soon it'll also be on Docker Hub as `tensoraico/atlas`.
A program named without a path is found at the drive's root, in System32, in `C:\Windows` or in PowerShell's folder.
The image `windows` is atlas's system layer and nothing else: its runtime DLLs, and no programs.

## What's coming

Each module's README has its part; together:

```sh
# gpu (`--gpu` and `carbon gpu`, listing the host's device and the CPU one, are built)
carbon run --gpu host=NAME …                  # one of several host GPUs
carbon ls                                     # gains a GPU column beside ISOLATION


# net: networks between machines (ports in, --allow and --deny are built: -p, --network nat|host|none, carbon port)
carbon run --net lab --dns 1.1.1.1 --hostname web …
carbon net ls | inspect lab | log web

# remote: serving a desktop, and connecting to one
carbon run --name app --remote 0.0.0.0:4433 setup.exe
carbon remote allow ~/.ssh/id_ed25519.pub --name laptop | keys | revoke laptop | start app | stop app
carbon connect box.example.com/app [--windows] [--view-only] [--headless out/]

# machines, and agents
carbon fork dev dev-2
carbon run --policy agent.toml --read-only /etc debian:13 -- bash
carbon mcp                                    # an MCP server over the library, for agents that speak MCP
```

## To do

- [ ] The options and commands above, as their modules come.
- [x] Rewritten over the library `carbon`, so the CLI and the library can't drift apart.
- [ ] An interactive console on Windows hosts (it works on macOS and Linux): see `host`.

## Open question

**The `auto` default.** Keep `auto`, with a loud notice when it falls back to the sandbox, or default to `vm` wherever
a hypervisor exists and require `--isolation sandbox` to use the sandbox? Today it's `auto`, and it never falls back
silently.
