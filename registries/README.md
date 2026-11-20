# registries

A machine's package managers, answered in Carbon's own process. Each of their requests goes upstream as one checked
call, never as itself.

```sh
carbon run --deny 0.0.0.0/0 --deny ::/0 --registries apt --upstream https://pkg.example.com --upstream-token-file tok \
  debian:13 -- apt-get install -y jq
carbon mcp --registries apt --upstream https://pkg.example.com --upstream-token-file tok
```

```
 machine                     Carbon (host process)                                   package server
 apt-get ── HTTP to ──►  net: the gateway's port 80 ──► registries-core              POST /v1/apt/deb
            registry.carbon                             ├ adapter (apt): path → call   {"id","params",…}
                                                        ├ schema check                ──────────────────►
                                                        ├ hooks and rules (Registry)   bytes, JSON, or
                                                        └ upstream: the call, token    {"error": …}
```

**The rule: no request leaves Carbon.** The tool's request is parsed and then dropped. What's sent is built from a
fixed shape and checked fields, so there's no method, path, query, header or body of the machine's to pass along. A
request that doesn't parse into a valid call is refused here, at once, and the server never hears of it. The upstream
token is in the host process, never in the machine.

**The protocol a server answers** is small. `GET /v1/` says what it serves (`{"services": {"apt": ["release",
"index", "deb"], …}}`), and Carbon asks before a machine starts, refusing to start it if a service is missing. Each
call is `POST /v1/{service}/{op}` with a JSON body, `{"id", "params", "expect", "offset", "meta"}`, and the token as a
bearer. An answer is JSON, bytes (with `Carbon-Size`, and `Carbon-Digest` when it's known), or a refusal,
`{"error": {"code", "message"}}`, whose code is `invalid`, `unauthorized`, `forbidden`, `not_found`, `too_large` or
`unavailable`. Each adapter turns a refusal into its tool's own failure, with the message, so the tool says why.

## Crates

| Crate | What it is |
| --- | --- |
| [`registries-core`](core/src/lib.rs) | `Registries` (what a machine gets: services, servers, token, meta), `Call` and op schemas, `Adapter` and `Conversation`, the HTTP/1.1 a tool is answered with, the upstream client, and `Server`, which ties them together |
| [`registries-apt`](apt/src/lib.rs) | apt's paths as `release`, `index` and `deb` calls, and the answers back as apt reads them |
| [`registries-pypi`](pypi/src/lib.rs) | PyPI's simple index (and so pip's and uv's) as `project` and `file` calls; each file's URL pointed back at the registry with its SHA-256 in it, and each file checked against it as it streams |
| [`registries-crates`](crates/src/lib.rs) | Cargo's sparse index as `index` and `crate` calls; `config.json` answered here, its downloads pointed back at the registry with each one's SHA-256 in its URL; `/.cargo/config.toml` in a tmpfs, replacing crates.io's source |
| [`registries-go`](go/src/lib.rs) | Go's module proxy protocol as `list`, `latest`, `info`, `mod` and `zip` calls, and its checksum database as `lookup`, `tile` and `head`; `GOPROXY` pointing here, with no `direct`, and `GOSUMDB` kept at `sum.golang.org`, so go checks every module against its signed log itself |
| [`registries-git`](git/src/lib.rs) | git's smart HTTP (v0/v1) as `refs`, `fetch` and `push` calls: every pkt-line read and checked, the ref advertisement written by Carbon, gzip'd and chunked bodies, `insteadOf` for each host the server names |
| [`registries-npm`](npm/src/lib.rs) | npm's registry protocol (and so yarn's, pnpm's and bun's) as `packument` and `tarball` calls; each packument's tarball URLs pointed back at the registry, and each tarball checked against its `integrity` as it streams |

`carbon` joins them to a machine ([`carbon/src/registries.rs`](../carbon/src/registries.rs)). With `--registries`,
its network's DNS answers `registry.carbon` with the gateway's address, and a connection to the gateway's port 80 is
an in-memory pipe whose other end a `Server` thread serves. Each adapter is a Cargo feature of `carbon` (`apt`, `npm`,
`pypi`, `crates`, `go` and `git`, all on by default), and says what its tool needs in the machine: environment (over the
image's, under `-e`), and files where no variable will do (cargo's source replacement), each in a read-only tmpfs at its
folder. A run gets them; the machine's record and its committed images never do.

