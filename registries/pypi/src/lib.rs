//! `registries-pypi`: PyPI's simple index as registries' calls, for pip, uv and everything that reads it.
//!
//! | The tool asks for | The call |
//! | --- | --- |
//! | `/simple/{project}/` | `project {name}`, the name normalized as PEP 503 has it |
//! | `/files/{project}/{sha256}/{filename}` | `file {name, filename, sha256}` |
//!
//! Nothing else has a call: the whole index (`/simple/`), uploads, the JSON API, search. A project's page is passed
//! on as the server built it, but for each file's URL, which is rewritten to this registry with the file's SHA-256 in
//! it, so the tool never learns another host and every file's call expects its hash: Carbon checks the file as it
//! streams, and so does the server. The page is PEP 691's JSON, or PEP 503's HTML for a tool that doesn't ask for
//! JSON.

use std::sync::Arc;

use registries_core::{Adapter, Answer, Call, Conversation, Expect, Field, Native, Op, Request, check, decode_segment};
use serde_json::Value;

const SERVICE: &str = "pypi";

/// A project's name, as PEP 503 normalizes it: lowercase, runs of `-`, `_` and `.` as one `-`.
pub fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            if !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// A normalized name: lowercase letters and digits, single `-`s between them, at most 200.
pub fn is_project(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && !s.starts_with('-') && !s.ends_with('-') && !s.contains("--") && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A wheel's or an sdist's name.
pub fn is_file(s: &str) -> bool {
    [".whl", ".tar.gz", ".zip", ".tar.bz2"].iter().any(|ext| s.ends_with(ext)) && check::token(s, 255, b"._+!-") && !s.contains("..")
}

fn sha256(s: &str) -> bool {
    check::hex(s, 64)
}

static OPS: &[Op] = &[
    Op { name: "project", fields: &[Field::required("name", is_project)] },
    Op { name: "file", fields: &[Field::required("name", is_project), Field::required("filename", is_file), Field::required("sha256", sha256)] },
];

/// PyPI's adapter. It keeps nothing: each file's hash is in its URL.
pub struct Pypi;

impl Pypi {
    pub fn adapter() -> Arc<dyn Adapter> {
        Arc::new(Pypi)
    }
}

impl Adapter for Pypi {
    fn service(&self) -> &'static str {
        SERVICE
    }
    fn ops(&self) -> &'static [Op] {
        OPS
    }
    fn conversation(&self) -> Box<dyn Conversation> {
        Box::new(PypiConversation { host: "registry.carbon".into(), json: true })
    }
    fn env(&self, base: &str, _details: &Value) -> Vec<(String, String)> {
        let host = base.trim_start_matches("http://").split('/').next().unwrap_or_default().to_string();
        let index = format!("{base}simple/");
        vec![
            ("PIP_INDEX_URL".into(), index.clone()),
            // Plain HTTP, to Carbon in this process: pip wants it named to use it.
            ("PIP_TRUSTED_HOST".into(), host),
            ("PIP_DISABLE_PIP_VERSION_CHECK".into(), "1".into()),
            ("UV_DEFAULT_INDEX".into(), index),
        ]
    }
}

struct PypiConversation {
    host: String,
    /// The tool takes PEP 691's JSON (else PEP 503's HTML).
    json: bool,
}

/// The call PyPI's `path` (under `/pypi`) is, or None if it's nothing a call is made for.
pub fn parse_path(path: &str) -> Option<Call> {
    let segments: Vec<String> = path.strip_prefix('/')?.trim_end_matches('/').split('/').map(decode_segment).collect::<Option<_>>()?;
    let s: Vec<&str> = segments.iter().map(String::as_str).collect();
    match s.as_slice() {
        ["simple", project] => Some(Call::new(SERVICE, "project").with("name", normalize(project))),
        ["files", project, sum, filename] => {
            Some(Call::new(SERVICE, "file").with("name", normalize(project)).with("filename", *filename).with("sha256", *sum))
        }
        _ => None,
    }
}

