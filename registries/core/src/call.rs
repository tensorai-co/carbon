//! A call: what an adapter read a tool's request as, checked against its service's schema before anything is sent.

use serde_json::{Map, Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// One of an op's fields: its name, whether it must be there, and what its value must look like.
#[derive(Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    pub required: bool,
    pub check: fn(&str) -> bool,
}

impl Field {
    pub const fn required(name: &'static str, check: fn(&str) -> bool) -> Field {
        Field { name, required: true, check }
    }
    pub const fn optional(name: &'static str, check: fn(&str) -> bool) -> Field {
        Field { name, required: false, check }
    }
}

/// One of a service's ops, and its fields.
#[derive(Clone, Copy)]
pub struct Op {
    pub name: &'static str,
    pub fields: &'static [Field],
}

/// What's already known about a byte answer: the server may check it, and so may Carbon.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expect {
    pub sha256: Option<String>,
    pub sha512: Option<String>,
    pub size: Option<u64>,
}

/// A call, before it's sent: `apt deb {archive: debian, name: jq, …}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub service: &'static str,
    pub op: &'static str,
    pub params: Vec<(&'static str, String)>,
    pub expect: Option<Expect>,
    pub offset: u64,
}

impl Call {
    pub fn new(service: &'static str, op: &'static str) -> Call {
        Call { service, op, params: Vec::new(), expect: None, offset: 0 }
    }

    /// This, with a field.
    pub fn with(mut self, name: &'static str, value: impl Into<String>) -> Call {
        self.params.push((name, value.into()));
        self
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
    }

    /// Checks it against `ops`, its service's schema: the op is one of them, every field is one of the op's and
    /// passes its check, none twice, and none it needs is missing. A refusal says what's wrong.
    pub fn check(&self, ops: &[Op]) -> Result<(), String> {
        let op = ops.iter().find(|o| o.name == self.op).ok_or_else(|| format!("{} has no op {}", self.service, self.op))?;
        for (i, (name, value)) in self.params.iter().enumerate() {
            let field = op.fields.iter().find(|f| f.name == *name).ok_or_else(|| format!("{} {} has no field {name}", self.service, self.op))?;
            if self.params[..i].iter().any(|(n, _)| n == name) {
                return Err(format!("{name} is given twice"));
            }
            if !(field.check)(value) {
                return Err(format!("{name} {value:?} isn't one {} {} takes", self.service, self.op));
            }
        }
        if let Some(missing) = op.fields.iter().find(|f| f.required && self.param(f.name).is_none()) {
            return Err(format!("{} {} needs {}", self.service, self.op, missing.name));
        }
        Ok(())
    }

    /// The event the machine's hooks are asked about.
    pub fn event(&self) -> hooks::Registry {
        hooks::Registry { service: self.service.into(), op: self.op.into(), params: self.params.iter().map(|(n, v)| (n.to_string(), v.clone())).collect() }
    }

    /// The body that's sent: `{"id", "params", "expect", "offset", "meta"}`.
    pub fn body(&self, meta: &[(String, String)]) -> Value {
        let params: Map<String, Value> = self.params.iter().map(|(n, v)| (n.to_string(), Value::String(v.clone()))).collect();
        let mut body = json!({ "id": new_id(), "params": params });
        if let Some(e) = &self.expect {
            let mut expect = Map::new();
            if let Some(s) = &e.sha256 {
                expect.insert("sha256".into(), json!(s));
            }
            if let Some(s) = &e.sha512 {
                expect.insert("sha512".into(), json!(s));
            }
            if let Some(n) = e.size {
                expect.insert("size".into(), json!(n));
            }
            body["expect"] = Value::Object(expect);
        }
        if self.offset > 0 {
            body["offset"] = json!(self.offset);
        }
        if !meta.is_empty() {
            body["meta"] = Value::Object(meta.iter().map(|(k, v)| (k.clone(), json!(v))).collect());
        }
        body
    }
}

/// A call's id: the time in milliseconds and a count, in hex, so ids sort as they were made.
fn new_id() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    format!("{ms:012x}{:04x}{:06x}", std::process::id() & 0xFFFF, COUNT.fetch_add(1, Ordering::Relaxed) & 0xFF_FFFF)
}

/// Checks for fields' values, without a pattern library.
pub mod check {
    /// At most `max` bytes, the first a lowercase letter or digit, the rest those or `+ . -`: Debian's names.
    pub fn word(s: &str, max: usize) -> bool {
        !s.is_empty() && s.len() <= max && s.as_bytes()[0].is_ascii_alphanumeric() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b))
    }

    /// `n` lowercase hex digits.
    pub fn hex(s: &str, n: usize) -> bool {
        s.len() == n && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    /// At most `max` bytes of letters, digits and `extra`, starting with a digit or letter.
    pub fn token(s: &str, max: usize, extra: &[u8]) -> bool {
        !s.is_empty() && s.len() <= max && s.as_bytes()[0].is_ascii_alphanumeric() && s.bytes().all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPS: &[Op] = &[Op { name: "deb", fields: &[Field::required("name", |s| check::word(s, 100)), Field::optional("arch", |s| s == "arm64")] }];

    #[test]
    fn calls_are_checked_against_their_schema() {
        let ok = Call::new("apt", "deb").with("name", "jq");
        assert_eq!(ok.check(OPS), Ok(()));
        assert!(Call::new("apt", "deb").with("name", "jq").with("arch", "arm64").check(OPS).is_ok());
        for bad in [
            Call::new("apt", "upload").with("name", "jq"),
            Call::new("apt", "deb"),
            Call::new("apt", "deb").with("name", "../x"),
            Call::new("apt", "deb").with("name", "jq").with("url", "http://x"),
            Call::new("apt", "deb").with("name", "jq").with("name", "curl"),
            Call::new("apt", "deb").with("name", "jq").with("arch", "riscv64"),
        ] {
            assert!(bad.check(OPS).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_body_has_only_the_protocols_fields() {
        let body = Call::new("apt", "deb").with("name", "jq").body(&[("sid".into(), "8f3a".into())]);
        let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["id", "meta", "params"]);
        assert_eq!(body["params"], json!({ "name": "jq" }));
        assert_eq!(body["meta"], json!({ "sid": "8f3a" }));
        assert!(check::word("libc6", 10) && !check::word("Libc6", 10) && !check::word("", 10) && !check::word("-x", 10));
        assert!(check::hex(&"a".repeat(64), 64) && !check::hex(&"A".repeat(64), 64));
    }
}
