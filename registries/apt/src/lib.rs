//! `registries-apt`: apt's HTTP as registries' calls.
//!
//! apt asks for paths under an archive (`http://registry.carbon/apt/debian/…`). Each path becomes fields, and no
//! path is ever sent:
//!
//! | apt asks for | The call |
//! | --- | --- |
//! | `/{archive}/dists/{suite}/InRelease` | `release {archive, suite}` |
//! | `/{archive}/dists/{suite}/{component}/binary-{arch}/by-hash/SHA256/{hash}` | `index {archive, suite, component, arch, sha256}` |
//! | `/{archive}/dists/{suite}/{component}/binary-{arch}/Packages{,.xz,.gz}` | `index {archive, suite, component, arch, file}` |
//! | `/{archive}/pool/[updates/]{component}/{prefix}/{source}/{name}_{version}_{arch}.deb` | `deb {archive, component, source, name, version, arch}` |
//!
//! Anything else (translations, Contents, DEP-11, `Release` and `Release.gpg`) is a 404 here, which apt reads as
//! "not there". The answers are passed on as they come: apt checks `InRelease`'s signature with its own keyring, every
//! index against `InRelease`, and every `.deb` against its index, so nothing here needs to know Debian's files. It keeps
//! no state between requests.

use std::sync::Arc;

use registries_core::{Adapter, Answer, Call, Conversation, Field, Native, Op, Request, check, decode_segment};

const SERVICE: &str = "apt";

fn word(s: &str) -> bool {
    check::word(s, 100)
}
fn arch(s: &str) -> bool {
    check::token(s, 32, b"-") && s.bytes().all(|b| !b.is_ascii_uppercase())
}
fn version(s: &str) -> bool {
    s.as_bytes().first().is_some_and(u8::is_ascii_digit) && check::token(s, 100, b".+~-")
}
fn sha256(s: &str) -> bool {
    check::hex(s, 64)
}
fn index_file(s: &str) -> bool {
    matches!(s, "Packages" | "Packages.xz" | "Packages.gz")
}

static OPS: &[Op] = &[
    Op { name: "release", fields: &[Field::required("archive", word), Field::required("suite", word)] },
    Op {
        name: "index",
        fields: &[
            Field::required("archive", word),
            Field::required("suite", word),
            Field::required("component", word),
            Field::required("arch", arch),
            Field::optional("sha256", sha256),
            Field::optional("file", index_file),
        ],
    },
    Op {
        name: "deb",
        fields: &[
            Field::required("archive", word),
            Field::required("component", word),
            Field::required("source", word),
            Field::required("name", word),
            Field::required("version", version),
            Field::required("arch", arch),
        ],
    },
];

/// apt's adapter.
pub struct Apt;

impl Apt {
    pub fn adapter() -> Arc<dyn Adapter> {
        Arc::new(Apt)
    }
}

impl Adapter for Apt {
    fn service(&self) -> &'static str {
        SERVICE
    }
    fn ops(&self) -> &'static [Op] {
        OPS
    }
    fn conversation(&self) -> Box<dyn Conversation> {
        Box::new(AptConversation)
    }
}

struct AptConversation;

fn not_here(why: &str) -> Native {
    Native::plain(404, "Not Found", why)
}

/// The call apt's `path` (under `/apt`) is, or None if it's nothing a call is made for.
pub fn parse_path(path: &str) -> Option<Call> {
    let segments: Vec<String> = path.strip_prefix('/')?.split('/').map(decode_segment).collect::<Option<_>>()?;
    let s: Vec<&str> = segments.iter().map(String::as_str).collect();
    let call = match s.as_slice() {
        [archive, "dists", suite, "InRelease"] => Call::new(SERVICE, "release").with("archive", *archive).with("suite", *suite),
        [archive, "dists", suite, component, binary, "by-hash", "SHA256", hash] => {
            Call::new(SERVICE, "index").with("archive", *archive).with("suite", *suite).with("component", *component).with("arch", binary.strip_prefix("binary-")?).with("sha256", *hash)
        }
        [archive, "dists", suite, component, binary, file] if index_file(file) => {
            Call::new(SERVICE, "index").with("archive", *archive).with("suite", *suite).with("component", *component).with("arch", binary.strip_prefix("binary-")?).with("file", *file)
        }
        // Debian's security archive keeps its pool under `pool/updates/`: where each archive's pool is, is the
        // server's to know, so it isn't part of the call.
        [archive, "pool", "updates", component, prefix, source, file] | [archive, "pool", component, prefix, source, file] => {
            let (name, version, arch) = deb_file(file)?;
            // The pool's folder is the source's: `c` for curl, `libc` for libcurl.
            let want = if source.starts_with("lib") && source.len() > 3 { source.get(..4)? } else { source.get(..1)? };
            if *prefix != want {
                return None;
            }
            Call::new(SERVICE, "deb").with("archive", *archive).with("component", *component).with("source", *source).with("name", name).with("version", version).with("arch", arch)
        }
        _ => return None,
    };
    Some(call)
}

