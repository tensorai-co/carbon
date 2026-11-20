//! `registries-git`: git's smart HTTP (protocol v0/v1) as registries' calls, for the machine's own `git`.
//!
//! | git asks for | The call |
//! | --- | --- |
//! | `GET /{host}/{owner}/{repo}[.git]/info/refs?service=git-upload-pack` | `refs {host, repo, service: upload-pack}` |
//! | `GET …/info/refs?service=git-receive-pack` | `refs {host, repo, service: receive-pack}` |
//! | `POST …/git-upload-pack` | `fetch {host, repo, wants, haves, shallows, depth, done, negotiating, caps}` |
//! | `POST …/git-receive-pack` | `push {host, repo, updates, caps}`, and the pack as data |
//!
//! Every pkt-line git sends is read here and checked: object ids, ref names, `deepen`, and capabilities, of which only
//! the ones this adapter offers are passed on. The ref advertisement is written here, from `refs`' JSON, so what git
//! is offered is Carbon's choice. A fetch's and a push's answers are git's own result bytes, made by the server, and
//! passed on as they come. A refusal is git's `ERR` line, which git prints as `remote error: …`.
//!
//! The machine gets `insteadOf` for each host the server names in its discovery, so `git clone https://github.com/…`
//! works unchanged.

use std::io::Read;
use std::sync::Arc;

use registries_core::{Adapter, Answer, Call, Conversation, Field, Native, Op, Request, decode_segment};
use serde_json::Value;

const SERVICE: &str = "git";
/// The most a request may be: a push's pack.
const MOST_BODY: u64 = 512 << 20;

/// What a fetch may ask for, as the advertisement offers it.
const UPLOAD_CAPS: &[&str] = &["side-band-64k", "side-band", "ofs-delta", "shallow", "no-progress", "include-tag", "allow-tip-sha1-in-want", "allow-reachable-sha1-in-want"];
/// What a push may ask for.
const RECEIVE_CAPS: &[&str] = &["report-status", "delete-refs", "ofs-delta"];