/// A `Host` header that's a name or an address with a port, and nothing else.
fn good_host(h: &str) -> bool {
    !h.is_empty() && h.len() <= 253 && h.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// PEP 503's HTML for a page, for a tool that doesn't take JSON.
fn html(name: &str, files: &[Value]) -> String {
    let mut out = format!("<!DOCTYPE html>\n<html><head><meta name=\"pypi:repository-version\" content=\"1.1\"><title>Links for {}</title></head><body><h1>Links for {}</h1>\n", escape(name), escape(name));
    for f in files {
        let (Some(filename), Some(url)) = (f.get("filename").and_then(Value::as_str), f.get("url").and_then(Value::as_str)) else { continue };
        let sum = f.pointer("/hashes/sha256").and_then(Value::as_str).unwrap_or_default();
        let mut attrs = String::new();
        if let Some(rp) = f.get("requires-python").and_then(Value::as_str) {
            attrs.push_str(&format!(" data-requires-python=\"{}\"", escape(rp)));
        }
        match f.get("yanked") {
            Some(Value::Bool(true)) => attrs.push_str(" data-yanked=\"\""),
            Some(Value::String(why)) => attrs.push_str(&format!(" data-yanked=\"{}\"", escape(why))),
            _ => {}
        }
        out.push_str(&format!("<a href=\"{}#sha256={sum}\"{attrs}>{}</a><br/>\n", escape(url), escape(filename)));
    }
    out.push_str("</body></html>\n");
    out
}

impl Conversation for PypiConversation {
    fn parse(&mut self, req: &Request, path: &str) -> Result<Call, Native> {
        if req.method != "GET" && req.method != "HEAD" {
            return Err(Native::plain(405, "Method Not Allowed", "this index only reads").header("Allow", "GET, HEAD"));
        }
        if let Some(host) = req.header("Host").filter(|h| good_host(h)) {
            self.host = host.to_string();
        }
        self.json = req.header("Accept").is_some_and(|a| a.contains("application/vnd.pypi.simple.v1+json"));
        let mut call = parse_path(path).ok_or_else(|| Native::plain(404, "Not Found", "not something pip is answered with here"))?;
        if call.op == "file" {
            call.expect = Some(Expect { sha256: call.param("sha256").map(String::from), ..Expect::default() });
        }
        Ok(call)
    }

    fn reply(&mut self, call: &Call, answer: Answer) -> Native {
        let name = call.param("name").unwrap_or_default().to_string();
        match (call.op, answer) {
            ("project", Answer::Json(mut page)) => {
                if page.get("name").and_then(Value::as_str) != Some(name.as_str()) {
                    return Native::plain(502, "Bad Gateway", &format!("the package server answered {name} with another project"));
                }
                if let Some(files) = page.get_mut("files").and_then(Value::as_array_mut) {
                    files.retain_mut(|f| {
                        let (Some(filename), Some(sum)) = (f.get("filename").and_then(Value::as_str), f.pointer("/hashes/sha256").and_then(Value::as_str)) else { return false };
                        if !is_file(filename) || !sha256(sum) {
                            return false;
                        }
                        let url = format!("http://{}/pypi/files/{name}/{sum}/{filename}", self.host);
                        f["url"] = Value::String(url);
                        true
                    });
                }
                if self.json {
                    Native::new(200, "OK").header("Content-Type", "application/vnd.pypi.simple.v1+json").bytes(page.to_string())
                } else {
                    let files = page.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
                    Native::new(200, "OK").header("Content-Type", "text/html; charset=utf-8").bytes(html(&name, &files))
                }
            }
            ("file", Answer::Bytes { reader, size, .. }) => Native::new(200, "OK").header("Content-Type", "application/octet-stream").stream(reader, size),
            (_, Answer::Refused { code, message }) => {
                let (status, reason) = match code.as_str() {
                    "not_found" | "invalid" => (404, "Not Found"),
                    "forbidden" | "unauthorized" => (403, "Forbidden"),
                    "too_large" => (413, "Too Large"),
                    _ => (503, "Unavailable"),
                };
                Native::plain(status, &format!("{reason} - {message}"), &message)
            }
            (op, _) => Native::plain(502, "Bad Gateway", &format!("the package server answered pypi {op} with the wrong kind of answer")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(path: &str) -> Option<(&'static str, Vec<(&'static str, String)>)> {
        parse_path(path).map(|c| (c.op, c.params))
    }

    #[test]
    fn pips_paths_are_calls() {
        assert_eq!(fields("/simple/requests/"), Some(("project", vec![("name", "requests".into())])));
        assert_eq!(fields("/simple/Zope.Interface"), Some(("project", vec![("name", "zope-interface".into())])));
        let sum = "a".repeat(64);
        let (op, p) = fields(&format!("/files/requests/{sum}/requests-2.31.0-py3-none-any.whl")).unwrap();
        assert_eq!(op, "file");
        assert_eq!(p, vec![("name", "requests".into()), ("filename", "requests-2.31.0-py3-none-any.whl".into()), ("sha256", sum)]);
        for path in ["/simple/", "/simple", "/pypi/requests/json", "/files/requests/x", "/search?q=x", "/legacy/", ""] {
            assert!(parse_path(path).is_none(), "{path}");
        }
    }

    #[test]
    fn calls_that_parse_still_meet_the_schema() {
        for path in ["/files/requests/XYZ/requests-2.31.0-py3-none-any.whl", "/files/requests/{}/run.exe", "/simple/-x-"] {
            let path = path.replace("{}", &"a".repeat(64));
            assert!(parse_path(&path).is_none_or(|c| c.check(OPS).is_err()), "{path}");
        }
        assert!(is_project("a") && !is_project("A") && !is_project("a--b") && is_file("zope.interface-6.0.tar.gz") && !is_file("a..b.whl"));
        assert_eq!(normalize("typing__Extensions"), "typing-extensions");
    }

    #[test]
    fn pages_point_at_this_registry_and_files_expect_their_hash() {
        let sum = "b".repeat(64);
        let req = Request { method: "GET".into(), path: "/pypi/simple/requests/".into(), query: String::new(), headers: vec![("Host".into(), "registry.carbon".into()), ("Accept".into(), "application/vnd.pypi.simple.v1+json, text/html;q=0.01".into())], body: Vec::new(), keep_alive: true };
        let mut conv = Pypi.conversation();
        let call = conv.parse(&req, "/simple/requests/").unwrap();
        let page = json!({ "meta": {"api-version": "1.1"}, "name": "requests", "files": [
            { "filename": "requests-2.31.0-py3-none-any.whl", "url": "https://files.pythonhosted.org/packages/x/requests-2.31.0-py3-none-any.whl", "hashes": {"sha256": sum}, "requires-python": ">=3.7" },
            { "filename": "bad name.whl", "url": "https://x", "hashes": {"sha256": sum} },
        ] });
        let native = conv.reply(&call, Answer::Json(page.clone()));
        let registries_core::Body::Bytes(body) = native.body else { panic!() };
        let back: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(back["files"].as_array().unwrap().len(), 1);
        assert_eq!(back["files"][0]["url"], format!("http://registry.carbon/pypi/files/requests/{sum}/requests-2.31.0-py3-none-any.whl"));
        let file = conv.parse(&req, &format!("/files/requests/{sum}/requests-2.31.0-py3-none-any.whl")).unwrap();
        assert_eq!(file.expect.and_then(|e| e.sha256), Some(sum.clone()));
        // A tool that doesn't take JSON gets PEP 503's HTML, its hash in the link.
        let old = Request { headers: vec![("Accept".into(), "text/html".into())], ..req };
        let call = conv.parse(&old, "/simple/requests/").unwrap();
        let registries_core::Body::Bytes(body) = conv.reply(&call, Answer::Json(page)).body else { panic!() };
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains(&format!("/pypi/files/requests/{sum}/requests-2.31.0-py3-none-any.whl#sha256={sum}\" data-requires-python=\"&gt;=3.7\"")), "{text}");
        assert!(Pypi.env("http://registry.carbon/pypi/", &Value::Null).contains(&("PIP_INDEX_URL".into(), "http://registry.carbon/pypi/simple/".into())));
    }
}
