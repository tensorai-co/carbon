//! Results for a model, a tool's arguments, and host paths.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// How much of one output stream comes back whole; past it, its head and its tail.
pub(crate) const OUTPUT_CAP: usize = 64 << 10;
const HEAD: usize = 8 << 10;

/// A tool's failure, as the model reads it: a result with `isError`, its message from the library.
#[derive(Debug)]
pub(crate) struct Failure(pub String);

impl<E: std::fmt::Display> From<E> for Failure {
    fn from(e: E) -> Failure {
        Failure(e.to_string())
    }
}

pub(crate) type Outcome = Result<Value, Failure>;

/// A result of text, and the same as structured content.
pub(crate) fn text_and(text: impl Into<String>, structured: Value) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "structuredContent": structured, "isError": false })
}

pub(crate) fn text(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": false })
}

/// Structured content, with its JSON as the text too, as the spec asks.
pub(crate) fn data(value: Value) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    json!({ "content": [{ "type": "text", "text": text }], "structuredContent": value, "isError": false })
}

pub(crate) fn failed(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

/// The first `n` characters of `s`.
pub(crate) fn first_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// An output stream as it comes back: whole, or its head and tail with what was cut said between; and whether it
/// was cut.
pub(crate) fn clip(bytes: &[u8], cap: usize) -> (String, bool) {
    if bytes.len() <= cap {
        return (String::from_utf8_lossy(bytes).into_owned(), false);
    }
    let head = HEAD.min(cap / 4);
    let tail = cap - head;
    let mut a = head;
    while a > 0 && (bytes[a] & 0xC0) == 0x80 {
        a -= 1;
    }
    let mut b = bytes.len() - tail;
    while b < bytes.len() && (bytes[b] & 0xC0) == 0x80 {
        b += 1;
    }
    let cut = b - a;
    let text = format!("{}\n[… {cut} bytes cut …]\n{}", String::from_utf8_lossy(&bytes[..a]), String::from_utf8_lossy(&bytes[b..]));
    (text, true)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

pub(crate) fn unbase64(text: &str) -> Result<Vec<u8>, Failure> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut n, mut bits) = (0u32, 0);
    for b in text.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=') {
        let v = ALPHABET.iter().position(|&a| a == b).ok_or_else(|| Failure("content_base64 isn't base64".into()))? as u32;
        n = n << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
            n &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// A time as RFC 3339, to the second.
pub(crate) fn time(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = secs / 86_400;
    let (h, m, s) = (secs % 86_400 / 3600, secs % 3600 / 60, secs % 60);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// A short id, for a machine or a process the server makes: hex of the time, the process and a count.
pub(crate) fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut x = now ^ u64::from(std::process::id()).rotate_left(32) ^ NEXT.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    // A mix, so ids made together don't look alike.
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    format!("{:08x}", x as u32)
}

/// `~/…` as the user's home.
pub(crate) fn expand_home(path: &Path) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix("~/")) {
        Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or_else(|| path.to_path_buf()),
        None => path.to_path_buf(),
    }
}

/// A host path a tool named, if it's under one of `dirs` once its links are resolved: for something there (`exists`),
/// itself; for something to be made, its folder. Else why not.
pub(crate) fn host_path(dirs: &[PathBuf], path: &str, exists: bool) -> Result<PathBuf, Failure> {
    if dirs.is_empty() {
        return Err(Failure("this server was given no host folders (carbon mcp --host-dir), so no tool reaches the host's files".into()));
    }
    let path = expand_home(Path::new(path));
    if !path.is_absolute() {
        return Err(Failure(format!("{}: a host path must be absolute", path.display())));
    }
    let real = if exists {
        std::fs::canonicalize(&path).map_err(|e| Failure(format!("{}: {e}", path.display())))?
    } else {
        let parent = path.parent().ok_or_else(|| Failure(format!("{}: no folder to put it in", path.display())))?;
        let name = path.file_name().ok_or_else(|| Failure(format!("{}: no name", path.display())))?;
        std::fs::canonicalize(parent).map_err(|e| Failure(format!("{}: {e}", parent.display())))?.join(name)
    };
    if dirs.iter().any(|d| real.starts_with(d)) {
        Ok(real)
    } else {
        let shared: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        Err(Failure(format!("{} isn't under a host folder this server shares ({})", path.display(), shared.join(", "))))
    }
}

/// A tool's arguments.
pub(crate) struct Args<'a>(pub &'a Value);

impl Args<'_> {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }

    pub fn str(&self, key: &str) -> Result<String, Failure> {
        self.opt_str(key).ok_or_else(|| Failure(format!("`{key}` is needed")))
    }

    pub fn opt_str(&self, key: &str) -> Option<String> {
        self.get(key).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    }

    pub fn u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64)).or_else(|| v.as_str()?.parse().ok()))
    }

    pub fn i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).or_else(|| v.as_str()?.parse().ok()))
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
    }

    pub fn bool(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| v.as_bool().unwrap_or_else(|| v.as_str() == Some("true")))
    }

    /// A list of strings: an array, or one string.
    pub fn strings(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        }
    }

    /// Environment variables: an object, or a list of `NAME=VALUE`.
    pub fn env(&self, key: &str) -> Vec<(String, String)> {
        match self.get(key) {
            Some(Value::Object(o)) => o.iter().map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect(),
            Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str()?.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect(),
            _ => Vec::new(),
        }
    }

    /// `timeout_secs`, at most `most`, else `default`.
    pub fn timeout(&self, default: Duration, most: Duration) -> Duration {
        self.u64("timeout_secs").map_or(default, Duration::from_secs).min(most).max(Duration::from_secs(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_clipped_head_and_tail() {
        let short = b"hello\n";
        assert_eq!(clip(short, 64), ("hello\n".into(), false));
        let long: Vec<u8> = (0..10_000).map(|i| b'a' + (i % 26) as u8).collect();
        let (text, cut) = clip(&long, 1000);
        assert!(cut && text.contains("bytes cut"));
        assert!(text.starts_with("abcdefghij") && text.len() < 1100);
        // A multi-byte character isn't split.
        let wide = "é".repeat(2000);
        let (text, _) = clip(wide.as_bytes(), 1001);
        assert!(!text.contains('\u{fffd}'));
    }

    #[test]
    fn base64_both_ways() {
        for s in ["", "f", "fo", "foo", "\u{1F600} bytes\0\u{ff}"] {
            assert_eq!(unbase64(&base64(s.as_bytes())).unwrap(), s.as_bytes());
        }
    }

    #[test]
    fn host_paths_stay_under_their_folders() {
        let root = std::env::temp_dir().join(format!("carbon-mcp-paths-{}", std::process::id()));
        let shared = root.join("shared");
        std::fs::create_dir_all(shared.join("in")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        let dirs = vec![std::fs::canonicalize(&shared).unwrap()];
        let at = |p: &Path| p.to_str().unwrap().to_string();
        assert!(host_path(&dirs, &at(&shared.join("in")), true).is_ok());
        assert!(host_path(&dirs, &at(&shared.join("new.tar")), false).is_ok(), "a file to be made, in a shared folder");
        assert!(host_path(&dirs, &at(&root.join("outside")), true).is_err());
        assert!(host_path(&dirs, &at(&shared.join("../outside")), true).is_err(), "`..` out of it");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("outside"), shared.join("escape")).unwrap();
            assert!(host_path(&dirs, &at(&shared.join("escape")), true).is_err(), "a link out of it");
        }
        assert!(host_path(&[], "/tmp", true).is_err(), "no folders, no paths");
        assert!(host_path(&dirs, "relative/path", true).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