fn host(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && !s.starts_with(['.', '-']) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

fn part(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.starts_with('.') && !s.ends_with(".lock") && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// `owner/name`.
fn repo(s: &str) -> bool {
    s.split_once('/').is_some_and(|(o, n)| part(o) && part(n))
}

/// An object id: SHA-1's 40 hex digits, or SHA-256's 64.
pub fn oid(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn zero(s: &str) -> bool {
    s.bytes().all(|b| b == b'0')
}

/// A ref git itself would take (`git check-ref-format`), under `refs/heads/` or `refs/tags/`.
pub fn ref_name(name: &str) -> bool {
    if !(name.starts_with("refs/heads/") || name.starts_with("refs/tags/")) || name.len() > 255 {
        return false;
    }
    if name.ends_with('/') || name.ends_with('.') || name.contains("..") || name.contains("//") || name.contains("@{") {
        return false;
    }
    if name.chars().any(|c| c.is_control() || " ~^:?*[\\".contains(c)) {
        return false;
    }
    name.split('/').all(|p| !p.is_empty() && !p.starts_with('.') && !p.ends_with(".lock")) && name != "refs/heads/" && name != "refs/tags/"
}

/// A push's update: `old:new:ref`.
fn update(s: &str) -> bool {
    let mut parts = s.splitn(3, ':');
    matches!((parts.next(), parts.next(), parts.next()), (Some(old), Some(new), Some(r)) if oid(old) && oid(new) && ref_name(r))
}

fn number(s: &str) -> bool {
    !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0')
}

fn boolean(s: &str) -> bool {
    s == "true" || s == "false"
}

fn service(s: &str) -> bool {
    s == "upload-pack" || s == "receive-pack"
}

fn upload_cap(s: &str) -> bool {
    UPLOAD_CAPS.contains(&s)
}

fn receive_cap(s: &str) -> bool {
    RECEIVE_CAPS.contains(&s)
}

static OPS: &[Op] = &[
    Op { name: "refs", fields: &[Field::required("host", host), Field::required("repo", repo), Field::required("service", service)] },
    Op {
        name: "fetch",
        fields: &[
            Field::required("host", host),
            Field::required("repo", repo),
            Field::list("wants", true, 10_000, oid),
            Field::list("haves", false, 100_000, oid),
            Field::list("shallows", false, 100_000, oid),
            Field::optional("depth", number),
            Field::required("done", boolean),
            Field::required("negotiating", boolean),
            Field::list("caps", false, 16, upload_cap),
        ],
    },
    Op {
        name: "push",
        fields: &[Field::required("host", host), Field::required("repo", repo), Field::list("updates", true, 1_000, update), Field::list("caps", false, 16, receive_cap)],
    },
];

/// git's adapter. It keeps nothing: each request of git's stateless protocol says everything again.
pub struct Git;

impl Git {
    pub fn adapter() -> Arc<dyn Adapter> {
        Arc::new(Git)
    }
}

impl Adapter for Git {
    fn service(&self) -> &'static str {
        SERVICE
    }
    fn ops(&self) -> &'static [Op] {
        OPS
    }
    fn conversation(&self) -> Box<dyn Conversation> {
        Box::new(GitConversation)
    }
    fn most_body(&self) -> u64 {
        MOST_BODY
    }
    /// `insteadOf` for each host the server names (`{"hosts": {"github.com": "https://github.com/"}}`), and no
    /// prompting: there's nothing to prompt for.
    fn env(&self, base: &str, details: &Value) -> Vec<(String, String)> {
        let mut env = vec![("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())];
        let hosts: Vec<(&String, &str)> = details.get("hosts").and_then(Value::as_object).map(|m| m.iter().filter_map(|(h, u)| Some((h, u.as_str()?))).collect()).unwrap_or_default();
        let hosts: Vec<_> = hosts.into_iter().filter(|(h, u)| host(h) && (u.starts_with("https://") || u.starts_with("http://")) && !u.contains(char::is_whitespace)).collect();
        env.push(("GIT_CONFIG_COUNT".into(), hosts.len().to_string()));
        for (i, (h, url)) in hosts.into_iter().enumerate() {
            env.push((format!("GIT_CONFIG_KEY_{i}"), format!("url.{base}{h}/.insteadOf")));
            env.push((format!("GIT_CONFIG_VALUE_{i}"), url.to_string()));
        }
        env
    }
}

struct GitConversation;

/// One pkt-line, or a flush.
#[derive(Debug, PartialEq, Eq)]
enum Pkt<'a> {
    Data(&'a [u8]),
    Flush,
}

/// Reads pkt-lines from `body`, from `at`; `None` at its end.
fn next_pkt<'a>(body: &'a [u8], at: &mut usize) -> Result<Option<Pkt<'a>>, String> {
    if *at == body.len() {
        return Ok(None);
    }
    let head = body.get(*at..*at + 4).ok_or("a pkt-line cut short")?;
    let len = std::str::from_utf8(head).ok().and_then(|h| usize::from_str_radix(h, 16).ok()).ok_or("a bad pkt-line length")?;
    *at += 4;
    match len {
        0 => Ok(Some(Pkt::Flush)),
        1..=3 => Err("a pkt-line git doesn't send".into()),
        n => {
            let data = body.get(*at..*at + n - 4).ok_or("a pkt-line cut short")?;
            *at += n - 4;
            Ok(Some(Pkt::Data(data)))
        }
    }
}

fn text(data: &[u8]) -> Result<&str, String> {
    let s = std::str::from_utf8(data).map_err(|_| "a pkt-line that isn't text".to_string())?;
    Ok(s.strip_suffix('\n').unwrap_or(s))
}

/// A fetch request's fields: what upload-pack is asked for in one stateless round.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Fetch {
    pub wants: Vec<String>,
    pub haves: Vec<String>,
    pub shallows: Vec<String>,
    pub depth: Option<String>,
    pub done: bool,
    pub negotiating: bool,
    pub caps: Vec<String>,
}

