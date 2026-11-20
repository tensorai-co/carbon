//! A call: what an adapter read a tool's request as, checked against its service's schema before anything is sent.

use serde_json::{Map, Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// One of an op's fields: its name, whether it must be there, and what its value must look like. A list field's
/// value is a list of items, each checked, at most `most` of them.
#[derive(Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    pub required: bool,
    pub check: fn(&str) -> bool,
    pub list: bool,
    pub most: usize,
}

impl Field {
    pub const fn required(name: &'static str, check: fn(&str) -> bool) -> Field {
        Field { name, required: true, check, list: false, most: 1 }
    }
    pub const fn optional(name: &'static str, check: fn(&str) -> bool) -> Field {
        Field { name, required: false, check, list: false, most: 1 }
    }
    /// A list of at most `most` items, each passing `check`; a required one has at least one.
    pub const fn list(name: &'static str, required: bool, most: usize, check: fn(&str) -> bool) -> Field {
        Field { name, required, check, list: true, most }
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

/// A call, before it's sent: `apt deb {archive: debian, name: jq, …}`. A list field's items are kept as one value,
/// separated by spaces (no item has one), so hooks and rules see them as text; they're sent as a JSON list.
#[derive(Clone, PartialEq, Eq)]
pub struct Call {
    pub service: &'static str,
    pub op: &'static str,
    pub params: Vec<(&'static str, String)>,
    /// Which params are lists.
    pub lists: Vec<&'static str>,
    pub expect: Option<Expect>,
    pub offset: u64,
    /// What's sent after the call's JSON, for an op that sends data (git's push: its pack).
    pub data: Option<Vec<u8>>,
}

impl std::fmt::Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Call")
            .field("service", &self.service)
            .field("op", &self.op)
            .field("params", &self.params)
            .field("expect", &self.expect)
            .field("data", &self.data.as_ref().map(Vec::len))
            .finish()
    }
}

impl Call {
    pub fn new(service: &'static str, op: &'static str) -> Call {
        Call { service, op, params: Vec::new(), lists: Vec::new(), expect: None, offset: 0, data: None }
    }

    /// This, with a field.
    pub fn with(mut self, name: &'static str, value: impl Into<String>) -> Call {
        self.params.push((name, value.into()));
        self
    }

    /// This, with a list field.
    pub fn with_list<I, S>(mut self, name: &'static str, items: I) -> Call
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let joined: Vec<String> = items.into_iter().map(|s| s.as_ref().to_string()).collect();
        self.params.push((name, joined.join(" ")));
        self.lists.push(name);
        self
    }

    /// This, with data after its JSON.
    pub fn with_data(mut self, data: Vec<u8>) -> Call {
        self.data = Some(data);
        self
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
    }

    /// A list field's items; none if it isn't there.
    pub fn items(&self, name: &str) -> Vec<&str> {
        self.param(name).map(|v| v.split(' ').filter(|s| !s.is_empty()).collect()).unwrap_or_default()
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
            if field.list != self.lists.contains(name) {
                return Err(format!("{name} is {}a list", if field.list { "" } else { "not " }));
            }
            if field.list {
                let items = self.items(name);
                if items.len() > field.most {
                    return Err(format!("{name} has {} items; {} {} takes at most {}", items.len(), self.service, self.op, field.most));
                }
                if field.required && items.is_empty() {
                    return Err(format!("{} {} needs at least one {name}", self.service, self.op));
                }
                if let Some(bad) = items.iter().find(|item| !(field.check)(item)) {
                    return Err(format!("{name} has {bad:?}, which isn't one {} {} takes", self.service, self.op));
                }
            } else if !(field.check)(value) {
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
        let params: Map<String, Value> = self
            .params
            .iter()
            .map(|(n, v)| {
                let value = if self.lists.contains(n) { Value::from(self.items(n)) } else { Value::String(v.clone()) };
                (n.to_string(), value)
            })
            .collect();
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
    fn list_fields() {
        const LISTS: &[Op] = &[Op { name: "fetch", fields: &[Field::list("wants", true, 2, |s| check::hex(s, 4)), Field::list("haves", false, 10, |s| check::hex(s, 4))] }];
        let ok = Call::new("git", "fetch").with_list("wants", ["abcd", "0123"]).with_list("haves", Vec::<String>::new());
        assert_eq!(ok.check(LISTS), Ok(()));
        assert_eq!(ok.body(&[])["params"], json!({ "wants": ["abcd", "0123"], "haves": [] }));
        for bad in [
            Call::new("git", "fetch").with_list("wants", Vec::<String>::new()),
            Call::new("git", "fetch").with_list("wants", ["abcd", "0123", "4567"]),
            Call::new("git", "fetch").with_list("wants", ["abcd", "xyz!"]),
            Call::new("git", "fetch").with("wants", "abcd"),
            Call::new("git", "fetch").with_list("wants", ["ab cd"]),
        ] {
            assert!(bad.check(LISTS).is_err(), "{bad:?}");
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
