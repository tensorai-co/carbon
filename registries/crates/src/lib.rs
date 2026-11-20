//! `registries-crates`: Cargo's sparse index protocol as registries' calls.
//!
//! | Cargo asks for | The call |
//! | --- | --- |
//! | `/config.json` | None: answered here, so its `dl` points every download back at this registry, with its hash |
//! | `/1/{name}`, `/2/{name}`, `/3/{c}/{name}`, `/{ab}/{cd}/{name}` | `index {name}` |
//! | `/dl/{crate}/{version}/{sha256}` | `crate {name, version, sha256}` |
//!
//! Cargo can't be pointed at another index by its environment: crates.io's source is replaced by a config file. So
//! the machine gets `/.cargo/config.toml` (which Cargo reads for every project, from the filesystem's root), in a
//! tmpfs for the run only:
//!
//! ```toml
//! [source.crates-io]
//! replace-with = "carbon"
//! [source.carbon]
//! registry = "sparse+http://registry.carbon/crates/"
//! ```
//!
//! An index file is passed on as the server built it. Each download's URL carries its version's `cksum`, so every
//! `.crate`'s call expects its SHA-256: Carbon checks it as it streams, and so does the server. It keeps nothing.

use std::sync::Arc;

use registries_core::{Adapter, Answer, Call, Conversation, Expect, Field, Native, Op, Request, check, decode_segment};
use serde_json::{Value, json};

const SERVICE: &str = "crates";

/// A crate's name, as crates.io takes it: letters, digits, `-` and `_`, starting with a letter, at most 64.
pub fn is_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.as_bytes()[0].is_ascii_alphabetic() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// SemVer 2.0, strictly.
pub fn is_version(s: &str) -> bool {
    let number = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && (n == "0" || !n.starts_with('0'));
    let ident = |i: &str| !i.is_empty() && i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if s.len() > 256 {
        return false;
    }
    let (rest, build) = s.split_once('+').map_or((s, None), |(r, b)| (r, Some(b)));
    let (core, pre) = rest.split_once('-').map_or((rest, None), |(c, p)| (c, Some(p)));
    let nums: Vec<&str> = core.split('.').collect();
    nums.len() == 3 && nums.iter().all(|n| number(n)) && pre.is_none_or(|p| p.split('.').all(|i| ident(i) && (!i.bytes().all(|b| b.is_ascii_digit()) || number(i)))) && build.is_none_or(|b| b.split('.').all(ident))
}

fn sha256(s: &str) -> bool {
    check::hex(s, 64)
}

static OPS: &[Op] = &[
    Op { name: "index", fields: &[Field::required("name", is_name)] },
    Op { name: "crate", fields: &[Field::required("name", is_name), Field::required("version", is_version), Field::required("sha256", sha256)] },
];

/// Cargo's adapter. It keeps nothing: each download's hash is in its URL.
pub struct Crates;

impl Crates {
    pub fn adapter() -> Arc<dyn Adapter> {
        Arc::new(Crates)
    }
}

impl Adapter for Crates {
    fn service(&self) -> &'static str {
        SERVICE
    }
    fn ops(&self) -> &'static [Op] {
        OPS
    }
    fn conversation(&self) -> Box<dyn Conversation> {
        Box::new(CratesConversation { host: "registry.carbon".into() })
    }
    /// crates.io's source replaced by this registry, for every project in the machine.
    fn files(&self, base: &str, _details: &Value) -> Vec<(String, Vec<u8>)> {
        let config = format!("# Carbon's registries: crates.io, through this machine's own registry.\n[source.crates-io]\nreplace-with = \"carbon\"\n\n[source.carbon]\nregistry = \"sparse+{base}\"\n");
        vec![("/.cargo/config.toml".into(), config.into_bytes())]
    }
}

struct CratesConversation {
    host: String,
}

/// Where a crate's file is in a sparse index, lowercased: `1/a`, `2/ab`, `3/a/abc`, `se/rd/serde`.
pub fn index_path(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    match n.len() {
        1 => format!("1/{n}"),
        2 => format!("2/{n}"),
        3 => format!("3/{}/{n}", &n[..1]),
        _ => format!("{}/{}/{n}", &n[..2], &n[2..4]),
    }
}

/// What Cargo's `path` (under `/crates`) is: a call, `config.json` (answered here), or nothing.
enum Asked {
    Call(Call),
    Config,
}

fn parse_path(path: &str) -> Option<Asked> {
    let segments: Vec<String> = path.strip_prefix('/')?.split('/').map(decode_segment).collect::<Option<_>>()?;
    let s: Vec<&str> = segments.iter().map(String::as_str).collect();
    match s.as_slice() {
        ["config.json"] => Some(Asked::Config),
        ["dl", name, version, sum] => Some(Asked::Call(Call::new(SERVICE, "crate").with("name", *name).with("version", *version).with("sha256", *sum))),
        // An index file: its folders must be the name's own, as Cargo makes them.
        [.., name] if is_name(name) && *name == name.to_ascii_lowercase() && index_path(name) == s.join("/") => Some(Asked::Call(Call::new(SERVICE, "index").with("name", *name))),
        _ => None,
    }
}