/// Reads an upload-pack request: wants (the first with capabilities), shallows and `deepen`, a flush, then haves in
/// rounds ended by flushes, and maybe `done`. Capabilities not offered are dropped.
pub fn parse_fetch(body: &[u8]) -> Result<Fetch, String> {
    let mut f = Fetch::default();
    let (mut at, mut in_haves) = (0, false);
    while let Some(pkt) = next_pkt(body, &mut at)? {
        let data = match pkt {
            Pkt::Flush if !in_haves => {
                in_haves = true;
                continue;
            }
            Pkt::Flush => {
                f.negotiating = true;
                continue;
            }
            Pkt::Data(d) => d,
        };
        let line = text(data)?;
        let (key, val) = line.split_once(' ').unwrap_or((line, ""));
        match (key, in_haves) {
            ("want", false) => {
                let (id, caps) = val.split_once(' ').unwrap_or((val, ""));
                if !oid(id) {
                    return Err(format!("a bad want {id:?}"));
                }
                if f.wants.is_empty() {
                    f.caps = caps.split(' ').filter(|c| upload_cap(c)).map(String::from).collect();
                }
                f.wants.push(id.into());
            }
            ("shallow", false) if oid(val) => f.shallows.push(val.into()),
            ("deepen", false) if number(val) => f.depth = Some(val.into()),
            ("have", true) if oid(val) => f.haves.push(val.into()),
            ("done", true) | ("done", false) => {
                f.done = true;
                f.negotiating = true;
                return Ok(f);
            }
            _ => return Err(format!("a line upload-pack isn't asked here: {:?}", line.chars().take(60).collect::<String>())),
        }
    }
    Ok(f)
}

/// A push request's fields, and where its pack starts.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Push {
    pub updates: Vec<String>,
    pub caps: Vec<String>,
    pub pack_at: usize,
}

/// Reads a receive-pack request's commands to their flush (the first with capabilities); the pack follows.
pub fn parse_push(body: &[u8]) -> Result<Push, String> {
    let mut p = Push::default();
    let mut at = 0;
    loop {
        let data = match next_pkt(body, &mut at)? {
            Some(Pkt::Flush) => break,
            Some(Pkt::Data(d)) => d,
            None => return Err("a push without its flush".into()),
        };
        let mut line = text(data)?;
        if p.updates.is_empty()
            && let Some((cmd, caps)) = line.split_once('\0')
        {
            line = cmd;
            p.caps = caps.split(' ').filter(|c| receive_cap(c)).map(String::from).collect();
        }
        // A shallow clone's boundary: its objects are checked like any others.
        if line.strip_prefix("shallow ").is_some_and(oid) {
            continue;
        }
        let mut parts = line.splitn(3, ' ');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(old), Some(new), Some(r)) if oid(old) && oid(new) && ref_name(r) => p.updates.push(format!("{old}:{new}:{r}")),
            _ => return Err(format!("a bad push command {:?}", line.chars().take(80).collect::<String>())),
        }
    }
    p.pack_at = at;
    let pack = &body[at..];
    let needs_pack = p.updates.iter().any(|u| !zero(u.split(':').nth(1).unwrap_or("0")));
    match (needs_pack, pack.is_empty()) {
        (false, true) => {}
        (true, true) => return Err("a push that creates or moves a ref, with no pack".into()),
        (_, false) => {
            let version = pack.get(4..8).map(|v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]));
            if pack.len() < 32 || &pack[..4] != b"PACK" || !matches!(version, Some(2 | 3)) {
                return Err("what follows the commands isn't a git pack".into());
            }
        }
    }
    Ok(p)
}

