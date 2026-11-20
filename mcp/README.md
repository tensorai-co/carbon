# mcp

`carbon mcp`: Carbon's front door. It's an MCP server for agents that speak MCP (Claude Code, Claude Desktop, IDEs,
any MCP client), and with `--engine` it's also the **Engine API**, for the programs that run agents and set their
machines up. An agent gets machines to work in, Linux and Windows, as Docker's CLI would give them to it: run a
command in a throwaway machine; start machines that keep running, `exec` into them, read and edit their files, copy in
and out; build images from a Carbonfile, pull, tag, push, save and load them; and drive a machine's screen.

```sh
claude mcp add carbon -- carbon mcp                       # Claude Code: the whole Carbon home, on stdio
claude mcp add dev -- carbon mcp --machine dev            # one machine, and nothing else
carbon mcp --host-dir ~/work/shared --deny-exec 'git push *' --deny 0.0.0.0/0 --allow pypi.org:443
carbon mcp -H 127.0.0.1:8080                              # HTTP and WebSocket, this host only
carbon mcp -H 0.0.0.0:8443 --tls --tlscert cert.pem --tlskey key.pem --token-file tokens --engine
carbon mcp -H 0.0.0.0:$PORT --engine --max-sessions 1 --no-auth   # a sandbox host: one session, then exit
```

**Built.**
- **Transports:** stdio by default. With `-H`, HTTP and WebSocket on the same address, over TCP or a Unix socket, with
  TLS 1.3 and tokens.
- **Both of the protocol's eras:** 2026-07-28's stateless requests (`server/discover`, each request's version and
  capabilities in its `_meta`), and the `initialize` handshake of 2025-11-25 and before, which most clients speak
  today.
- **The Engine API** with `--engine`, as JSON-RPC and REST.
- Every tool and every Engine API method is a call or two into the library `carbon`; nothing below them is this
  crate's.

## Two planes

