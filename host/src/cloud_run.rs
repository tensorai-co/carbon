//! Calls to private Cloud Run services from a Carbon that runs on Google Cloud: with
//! `CARBON_CLOUD_RUN_ID_TOKENS=1`, every request this host sends to an `https://….run.app` server carries a Google ID
//! token for that server in `X-Serverless-Authorization`, which Cloud Run's IAM check reads, leaving `Authorization`
//! to whatever the call already sends (a registries token, a registry's own bearer).
//!
//! The tokens are the host's own service account's, from the metadata server, and never go in a machine. Each is for
//! one server (its audience is the server's origin), so one sent to a server can't be used at any other; they're kept
//! until five minutes before they expire. `GCE_METADATA_HOST` names another metadata server, as Google's libraries
//! take it.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The header Cloud Run's IAM check reads before `Authorization`.
pub const HEADER: &str = "X-Serverless-Authorization";

/// Whether this host adds ID tokens: `CARBON_CLOUD_RUN_ID_TOKENS=1`.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| matches!(std::env::var("CARBON_CLOUD_RUN_ID_TOKENS").as_deref(), Ok("1" | "true")))
}

/// The audience a token for `url` needs: its origin, if it's an `https://….run.app` server.
pub fn audience(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.contains('@') {
        return None;
    }
    let host = authority.rsplit_once(':').filter(|(_, p)| p.parse::<u16>().is_ok()).map_or(authority, |(h, _)| h);
    let host = host.to_ascii_lowercase();
    let label = host.strip_suffix(".run.app")?;
    (!label.is_empty() && !label.ends_with('.')).then(|| format!("https://{host}"))
}

/// A token for `audience`, kept or from the metadata server.
pub fn id_token(audience: &str) -> Result<String, String> {
    static KEPT: Mutex<Vec<(String, String, u64)>> = Mutex::new(Vec::new());
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Some((_, token, _)) = KEPT.lock().unwrap().iter().find(|(a, _, exp)| a == audience && *exp > now + 300) {
        return Ok(token.clone());
    }
    let token = fetch(audience)?;
    let exp = expiry(&token).ok_or("the metadata server's ID token has no expiry")?;
    let mut kept = KEPT.lock().unwrap();
    kept.retain(|(a, _, exp)| a != audience && *exp > now);
    kept.push((audience.to_string(), token.clone(), exp));
    Ok(token)
}

fn fetch(audience: &str) -> Result<String, String> {
    let host = std::env::var("GCE_METADATA_HOST").unwrap_or_else(|_| "169.254.169.254".into());
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).redirects(0).build();
    let response = agent
        .get(&format!("http://{host}/computeMetadata/v1/instance/service-accounts/default/identity"))
        .query("audience", audience)
        .set("Metadata-Flavor", "Google")
        .call()
        .map_err(|e| format!("an ID token for {audience} from the metadata server: {e}"))?;
    let token = response.into_string().map_err(|e| format!("an ID token for {audience}: {e}"))?.trim().to_string();
    if token.split('.').count() != 3 {
        return Err(format!("an ID token for {audience}: the metadata server's answer isn't a JWT"));
    }
    Ok(token)
}

/// A JWT's `exp`, unchecked: only for knowing when to ask again.
fn expiry(jwt: &str) -> Option<u64> {
    let payload = base64url(jwt.split('.').nth(1)?)?;
    let text = String::from_utf8(payload).ok()?;
    let at = text.find("\"exp\"")? + 5;
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start();
    rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())].parse().ok()
}

fn base64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes().take_while(|&c| c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Adds the token to each request an agent sends to a `….run.app` server, when [`enabled`]. A token that can't be had
/// fails the request, so the reason is said rather than Cloud Run's bare 403.
pub struct Invoker;

impl ureq::Middleware for Invoker {
    fn handle(&self, request: ureq::Request, next: ureq::MiddlewareNext) -> Result<ureq::Response, ureq::Error> {
        match audience(request.url()).filter(|_| enabled()) {
            Some(audience) if request.header(HEADER).is_none() => match id_token(&audience) {
                Ok(token) => next.handle(request.set(HEADER, &format!("Bearer {token}"))),
                Err(e) => Err(ureq::Error::from(std::io::Error::other(e))),
            },
            _ => next.handle(request),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::sync::Arc;

    #[test]
    fn audiences() {
        let a = |u| audience(u);
        assert_eq!(a("https://packages-6738.us-central1.run.app/v1/apt/deb").as_deref(), Some("https://packages-6738.us-central1.run.app"));
        assert_eq!(a("https://Reg-x.a.RUN.app:443/v2/").as_deref(), Some("https://reg-x.a.run.app"));
        assert_eq!(a("https://reg.a.run.app?x").as_deref(), Some("https://reg.a.run.app"));
        assert_eq!(a("http://reg.a.run.app/"), None);
        assert_eq!(a("https://run.app/"), None);
        assert_eq!(a("https://evil.run.app.example.com/"), None);
        assert_eq!(a("https://x.run.app@evil.com/"), None);
        assert_eq!(a("https://registry-1.docker.io/v2/"), None);
    }

    #[test]
    fn expiries() {
        // {"aud":"a","exp":1764000000}
        assert_eq!(expiry("e30.eyJhdWQiOiJhIiwiZXhwIjoxNzY0MDAwMDAwfQ.sig"), Some(1764000000));
        assert_eq!(expiry("e30.e30.sig"), None);
        assert_eq!(expiry("garbage"), None);
    }

    /// Against a fake metadata server: the header goes to `https://….run.app` only, and the token is asked for once.
    /// A second middleware sees what would be sent, so nothing goes out.
    #[test]
    fn adds_the_header() {
        let metadata = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        // SAFETY: set before anything in this test process reads them, and no other test reads them.
        unsafe {
            std::env::set_var("GCE_METADATA_HOST", metadata.local_addr().unwrap().to_string());
            std::env::set_var("CARBON_CLOUD_RUN_ID_TOKENS", "1");
        }
        // {"exp":4102444800}: 2100.
        let token = "e30.eyJleHAiOjQxMDI0NDQ4MDB9.sig";
        let asked = std::thread::spawn(move || {
            let mut heads = Vec::new();
            // Once: the second request to the same server uses the kept token.
            for _ in 0..1 {
                let (mut s, _) = metadata.accept().unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    head.push_str(&line);
                }
                write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{token}", token.len()).unwrap();
                heads.push(head);
            }
            heads
        });

        struct Seen(Arc<Mutex<Vec<Option<String>>>>);
        impl ureq::Middleware for Seen {
            fn handle(&self, request: ureq::Request, _: ureq::MiddlewareNext) -> Result<ureq::Response, ureq::Error> {
                self.0.lock().unwrap().push(request.header(HEADER).map(String::from));
                Err(ureq::Error::from(std::io::Error::other("seen")))
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let agent = ureq::AgentBuilder::new().middleware(Invoker).middleware(Seen(seen.clone())).build();
        for url in ["https://svc-1.a.run.app/v1/apt/deb", "https://svc-1.a.run.app/v2/", "https://registry-1.docker.io/v2/", "http://svc-1.a.run.app/"] {
            let _ = agent.get(url).call();
        }
        let bearer = Some(format!("Bearer {token}"));
        assert_eq!(*seen.lock().unwrap(), [bearer.clone(), bearer, None, None]);
        let heads = asked.join().unwrap();
        assert!(heads[0].contains("audience=https%3A%2F%2Fsvc-1.a.run.app"), "{heads:?}");
        assert!(heads[0].to_ascii_lowercase().contains("metadata-flavor: google"), "{heads:?}");
    }
}
