//! Pulling an image from an OCI registry (Docker Hub, GHCR, any that speaks the distribution API), over HTTPS with the
//! host's trust (`tls.rs`). Every blob is checked against its digest, which is what makes the registry not need
//! trusting for what it serves; the layers are stacked in order, whiteouts and all, and frozen into one index.

use std::io::Read;

use flate2::read::GzDecoder;
use layers::Persist;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Config, Entry, Image, Images, Platform, full_name, store, strings};

const ACCEPT: &str = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, \
                      application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json";

/// What a pull is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Asking the registry for the image's manifest and config.
    Resolving,
    /// Downloading a layer.
    Downloading,
    /// Unpacking a downloaded layer onto the image's disk.
    Unpacking,
    /// Writing the finished image's index.
    Indexing,
}

/// Where a pull is: what it's doing, on which layer (from 1) of how many, and how many of all its layers' bytes are
/// here (`done` of `total`, as the registry sends them, compressed). Reported as each piece arrives, so a progress bar
/// can follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub stage: Stage,
    pub layer: usize,
    pub layers: usize,
    pub done: u64,
    pub total: u64,
}

impl Progress {
    /// How far along, from 0 to 1, by bytes.
    pub fn fraction(&self) -> f64 {
        match self.stage {
            Stage::Indexing => 1.0,
            _ if self.total == 0 => 0.0,
            _ => (self.done as f64 / self.total as f64).min(1.0),
        }
    }
}

impl std::fmt::Display for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mib = |n: u64| n as f64 / f64::from(1 << 20);
        match self.stage {
            Stage::Resolving => f.write_str("finding the image"),
            Stage::Downloading => write!(f, "layer {} of {}: {:.1} of {:.1} MiB", self.layer, self.layers, mib(self.done), mib(self.total)),
            Stage::Unpacking => write!(f, "unpacking layer {} of {}", self.layer, self.layers),
            Stage::Indexing => f.write_str("writing the index"),
        }
    }
}

/// How often a download reports itself: after this many bytes, at most.
const REPORT_EVERY: u64 = 256 << 10;

/// Where an image is: its registry's host, its repository there, and its tag or digest.
#[derive(Debug, PartialEq, Eq)]
struct Reference {
    host: String,
    repository: String,
    tag: String,
}

fn reference(name: &str) -> Reference {
    let full = full_name(name);
    let (registry, rest) = full.split_once('/').unwrap_or(("docker.io", &full));
    let (repository, tag) = rest.rsplit_once(':').unwrap_or((rest, "latest"));
    // Docker Hub's API is on another host than its name.
    let host = if registry == "docker.io" { "registry-1.docker.io" } else { registry };
    Reference { host: host.into(), repository: repository.into(), tag: tag.into() }
}

/// A connection to one repository, with the token it asked for, if it did.
struct Client {
    agent: ureq::Agent,
    base: String,
    token: Option<String>,
}

/// A field of a `WWW-Authenticate` challenge: `realm="…"`.
fn field(header: &str, name: &str) -> Option<String> {
    let start = header.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = header.get(start..)?;
    Some(rest.get(..rest.find('"')?)?.to_string())
}

impl Client {
    fn connect(reference: &Reference) -> Result<Client, String> {
        let agent = ureq::AgentBuilder::new().tls_config(crate::tls::client_config()?).build();
        let base = format!("https://{}/v2/{}", reference.host, reference.repository);
        let mut client = Client { agent, base, token: None };
        // A registry says how to get a token by refusing without one.
        match client.agent.get(&format!("https://{}/v2/", reference.host)).call() {
            Ok(_) => {}
            Err(ureq::Error::Status(401, response)) => {
                let header = response.header("www-authenticate").unwrap_or("").to_string();
                let realm = field(&header, "realm").ok_or("the registry wants a login this doesn't understand")?;
                let service = field(&header, "service").unwrap_or_default();
                let scope = format!("repository:{}:pull", reference.repository);
                let body: Value = client
                    .agent
                    .get(&realm)
                    .query("service", &service)
                    .query("scope", &scope)
                    .call()
                    .map_err(|e| format!("getting a token: {e}"))?
                    .into_string()
                    .ok()
                    .and_then(|text| serde_json::from_str(&text).ok())
                    .ok_or("getting a token: the answer wasn't JSON")?;
                client.token = body.get("token").or(body.get("access_token")).and_then(Value::as_str).map(String::from);
            }
            Err(e) => return Err(format!("reaching {}: {e}", reference.host)),
        }
        Ok(client)
    }