/// One pkt-line of `text`.
fn pkt(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(format!("{:04x}", text.len() + 4).as_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// The answer to `info/refs`: the service line, then each ref, the first with the capabilities. For upload-pack, HEAD
/// first when there is one.
pub fn advertise(service: &str, refs: &Value) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    pkt(&mut out, &format!("# service=git-{service}\n"));
    out.extend_from_slice(b"0000");
    let mut caps: Vec<String> = if service == "upload-pack" { UPLOAD_CAPS.iter().map(|s| s.to_string()).collect() } else { RECEIVE_CAPS.iter().map(|s| s.to_string()).collect() };
    let mut lines: Vec<(String, String)> = Vec::new();
    let list = refs.get("refs").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut all: Vec<(String, String, Option<String>)> = Vec::new();
    for r in &list {
        let (Some(name), Some(id)) = (r.get("name").and_then(Value::as_str), r.get("oid").and_then(Value::as_str)) else { continue };
        if !ref_name(name) || !oid(id) {
            return Err(format!("the server answered with a bad ref {name:?}"));
        }
        let peeled = r.get("peeled").and_then(Value::as_str).filter(|p| oid(p)).map(String::from);
        all.push((name.into(), id.into(), peeled));
    }
    all.sort();
    if service == "upload-pack"
        && let Some(head) = refs.get("head").and_then(Value::as_str).filter(|h| ref_name(h))
        && let Some((_, id, _)) = all.iter().find(|(n, _, _)| n == head)
    {
        lines.push((id.clone(), "HEAD".into()));
        caps.push(format!("symref=HEAD:{head}"));
    }
    caps.push(concat!("agent=carbon/", env!("CARGO_PKG_VERSION")).into());
    for (name, id, peeled) in &all {
        lines.push((id.clone(), name.clone()));
        if let Some(p) = peeled {
            lines.push((p.clone(), format!("{name}^{{}}")));
        }
    }
    let caps = caps.join(" ");
    if lines.is_empty() {
        pkt(&mut out, &format!("{} capabilities^{{}}\0{caps}\n", "0".repeat(40)));
    }
    for (i, (id, name)) in lines.iter().enumerate() {
        if i == 0 {
            pkt(&mut out, &format!("{id} {name}\0{caps}\n"));
        } else {
            pkt(&mut out, &format!("{id} {name}\n"));
        }
    }
    out.extend_from_slice(b"0000");
    Ok(out)
}

/// git's refusal: an `ERR` line in the answer git expects, after the service line for `info/refs`.
fn err(op: &str, service: &str, why: &str) -> Native {
    let why: String = why.chars().map(|c| if c.is_control() { ' ' } else { c }).take(400).collect();
    let mut body = Vec::new();
    let kind = if op == "refs" {
        pkt(&mut body, &format!("# service=git-{service}\n"));
        body.extend_from_slice(b"0000");
        format!("application/x-git-{service}-advertisement")
    } else {
        format!("application/x-git-{service}-result")
    };
    pkt(&mut body, &format!("ERR {why}\n"));
    Native::new(200, "OK").header("Content-Type", kind).header("Cache-Control", "no-cache").bytes(body)
}

/// The host, `owner/name` and what's asked (`info/refs`, `git-upload-pack`, `git-receive-pack`) of a path under `/git`.
fn split_path(path: &str) -> Option<(String, String, String)> {
    let segments: Vec<String> = path.strip_prefix('/')?.split('/').map(decode_segment).collect::<Option<_>>()?;
    let s: Vec<&str> = segments.iter().map(String::as_str).collect();
    let (h, owner, name, what) = match s.as_slice() {
        [h, owner, name, "info", "refs"] => (*h, *owner, *name, "info/refs"),
        [h, owner, name, what @ ("git-upload-pack" | "git-receive-pack")] => (*h, *owner, *name, *what),
        _ => return None,
    };
    let name = name.strip_suffix(".git").unwrap_or(name);
    (host(h) && part(owner) && part(name)).then(|| (h.to_string(), format!("{owner}/{name}"), what.to_string()))
}

fn query(q: &str, key: &str) -> Option<String> {
    q.split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=').map(String::from))
}

/// A request's body, gunzipped if git sent it so, at most `MOST_BODY`.
fn body(req: &Request) -> Result<Vec<u8>, String> {
    match req.header("Content-Encoding") {
        Some(e) if e.eq_ignore_ascii_case("gzip") => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(&req.body[..]).take(MOST_BODY + 1).read_to_end(&mut out).map_err(|e| format!("a gzip'd body that doesn't inflate: {e}"))?;
            if out.len() as u64 > MOST_BODY {
                return Err("a request body too big".into());
            }
            Ok(out)
        }
        None => Ok(req.body.clone()),
        Some(other) => Err(format!("a body sent as {other}")),
    }
}

