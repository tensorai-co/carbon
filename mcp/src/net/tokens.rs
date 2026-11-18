//! `--token-file`: static tokens, one per line, `<scopes> <sha256 of the token, hex> <name>`, with `#` comments. Only
//! hashes are on disk. The file is read again when it changes (its modified time), so revoking a token is deleting its
//! line.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::Scopes;

pub const MCP_SCOPE: &str = "carbon:mcp";
pub const ENGINE_SCOPE: &str = "carbon:engine";

/// A token's line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub scopes: Scopes,
    pub hash: [u8; 32],
    pub name: String,
}

/// Scopes as a line writes them: comma-separated.
pub fn parse_scopes(text: &str) -> Result<Scopes, String> {
    let mut scopes = Scopes::default();
    for s in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match s {
            MCP_SCOPE => scopes.mcp = true,
            ENGINE_SCOPE => scopes.engine = true,
            other => return Err(format!("unknown scope {other:?} (the scopes are {MCP_SCOPE} and {ENGINE_SCOPE})")),
        }
    }
    if scopes == Scopes::default() {
        return Err("no scopes".into());
    }
    Ok(scopes)
}

pub fn format_scopes(scopes: Scopes) -> String {
    let mut out = Vec::new();
    if scopes.mcp {
        out.push(MCP_SCOPE);
    }
    if scopes.engine {
        out.push(ENGINE_SCOPE);
    }
    out.join(",")
}

/// A token file's lines.
pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |why: String| format!("line {}: {why}", i + 1);
        let mut fields = line.split_whitespace();
        let (Some(scopes), Some(hash), Some(name)) = (fields.next(), fields.next(), fields.next()) else {
            return Err(at("want `<scopes> <sha256 hex> <name>`".into()));
        };
        if fields.next().is_some() {
            return Err(at("a name is one word".into()));
        }
        let scopes = parse_scopes(scopes).map_err(at)?;
        let hash = decode_hex(hash).ok_or_else(|| at("the hash isn't 64 hex digits".into()))?;
        entries.push(Entry { scopes, hash, name: name.to_string() });
    }
    Ok(entries)
}

fn decode_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The hash a line holds for a token.
pub fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.trim().as_bytes()).into()
}

/// Equal, in time that doesn't depend on where they differ.
fn same(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A new token, and the line that lets it in: what `carbon mcp token` prints.
pub fn new_token(scopes: Scopes, name: &str) -> Result<(String, String), String> {
    if name.is_empty() || name.split_whitespace().count() != 1 {
        return Err("a token's name is one word".into());
    }
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("no randomness: {e}"))?;
    let token = format!("carbon_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes));
    let line = format!("{} {} {name}", format_scopes(scopes), hex(&hash(&token)));
    Ok((token, line))
}

/// A token file, read again when it changes.
pub struct Tokens {
    path: PathBuf,
    state: Mutex<(Option<SystemTime>, Vec<Entry>)>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Tokens {
    pub fn load(path: &Path) -> Result<Tokens, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("--token-file {}: {e}", path.display()))?;
        let entries = parse(&text).map_err(|e| format!("--token-file {}: {e}", path.display()))?;
        Ok(Tokens { path: path.to_path_buf(), state: Mutex::new((modified(path), entries)) })
    }

    /// The scopes and name of a token, if a line lets it in.
    pub fn check(&self, token: &str) -> Option<(Scopes, String)> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = modified(&self.path);
        if now != state.0 {
            // A file that can't be read, or doesn't parse, lets no one in until it's fixed: a revocation half-written
            // shouldn't leave the old tokens working.
            let entries = match std::fs::read_to_string(&self.path).map_err(|e| e.to_string()).and_then(|t| parse(&t)) {
                Ok(entries) => entries,
                Err(e) => {
                    eprintln!("carbon mcp: --token-file {}: {e}; refusing every token until it's fixed", self.path.display());
                    Vec::new()
                }
            };
            *state = (now, entries);
        }
        let h = hash(token);
        let mut found = None;
        for e in &state.1 {
            if same(&e.hash, &h) && found.is_none() {
                found = Some((e.scopes, e.name.clone()));
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_scopes_and_new_tokens() {
        let (token, line) = new_token(Scopes::ALL, "ci-runner").unwrap();
        assert!(token.starts_with("carbon_"));
        let (t2, line2) = new_token(Scopes { mcp: true, engine: false }, "desktop").unwrap();
        assert!(line.starts_with("carbon:mcp,carbon:engine "));
        let text = format!("# scopes hash name\n\n{line}\n{line2}\n");
        let entries = parse(&text).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "ci-runner");
        assert_eq!(entries[1].scopes, Scopes { mcp: true, engine: false });

        let dir = std::env::temp_dir().join(format!("carbon-tokens-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tokens");
        std::fs::write(&file, &text).unwrap();
        let tokens = Tokens::load(&file).unwrap();
        assert_eq!(tokens.check(&token), Some((Scopes::ALL, "ci-runner".into())));
        assert_eq!(tokens.check(&t2).map(|t| t.0), Some(Scopes { mcp: true, engine: false }));
        assert_eq!(tokens.check("carbon_wrong"), None);

        // Revoking: the line's gone, and the file's read again.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, format!("{line2}\n")).unwrap();
        let f = std::fs::File::options().write(true).open(&file).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(1)).unwrap();
        assert_eq!(tokens.check(&token), None);
        assert!(tokens.check(&t2).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_lines() {
        assert!(parse("carbon:mcp abc name").is_err());
        assert!(parse(&format!("carbon:root {} name", "0".repeat(64))).is_err());
        assert!(parse(&format!("carbon:mcp {}", "0".repeat(64))).is_err());
        assert!(parse(&format!("carbon:mcp {} two words", "0".repeat(64))).is_err());
        assert!(new_token(Scopes::ALL, "two words").is_err());
    }
}
