//! Talking to a registry with the distribution API: where an image is, the token or login it wants, manifests and
//! blobs both ways. Over HTTPS with the host's trust (`tls.rs`), or plain HTTP to one on this host (`localhost:5000`),
//! as Docker treats those.

use std::io::Read;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::Credentials;
use crate::{full_name, store};

pub(crate) const ACCEPT: &str = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, \
                                 application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json";

/// Where an image is: its registry as written (`docker.io`), the host its API is on, its repository there, and its
/// tag or digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reference {
    pub registry: String,
    pub host: String,
    pub repository: String,
    pub tag: String,
}

impl Reference {
    pub fn parse(name: &str) -> Reference {
        let full = full_name(name);
        let (registry, rest) = full.split_once('/').unwrap_or(("docker.io", &full));
        let (repository, tag) = rest.rsplit_once(':').unwrap_or((rest, "latest"));
        Reference::at(registry, repository, tag)
    }

    /// A repository in full (`docker.io/library/debian`), at a tag.
    pub fn repository(full: &str, tag: &str) -> Reference {
        let (registry, repository) = full.split_once('/').unwrap_or(("docker.io", full));
        Reference::at(registry, repository, tag)
    }

    fn at(registry: &str, repository: &str, tag: &str) -> Reference {
        // Docker Hub's API is on another host than its name.
        let host = if registry == "docker.io" { "registry-1.docker.io" } else { registry };
        Reference { registry: registry.into(), host: host.into(), repository: repository.into(), tag: tag.into() }
    }

    /// The repository in full, as an image's lineage keeps it.
    pub fn full_repository(&self) -> String {
        format!("{}/{}", self.registry, self.repository)
    }

    fn scheme(&self) -> &'static str {
        let host = self.host.split(':').next().unwrap_or("");
        if matches!(host, "localhost" | "127.0.0.1" | "[::1]") { "http" } else { "https" }
    }

    fn origin(&self) -> String {
        format!("{}://{}", self.scheme(), self.host)
    }
}

/// What a registry was asked for, in its tokens' words: `repository:NAME:pull,push`.
pub(crate) fn scope(repository: &str, actions: &str) -> String {
    format!("repository:{repository}:{actions}")
}

/// How a request is let in.
enum Auth {
    None,
    Bearer(String),
    Basic(String),
}

/// A connection to one repository, let in as it asked.
pub(crate) struct Client {
    agent: ureq::Agent,
    origin: String,
    base: String,
    auth: Auth,
}

/// A field of a `WWW-Authenticate` challenge: `realm="…"`.
fn field(header: &str, name: &str) -> Option<String> {
    let start = header.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = header.get(start..)?;
    Some(rest.get(..rest.find('"')?)?.to_string())
}

/// A registry's refusal in words: its own errors' messages if it sent them.
pub(crate) fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, response) => {
            let text = response.into_string().unwrap_or_default();
            let said: Vec<String> = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("errors").and_then(Value::as_array).cloned())
                .map(|errors| errors.iter().filter_map(|e| Some(format!("{} ({})", e.get("message")?.as_str()?, e.get("code").and_then(Value::as_str).unwrap_or("")))).collect())
                .unwrap_or_default();
            match code {
                401 => format!("not allowed (401){}: log in with `carbon login`", if said.is_empty() { String::new() } else { format!(": {}", said.join("; ")) }),
                404 if said.is_empty() => "not found".into(),
                _ if said.is_empty() => format!("the registry answered {code}"),
                _ => format!("the registry answered {code}: {}", said.join("; ")),
            }
        }
        other => other.to_string(),
    }
}

