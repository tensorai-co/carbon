//! Logins to registries: who to be to each, for pulls that want it and every push.
//!
//! Carbon keeps its own in `<home>/auth.json`, written only by its owner (mode 600), in the form Docker's
//! `config.json` has: `{"auths": {"docker.io": {"auth": "<base64 of user:secret>"}}}`. A registry Carbon has no login
//! for is looked for in Docker's (`$DOCKER_CONFIG/config.json`, else `~/.docker/config.json`): its credential helper
//! for that registry, or its store (`docker-credential-osxkeychain` and the like, asked as Docker asks them), else its
//! `auths`. So a host that has run `docker login` needs no `carbon login`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Who to be to a registry.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub secret: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").field("user", &self.user).field("secret", &"…").finish()
    }
}

impl Credentials {
    /// `Authorization: Basic …`'s value.
    pub(crate) fn basic(&self) -> String {
        format!("Basic {}", base64(format!("{}:{}", self.user, self.secret).as_bytes()))
    }
}

/// A registry's name as logins are kept by: `docker.io` for Docker Hub however it's written, else its host.
pub fn registry_key(registry: &str) -> String {
    let host = registry.trim_start_matches("https://").trim_start_matches("http://");
    let host = host.split('/').next().unwrap_or(host);
    match host {
        "" | "docker.io" | "index.docker.io" | "registry-1.docker.io" | "registry.hub.docker.com" => "docker.io".into(),
        other => other.to_string(),
    }
}

/// The names Docker's `config.json` might keep a registry's login under.
fn docker_keys(key: &str) -> Vec<String> {
    if key == "docker.io" {
        vec!["https://index.docker.io/v1/".into(), "index.docker.io".into(), "docker.io".into(), "registry-1.docker.io".into()]
    } else {
        vec![key.into(), format!("https://{key}"), format!("https://{key}/"), format!("http://{key}")]
    }
}

fn carbon_file(home: &Path) -> PathBuf {
    home.join("auth.json")
}

fn docker_file() -> Option<PathBuf> {
    match std::env::var_os("DOCKER_CONFIG") {
        Some(dir) => Some(PathBuf::from(dir).join("config.json")),
        None => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".docker").join("config.json")),
    }
}

fn read_json(path: &Path) -> Value {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}))
}

/// An `auths` entry's login: its `auth` (base64 of `user:secret`), or its `username` and `password`.
fn from_entry(v: &Value) -> Option<Credentials> {
    if let Some(auth) = v.get("auth").and_then(Value::as_str).filter(|a| !a.is_empty()) {
        let text = String::from_utf8(unbase64(auth)?).ok()?;
        let (user, secret) = text.split_once(':')?;
        return Some(Credentials { user: user.into(), secret: secret.into() });
    }
    let user = v.get("username").and_then(Value::as_str)?;
    let secret = v.get("password").and_then(Value::as_str)?;
    Some(Credentials { user: user.into(), secret: secret.into() })
}

/// The login for `registry`: Carbon's, else Docker's. None to go without.
pub fn credentials(home: &Path, registry: &str) -> Option<Credentials> {
    let key = registry_key(registry);
    if let Some(c) = read_json(&carbon_file(home)).get("auths").and_then(|a| a.get(&key)).and_then(from_entry) {
        return Some(c);
    }
    let docker = read_json(&docker_file()?);
    let keys = docker_keys(&key);
    let helper = keys
        .iter()
        .find_map(|k| docker.get("credHelpers").and_then(|h| h.get(k)).and_then(Value::as_str))
        .or_else(|| docker.get("credsStore").and_then(Value::as_str));
    if let Some(helper) = helper {
        for k in &keys {
            if let Some(c) = from_helper(helper, k) {
                return Some(c);
            }
        }
    }
    keys.iter().find_map(|k| docker.get("auths").and_then(|a| a.get(k)).and_then(from_entry))
}

/// Asks a Docker credential helper (`docker-credential-NAME get`, the server on its stdin) for a login.
fn from_helper(helper: &str, server: &str) -> Option<Credentials> {
    if !helper.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return None;
    }
    let mut child = std::process::Command::new(format!("docker-credential-{helper}"))
        .arg("get")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(server.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let user = v.get("Username").and_then(Value::as_str)?;
    let secret = v.get("Secret").and_then(Value::as_str)?;
    // A helper's "<token>" user means the secret is an identity token, which this doesn't speak.
    (user != "<token>" && !secret.is_empty()).then(|| Credentials { user: user.into(), secret: secret.into() })
}

/// Keeps a login in Carbon's file, readable by its owner alone.
pub fn store(home: &Path, registry: &str, credentials: &Credentials) -> Result<(), String> {
    let path = carbon_file(home);
    let mut v = read_json(&path);
    if !v.get("auths").is_some_and(Value::is_object) {
        v["auths"] = json!({});
    }
    v["auths"][registry_key(registry)] = json!({ "auth": base64(format!("{}:{}", credentials.user, credentials.secret).as_bytes()) });
    write_private(&path, &serde_json::to_vec_pretty(&v).unwrap_or_default())
}

/// Forgets Carbon's login for a registry; whether it had one. Docker's own are left alone.
pub fn forget(home: &Path, registry: &str) -> Result<bool, String> {
    let path = carbon_file(home);
    let mut v = read_json(&path);
    let had = v.get_mut("auths").and_then(Value::as_object_mut).and_then(|a| a.remove(&registry_key(registry))).is_some();
    if had {
        write_private(&path, &serde_json::to_vec_pretty(&v).unwrap_or_default())?;
    }
    Ok(had)
}

/// Writes a file only its owner can read, by rename, so no one ever sees it half written or open to others.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("json.new");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|e| format!("{}: {e}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
    file.write_all(bytes).map_err(|e| format!("{}: {e}", temporary.display()))?;
    drop(file);
    fs::rename(&temporary, path).map_err(|e| format!("{}: {e}", path.display()))
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub(crate) fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut n, mut bits) = (0u32, 0);
    for b in text.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=') {
        let v = match b {
            b'-' => 62,
            b'_' => 63,
            _ => ALPHABET.iter().position(|&a| a == b)? as u32,
        };
        n = n << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
            n &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_both_ways() {
        for (plain, coded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("user:pa:ss", "dXNlcjpwYTpzcw==")] {
            assert_eq!(base64(plain.as_bytes()), coded);
            assert_eq!(unbase64(coded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn registries_by_name() {
        for name in ["docker.io", "index.docker.io", "https://index.docker.io/v1/", "registry-1.docker.io", ""] {
            assert_eq!(registry_key(name), "docker.io", "{name}");
        }
        assert_eq!(registry_key("ghcr.io"), "ghcr.io");
        assert_eq!(registry_key("https://localhost:5000/"), "localhost:5000");
    }

    #[test]
    fn kept_private_found_and_forgotten() {
        let home = std::env::temp_dir().join(format!("carbon-auth-test-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        let login = Credentials { user: "someone".into(), secret: "s3cret:with:colons".into() };
        store(&home, "https://index.docker.io/v1/", &login).unwrap();
        assert_eq!(credentials(&home, "docker.io"), Some(login.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(carbon_file(&home)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(forget(&home, "docker.io").unwrap());
        assert!(!forget(&home, "docker.io").unwrap());
        assert!(read_json(&carbon_file(&home))["auths"].as_object().unwrap().is_empty());
        fs::remove_dir_all(&home).unwrap();
    }
}
