//! `registries-npm`: npm's registry protocol as registries' calls, for npm and everything that speaks it (yarn, pnpm,
//! bun).
//!
//! | The tool asks for | The call |
//! | --- | --- |
//! | `/{name}`, `/@scope%2fname`, `/@scope/name` | `packument {name}` |
//! | `/{name}/-/{base}-{version}.tgz` (`name` with its scope, `base` without) | `tarball {name, version}` |
//!
//! Nothing else has a call: search, `audit`, `login`, `publish`, `dist-tags`, `whoami`, and `/-/` paths in general
//! are a 404 here. A packument is passed on as the server built it, but for each version's `dist.tarball`, which is
//! rewritten to this registry, so the tool never learns another host. Each version's `integrity` is kept for the
//! machine, and the tarball's call expects it: Carbon checks the tarball as it streams, and so does the server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use registries_core::{Adapter, Answer, Call, Conversation, Expect, Field, Native, Op, Request, base64_decode, hex};
use serde_json::{Value, json};

const SERVICE: &str = "npm";
/// The most integrities a machine keeps; past it, the oldest are forgotten all at once, and their tarballs go
/// unchecked here (the server still checks them).
const MOST_KEPT: usize = 200_000;

/// npm's rules for a name, with the capitals old packages have (`JSONStream`): `[@scope/]name`, at most 214.
pub fn is_name(s: &str) -> bool {
    let part = |p: &str| !p.is_empty() && !p.starts_with(['.', '_']) && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b));
    if s.len() > 214 {
        return false;
    }
    match s.strip_prefix('@') {
        Some(scoped) => scoped.split_once('/').is_some_and(|(scope, name)| part(scope) && part(name)),
        None => part(s),
    }
}

/// SemVer 2.0, strictly: `1.2.3`, `1.0.0-beta.1`, `1.0.0+build.5`.
pub fn is_version(s: &str) -> bool {
    let number = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && (n == "0" || !n.starts_with('0'));
    let ident = |i: &str| !i.is_empty() && i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if s.len() > 256 {
        return false;
    }
    let (rest, build) = match s.split_once('+') {
        Some((r, b)) => (r, Some(b)),
        None => (s, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (rest, None),
    };
    let nums: Vec<&str> = core.split('.').collect();
    nums.len() == 3
        && nums.iter().all(|n| number(n))
        && pre.is_none_or(|p| p.split('.').all(|i| ident(i) && (!i.bytes().all(|b| b.is_ascii_digit()) || number(i))))
        && build.is_none_or(|b| b.split('.').all(ident))
}

static OPS: &[Op] = &[
    Op { name: "packument", fields: &[Field::required("name", is_name)] },
    Op { name: "tarball", fields: &[Field::required("name", is_name), Field::required("version", is_version)] },
];

/// npm's adapter: one a machine, keeping the integrities of the versions its packuments offered.
pub struct Npm {
    integrity: Arc<Mutex<HashMap<String, String>>>,
}

impl Npm {
    pub fn adapter() -> Arc<dyn Adapter> {
        Arc::new(Npm { integrity: Arc::default() })
    }
}

impl Adapter for Npm {
    fn service(&self) -> &'static str {
        SERVICE
    }
    fn ops(&self) -> &'static [Op] {
        OPS
    }
    fn conversation(&self) -> Box<dyn Conversation> {
        Box::new(NpmConversation { integrity: self.integrity.clone(), host: "registry.carbon".into() })
    }
    fn env(&self, base: &str, _details: &Value) -> Vec<(String, String)> {
        let host = base.trim_start_matches("http://").split('/').next().unwrap_or_default().to_string();
        vec![
            ("NPM_CONFIG_REGISTRY".into(), base.into()),
            // What would call home: refused here anyway, so not asked.
            ("NPM_CONFIG_AUDIT".into(), "false".into()),
            ("NPM_CONFIG_FUND".into(), "false".into()),
            ("NPM_CONFIG_UPDATE_NOTIFIER".into(), "false".into()),
            // Yarn 2 and later read their own, and want plain HTTP to the registry allowed by name.
            ("YARN_NPM_REGISTRY_SERVER".into(), base.trim_end_matches('/').into()),
            ("YARN_UNSAFE_HTTP_WHITELIST".into(), host),
        ]
    }
}