impl Client {
    /// Connects for `scopes` (`repository:NAME:pull`), with `credentials` if there are any: a token from the registry's
    /// challenge, asked for with them, or them on each request if it wants that.
    pub fn connect(reference: &Reference, scopes: &[String], credentials: Option<&Credentials>) -> Result<Client, String> {
        // A private run.app registry's IAM check gets the host's ID token (host::cloud_run), the registry's own auth Authorization.
        let agent = ureq::AgentBuilder::new().tls_config(crate::tls::client_config()?).middleware(host::cloud_run::Invoker).build();
        let origin = reference.origin();
        let base = format!("{origin}/v2/{}", reference.repository);
        let mut client = Client { agent, origin: origin.clone(), base, auth: Auth::None };
        // A registry says how to be let in by refusing.
        match client.agent.get(&format!("{origin}/v2/")).call() {
            Ok(_) => {}
            Err(ureq::Error::Status(401, response)) => {
                let header = response.header("www-authenticate").unwrap_or("").to_string();
                if header.to_ascii_lowercase().starts_with("basic") {
                    let c = credentials.ok_or_else(|| format!("{} wants a login: `carbon login {}`", reference.registry, reference.registry))?;
                    client.auth = Auth::Basic(c.basic());
                    return Ok(client);
                }
                let realm = field(&header, "realm").ok_or("the registry wants a login this doesn't understand")?;
                let service = field(&header, "service").unwrap_or_default();
                let mut request = client.agent.get(&realm).query("service", &service);
                for scope in scopes {
                    request = request.query("scope", scope);
                }
                if let Some(c) = credentials {
                    request = request.set("Authorization", &c.basic());
                }
                let body: Value = request
                    .call()
                    .map_err(|e| match e {
                        ureq::Error::Status(401, _) if credentials.is_some() => format!("{} refused the login: `carbon login {}` again", reference.registry, reference.registry),
                        e => format!("getting a token: {}", describe(e)),
                    })?
                    .into_string()
                    .ok()
                    .and_then(|text| serde_json::from_str(&text).ok())
                    .ok_or("getting a token: the answer wasn't JSON")?;
                client.auth = match body.get("token").or(body.get("access_token")).and_then(Value::as_str) {
                    Some(token) => Auth::Bearer(token.into()),
                    None => Auth::None,
                };
            }
            Err(e) => return Err(format!("reaching {}: {}", reference.host, describe(e))),
        }
        Ok(client)
    }

    /// Whether `credentials` let `registry` in: a token asked for with them, or `/v2/` with them.
    pub fn check_login(reference: &Reference, credentials: &Credentials) -> Result<(), String> {
        let client = Client::connect(reference, &[], Some(credentials))?;
        if let Auth::Basic(_) = client.auth {
            client.call(client.agent.get(&format!("{}/v2/", client.origin))).map_err(|e| match e {
                ureq::Error::Status(401, _) => format!("{} refused the login", reference.registry),
                e => describe(e),
            })?;
        }
        Ok(())
    }

    #[allow(clippy::result_large_err)] // ureq's own error, matched by its callers
    fn call(&self, mut request: ureq::Request) -> Result<ureq::Response, ureq::Error> {
        match &self.auth {
            Auth::Bearer(token) => request = request.set("Authorization", &format!("Bearer {token}")),
            Auth::Basic(basic) => request = request.set("Authorization", basic),
            Auth::None => {}
        }
        request.call()
    }

    #[allow(clippy::result_large_err)] // ureq's own error, matched by its callers
    fn send(&self, mut request: ureq::Request, body: impl Read) -> Result<ureq::Response, ureq::Error> {
        match &self.auth {
            Auth::Bearer(token) => request = request.set("Authorization", &format!("Bearer {token}")),
            Auth::Basic(basic) => request = request.set("Authorization", basic),
            Auth::None => {}
        }
        request.send(body)
    }

    pub fn get(&self, url: &str) -> Result<ureq::Response, String> {
        self.call(self.agent.get(url).set("Accept", ACCEPT)).map_err(describe)
    }

    pub fn manifest(&self, reference: &str) -> Result<Value, String> {
        let text = self.get(&format!("{}/manifests/{reference}", self.base))?.into_string().map_err(|e| format!("reading a manifest: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("reading a manifest: {e}"))
    }

    /// A blob, all of it, checked against its digest.
    pub fn blob(&self, digest: &str) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        self.blob_into(digest, &mut bytes, &mut |_| {})?;
        Ok(bytes)
    }

    /// A blob, read a piece at a time into `out` and hashed as it comes, telling `arrived` how many bytes are here so
    /// far (at most every [`REPORT_EVERY`], and at the end); then checked against its digest.
    pub fn blob_into(&self, digest: &str, out: &mut dyn std::io::Write, arrived: &mut dyn FnMut(u64)) -> Result<u64, String> {
        let mut reader = self.get(&format!("{}/blobs/{digest}", self.base))?.into_reader();
        let mut hash = Sha256::new();
        let mut piece = vec![0u8; 64 << 10];
        let (mut here, mut told) = (0u64, 0u64);
        loop {
            let n = reader.read(&mut piece).map_err(|e| format!("downloading {digest}: {e}"))?;
            if n == 0 {
                break;
            }
            hash.update(&piece[..n]);
            out.write_all(&piece[..n]).map_err(|e| format!("keeping {digest}: {e}"))?;
            here += n as u64;
            if here - told >= REPORT_EVERY {
                told = here;
                arrived(told);
            }
        }
        arrived(here);
        check_hash(digest, &store::hex(&hash.finalize()))?;
        Ok(here)
    }

    /// Whether the repository has a blob.
    pub fn has_blob(&self, digest: &str) -> Result<bool, String> {
        match self.call(self.agent.head(&format!("{}/blobs/{digest}", self.base))) {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(404, _)) => Ok(false),
            Err(e) => Err(describe(e)),
        }
    }

