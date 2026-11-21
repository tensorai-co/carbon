//! `registries-rubygems`: RubyGems' compact index, as Bundler and `gem` read it, as registries' calls.
//!
//! | They ask for | The call |
//! | --- | --- |
//! | `GET /versions` | `versions` |
//! | `HEAD /versions` | None: answered here, as `gem` asks only whether there's a compact index |
//! | `/info/{name}` | `info {name}` |
//! | `/gems/{name}-{version}[-{platform}].gem` | `gem {file}` |
//! | `/quick/Marshal.4.8/{name}-{version}[-{platform}].gemspec.rz` | `gemspec {file}` |
//!
//! A file's name is passed whole (`nokogiri-1.16.0-aarch64-linux`): a gem's name may have dashes and a version a
//! platform after one, so the server splits it by what its index has.
//!
//! The machine gets `BUNDLE_MIRROR__ALL`, so Bundler fetches every source's gems here, and `GEMRC` naming a config in a
//! tmpfs, `/etc/carbon/rubygems/gemrc`, whose one source is this registry, for `gem`.

use std::sync::Arc;

use registries_core::{Adapter, Answer, Call, Conversation, Field, Native, Op, Request, clean, decode_segment};
use serde_json::Value;

const SERVICE: &str = "rubygems";
/// `gem`'s config, which `GEMRC` names.
pub const GEMRC: &str = "/etc/carbon/rubygems/gemrc";

/// A gem's name: letters, digits, `. _ -`, not starting with a dot or a dash, no `..`.
pub fn is_name(s: &str) -> bool {
    s.len() <= 100 && is_name_chars(s)
}

/// A version's file name without its extension, `{name}-{version}[-{platform}]`: a name's characters, with a dash.
pub fn is_file(s: &str) -> bool {
    s.len() <= 200 && s.contains('-') && is_name_chars(s)
}

fn is_name_chars(s: &str) -> bool {
    !s.is_empty() && (s.as_bytes()[0].is_ascii_alphanumeric() || s.as_bytes()[0] == b'_') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) && !s.contains("..")
}

static OPS: &[Op] = &[
    Op { name: "versions", fields: &[] },
    Op { name: "info", fields: &[Field::required("name", is_name)] },
    Op { name: "gem", fields: &[Field::required("file", is_file)] },
    Op { name: "gemspec", fields: &[Field::required("file", is_file)] },
];

/// RubyGems' adapter. It keeps nothing.
pub struct Rubygems;

impl Rubygems {
    pub fn adapter() -> Arc<dyn Adapter> {
        Arc::new(Rubygems)
    }
}

impl Adapter for Rubygems {
    fn service(&self) -> &'static str {
        SERVICE
    }
    fn ops(&self) -> &'static [Op] {
        OPS
    }
    fn conversation(&self) -> Box<dyn Conversation> {
        Box::new(GemsConversation)
    }
    /// Bundler mirrors every source here; `gem` reads the config below.
    fn env(&self, base: &str, _details: &Value) -> Vec<(String, String)> {
        vec![("BUNDLE_MIRROR__ALL".into(), base.to_string()), ("GEMRC".into(), GEMRC.into())]
    }
    /// `gem`'s one source: this registry.
    fn files(&self, base: &str, _details: &Value) -> Vec<(String, Vec<u8>)> {
        let gemrc = format!("# Carbon's registries: RubyGems, through this machine's own registry.\n:sources:\n- {base}\n");
        vec![(GEMRC.into(), gemrc.into_bytes())]
    }
}

struct GemsConversation;

fn parse_path(path: &str) -> Option<Call> {
    let segments: Vec<String> = path.strip_prefix('/')?.split('/').map(decode_segment).collect::<Option<_>>()?;
    let s: Vec<&str> = segments.iter().map(String::as_str).collect();
    let call = |op: &'static str| Call::new(SERVICE, op);
    match s.as_slice() {
        ["versions"] => Some(call("versions")),
        ["info", name] => Some(call("info").with("name", *name)),
        ["gems", file] => Some(call("gem").with("file", file.strip_suffix(".gem")?)),
        ["quick", "Marshal.4.8", file] => Some(call("gemspec").with("file", file.strip_suffix(".gemspec.rz")?)),
        _ => None,
    }
}

