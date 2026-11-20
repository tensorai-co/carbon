//! `registries-go`: Go's module proxy protocol (`GOPROXY`) and its checksum database, as registries' calls.
//!
//! | go asks for | The call |
//! | --- | --- |
//! | `/{module}/@v/list` | `list {module}` |
//! | `/{module}/@latest` | `latest {module}` (a module with no tags) |
//! | `/{module}/@v/{version}.info`, `.mod`, `.zip` | `info`, `mod`, `zip {module, version}` |
//! | `/sumdb/sum.golang.org/supported` | None: answered here, so go asks this registry for the database too |
//! | `/sumdb/sum.golang.org/lookup/{module}@{version}` | `lookup {module, version}` |
//! | `/sumdb/sum.golang.org/tile/{tile}` | `tile {tile}` |
//! | `/sumdb/sum.golang.org/latest` | `head` |
//!
//! Paths and versions come escaped, a capital letter as `!` and its lowercase; a call has them as they are
//! (`github.com/BurntSushi/toml`), checked as Go checks module paths and canonical versions. The machine gets
//! `GOPROXY` pointing here (with no `direct`, so go never fetches from a repository itself) and `GOSUMDB` kept at
//! `sum.golang.org`, whose signed log go checks every module against with the key it was built with, through this
//! registry: neither the server nor Carbon could hand it a module the log doesn't have.

use std::sync::Arc;

use registries_core::{Adapter, Answer, Call, Conversation, Field, Native, Op, Request, decode_segment};
use serde_json::Value;

const SERVICE: &str = "go";
/// The checksum database go checks against, by the name it knows its key by.
const SUMDB: &str = "sum.golang.org";

/// A module path as Go has it: elements of letters, digits, `- . _ ~`, none empty or starting or ending with a dot;
/// the first a domain name (lowercase letters, digits, dots and dashes, with a dot, not starting with a dash).
pub fn is_module(p: &str) -> bool {
    if p.is_empty() || p.len() > 300 {
        return false;
    }
    let elems: Vec<&str> = p.split('/').collect();
    let good = elems.iter().enumerate().all(|(i, e)| {
        !e.is_empty()
            && !e.starts_with('.')
            && !e.ends_with('.')
            && e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.' || (i > 0 && (b.is_ascii_uppercase() || b == b'_' || b == b'~')))
    });
    good && elems[0].contains('.') && !elems[0].starts_with('-')
}

/// A canonical Go version: `v` and SemVer 2.0, whose only build metadata is `+incompatible`. Pseudo-versions
/// (`v0.0.0-20191109021931-daa7c04131f5`) are versions too.
pub fn is_version(s: &str) -> bool {
    let number = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && (n == "0" || !n.starts_with('0'));
    let ident = |i: &str| !i.is_empty() && i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && (!i.bytes().all(|b| b.is_ascii_digit()) || number(i));
    let Some(rest) = s.strip_prefix('v').filter(|_| s.len() <= 128) else { return false };
    let rest = rest.strip_suffix("+incompatible").unwrap_or(rest);
    let (core, pre) = rest.split_once('-').map_or((rest, None), |(c, p)| (c, Some(p)));
    let nums: Vec<&str> = core.split('.').collect();
    nums.len() == 3 && nums.iter().all(|n| number(n)) && pre.is_none_or(|p| p.split('.').all(ident))
}

/// A tile of the checksum database, under `tile/`: its height, its level (or `data`), its index in `x123/` steps, and
/// a partial tile's width: `8/0/x001/234.p/5`.
pub fn is_tile(s: &str) -> bool {
    let digits = |p: &str, n: std::ops::RangeInclusive<usize>| n.contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit());
    let parts: Vec<&str> = s.split('/').collect();
    if s.len() > 128 || parts.len() < 3 || !digits(parts[0], 1..=2) || !(parts[1] == "data" || digits(parts[1], 1..=2)) {
        return false;
    }
    let mut rest = &parts[2..];
    if let [.., last, width] = rest
        && let Some(n) = last.strip_suffix(".p")
    {
        if !digits(n, 3..=3) || !digits(width, 1..=3) {
            return false;
        }
        rest = &rest[..rest.len() - 2];
        return rest.iter().all(|p| p.strip_prefix('x').is_some_and(|d| digits(d, 3..=3)));
    }
    let (last, steps) = rest.split_last().unwrap_or((&"", &[]));
    digits(last, 3..=3) && steps.iter().all(|p| p.strip_prefix('x').is_some_and(|d| digits(d, 3..=3)))
}