    fn get(&self, url: &str) -> Result<ureq::Response, String> {
        let mut request = self.agent.get(url).set("Accept", ACCEPT);
        if let Some(token) = &self.token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        request.call().map_err(|e| match e {
            ureq::Error::Status(404, _) => "not found".to_string(),
            other => other.to_string(),
        })
    }

    fn manifest(&self, reference: &str) -> Result<Value, String> {
        let text = self.get(&format!("{}/manifests/{reference}", self.base))?.into_string().map_err(|e| format!("reading a manifest: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("reading a manifest: {e}"))
    }

    /// A blob, all of it, checked against its digest.
    fn blob(&self, digest: &str) -> Result<Vec<u8>, String> {
        self.blob_reporting(digest, 0, &mut |_| {})
    }

    /// A blob, read a piece at a time and hashed as it comes, telling `arrived` how many bytes are here so far (at
    /// most every [`REPORT_EVERY`], and at the end); then checked against its digest. `expected` sizes the buffer.
    fn blob_reporting(&self, digest: &str, expected: u64, arrived: &mut dyn FnMut(u64)) -> Result<Vec<u8>, String> {
        let mut reader = self.get(&format!("{}/blobs/{digest}", self.base))?.into_reader();
        let mut bytes = Vec::with_capacity(usize::try_from(expected).unwrap_or(0).min(1 << 30));
        let mut hash = Sha256::new();
        let mut piece = vec![0u8; 64 << 10];
        let mut told = 0u64;
        loop {
            let n = reader.read(&mut piece).map_err(|e| format!("downloading {digest}: {e}"))?;
            if n == 0 {
                break;
            }
            hash.update(&piece[..n]);
            bytes.extend_from_slice(&piece[..n]);
            if bytes.len() as u64 - told >= REPORT_EVERY {
                told = bytes.len() as u64;
                arrived(told);
            }
        }
        arrived(bytes.len() as u64);
        check_hash(digest, &store::hex(&hash.finalize()))?;
        Ok(bytes)
    }
}

#[cfg(test)]
fn check_digest(digest: &str, bytes: &[u8]) -> Result<(), String> {
    check_hash(digest, &store::hex(&Sha256::digest(bytes)))
}

fn check_hash(digest: &str, actual: &str) -> Result<(), String> {
    match digest.strip_prefix("sha256:") {
        Some(expected) if expected == actual => Ok(()),
        Some(expected) => Err(format!("a download doesn't match its digest: expected {expected}, got {actual}")),
        None => Err(format!("{digest} isn't a digest this understands")),
    }
}

/// The manifest for `platform` from an index, by its digest.
fn choose(index: &[Value], platform: &Platform) -> Option<String> {
    index
        .iter()
        .find(|m| {
            let p = m.get("platform");
            let get = |key: &str| p.and_then(|p| p.get(key)).and_then(Value::as_str);
            get("os") == Some(&platform.os) && get("architecture") == Some(&platform.arch)
        })
        .and_then(|m| m.get("digest"))
        .and_then(Value::as_str)
        .map(String::from)
}

pub fn pull<M: Persist>(images: &Images, name: &str, platform: &Platform, mut meta: impl FnMut(Entry) -> M, root: M, progress: &mut dyn FnMut(&Progress)) -> Result<Image, String> {
    let full = full_name(name);
    let reference = reference(name);
    progress(&Progress { stage: Stage::Resolving, layer: 0, layers: 0, done: 0, total: 0 });
    let client = Client::connect(&reference)?;
    let mut manifest = client.manifest(&reference.tag).map_err(|e| format!("{full}: {e}"))?;
    if let Some(list) = manifest.get("manifests").and_then(Value::as_array) {
        let chosen = choose(list, platform).ok_or_else(|| format!("{full} isn't built for {platform}"))?;
        manifest = client.manifest(&chosen)?;
    }

    let config_digest = manifest.get("config").and_then(|c| c.get("digest")).and_then(Value::as_str).ok_or("the manifest has no config")?;
    let config: Value = serde_json::from_slice(&client.blob(config_digest)?).map_err(|e| format!("reading its config: {e}"))?;
    let os = config.get("os").and_then(Value::as_str).unwrap_or("linux");
    if os != platform.os {
        return Err(format!("{full} is for {os}, not {}", platform.os));
    }
    let run = config.get("config");
    let text = |key: &str| run.and_then(|c| c.get(key)).and_then(Value::as_str).unwrap_or_default().to_string();
    let image_config = Config {
        env: strings(run.and_then(|c| c.get("Env"))),
        entrypoint: strings(run.and_then(|c| c.get("Entrypoint"))),
        cmd: strings(run.and_then(|c| c.get("Cmd"))),
        workdir: text("WorkingDir"),
        user: text("User"),
    };

    let layers = manifest.get("layers").and_then(Value::as_array).ok_or("the manifest has no layers")?;
    let total: u64 = layers.iter().filter_map(|l| l.get("size").and_then(Value::as_u64)).sum();
    let count = layers.len();
    let mut before = 0u64;
    let mut disk = layers::Fs::building(root, platform.os == "windows", Box::new(images.store()?));
    for (i, layer) in layers.iter().enumerate() {
        let digest = layer.get("digest").and_then(Value::as_str).ok_or("a layer has no digest")?;
        let media = layer.get("mediaType").and_then(Value::as_str).unwrap_or("");
        let size = layer.get("size").and_then(Value::as_u64).unwrap_or(0);
        let at = |stage, done| Progress { stage, layer: i + 1, layers: count, done, total: total.max(done) };
        progress(&at(Stage::Downloading, before));
        let blob = client.blob_reporting(digest, size, &mut |here| progress(&at(Stage::Downloading, before + here)))?;
        before += blob.len() as u64;
        progress(&at(Stage::Unpacking, before));
        let tar = if media.contains("zstd") {
            return Err(format!("layer {digest} is compressed with zstd, which isn't supported yet"));
        } else if media.contains("gzip") || media.is_empty() {
            let mut out = Vec::new();
            GzDecoder::new(blob.as_slice()).read_to_end(&mut out).map_err(|e| format!("unpacking layer {digest}: {e}"))?;
            out
        } else {
            blob
        };
        layers::tar::import(&mut disk, &tar, &mut meta).map_err(|e| format!("layer {digest}: {e}"))?;
    }
    progress(&Progress { stage: Stage::Indexing, layer: count, layers: count, done: before, total: total.max(before) });
    let index = disk.freeze().map_err(|e| format!("writing the index: {e}"))?;
    images.save(name, &index, platform, &image_config, &format!("pulled from {}", reference.host))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reaches the network: `cargo test -p images -- --ignored progress`.
    #[test]
    #[ignore]
    fn progress_climbs_to_the_whole_image() {
        let home = std::env::temp_dir().join(format!("carbon-pull-progress-{}", std::process::id()));
        let images = Images::open(Some(&home)).unwrap();
        let mut seen: Vec<Progress> = Vec::new();
        let image = images.pull("python:3.13-slim", &Platform::linux_here(), |_| (), (), &mut |p| seen.push(*p)).unwrap();
        assert!(images.image(&image.name).is_some());
        assert_eq!(seen.first().map(|p| p.stage), Some(Stage::Resolving));
        assert_eq!(seen.last().map(|p| p.stage), Some(Stage::Indexing));
        let downloads: Vec<&Progress> = seen.iter().filter(|p| p.stage == Stage::Downloading).collect();
        assert!(downloads.len() > 10, "reports as it downloads, not only per layer: {}", downloads.len());
        assert!(downloads.windows(2).all(|w| w[1].done >= w[0].done), "bytes only climb");
        assert!(seen.iter().all(|p| p.done <= p.total), "never past the total");
        let last = seen.last().unwrap();
        assert_eq!((last.done, last.fraction()), (last.total, 1.0));
        eprintln!("{} reports, {} layers, {} bytes", seen.len(), last.layers, last.total);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn references() {
        assert_eq!(reference("debian:13"), Reference { host: "registry-1.docker.io".into(), repository: "library/debian".into(), tag: "13".into() });
        assert_eq!(reference("ghcr.io/org/app:1.0"), Reference { host: "ghcr.io".into(), repository: "org/app".into(), tag: "1.0".into() });
        assert_eq!(reference("localhost:5000/app").host, "localhost:5000");
    }

    #[test]
    fn digests_and_platforms() {
        assert!(check_digest("sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824", b"hello").is_ok());
        assert!(check_digest("sha256:00", b"hello").is_err());
        assert!(check_digest("md5:00", b"hello").is_err());
        let index: Vec<Value> = serde_json::from_str(
            r#"[{"digest":"sha256:a","platform":{"os":"linux","architecture":"amd64"}},
                {"digest":"sha256:b","platform":{"os":"linux","architecture":"arm64","variant":"v8"}}]"#,
        )
        .unwrap();
        assert_eq!(choose(&index, &Platform::parse("linux/arm64").unwrap()).as_deref(), Some("sha256:b"));
        assert_eq!(choose(&index, &Platform::parse("windows/amd64").unwrap()), None);
    }
}