| | MCP | The Engine API (`--engine`) |
| --- | --- | --- |
| For | A model | A program that runs models: a sandbox's gateway, CI, an app |
| Has | Task-shaped [tools](#tools) with descriptions and annotations for models | [Machines, files and images](#the-engine-api) with every setting: hostname, env, secrets, mounts, rules, commit, push |
| Methods | MCP's own: `initialize`, `tools/list`, `tools/call`, … | `engine/…`, never listed in `tools/list` |
| Trust | What a model may do | Whoever may set a machine up |

Both share a connection. **Anything that relays an agent's calls forwards MCP's methods and never `engine/…`**: the
namespace makes that one check, so an agent can't write its own keys or loosen its own network.

## Tools

| | Tools |
| --- | --- |
| **Machines** | `run` (a command in a new machine, removed after, as `docker run --rm`), `start_machine` (`docker run -d`; a Linux machine runs `sleep infinity` unless given a command, so it's there for `exec`), `restart_machine`, `stop_machine`, `remove_machine`, `list_machines`, `inspect_machine`, `machine_logs` |
| **Processes** | `exec` (waited for, with its exit code, stdout and stderr; or `background`), `process_output`, `process_input`, `process_signal` |
| **Files** (running or stopped) | `read_file` (by bytes or lines; pictures as images; binary as base64), `write_file`, `edit_file` (an exact string replaced, keeping the file's mode and owner), `list_files`, `move_file`, `remove_file`, `make_folder`, `copy_to_machine`, `copy_from_machine`, `copy_between_machines` |
| **Images** | `list_images`, `inspect_image`, `pull_image`, `build_image` (its context as files in the call, a folder in a machine, or a shared host folder), `commit_machine`, `tag_image`, `push_image`, `remove_image`, `save_image`, `load_image` |
| **Volumes** | `list_volumes`, `create_volume`, `remove_volume`; a machine's `mounts` as `docker run --mount` |
| **Screens** | `screenshot`, `screen_input` (click, drag, scroll, type, keys), `list_windows`, `clipboard`, `resize_screen` |
| **Events** | `machine_events`: each program a machine starts, its connections and lookups, and what the rules decided |

Each has annotations (read-only, destructive, idempotent, open world) so a client asks the user before the ones that
need it. Resources: `carbon://images`, `carbon://machines`, `carbon://volumes`, and a machine's record, logs, screen
and any file in it (`carbon://machines/{name}/files/{path}`).

## What holds an agent in

What holds any machine in: the machine (a VM or the sandbox, its own disk, network and users), the flags it's
started with, and Carbon's rules, given to the server as `carbon run` takes them. There's no permission system of
the server's own. Its own lines:

- **Host paths only under a `--host-dir`**, checked after links and `..` are resolved, for bind mounts, copies, build
  contexts and archives. Without one, no tool names a host path at all.
- **`--machine NAME`** serves one machine: its tools only, with no `machine` argument, and a call naming another is
  refused. It can't stop, remove or leave its machine.
- **Cloud metadata** (`169.254.169.254`, `fd00:ec2::254`) is refused to every machine the server starts, unless an
  `--allow` says otherwise.
- **`--tools a,b`** lists only those.

A call can add `deny_exec` and `deny` rules of its own to a machine it starts, never an `allow`. Logins for registries
are the host's: `push_image` uses them, and no tool takes or returns a credential.

## Results

- **Output** comes back as text with the exit code, and as structured content. Past 64 KiB a stream, its head and
  tail come back with what was cut said between, and the whole is kept in the machine (`/tmp/carbon-exec-<id>.out`)
  for `read_file`.
- **Long work** (a command, a build, a pull, a push) sends `notifications/progress` when the client gave a
  `progressToken`.
- **A failed call** (no such machine, a refused path, a build step that failed) is a result with `isError` and the
  library's message; a command that exits non-zero isn't a failed call, and its code is in the result.
- **Machines and processes:** `run` launches a worker per command (so the server holds no VM, and runs many at once
  on macOS), waits for it, and removes it. Background processes and event feeds are kept in the server's process;
  logs and events are read by cursor (a line count; an event count).

## On the network: `-H`

`-H ADDR` (as many as wanted) takes `host:port` (`:8080` is every address) or a Unix socket's path, with no scheme.
Each address serves:

| Path | What |
| --- | --- |
| `POST /mcp` | MCP's Streamable HTTP: one JSON-RPC message per request. A request's answer is JSON, or server-sent events (its progress, then the answer) when the client takes `text/event-stream` and gave a `progressToken`. Notifications get `202`. `GET` and `DELETE` get `405`: every request stands alone. |
| `/v1/…` | The Engine API as REST, with `--engine` ([`openapi.yaml`](openapi.yaml)) |
| WebSocket at `/` or `/mcp` | Everything on one connection (subprotocol `mcp`). Text messages are JSON-RPC, MCP's and `engine/…` alike. Binary messages are streams' bytes: `[stream: u32, big-endian][kind: u8][bytes]`, kinds `0` stdin, `1` stdout, `2` stderr, `3` exit (an `i32`, big-endian), `4` data, `5` end. |
| `GET /_ping` | `OK`, with no token, for health checks |

**Who may connect:**

| Where | Who | Token |
| --- | --- | --- |
| stdio | Whoever started it | None: it reaches everything |
| A Unix socket | Whoever its mode lets in: `0600`, the owner (a stale socket is replaced; it's removed at exit) | Optional |
| TCP on loopback | This host | Optional, with a warning at start |
| TCP beyond loopback | Anyone who reaches the port | Required: it won't start without `--token-file`, unless `--no-auth` says something in front of it does auth |

- **Tokens:** `--token-file FILE` has a line per token, `SCOPES SHA256-HEX NAME` (`#` for comments). The scopes are
  `carbon:mcp` and `carbon:engine`, comma-separated. Clients send `Authorization: Bearer <token>`, on each HTTP
  request or once on a WebSocket's upgrade.
- **Making and revoking tokens:** `carbon mcp token [--scope …] [--name …]` prints a new token and its line; only
  hashes go on disk. The file is read again whenever it changes, so revoking a token is deleting its line. A file
  that doesn't parse lets no one in until it's fixed.
- **Scopes:** an `engine/…` call without `carbon:engine` gets `-32001` (forbidden), and REST gets `403`. Without
  `--engine`, `engine/…` doesn't exist (`-32601`; REST `404`).
- **`Origin`:** a request carrying one must come from an `--allow-origin` (`*` for any), so a web page can't drive a
  local Carbon.
- **TLS:** Docker's flags, `--tls --tlscert FILE --tlskey FILE` (PEM; RSA, Ed25519, or ECDSA on a named curve), TLS
  1.3 only, no client certificates.
- **Logged** to stderr: each connection by its token's name, and each `engine/…` method it calls. `tools/call`s are
  logged by name only, never their arguments.

**`--max-sessions N`** serves N sessions in all, then exits once the last has closed. It's a lifetime count, not a
limit on how many at once. A session is a connection that's made a request: a probe that connects and leaves, or a
`GET /_ping`, isn't one. When the Nth begins, the server stops accepting. A connection that was already open and only
then makes its first request gets `503`. This is for hosts that should serve one session and go, as a sandbox host on
Cloud Run does.

## The Engine API

Carbon's own nouns: `machines.*` (a machine is a VM or a process sandbox, never a container), `files.*` and
`images.*`. Docker's field names are used where they mean what Docker's do. Over JSON-RPC a method is `engine/<name>`,
and its params are an object; REST has the same methods at the paths in [`openapi.yaml`](openapi.yaml), where the
path's and query's parts join the JSON body as params.

| Method | Params | Answer |
| --- | --- | --- |
| `system.version`, `system.ping` | | `{Version, ApiVersion, Os, Arch}`; `"OK"` |
| `machines.create` | `Image`, `Name`, `Hostname`, `Platform`, `Env` (`NAME=VALUE`s), `Cmd`, `WorkingDir`, `User`, `Labels` (`{key: value}`), `HostConfig` (`ExtraHosts`: `NAME:IP`s; `NetworkMode`: `nat`/`bridge`, `none`, `host`; `Memory` in bytes; `Tmpfs`: `{target: ""}`; `Binds`: `SOURCE:TARGET[:ro]`; `Mounts`: `{Type, Source, Target, ReadOnly}`), `Rules`: `{Deny, Allow, DenyExec, DenyRegistry}`, and `Registries`: `{Services, Upstream, Upstreams: {service: URL}, Token, Meta: {key: value}}` (its package managers answered by Carbon, each request sent to the server as a checked call with `Token`, which never goes in the machine; see [registries](../registries/README.md)) | `{Id}` |
| `machines.start`, `.stop` (`Timeout` s, 10 by default, then killed; answered once stopped), `.kill`, `.wait` (`Timeout`), `.remove` (`Force`) | `Id` | `{}`; `.wait`: `{StatusCode, Ended}` |
| `machines.inspect`, `.list` (`Label`: `KEY` or `KEY=VALUE`, one or a list, all matching), `.logs` (`Tail`) | `Id` | the machine, with `Config` (`Hostname`, `Env`, `Cmd`, `WorkingDir`, `User`, `Labels`) and `HostConfig.ExtraHosts`; rows `{Id, Name, Image, Created, State, Platform, Labels}`; lines |
| `machines.exec` | `Id`, `Cmd`, `Env`, `WorkingDir`, `User`, `Timeout`, `Stream` | without `Stream`: `{ExitCode, Stdout, Stderr, TimedOut}`. With one, on a WebSocket: stdin from the client's `stdin` frames until `end`, output as `stdout` and `stderr` frames, then an `exit` frame, and `{ExitCode}` |
| `machines.commit` | `Id`, `Repo`, `Tag`, `Comment`, `Changes` (Dockerfile instructions) | `{Id}`: the image, running or stopped |
| `files.stat`, `.list`, `.remove` (`Recursive`) | `Id`, `Path` | `{Kind, Size, Mode, Uid, Gid, Modified, Link}`; entries with `Name` |
| `files.read` | `Id`, `Path`, `Stream` | `data` frames then `end`, and `{Size}`; or `{Data}` in base64 |
| `files.write` | `Id`, `Path`, `Mode`, `Owner`, `Parents`, `Stream` or `Data` (base64) | the bytes from `data` frames until `end`; `{Size}` |
| `images.pull`, `.push` (`Squash`, `To`), `.tag` (`Source`, `Repo`, `Tag`), `.remove`, `.inspect`, `.list` | `Name` | `{Id, Platform}`; `{…}`; `{}`; rows `{Id, RepoTags, Platform, Created, Source}`, with `Config` and `History` from `inspect` |

- **Files on a running machine are what its programs see:** a file written into a tmpfs is in the tmpfs. Secrets go in
  one (`HostConfig.Tmpfs: {"/run/secrets": ""}`) because a tmpfs isn't the disk: `commit` never saves it.
- **`WorkingDir`** is made, root's, if the image hasn't it, as Docker makes it.
- **`Cmd`:** as in Docker, a machine runs its image's command, and `bash` with no stdin ends at once. A machine meant
  for `exec` is given one that waits, `["sleep", "infinity"]` (MCP's `start_machine` does this itself).
- **A stream's frames can follow its request at once.** The stream opens as the request arrives, before its handler
  runs, and the stream's id is the client's to choose, unique among its open streams.
- **Rules** given to `carbon mcp` (and cloud metadata refused) apply to every machine, merged with a request's own
  `Rules`.
- **Registries** given to `carbon mcp` (`--registries`, `--upstream`, `--upstream-token-file`, `--registry-meta`) go to
  every machine it starts, unless `machines.create` gives its own `Registries`. No tool takes them, so an agent can't
  point its machine's package managers anywhere else.
- **Errors:** JSON-RPC codes `-32602` invalid, `-32004` not found, `-32009` conflict, `-32001` forbidden, `-32005`
  unsupported, `-32000` failed, with `data.kind`. REST uses 400, 404, 409, 403, 501 and 500, with
  `{"message", "kind"}`.

## Inside

| File | What it is |
| --- | --- |
| [`lib.rs`](src/lib.rs) | `Server`, `Options`, `Peer` (a client: its outbox, scopes and streams), stdio, `dispatch` (a thread per request) |
| [`protocol.rs`](src/protocol.rs) | JSON-RPC and both eras: `initialize`, `server/discover`, versions, `tools/*`, `resources/*`, and `engine/…` behind its scope |
| [`engine/`](src/engine/mod.rs) | The Engine API's methods, each a library call, and [`rest.rs`](src/engine/rest.rs): its paths |
| [`net/`](src/net/mod.rs) | `-H`: listeners, who may connect, Streamable HTTP, WebSocket, `--max-sessions`; [`tokens.rs`](src/net/tokens.rs), [`tls.rs`](src/net/tls.rs) |
| [`shape.rs`](src/shape.rs) | Results for a model (output's head and tail), arguments, host paths, base64 |
| [`resources.rs`](src/resources.rs) | `carbon://` resources |
| [`tools/`](src/tools/mod.rs) | Each tool: its schema, its annotations, and its calls into the library |

The network side is axum, hyper and tokio-rustls; the library's calls, which block, run on tokio's blocking threads.

## Seen working

On macOS, through a stdio client speaking both eras: the handshake at 2025-11-25 and 2025-06-18, `server/discover` at
2026-07-28, an unknown version refused (-32022). `run` in `alpine:3.20` (an exit code of 3, stdout and stderr apart,
in 0.4 s, the machine gone after) and in `atlas` (`ver`); cloud metadata refused. A `python:3.13-slim` machine started,
a file written, run with its bug, fixed with `edit_file`, run again; a line range read, a binary read as base64;
stdin to `exec`; a background process read by cursor, written to, and killed (137); a timeout (137 after 2 s);
1.2 MB of output clipped to its head and tail and kept whole in the machine; events; stopped, its file read while
stopped, started again with it. A `--deny-exec` rule refusing `python3 -c` (126). Copies to and from a shared host
folder, and `..`, an outside path and an outside bind refused. An image built from a Dockerfile an agent wrote in a
machine, and one from files in the call, each with progress, then run; saved, removed, loaded; tagged; a stopped
machine committed and run; a volume kept between two runs; a read-only bind. `--machine`: 19 tools, no `machine`
argument, another machine refused; a screenshot (18 KB), the clipboard, a click; resources. `--tools`.

On the network ([`tests/net.rs`](tests/net.rs), on loopback): MCP over HTTP (JSON and server-sent events) and over a
WebSocket; `engine/…` absent without `--engine`; an `Origin` refused; tokens: an unknown one 401, an MCP-only one
reaching tools but refused the Engine API (-32001, 403), an engine one reaching both over REST and WebSocket;
`--max-sessions 2` ignoring a TCP probe and a health check, then exiting when its two sessions closed. With the
binary: TLS 1.3 (1.2 refused), a bad token not counted as a session, exiting after `--max-sessions 1`; a Unix socket
at `0600`, removed at exit; startup refused beyond loopback without a token file, and for a scheme in `-H`.

A gateway's session against real machines ([`tests/session.rs`](tests/session.rs), `--ignored`; it runs the signed
binary, `$CARBON` or `target/release/carbon`) on one WebSocket: `debian:latest` created with a hostname, env, `Cmd`,
a tmpfs and rules, started; a token written into the tmpfs as frames right behind the request (`0600`); `exec` with
stdout, stderr and an exit frame (3), stdin from frames, and without a stream; the file read back as frames; a
connection out refused by the rules; a file written, committed while running, stopped (and waited for) and removed; a
machine from the image has the file and not the token; the image removed; the server exiting after its one session.
The same with REST over a Unix socket with curl.

## To do

- [ ] The rest of the Engine API: `machines.events` and `watch` as streams, `images.build`, `.save`, `.load`,
      `.history`, `volumes.*`, `display.*`, `rules` on a running machine.
- [ ] `files.move`, and copies in and out (whole folders, as `carbon cp`), in the Engine API.
- [ ] REST streams: exec, logs and events upgrading to a WebSocket at their own paths.
- [ ] API versions: `/v1` and `engine/` now; how a client asks for a version over a WebSocket.
- [ ] `carbon start NAME` in the CLI, to match `machines.start` (a stopped machine starts with `run --name` today).
- [ ] OAuth 2.1 for MCP's clients; `subscriptions/listen`.
- [ ] The Tasks extension, for builds and commands longer than a client waits.
- [ ] Machines labeled with who started them, for one server shared by several users.
- [ ] `notifications/cancelled` ending the call it names (a call runs to its end, or its timeout, today).
- [ ] Windows hosts: a running machine's control socket, which `--engine`'s files and commit need.
