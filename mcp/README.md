# mcp

`carbon mcp`: Carbon as an MCP server, for agents that speak MCP (Claude Code, Claude Desktop, IDEs, any MCP client).
An agent gets machines to work in, Linux and Windows, as Docker's CLI would give them to it: run a command in a
throwaway machine; start machines that keep running, `exec` into them, read and edit their files, copy in and out;
build images from a Carbonfile, pull, tag, push, save and load them; and drive a machine's screen.

```sh
claude mcp add carbon -- carbon mcp                       # Claude Code: the whole Carbon home
claude mcp add dev -- carbon mcp --machine dev            # one machine, and nothing else
carbon mcp --host-dir ~/work/shared --deny-exec 'git push *' --deny 0.0.0.0/0 --allow pypi.org:443
```

**Built.** stdio, in both of the protocol's eras: 2026-07-28's stateless requests (`server/discover`, each request's
version and capabilities in its `_meta`), and the `initialize` handshake of 2025-11-25 and before, which most clients
speak today. Every tool is a call or two into the library `carbon`; nothing below a tool is this crate's.

## Tools

| | Tools |
| --- | --- |
| **Machines** | `run` (a command in a new machine, removed after, as `docker run --rm`), `start_machine` (`docker run -d`; a Linux machine runs `sleep infinity` unless given a command, so it's there for `exec`), `restart_machine`, `stop_machine`, `remove_machine`, `list_machines`, `inspect_machine`, `machine_logs` |
| **Processes** | `exec` (waited for, with its exit code, stdout and stderr; or `background`), `process_output`, `process_input`, `process_signal` |
| **Files** (running or stopped) | `read_file` (by bytes or lines; pictures as images; binary as base64), `write_file`, `edit_file` (an exact string replaced, keeping the file's mode and owner), `list_files`, `move_file`, `remove_file`, `make_folder`, `copy_to_machine`, `copy_from_machine`, `copy_between_machines` |
| **Images** | `list_images`, `inspect_image`, `pull_image`, `build_image` (its context as files in the call, a folder in a machine, or a shared host folder), `freeze_machine`, `tag_image`, `push_image`, `remove_image`, `save_image`, `load_image` |
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

## Inside

| File | What it is |
| --- | --- |
| [`lib.rs`](src/lib.rs) | `Server`, `Options`, stdio (a thread per request), progress |
| [`protocol.rs`](src/protocol.rs) | JSON-RPC and both eras: `initialize`, `server/discover`, versions, `tools/*`, `resources/*` |
| [`shape.rs`](src/shape.rs) | Results for a model (output's head and tail), arguments, host paths, base64 |
| [`resources.rs`](src/resources.rs) | `carbon://` resources |
| [`tools/`](src/tools/mod.rs) | Each tool: its schema, its annotations, and its calls into the library |

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
machine frozen and run; a volume kept between two runs; a read-only bind. `--machine`: 19 tools, no `machine`
argument, another machine refused; a screenshot (18 KB), the clipboard, a click; resources. `--tools`.

## To do

- [ ] Streamable HTTP (`--http`), with tokens, `Origin` checks and TLS; OAuth 2.1; `subscriptions/listen`.
- [ ] The Tasks extension, for builds and commands longer than a client waits.
- [ ] `freeze_machine` on a running machine, once the library has checkpoints.
- [ ] Machines labeled with who started them, for one HTTP server shared by several users (the library has no
      labels yet).
- [ ] `notifications/cancelled` ending the call it names (a call runs to its end, or its timeout, today).