struct NpmConversation {
    integrity: Arc<Mutex<HashMap<String, String>>>,
    /// What the tool calls this registry, for the tarball URLs it's given.
    host: String,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// npm's own refusal: a status, and `{"error": why}`, which npm prints.
fn refusal(status: u16, reason: &str, why: &str) -> Native {
    Native::new(status, reason).header("Content-Type", "application/json").bytes(json!({ "error": why }).to_string())
}

/// `%XX` decoded, with `/` allowed (a scoped name's): None if it isn't UTF-8.
fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The call npm's `path` (under `/npm`) is, or None if it's nothing a call is made for.
pub fn parse_path(path: &str) -> Option<Call> {
    let raw: Vec<&str> = path.strip_prefix('/')?.trim_end_matches('/').split('/').collect();
    let segments: Vec<String> = raw.iter().map(|s| decode(s)).collect::<Option<_>>()?;
    let s: Vec<&str> = segments.iter().map(String::as_str).collect();
    let (name, rest): (String, &[&str]) = match s.as_slice() {
        [scope, name, rest @ ..] if scope.starts_with('@') && !scope.contains('/') => (format!("{scope}/{name}"), rest),
        [name, rest @ ..] => (name.to_string(), rest),
        [] => return None,
    };
    if name.starts_with('-') || !is_name(&name) {
        return None;
    }
    match rest {
        [] => Some(Call::new(SERVICE, "packument").with("name", name)),
        ["-", file] => {
            let base = name.rsplit('/').next()?;
            let version = file.strip_suffix(".tgz")?.strip_prefix(base)?.strip_prefix('-')?;
            is_version(version).then(|| Call::new(SERVICE, "tarball").with("name", name.clone()).with("version", version))
        }
        _ => None,
    }
}

/// What a tarball's URL is, at this registry.
fn tarball_url(host: &str, name: &str, version: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    format!("http://{host}/npm/{name}/-/{base}-{version}.tgz")
}

/// A `Host` header that's a name or an address with a port, and nothing else.
fn good_host(h: &str) -> bool {
    !h.is_empty() && h.len() <= 253 && h.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b))
}

impl Conversation for NpmConversation {
    fn parse(&mut self, req: &Request, path: &str) -> Result<Call, Native> {
        if req.method != "GET" && req.method != "HEAD" {
            return Err(refusal(405, "Method Not Allowed", "this registry only reads").header("Allow", "GET, HEAD"));
        }
        if let Some(host) = req.header("Host").filter(|h| good_host(h)) {
            self.host = host.to_string();
        }
        let mut call = parse_path(path).ok_or_else(|| refusal(404, "Not Found", "not something npm is answered with here"))?;
        if call.op == "tarball" {
            let key = format!("{}@{}", call.param("name").unwrap_or_default(), call.param("version").unwrap_or_default());
            if let Some(sum) = lock(&self.integrity).get(&key) {
                call.expect = Some(Expect { sha512: Some(sum.clone()), ..Expect::default() });
            }
        }
        Ok(call)
    }