/// A pool file's name, version and architecture: `jq_1.7.1-6+deb13u1_arm64.deb`.
fn deb_file(file: &str) -> Option<(&str, &str, &str)> {
    let stem = file.strip_suffix(".deb")?;
    let mut parts = stem.split('_');
    let (name, version, arch) = (parts.next()?, parts.next()?, parts.next()?);
    parts.next().is_none().then_some((name, version, arch))
}

impl Conversation for AptConversation {
    fn parse(&mut self, req: &Request, path: &str) -> Result<Call, Native> {
        if req.method != "GET" && req.method != "HEAD" {
            return Err(Native::plain(405, "Method Not Allowed", "apt only reads").header("Allow", "GET, HEAD"));
        }
        // A Range is answered with the whole file, which apt takes in place of the rest.
        parse_path(path).ok_or_else(|| not_here("not something apt is answered with here"))
    }

    fn reply(&mut self, call: &Call, answer: Answer) -> Native {
        match answer {
            Answer::Bytes { reader, size, offset: _ } => {
                let kind = if call.op == "release" { "text/plain" } else { "application/octet-stream" };
                Native::new(200, "OK").header("Content-Type", kind).stream(reader, size)
            }
            Answer::Json(_) => Native::plain(502, "Bad Gateway", "the package server answered apt with JSON"),
            // apt shows the status line, so the reason says why.
            Answer::Refused { code, message } => {
                let (status, reason) = match code.as_str() {
                    "not_found" | "invalid" => (404, "Not Found"),
                    "forbidden" | "unauthorized" => (403, "Forbidden"),
                    "too_large" => (413, "Too Large"),
                    _ => (503, "Unavailable"),
                };
                Native::plain(status, &format!("{reason} - {message}"), &message)
            }
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
    fn apts_paths_are_calls() {
        let (op, p) = fields("/debian/dists/trixie/InRelease").unwrap();
        assert_eq!((op, p), ("release", vec![("archive", "debian".into()), ("suite", "trixie".into())]));
        let hash = "a".repeat(64);
        let (op, p) = fields(&format!("/debian/dists/trixie/main/binary-arm64/by-hash/SHA256/{hash}")).unwrap();
        assert_eq!(op, "index");
        assert_eq!(p[3], ("arch", "arm64".into()));
        assert_eq!(p[4], ("sha256", hash));
        assert_eq!(fields("/debian/dists/trixie/main/binary-all/Packages.xz").unwrap().1[4], ("file", "Packages.xz".into()));
        let (op, p) = fields("/debian/pool/main/c/curl/libcurl4t64_8.14.1-2%2bdeb13u1_arm64.deb").unwrap();
        assert_eq!(op, "deb");
        assert_eq!(p, vec![("archive", "debian".into()), ("component", "main".into()), ("source", "curl".into()), ("name", "libcurl4t64".into()), ("version", "8.14.1-2+deb13u1".into()), ("arch", "arm64".into())]);
        assert!(fields("/debian/pool/main/libx/libxml2/libxml2_2.12.7+dfsg-3_arm64.deb").is_some());
        // The security archive's pool, whose `updates/` is the server's to know.
        let (_, p) = fields("/debian-security/pool/updates/main/j/jq/jq_1.7.1-6%2bdeb13u4_arm64.deb").unwrap();
        assert_eq!((p[0].1.as_str(), p[1].1.as_str(), p[4].1.as_str()), ("debian-security", "main", "1.7.1-6+deb13u4"));
    }

    #[test]
    fn everything_else_is_not_here() {
        for path in [
            "/debian/dists/trixie/Release",
            "/debian/dists/trixie/main/i18n/Translation-en",
            "/debian/dists/trixie/main/binary-arm64/Packages.bz2",
            "/debian/pool/main/x/curl/curl_8_arm64.deb",
            "/debian/pool/main/c/curl/curl_8_arm64.deb/x",
            "/debian/pool/main/c/curl/curl_8_arm64_extra.deb",
            "/debian/dists/a%2fb/InRelease",
            "/../../etc/passwd",
            "",
        ] {
            assert!(parse_path(path).is_none(), "{path}");
        }
    }

    #[test]
    fn calls_that_parse_still_meet_the_schema() {
        let ok = parse_path("/debian/pool/main/j/jq/jq_1.7.1-6_arm64.deb").unwrap();
        assert_eq!(ok.check(OPS), Ok(()));
        // A path apt could send, with fields no schema takes.
        for path in ["/Debian/dists/trixie/InRelease", "/debian/dists/trixie/main/binary-arm64/by-hash/SHA256/XYZ", "/debian/pool/main/j/jq/jq_v1_arm64.deb"] {
            assert!(parse_path(path).is_none_or(|c| c.check(OPS).is_err()), "{path}");
        }
    }

    #[test]
    fn refusals_say_why_in_the_status_line() {
        let call = Call::new(SERVICE, "deb");
        let n = AptConversation.reply(&call, Answer::refused("forbidden", "nmap isn't allowed"));
        assert_eq!((n.status, n.reason.as_str()), (403, "Forbidden - nmap isn't allowed"));
        assert_eq!(AptConversation.reply(&call, Answer::refused("unavailable", "down")).status, 503);
    }
}