impl Conversation for GemsConversation {
    fn parse(&mut self, req: &Request, path: &str) -> Result<Call, Native> {
        if req.method != "GET" && req.method != "HEAD" {
            return Err(Native::plain(405, "Method Not Allowed", "this registry only reads").header("Allow", "GET, HEAD"));
        }
        // `gem` asks whether there's a compact index with a HEAD of /versions: there is, and it needn't come here.
        if req.method == "HEAD" && path == "/versions" {
            return Err(Native::new(200, "OK").header("Content-Type", "text/plain; charset=utf-8").bytes(Vec::new()));
        }
        parse_path(path).ok_or_else(|| Native::plain(404, "Not Found", "not something Bundler or gem is answered with here"))
    }

    fn reply(&mut self, call: &Call, answer: Answer) -> Native {
        let kind = match call.op {
            "versions" | "info" => "text/plain; charset=utf-8",
            _ => "application/octet-stream",
        };
        match answer {
            // No ETag and no digest: an /info file is the server's, not RubyGems.org's, so theirs wouldn't match it.
            Answer::Bytes { reader, size, .. } => Native::new(200, "OK").header("Content-Type", kind).stream(reader, size),
            Answer::Refused { code, message } => {
                let (status, reason) = match code.as_str() {
                    "not_found" | "invalid" => (404, "Not Found"),
                    "forbidden" | "unauthorized" => (403, "Forbidden"),
                    "too_large" => (413, "Too Large"),
                    _ => (503, "Unavailable"),
                };
                Native::plain(status, &format!("{reason} - {}", clean(&message)), &message)
            }
            Answer::Json(_) => Native::plain(502, "Bad Gateway", &format!("the package server answered rubygems {} with JSON", call.op)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(path: &str) -> Option<(&'static str, Vec<(&'static str, String)>)> {
        let c = parse_path(path)?;
        c.check(OPS).ok()?;
        Some((c.op, c.params))
    }

    #[test]
    fn bundlers_and_gems_paths_are_calls() {
        assert_eq!(call("/versions"), Some(("versions", vec![])));
        assert_eq!(call("/info/rack-test"), Some(("info", vec![("name", "rack-test".into())])));
        assert_eq!(call("/gems/nokogiri-1.16.0-aarch64-linux.gem"), Some(("gem", vec![("file", "nokogiri-1.16.0-aarch64-linux".into())])));
        assert_eq!(call("/quick/Marshal.4.8/rack-3.0.8.gemspec.rz"), Some(("gemspec", vec![("file", "rack-3.0.8".into())])));
        for path in ["/names", "/api/v1/dependencies", "/specs.4.8.gz", "/info/../x", "/info/.x", "/gems/rack.gem", "/gems/rack-1.0.tar", "/gems/a%2Fb-1.gem", "/quick/Marshal.4.8/rack-1.gemspec", "/"] {
            assert!(call(path).is_none(), "{path}");
        }
    }

    #[test]
    fn head_of_versions_is_answered_here() {
        let req = |method: &str| Request { method: method.into(), path: "/rubygems/versions".into(), query: String::new(), headers: vec![], body: Vec::new(), keep_alive: true };
        assert!(matches!(Rubygems.conversation().parse(&req("HEAD"), "/versions"), Err(n) if n.status == 200));
        assert!(Rubygems.conversation().parse(&req("GET"), "/versions").is_ok());
    }

    #[test]
    fn the_machine_uses_only_this_registry() {
        let env = Rubygems.env("http://registry.carbon/rubygems/", &Value::Null);
        assert!(env.contains(&("BUNDLE_MIRROR__ALL".into(), "http://registry.carbon/rubygems/".into())));
        let files = Rubygems.files("http://registry.carbon/rubygems/", &Value::Null);
        assert_eq!(files[0].0, GEMRC);
        assert!(String::from_utf8(files[0].1.clone()).unwrap().contains(":sources:\n- http://registry.carbon/rubygems/\n"));
    }
}