    fn reply(&mut self, call: &Call, answer: Answer) -> Native {
        let name = call.param("name").unwrap_or_default().to_string();
        match (call.op, answer) {
            ("packument", Answer::Json(mut doc)) => {
                if doc.get("name").and_then(Value::as_str) != Some(name.as_str()) {
                    return refusal(502, "Bad Gateway", &format!("the package server answered {name} with another package"));
                }
                let mut learned = Vec::new();
                if let Some(versions) = doc.get_mut("versions").and_then(Value::as_object_mut) {
                    for (version, v) in versions.iter_mut() {
                        let Some(dist) = v.get_mut("dist").and_then(Value::as_object_mut) else { continue };
                        dist.insert("tarball".into(), Value::String(tarball_url(&self.host, &name, version)));
                        let sum = dist.get("integrity").and_then(Value::as_str).and_then(|i| i.strip_prefix("sha512-")).and_then(base64_decode);
                        if let Some(sum) = sum.filter(|s| s.len() == 64) {
                            learned.push((format!("{name}@{version}"), hex(&sum)));
                        }
                    }
                }
                {
                    let mut kept = lock(&self.integrity);
                    if kept.len() + learned.len() > MOST_KEPT {
                        kept.clear();
                    }
                    kept.extend(learned);
                }
                Native::new(200, "OK").header("Content-Type", "application/json").bytes(doc.to_string())
            }
            ("tarball", Answer::Bytes { reader, size, .. }) => Native::new(200, "OK").header("Content-Type", "application/octet-stream").stream(reader, size),
            (_, Answer::Refused { code, message }) => {
                let (status, reason) = match code.as_str() {
                    "not_found" | "invalid" => (404, "Not Found"),
                    "forbidden" | "unauthorized" => (403, "Forbidden"),
                    "too_large" => (413, "Too Large"),
                    _ => (503, "Unavailable"),
                };
                refusal(status, reason, &message)
            }
            (op, _) => refusal(502, "Bad Gateway", &format!("the package server answered npm {op} with the wrong kind of answer")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(path: &str) -> Option<(&'static str, Vec<(&'static str, String)>)> {
        parse_path(path).map(|c| (c.op, c.params))
    }

    #[test]
    fn npms_paths_are_calls() {
        assert_eq!(fields("/left-pad"), Some(("packument", vec![("name", "left-pad".into())])));
        assert_eq!(fields("/@types%2fnode"), Some(("packument", vec![("name", "@types/node".into())])));
        assert_eq!(fields("/@types%2Fnode"), Some(("packument", vec![("name", "@types/node".into())])));
        assert_eq!(fields("/@types/node"), Some(("packument", vec![("name", "@types/node".into())])));
        assert_eq!(fields("/JSONStream"), Some(("packument", vec![("name", "JSONStream".into())])));
        assert_eq!(fields("/left-pad/-/left-pad-1.3.0.tgz"), Some(("tarball", vec![("name", "left-pad".into()), ("version", "1.3.0".into())])));
        assert_eq!(fields("/@types/node/-/node-24.10.1.tgz"), Some(("tarball", vec![("name", "@types/node".into()), ("version", "24.10.1".into())])));
        assert_eq!(fields("/@scope%2fpkg/-/pkg-1.0.0-beta.2.tgz").unwrap().1[1], ("version", "1.0.0-beta.2".into()));
    }

    #[test]
    fn everything_else_is_not_here() {
        for path in [
            "/-/npm/v1/security/advisories/bulk",
            "/-/v1/search",
            "/-/whoami",
            "/left-pad/latest",
            "/left-pad/-/other-1.0.0.tgz",
            "/left-pad/-/left-pad-1.3.tgz",
            "/left-pad/-/left-pad-01.3.0.tgz",
            "/@types",
            "/@a/b/c",
            "/.hidden",
            "/a%2f..%2fb",
            "/",
            "",
        ] {
            assert!(parse_path(path).is_none(), "{path}");
        }
        assert!(!is_name(&"a".repeat(215)) && is_name("@a/b") && !is_name("@a") && !is_name("a b"));
        assert!(is_version("1.0.0-alpha.1+build.5") && !is_version("1.0") && !is_version("1.0.0-01") && !is_version("v1.0.0"));
    }

    #[test]
    fn packuments_point_at_this_registry_and_tarballs_expect_their_integrity() {
        let npm = Npm { integrity: Arc::default() };
        let mut conv = npm.conversation();
        let req = Request { method: "GET".into(), path: "/npm/left-pad".into(), query: String::new(), headers: vec![("Host".into(), "registry.carbon".into())], body: Vec::new(), keep_alive: true };
        let call = conv.parse(&req, "/left-pad").unwrap();
        let sum = [7u8; 64];
        let b64 = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw==";
        assert_eq!(base64_decode(b64).unwrap(), sum);
        let doc = json!({ "name": "left-pad", "dist-tags": {"latest": "1.3.0"}, "versions": { "1.3.0": { "dist": { "integrity": format!("sha512-{b64}"), "tarball": "https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz" } } } });
        let native = conv.reply(&call, Answer::Json(doc));
        let registries_core::Body::Bytes(body) = native.body else { panic!() };
        let back: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(back["versions"]["1.3.0"]["dist"]["tarball"], "http://registry.carbon/npm/left-pad/-/left-pad-1.3.0.tgz");
        // Another connection of the same machine expects the integrity its packument gave.
        let mut other = npm.conversation();
        let tarball = other.parse(&req, "/left-pad/-/left-pad-1.3.0.tgz").unwrap();
        assert_eq!(tarball.expect.and_then(|e| e.sha512), Some(hex(&sum)));
        // A packument for another package than the one asked for is refused.
        assert_eq!(conv.reply(&call, Answer::Json(json!({ "name": "evil" }))).status, 502);
    }

    #[test]
    fn refusals_are_npms_own() {
        let call = Call::new(SERVICE, "packument").with("name", "fresh");
        let n = NpmConversation { integrity: Arc::default(), host: "x".into() }.reply(&call, Answer::refused("not_found", "fresh has no version published before 2025-11-20"));
        assert_eq!(n.status, 404);
        let registries_core::Body::Bytes(body) = n.body else { panic!() };
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"], "fresh has no version published before 2025-11-20");
        let npm = Npm { integrity: Arc::default() };
        let env = npm.env("http://registry.carbon/npm/", &Value::Null);
        assert!(env.contains(&("NPM_CONFIG_REGISTRY".into(), "http://registry.carbon/npm/".into())));
        assert!(env.contains(&("YARN_UNSAFE_HTTP_WHITELIST".into(), "registry.carbon".into())));
    }
}