/// The call git's request is, or the refusal git gets.
pub fn parse(req: &Request, path: &str) -> Result<Call, Native> {
    let not_here = || Native::plain(404, "Not Found", "not something git is answered with here");
    let (h, r, what) = split_path(path).ok_or_else(not_here)?;
    match (req.method.as_str(), what.as_str()) {
        ("GET", "info/refs") => {
            let service = match query(&req.query, "service").as_deref() {
                Some("git-upload-pack") => "upload-pack",
                Some("git-receive-pack") => "receive-pack",
                // The dumb protocol: not served.
                _ => return Err(Native::plain(403, "Forbidden", "only git's smart HTTP protocol is served")),
            };
            Ok(Call::new(SERVICE, "refs").with("host", h).with("repo", r).with("service", service))
        }
        ("POST", "git-upload-pack") => {
            let f = body(req).and_then(|b| parse_fetch(&b)).map_err(|why| err("fetch", "upload-pack", &why))?;
            let mut call = Call::new(SERVICE, "fetch")
                .with("host", h)
                .with("repo", r)
                .with_list("wants", &f.wants)
                .with_list("haves", &f.haves)
                .with_list("shallows", &f.shallows)
                .with("done", f.done.to_string())
                .with("negotiating", f.negotiating.to_string())
                .with_list("caps", &f.caps);
            if let Some(d) = f.depth {
                call = call.with("depth", d);
            }
            Ok(call)
        }
        ("POST", "git-receive-pack") => {
            let body = body(req).map_err(|why| err("push", "receive-pack", &why))?;
            let p = parse_push(&body).map_err(|why| err("push", "receive-pack", &why))?;
            Ok(Call::new(SERVICE, "push").with("host", h).with("repo", r).with_list("updates", &p.updates).with_list("caps", &p.caps).with_data(body[p.pack_at..].to_vec()))
        }
        _ => Err(Native::plain(405, "Method Not Allowed", "git's smart HTTP is GET info/refs, and POST git-upload-pack or git-receive-pack")),
    }
}

impl Conversation for GitConversation {
    fn parse(&mut self, req: &Request, path: &str) -> Result<Call, Native> {
        parse(req, path)
    }