/// A path or version as go sends it: `!` and a lowercase letter for each capital. None if it isn't escaped that way
/// (a bare capital, a `!` before anything else).
pub fn unescape(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut bang = false;
    for c in s.chars() {
        match (bang, c) {
            (true, 'a'..='z') => {
                out.push(c.to_ascii_uppercase());
                bang = false;
            }
            (true, _) | (false, 'A'..='Z') => return None,
            (false, '!') => bang = true,
            (false, _) => out.push(c),
        }
    }
    (!bang).then_some(out)
}

static MODULE: Field = Field::required("module", is_module);
static VERSION: Field = Field::required("version", is_version);

static OPS: &[Op] = &[
    Op { name: "list", fields: &[MODULE] },
    Op { name: "latest", fields: &[MODULE] },
    Op { name: "info", fields: &[MODULE, VERSION] },
    Op { name: "mod", fields: &[MODULE, VERSION] },
    Op { name: "zip", fields: &[MODULE, VERSION] },
    Op { name: "lookup", fields: &[MODULE, VERSION] },
    Op { name: "tile", fields: &[Field::required("tile", is_tile)] },
    Op { name: "head", fields: &[] },
];

/// go's adapter. It keeps nothing.
pub struct Go;

impl Go {
    pub fn adapter() -> Arc<dyn Adapter> {
        Arc::new(Go)
    }
}

impl Adapter for Go {
    fn service(&self) -> &'static str {
        SERVICE
    }
    fn ops(&self) -> &'static [Op] {
        OPS
    }
    fn conversation(&self) -> Box<dyn Conversation> {
        Box::new(GoConversation)
    }
    /// go's proxy is this registry, and nothing else: no `direct`, so no module comes from a repository. The checksum
    /// database is Go's own, and none of the machine's modules are private to it.
    fn env(&self, base: &str, _details: &Value) -> Vec<(String, String)> {
        let proxy = base.trim_end_matches('/').to_string();
        vec![
            ("GOPROXY".into(), proxy),
            ("GOSUMDB".into(), SUMDB.into()),
            ("GONOSUMDB".into(), String::new()),
            ("GOPRIVATE".into(), String::new()),
            ("GONOPROXY".into(), String::new()),
            ("GOINSECURE".into(), String::new()),
        ]
    }
}

struct GoConversation;

/// What go's `path` (under `/go`) is: a call, or the database's `supported` (answered here).
enum Asked {
    Call(Call),
    Supported,
}

fn parse_path(path: &str) -> Option<Asked> {
    let raw: Vec<&str> = path.strip_prefix('/')?.split('/').collect();
    // Each segment decoded on its own, so a `/` can't come from `%2F`.
    let segments: Vec<String> = raw.iter().map(|s| decode_segment(s)).collect::<Option<_>>()?;
    let s: Vec<&str> = segments.iter().map(String::as_str).collect();
    let call = |op: &'static str| Call::new(SERVICE, op);
    match s.as_slice() {
        ["sumdb", SUMDB, "supported"] => return Some(Asked::Supported),
        ["sumdb", SUMDB, "latest"] => return Some(Asked::Call(call("head"))),
        ["sumdb", SUMDB, "tile", tile @ ..] => return Some(Asked::Call(call("tile").with("tile", tile.join("/")))),
        ["sumdb", SUMDB, "lookup", rest @ ..] => {
            let (module, version) = rest.join("/").rsplit_once('@').map(|(m, v)| (unescape(m), unescape(v)))?;
            return Some(Asked::Call(call("lookup").with("module", module?).with("version", version?)));
        }
        ["sumdb", ..] => return None,
        _ => {}
    }
    let at = s.iter().position(|p| p.starts_with('@'))?;
    let module = unescape(&s[..at].join("/"))?;
    match &s[at..] {
        ["@latest"] => Some(Asked::Call(call("latest").with("module", module))),
        ["@v", "list"] => Some(Asked::Call(call("list").with("module", module))),
        ["@v", file] => {
            let (version, op) = [(".info", "info"), (".mod", "mod"), (".zip", "zip")].iter().find_map(|(ext, op)| file.strip_suffix(ext).map(|v| (v, *op)))?;
            Some(Asked::Call(call(op).with("module", module).with("version", unescape(version)?)))
        }
        _ => None,
    }
}