/// A `Host` header that's a name or an address with a port, and nothing else.
fn good_host(h: &str) -> bool {
    !h.is_empty() && h.len() <= 253 && h.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b))
}

impl Conversation for CratesConversation {
    fn parse(&mut self, req: &Request, path: &str) -> Result<Call, Native> {
        if req.method != "GET" && req.method != "HEAD" {
            return Err(Native::plain(405, "Method Not Allowed", "this registry only reads").header("Allow", "GET, HEAD"));
        }
        if let Some(host) = req.header("Host").filter(|h| good_host(h)) {
            self.host = host.to_string();
        }
        match parse_path(path) {
            Some(Asked::Call(mut call)) => {
                if call.op == "crate" {
                    call.expect = Some(Expect { sha256: call.param("sha256").map(String::from), ..Expect::default() });
                }
                Ok(call)
            }
            // Cargo's first ask: where downloads are. Here, with each one's hash in its URL. No `api`, so nothing can
            // be published or searched.
            Some(Asked::Config) => {
                let config = json!({ "dl": format!("http://{}/crates/dl/{{crate}}/{{version}}/{{sha256-checksum}}", self.host) });
                Err(Native::new(200, "OK").header("Content-Type", "application/json").bytes(config.to_string()))
            }
            None => Err(Native::plain(404, "Not Found", "not something cargo is answered with here")),
        }
    }

    fn reply(&mut self, call: &Call, answer: Answer) -> Native {
        match (call.op, answer) {
            ("index", Answer::Bytes { reader, size, .. }) => Native::new(200, "OK").header("Content-Type", "text/plain; charset=utf-8").stream(reader, size),
            ("crate", Answer::Bytes { reader, size, .. }) => Native::new(200, "OK").header("Content-Type", "application/octet-stream").stream(reader, size),
            // Cargo shows a refusal's body after its status, so it says why.
            (_, Answer::Refused { code, message }) => {
                let (status, reason) = match code.as_str() {
                    "not_found" | "invalid" => (404, "Not Found"),
                    "forbidden" | "unauthorized" => (403, "Forbidden"),
                    "too_large" => (413, "Too Large"),
                    _ => (503, "Unavailable"),
                };
                Native::plain(status, reason, &message)
            }
            (op, _) => Native::plain(502, "Bad Gateway", &format!("the package server answered crates {op} with the wrong kind of answer")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(path: &str) -> Option<(&'static str, Vec<(&'static str, String)>)> {
        match parse_path(path)? {
            Asked::Call(c) => Some((c.op, c.params)),
            Asked::Config => None,
        }
    }

    #[test]
    fn cargos_paths_are_calls() {
        assert_eq!(call("/se/rd/serde"), Some(("index", vec![("name", "serde".into())])));
        assert_eq!(call("/1/a"), Some(("index", vec![("name", "a".into())])));
        assert_eq!(call("/2/ab"), Some(("index", vec![("name", "ab".into())])));
        assert_eq!(call("/3/a/abc"), Some(("index", vec![("name", "abc".into())])));
        let sum = "c".repeat(64);
        assert_eq!(call(&format!("/dl/Inflector/0.11.4/{sum}")), Some(("crate", vec![("name", "Inflector".into()), ("version", "0.11.4".into()), ("sha256", sum)])));
        assert!(matches!(parse_path("/config.json"), Some(Asked::Config)));
        for path in ["/xx/rd/serde", "/se/rd/Serde", "/3/b/abc", "/1/ab", "/api/v1/crates", "/se/rd/../x", "/"] {
            assert!(call(path).is_none() || parse_path(path).is_none(), "{path}");
        }
    }

    #[test]
    fn the_config_points_downloads_here_with_their_hash() {
        let req = Request { method: "GET".into(), path: "/crates/config.json".into(), query: String::new(), headers: vec![("Host".into(), "registry.carbon".into())], body: Vec::new(), keep_alive: true };
        let Err(native) = Crates.conversation().parse(&req, "/config.json") else { panic!() };
        let registries_core::Body::Bytes(body) = native.body else { panic!() };
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["dl"], "http://registry.carbon/crates/dl/{crate}/{version}/{sha256-checksum}");
        assert!(v.get("api").is_none());
        let crate_call = Crates.conversation().parse(&req, &format!("/dl/serde/1.0.0/{}", "d".repeat(64))).unwrap();
        assert_eq!(crate_call.expect.clone().and_then(|e| e.sha256), Some("d".repeat(64)));
        assert_eq!(crate_call.check(OPS), Ok(()));
    }

    #[test]
    fn the_source_replacement_is_a_file() {
        let files = Crates.files("http://registry.carbon/crates/", &Value::Null);
        assert_eq!(files[0].0, "/.cargo/config.toml");
        let text = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(text.contains("replace-with = \"carbon\"") && text.contains("registry = \"sparse+http://registry.carbon/crates/\""));
        assert!(is_version("1.0.0+build.5") && !is_version("1.0") && is_name("serde_json") && !is_name("1x") && !is_name("a/b"));
    }
}