    fn reply(&mut self, call: &Call, answer: Answer) -> Native {
        let service = match call.op {
            "refs" => call.param("service").unwrap_or("upload-pack"),
            "push" => "receive-pack",
            _ => "upload-pack",
        };
        match (call.op, answer) {
            ("refs", Answer::Json(refs)) => match advertise(service, &refs) {
                Ok(body) => Native::new(200, "OK").header("Content-Type", format!("application/x-git-{service}-advertisement")).header("Cache-Control", "no-cache").bytes(body),
                Err(why) => err("refs", service, &why),
            },
            ("fetch" | "push", Answer::Bytes { reader, size, .. }) => Native::new(200, "OK").header("Content-Type", format!("application/x-git-{service}-result")).header("Cache-Control", "no-cache").stream(reader, size),
            // A repo that isn't there, or isn't the session's: git says "repository not found".
            ("refs", Answer::Refused { code, .. }) if code == "not_found" => Native::plain(404, "Not Found", "repository not found"),
            (op, Answer::Refused { message, .. }) => err(op, service, &message),
            (op, _) => err(op, service, "the package server answered with the wrong kind of answer"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";
    const Z: &str = "0000000000000000000000000000000000000000";

    fn pkts(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for l in lines {
            if *l == "0000" {
                out.extend_from_slice(b"0000");
            } else {
                pkt(&mut out, l);
            }
        }
        out
    }

    fn request(method: &str, query: &str, body: Vec<u8>) -> Request {
        Request { method: method.into(), path: String::new(), query: query.into(), headers: Vec::new(), body, keep_alive: true }
    }

    #[test]
    fn paths_and_refs() {
        let call = parse(&request("GET", "service=git-upload-pack", Vec::new()), "/github.com/serde-rs/serde.git/info/refs").unwrap();
        assert_eq!(call.params, vec![("host", "github.com".into()), ("repo", "serde-rs/serde".into()), ("service", "upload-pack".into())]);
        assert_eq!(call.check(OPS), Ok(()));
        assert_eq!(parse(&request("GET", "", Vec::new()), "/tensorai/o/r/info/refs").unwrap_err().status, 403);
        for path in ["/github.com/a/info/refs", "/github.com/a/b/objects/info/packs", "/GitHub.com/a/b/info/refs", "/github.com/../b/info/refs", "/github.com/a/.b/info/refs"] {
            assert!(parse(&request("GET", "service=git-upload-pack", Vec::new()), path).is_err(), "{path}");
        }
        assert!(ref_name("refs/heads/feature/x") && !ref_name("refs/heads/a..b") && !ref_name("refs/remotes/x") && !ref_name("refs/heads/a b") && !ref_name("refs/heads/x.lock"));
    }

    #[test]
    fn fetch_requests_are_read_whole_and_checked() {
        let body = pkts(&[&format!("want {A} side-band-64k ofs-delta agent=git/2.47 filter thin-pack\n"), &format!("want {B}\n"), &format!("shallow {B}\n"), "deepen 1\n", "0000", &format!("have {A}\n"), "0000", "done\n"]);
        let f = parse_fetch(&body).unwrap();
        assert_eq!((f.wants.len(), f.haves, f.shallows.len(), f.depth.as_deref(), f.done, f.negotiating), (2, vec![A.to_string()], 1, Some("1"), true, true));
        // Only what was offered is passed on.
        assert_eq!(f.caps, ["side-band-64k", "ofs-delta"]);
        let call = parse(&request("POST", "", body), "/tensorai/o/r/git-upload-pack").unwrap();
        assert_eq!(call.check(OPS), Ok(()));
        assert_eq!(call.body(&[])["params"]["wants"], json!([A, B]));
        // A shallow clone's first round: wants and deepen, a flush, and nothing else.
        let first = parse_fetch(&pkts(&[&format!("want {A}\n"), "deepen 1\n", "0000"])).unwrap();
        assert!(!first.negotiating && !first.done);
        for bad in [
            pkts(&["want nothex\n", "0000"]),
            pkts(&[&format!("want {A}\n"), "filter blob:none\n", "0000"]),
            pkts(&[&format!("want {A}\n"), "0000", &format!("want {B}\n")]),
            pkts(&[&format!("want {A}\n"), "deepen 0\n", "0000"]),
            b"zzzz".to_vec(),
            b"0010want".to_vec(),
        ] {
            assert!(parse_fetch(&bad).is_err(), "{:?}", String::from_utf8_lossy(&bad));
        }
    }

    #[test]
    fn gzipped_bodies_inflate() {
        use std::io::Write;
        let plain = pkts(&[&format!("want {A}\n"), "0000", "done\n"]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&plain).unwrap();
        let mut req = request("POST", "", gz.finish().unwrap());
        req.headers.push(("Content-Encoding".into(), "gzip".into()));
        assert_eq!(parse(&req, "/tensorai/o/r/git-upload-pack").unwrap().items("wants"), [A]);
    }

    #[test]
    fn pushes_are_commands_then_a_pack() {
        let mut pack = b"PACK".to_vec();
        pack.extend_from_slice(&2u32.to_be_bytes());
        pack.extend_from_slice(&1u32.to_be_bytes());
        pack.extend_from_slice(&[0u8; 24]);
        let mut body = pkts(&[&format!("{Z} {A} refs/heads/main\0report-status side-band-64k quiet agent=git/2\n"), &format!("{A} {Z} refs/tags/old\n"), "0000"]);
        body.extend_from_slice(&pack);
        let call = parse(&request("POST", "", body), "/tensorai/o/r.git/git-receive-pack").unwrap();
        assert_eq!(call.check(OPS), Ok(()));
        assert_eq!(call.items("updates"), [format!("{Z}:{A}:refs/heads/main"), format!("{A}:{Z}:refs/tags/old")]);
        assert_eq!(call.items("caps"), ["report-status"]);
        assert_eq!(call.data.as_deref(), Some(&pack[..]));
        // Deletes alone need no pack; anything else does, and it must be one.
        assert!(parse_push(&pkts(&[&format!("{A} {Z} refs/heads/x\0report-status\n"), "0000"])).is_ok());
        for bad in [
            pkts(&[&format!("{Z} {A} refs/heads/main\n"), "0000"]),
            [pkts(&[&format!("{Z} {A} refs/heads/main\n"), "0000"]), b"not a pack, not at all, really no".to_vec()].concat(),
            pkts(&[&format!("{Z} {A} refs/remotes/x\n"), "0000"]),
            pkts(&[&format!("{Z} {A} refs/heads/main\n")]),
        ] {
            assert!(parse_push(&bad).is_err());
        }
    }

    #[test]
    fn the_advertisement_is_carbons() {
        let refs = json!({ "head": "refs/heads/main", "refs": [{ "name": "refs/tags/v1", "oid": B, "peeled": A }, { "name": "refs/heads/main", "oid": A }] });
        let ad = String::from_utf8(advertise("upload-pack", &refs).unwrap()).unwrap();
        assert!(ad.starts_with("001e# service=git-upload-pack\n0000"), "{ad}");
        assert!(ad.contains(&format!("{A} HEAD\0side-band-64k ")), "{ad}");
        assert!(ad.contains("symref=HEAD:refs/heads/main agent=carbon/"));
        assert!(ad.contains(&format!("{A} refs/tags/v1^{{}}\n")) && ad.ends_with("0000"));
        let empty = String::from_utf8(advertise("receive-pack", &json!({ "refs": [] })).unwrap()).unwrap();
        assert!(empty.contains(&format!("{Z} capabilities^{{}}\0report-status delete-refs ofs-delta agent=")), "{empty}");
        assert!(advertise("upload-pack", &json!({ "refs": [{ "name": "refs/heads/a b", "oid": A }] })).is_err());
        // A refusal is git's ERR line, after the service line for info/refs.
        let n = err("refs", "upload-pack", "this machine's rules refuse it");
        let registries_core::Body::Bytes(b) = n.body else { panic!() };
        assert!(String::from_utf8(b).unwrap().ends_with("ERR this machine's rules refuse it\n"));
    }

    #[test]
    fn insteadof_for_each_host() {
        let env = Git.env("http://registry.carbon/git/", &json!({ "hosts": { "github.com": "https://github.com/", "bad host": "x" } }));
        assert!(env.contains(&("GIT_CONFIG_COUNT".into(), "1".into())));
        assert!(env.contains(&("GIT_CONFIG_KEY_0".into(), "url.http://registry.carbon/git/github.com/.insteadOf".into())));
        assert!(env.contains(&("GIT_CONFIG_VALUE_0".into(), "https://github.com/".into())));
    }
}