impl Conversation for GoConversation {
    fn parse(&mut self, req: &Request, path: &str) -> Result<Call, Native> {
        if req.method != "GET" && req.method != "HEAD" {
            return Err(Native::plain(405, "Method Not Allowed", "this registry only reads").header("Allow", "GET, HEAD"));
        }
        match parse_path(path) {
            Some(Asked::Call(call)) => Ok(call),
            // The proxy serves the checksum database too, so go asks for it here rather than of sum.golang.org itself.
            Some(Asked::Supported) => Err(Native::new(200, "OK").header("Content-Type", "text/plain; charset=utf-8").bytes(Vec::new())),
            // A 404 is go's "not this module": it tries the next path up.
            None => Err(Native::plain(404, "Not Found", "not something go is answered with here")),
        }
    }

    fn reply(&mut self, call: &Call, answer: Answer) -> Native {
        let kind = match call.op {
            "zip" => "application/zip",
            "list" | "mod" | "lookup" | "head" => "text/plain; charset=utf-8",
            _ => "application/octet-stream",
        };
        match (call.op, answer) {
            ("info" | "latest", Answer::Json(v)) => Native::new(200, "OK").header("Content-Type", "application/json").bytes(v.to_string()),
            ("list" | "mod" | "zip" | "lookup" | "tile" | "head", Answer::Bytes { reader, size, .. }) => Native::new(200, "OK").header("Content-Type", kind).stream(reader, size),
            // go prints a proxy's body after its status ("server response: …"), so it says why. A 404 or 410 is "not
            // found", which go takes as "try elsewhere" for a module's path; anything else stops it.
            (_, Answer::Refused { code, message }) => {
                let (status, reason) = match code.as_str() {
                    "not_found" | "invalid" => (404, "Not Found"),
                    "forbidden" | "unauthorized" => (403, "Forbidden"),
                    "too_large" => (413, "Too Large"),
                    _ => (503, "Unavailable"),
                };
                Native::plain(status, reason, &message)
            }
            (op, _) => Native::plain(502, "Bad Gateway", &format!("the package server answered go {op} with the wrong kind of answer")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(path: &str) -> Option<(&'static str, Vec<(&'static str, String)>)> {
        match parse_path(path)? {
            Asked::Call(c) => {
                assert_eq!(c.check(OPS), Ok(()), "{path}");
                Some((c.op, c.params))
            }
            Asked::Supported => None,
        }
    }

    fn m(s: &str) -> (&'static str, String) {
        ("module", s.into())
    }
    fn v(s: &str) -> (&'static str, String) {
        ("version", s.into())
    }

    #[test]
    fn gos_paths_are_calls() {
        assert_eq!(call("/golang.org/x/text/@v/list"), Some(("list", vec![m("golang.org/x/text")])));
        assert_eq!(call("/github.com/!burnt!sushi/toml/@v/v1.3.2.info"), Some(("info", vec![m("github.com/BurntSushi/toml"), v("v1.3.2")])));
        assert_eq!(call("/gopkg.in/yaml.v3/@v/v3.0.1.mod"), Some(("mod", vec![m("gopkg.in/yaml.v3"), v("v3.0.1")])));
        assert_eq!(call("/k8s.io/api/@v/v0.0.0-20191109021931-daa7c04131f5.zip"), Some(("zip", vec![m("k8s.io/api"), v("v0.0.0-20191109021931-daa7c04131f5")])));
        assert_eq!(call("/example.com/m/@v/v1.0.0-!r!c1.zip"), Some(("zip", vec![m("example.com/m"), v("v1.0.0-RC1")])));
        assert_eq!(call("/example.com/m/@latest"), Some(("latest", vec![m("example.com/m")])));
        assert_eq!(call("/sumdb/sum.golang.org/lookup/github.com/!google/uuid@v1.6.0"), Some(("lookup", vec![m("github.com/Google/uuid"), v("v1.6.0")])));
        assert_eq!(call("/sumdb/sum.golang.org/tile/8/0/x001/234.p/5"), Some(("tile", vec![("tile", "8/0/x001/234.p/5".into())])));
        assert_eq!(call("/sumdb/sum.golang.org/latest"), Some(("head", vec![])));
        assert!(matches!(parse_path("/sumdb/sum.golang.org/supported"), Some(Asked::Supported)));
    }

    #[test]
    fn anything_else_is_refused_before_its_call() {
        for path in ["/sumdb/evil.example/lookup/a.b/c@v1.0.0", "/sumdb/sum.golang.org/../x", "/golang.org/x/text/@v/master.info", "/golang.org/x/text/@v/v1.0.0.tar", "/github.com/Google/uuid/@v/list", "/x/@v/list", "/a.b/c%2Fd/@v/list", "/a.b/c/@v", "/a.b/c/@v/list/x", "/a.b/c/@v/v1.0.0!.info", "/"] {
            let checked = match parse_path(path) {
                Some(Asked::Call(c)) => c.check(OPS).is_ok(),
                Some(Asked::Supported) => true,
                None => false,
            };
            assert!(!checked, "{path}");
        }
    }

    #[test]
    fn checks() {
        for p in ["golang.org/x/text", "github.com/BurntSushi/toml", "go.uber.org/zap", "example.com/a_b/c~d/v2"] {
            assert!(is_module(p), "{p}");
        }
        for p in ["", "text", "Golang.org/x", "-x.org/a", "x.org/../a", "x.org//a", "x.org/a/", "x.org/a b", "x_y.org/a"] {
            assert!(!is_module(p), "{p}");
        }
        for s in ["v1.2.3", "v2.0.0+incompatible", "v0.0.1-go1.25.0.linux-arm64", "v1.0.0-rc.1"] {
            assert!(is_version(s), "{s}");
        }
        for s in ["1.2.3", "v1.2", "v01.2.3", "v1.2.3+build", "v1.0.0-01", "master"] {
            assert!(!is_version(s), "{s}");
        }
        for t in ["8/0/000", "8/0/x001/234", "8/1/x001/x002/003.p/12", "8/data/x123/456"] {
            assert!(is_tile(t), "{t}");
        }
        for t in ["8/0", "8/0/1234", "8/0/x01/234", "8/0/234.p", "8/0/234.p/x", "../0/000", "8/0/000/"] {
            assert!(!is_tile(t), "{t}");
        }
        assert_eq!(unescape("github.com/!burnt!sushi"), Some("github.com/BurntSushi".into()));
        assert_eq!(unescape("github.com/Burnt"), None);
        assert_eq!(unescape("a!"), None);
        assert_eq!(unescape("a!1"), None);
    }

    #[test]
    fn the_machine_uses_only_this_proxy_and_gos_own_database() {
        let env = Go.env("http://registry.carbon/go/", &Value::Null);
        assert!(env.contains(&("GOPROXY".into(), "http://registry.carbon/go".into())));
        assert!(env.contains(&("GOSUMDB".into(), "sum.golang.org".into())));
        assert!(env.iter().all(|(_, v)| !v.contains("direct")));
    }
}