    /// Asks for a blob another repository of the same registry has to be this one's too, with nothing sent; whether
    /// it was (a registry may decline, and then it's sent).
    pub fn mount(&self, digest: &str, from: &str) -> Result<bool, String> {
        let request = self.agent.post(&format!("{}/blobs/uploads/", self.base)).query("mount", digest).query("from", from);
        match self.send(request, std::io::empty()) {
            Ok(r) if r.status() == 201 => Ok(true),
            Ok(_) => Ok(false),
            Err(ureq::Error::Status(404 | 401 | 403, _)) => Ok(false),
            Err(e) => Err(describe(e)),
        }
    }

    /// Sends a blob of `size` bytes from `body`, in one request after its upload is begun.
    pub fn upload(&self, digest: &str, size: u64, body: impl Read) -> Result<(), String> {
        let begun = self.send(self.agent.post(&format!("{}/blobs/uploads/", self.base)), std::io::empty()).map_err(|e| format!("beginning an upload: {}", describe(e)))?;
        let location = begun.header("location").ok_or("the registry began an upload but didn't say where")?.to_string();
        let mut url = if location.starts_with('/') { format!("{}{location}", self.origin) } else { location };
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&format!("digest={digest}"));
        let request = self.agent.put(&url).set("Content-Type", "application/octet-stream").set("Content-Length", &size.to_string());
        self.send(request, body).map_err(|e| format!("sending {digest}: {}", describe(e)))?;
        Ok(())
    }

    /// Puts a manifest at a tag; its digest, as the registry says it (else as its bytes are).
    pub fn put_manifest(&self, tag: &str, media_type: &str, bytes: &[u8]) -> Result<String, String> {
        let request = self.agent.put(&format!("{}/manifests/{tag}", self.base)).set("Content-Type", media_type);
        let response = self.send(request, bytes).map_err(|e| format!("putting the manifest: {}", describe(e)))?;
        Ok(response.header("docker-content-digest").map(String::from).unwrap_or_else(|| format!("sha256:{}", store::hex(&Sha256::digest(bytes)))))
    }
}

/// How often a download reports itself: after this many bytes, at most.
pub(crate) const REPORT_EVERY: u64 = 256 << 10;

pub(crate) fn check_hash(digest: &str, actual: &str) -> Result<(), String> {
    match digest.strip_prefix("sha256:") {
        Some(expected) if expected == actual => Ok(()),
        Some(expected) => Err(format!("a download doesn't match its digest: expected {expected}, got {actual}")),
        None => Err(format!("{digest} isn't a digest this understands")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references() {
        let r = Reference::parse("debian:13");
        assert_eq!((r.registry.as_str(), r.host.as_str(), r.repository.as_str(), r.tag.as_str()), ("docker.io", "registry-1.docker.io", "library/debian", "13"));
        assert_eq!(r.full_repository(), "docker.io/library/debian");
        let g = Reference::parse("ghcr.io/org/app:1.0");
        assert_eq!((g.host.as_str(), g.repository.as_str(), g.tag.as_str()), ("ghcr.io", "org/app", "1.0"));
        assert_eq!(Reference::parse("localhost:5000/app").origin(), "http://localhost:5000");
        assert_eq!(Reference::parse("ghcr.io/org/app").origin(), "https://ghcr.io");
        assert_eq!(Reference::repository("docker.io/library/debian", "13"), r);
    }

    #[test]
    fn digests() {
        let hello = store::hex(&Sha256::digest(b"hello"));
        assert!(check_hash("sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824", &hello).is_ok());
        assert!(check_hash("sha256:00", &hello).is_err());
        assert!(check_hash("md5:00", &hello).is_err());
    }
}
