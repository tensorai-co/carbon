//! What a machine's registries are: which services it gets, the server each is sent to, the token sent with every
//! call, and `meta`. Set by whoever starts the machine (`--registries`, the Engine API), never by the machine.

use serde_json::{Value, json};

/// A machine's registries.
///
/// ```
/// let r = registries_core::Registries::new(["apt"]).upstream("https://pkg.example.com").unwrap().meta("sid", "8f3a");
/// assert_eq!(r.upstream_for("apt"), Some("https://pkg.example.com"));
/// ```
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Registries {
    pub services: Vec<String>,
    /// The server every service is sent to, unless it has its own.
    pub upstream: Option<String>,
    /// A service's own server: `npm` → `https://npm.example.com`.
    pub upstreams: Vec<(String, String)>,
    /// Sent as `Authorization: Bearer` with every call, to every server.
    pub token: Option<String>,
    pub meta: Vec<(String, String)>,
}

impl std::fmt::Debug for Registries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registries")
            .field("services", &self.services)
            .field("upstream", &self.upstream)
            .field("upstreams", &self.upstreams)
            .field("token", &self.token.as_ref().map(|_| "…"))
            .field("meta", &self.meta)
            .finish()
    }
}

impl Registries {
    pub fn new<I, S>(services: I) -> Registries
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Registries { services: services.into_iter().map(Into::into).collect(), ..Registries::default() }
    }

    /// The server for every service: `https://…`, or `http://` to this host's loopback only, since the token goes
    /// with every call.
    pub fn upstream(mut self, url: &str) -> Result<Registries, String> {
        self.upstream = Some(check_url(url)?);
        Ok(self)
    }

    /// A service's own server.
    pub fn upstream_for_service(mut self, service: &str, url: &str) -> Result<Registries, String> {
        let url = check_url(url)?;
        self.upstreams.retain(|(s, _)| s != service);
        self.upstreams.push((service.to_string(), url));
        Ok(self)
    }

    /// `URL` or `SERVICE=URL`, as `--upstream` takes it.
    pub fn upstream_flag(self, text: &str) -> Result<Registries, String> {
        match text.split_once('=') {
            Some((service, url)) if !service.contains('/') => self.upstream_for_service(service, url),
            _ => self.upstream(text),
        }
    }

    pub fn token(mut self, token: impl Into<String>) -> Registries {
        self.token = Some(token.into());
        self
    }

    pub fn meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Registries {
        let key = key.into();
        self.meta.retain(|(k, _)| *k != key);
        self.meta.push((key, value.into()));
        self
    }

    /// `K=V`, as `--registry-meta` takes it.
    pub fn meta_flag(self, text: &str) -> Result<Registries, String> {
        let (k, v) = text.split_once('=').filter(|(k, _)| !k.is_empty()).ok_or_else(|| format!("--registry-meta {text}: KEY=VALUE"))?;
        Ok(self.meta(k, v))
    }

    /// A comma-separated list, as `--registries` takes it: `apt,npm,git`.
    pub fn services_flag(mut self, text: &str) -> Registries {
        for s in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if !self.services.iter().any(|have| have == s) {
                self.services.push(s.to_string());
            }
        }
        self
    }

    /// The server `service`'s calls go to.
    pub fn upstream_for(&self, service: &str) -> Option<&str> {
        self.upstreams.iter().find(|(s, _)| s == service).map(|(_, u)| u.as_str()).or(self.upstream.as_deref())
    }

    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    /// That every service is one of `known` and has a server.
    pub fn check(&self, known: &[&str]) -> Result<(), String> {
        for s in &self.services {
            if !known.contains(&s.as_str()) {
                return Err(format!("--registries {s}: this Carbon has {}", if known.is_empty() { "no registries".to_string() } else { known.join(", ") }));
            }
            if self.upstream_for(s).is_none() {
                return Err(format!("--registries {s} needs a server: --upstream URL, or --upstream {s}=URL"));
            }
        }
        Ok(())
    }

    /// As JSON, token and all: for a worker, over a pipe, never for a file.
    pub fn to_json(&self) -> Value {
        json!({
            "services": self.services,
            "upstream": self.upstream,
            "upstreams": self.upstreams.iter().map(|(s, u)| json!([s, u])).collect::<Vec<_>>(),
            "token": self.token,
            "meta": self.meta.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(v: &Value) -> Result<Registries, String> {
        let pairs = |key: &str| -> Vec<(String, String)> {
            v.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()))).collect()).unwrap_or_default()
        };
        let mut r = Registries::new(v.get("services").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(String::from).collect::<Vec<_>>()).unwrap_or_default());
        if let Some(u) = v.get("upstream").and_then(Value::as_str) {
            r = r.upstream(u)?;
        }
        for (s, u) in pairs("upstreams") {
            r = r.upstream_for_service(&s, &u)?;
        }
        r.token = v.get("token").and_then(Value::as_str).map(String::from);
        r.meta = pairs("meta");
        Ok(r)
    }
}

/// A server's URL without its trailing `/`: HTTPS, or HTTP to loopback.
fn check_url(url: &str) -> Result<String, String> {
    let url = url.trim_end_matches('/');
    let bad = || format!("--upstream {url}: https://HOST[:PORT][/PATH], or http:// to this host's loopback");
    let rest = if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split('/').next().unwrap_or("");
        let host = host.rsplit_once(':').filter(|(_, p)| p.parse::<u16>().is_ok()).map_or(host, |(h, _)| h);
        if !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
            return Err(format!("--upstream {url}: plain http only to this host's loopback, since the token goes with every call"));
        }
        rest
    } else {
        return Err(bad());
    };
    if rest.is_empty() || rest.starts_with('/') || rest.contains(['?', '#', '@', ' ']) {
        return Err(bad());
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_and_json() {
        let r = Registries::new(["apt"])
            .services_flag("npm, apt")
            .upstream_flag("https://pkg.example.com/")
            .unwrap()
            .upstream_flag("npm=https://npm.example.com")
            .unwrap()
            .meta_flag("sid=8f3a")
            .unwrap()
            .token("t");
        assert_eq!(r.services, ["apt", "npm"]);
        assert_eq!((r.upstream_for("apt"), r.upstream_for("npm")), (Some("https://pkg.example.com"), Some("https://npm.example.com")));
        assert_eq!(Registries::from_json(&r.to_json()).unwrap(), r);
        assert!(!format!("{r:?}").contains("\"t\""));
        assert!(r.check(&["apt", "npm"]).is_ok());
        assert!(r.check(&["apt"]).unwrap_err().contains("npm"));
        assert!(Registries::new(["apt"]).check(&["apt"]).unwrap_err().contains("--upstream"));
    }

    #[test]
    fn plain_http_only_to_loopback() {
        assert!(Registries::new(["apt"]).upstream("http://127.0.0.1:18091").is_ok());
        assert!(Registries::new(["apt"]).upstream("http://localhost").is_ok());
        assert!(Registries::new(["apt"]).upstream("http://pkg.example.com").is_err());
        assert!(Registries::new(["apt"]).upstream("ftp://pkg.example.com").is_err());
        assert!(Registries::new(["apt"]).upstream("https://user@pkg.example.com").is_err());
        assert!(Registries::new(["apt"]).meta_flag("novalue").is_err());
    }
}