```
carbon ──► registries-core ──► hooks
   │  └──► registries-apt ──► registries-core
   └─────► net   (Host::gateway: the gateway's port 80; Config::gateway_names: registry.carbon)
```

## What's built

- **apt**, end to end: `apt-get update` and `apt-get install` in `debian:13` under `--deny 0.0.0.0/0`, through
  a package server answering the protocol above.

  | apt asks for | The call |
  | --- | --- |
  | `/{archive}/dists/{suite}/InRelease` | `release {archive, suite}` |
  | `…/{component}/binary-{arch}/by-hash/SHA256/{hash}` | `index {archive, suite, component, arch, sha256}` |
  | `…/{component}/binary-{arch}/Packages{,.xz,.gz}` | `index {archive, suite, component, arch, file}` |
  | `/{archive}/pool/{component}/{prefix}/{source}/{name}_{version}_{arch}.deb` | `deb {archive, component, source, name, version, arch}` |

  Anything else apt asks for is a 404 here, which apt reads as "not there". A refusal's reason is in the status line,
  so apt prints why: `403  Forbidden - this machine's rules refuse it`.
- **npm**, end to end: `npm install` of packages and their dependencies in `node:22-slim` under `--deny 0.0.0.0/0`.

  | npm asks for | The call |
  | --- | --- |
  | `/{name}`, `/@scope%2fname` | `packument {name}` |
  | `/{name}/-/{base}-{version}.tgz` | `tarball {name, version}` |

  The machine gets `NPM_CONFIG_REGISTRY=http://registry.carbon/npm/` (and Yarn's `YARN_NPM_REGISTRY_SERVER`), with
  audit, fund and the update notifier off. Each packument's `dist.tarball`s are pointed back at the registry, and the
  `integrity` of each version it offered is kept for the machine: that version's tarball call carries it as
  `expect.sha512`, and Carbon checks the tarball as it streams, so one that doesn't match is cut short. A refusal is
  npm's own: a status and `{"error": why}`, which npm prints.
- **PyPI**, end to end: `pip install` of a package and its dependencies in `python:3.13-slim` under `--deny
  0.0.0.0/0`.

  | pip asks for | The call |
  | --- | --- |
  | `/simple/{project}/` | `project {name}`, normalized as PEP 503 has it |
  | `/files/{project}/{sha256}/{filename}` | `file {name, filename, sha256}` |

  The machine gets `PIP_INDEX_URL=http://registry.carbon/pypi/simple/`, `PIP_TRUSTED_HOST` (it's plain HTTP, to Carbon)
  and uv's `UV_DEFAULT_INDEX`, with pip's version check off. A page is PEP 691's JSON, or PEP 503's HTML for a tool
  that doesn't ask for JSON. Each file's URL carries its SHA-256, so the adapter keeps nothing, and every file's call
  expects its hash.
- **crates.io**, end to end: `cargo run` of a project depending on `itoa` and `serde` with `derive` (so `syn`,
  `quote`, `proc-macro2` and the rest) in `rust:1-slim` under `--deny 0.0.0.0/0`.

  | cargo asks for | The call |
  | --- | --- |
  | `/config.json` | None: answered here, `dl` pointing at `/crates/dl/{crate}/{version}/{sha256-checksum}` |
  | `/1/{n}`, `/2/{n}`, `/3/{c}/{n}`, `/{ab}/{cd}/{n}` | `index {name}` |
  | `/dl/{crate}/{version}/{sha256}` | `crate {name, version, sha256}` |

  No environment variable can point cargo at another index, so the machine gets `/.cargo/config.toml`, which cargo
  reads for every project from the filesystem's root, replacing crates.io's source with `sparse+http://registry.carbon/crates/`.
  It's in a tmpfs, never on the disk.
- **Go modules**, end to end: `go mod tidy` and `go run` of a project using `golang.org/x/text` (not on GitHub) and
  `github.com/google/uuid` in `golang:1` under `--deny 0.0.0.0/0`, go checking both against sum.golang.org's log.

  | go asks for | The call |
  | --- | --- |
  | `/{module}/@v/list`, `/{module}/@latest` | `list {module}`, `latest {module}` |
  | `/{module}/@v/{version}.info`, `.mod`, `.zip` | `info`, `mod`, `zip {module, version}` |
  | `/sumdb/sum.golang.org/supported` | None: answered here, so go asks this registry for the database too |
  | `/sumdb/sum.golang.org/lookup/…`, `/tile/…`, `/latest` | `lookup {module, version}`, `tile {tile}`, `head` |

  Paths and versions come escaped (`!b` for `B`) and are unescaped into the call, then checked as Go checks module
  paths and canonical versions. The machine gets `GOPROXY=http://registry.carbon/go`, with no `direct` (so go never
  clones a repository itself), `GOSUMDB=sum.golang.org`, and `GONOSUMDB`, `GOPRIVATE`, `GONOPROXY` and `GOINSECURE`
  emptied. go checks the database's signed log with the key it was built with, so a module the log doesn't have
  can't be handed to it. A zip has no hash Carbon could check as it streams (its `h1:` is over the files in it), so
  the server checks it whole before sending it, and go checks it again.
- **git**, end to end: clone (empty, full and shallow), push, racing pushes, and a public GitHub repo, in `debian:13`
  under `--deny 0.0.0.0/0`, through a package server.

  | git asks for | The call |
  | --- | --- |
  | `GET /{host}/{owner}/{repo}/info/refs?service=git-upload-pack` (or `-receive-pack`) | `refs {host, repo, service}` |
  | `POST …/git-upload-pack` | `fetch {host, repo, wants, haves, shallows, depth, done, negotiating, caps}` |
  | `POST …/git-receive-pack` | `push {host, repo, updates, caps}`, the pack after the JSON as data |

  The server's discovery names its hosts (`{"git": {"hosts": {"github.com": "https://github.com/", …}}}`), and the
  machine gets `insteadOf` for each, so `git clone https://github.com/…` works unchanged. Carbon writes the ref
  advertisement itself, offering only what it parses back; a client's other capabilities are dropped. A push's pack is
  checked for shape (`PACK`, version, count) before it's sent, and the server indexes and checks every object. A fetch's
  and a push's answers are git's own result bytes, from the server. A refusal is git's `ERR` line: `remote error: …`.
- **Checked streams:** any byte answer whose call has an `expect` (a hash, a size) is checked as it passes, its last
  read held back until the whole matches.
- **Discovery when a machine starts:** each server's `GET /v1/` must list every op of every service asked for, or
  the machine doesn't start, and says which.
- **Hooks and rules:** each call is a `hooks::Registry` event, decided before it's sent. `--deny-registry 'apt:deb
  name=nmap*'` is a rule; `MachineSpec::on_registry` a hook; `carbon watch --registry` shows them.
- **The Engine API:** `machines.create`'s `Registries` (`Services`, `Upstream`, `Upstreams`, `Token`, `Meta`), and
  `Rules.DenyRegistry`.

## Testing

```sh
cargo test -p registries-core -p registries-apt -p registries-npm -p registries-pypi -p registries-crates -p registries-go -p registries-git
cargo test -p carbon --lib registries   # the pipe between the machine and a server thread
cargo test -p net gateway               # the gateway's port and names
```

End to end, each tool is run in a machine with no network out against a real package server answering the
protocol above, as the sections under "What's built" say.

## To do

1. git's protocol v2 offered (clients fall back to v0 today), and LFS.
2. apt with no changed sources: answering `deb.debian.org` and `security.debian.org` themselves (open question 2 in
   the proposal).
3. PyPI's metadata files (PEP 658), so pip resolves without downloading every wheel it considers.
4. Resumed downloads (`Range` as `offset`): today a `Range` gets the whole file, which apt takes in its place.
5. Big pushes streamed upstream, not held in memory first (up to 512 MiB today).
