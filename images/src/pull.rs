//! Pulling an image from an OCI registry (Docker Hub, GHCR, any that speaks the distribution API), over HTTPS with the
//! host's trust (`tls.rs`). Every blob is checked against its digest, which is what makes the registry not need
//! trusting for what it serves; the layers are stacked in order, whiteouts and all, and frozen into one index.

use std::io::Read;

use flate2::read::GzDecoder;
use layers::Persist;
use serde_json::Value;

use crate::registry::{Client, Reference, scope};
use crate::{Config, Entry, Image, Images, Platform, full_name};

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
    let reference = Reference::parse(name);
    progress(&Progress { stage: Stage::Resolving, layer: 0, layers: 0, done: 0, total: 0 });
    let login = crate::auth::credentials(images.home(), &reference.registry);
    let client = Client::connect(&reference, &[scope(&reference.repository, "pull")], login.as_ref())?;
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
    let image_config = Config::from_oci(config.get("config"));
    let history: Vec<crate::History> = config.get("history").and_then(Value::as_array).map(|h| h.iter().map(crate::History::from_json).collect()).unwrap_or_default();

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
        let mut blob = Vec::with_capacity(usize::try_from(size).unwrap_or(0).min(1 << 30));
        client.blob_into(digest, &mut blob, &mut |here| progress(&at(Stage::Downloading, before + here)))?;
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
    // Its layers as the registry has them, with what each holds, so a push of an image made from it sends them as they are.
    let diff_ids: Vec<&str> = config.get("rootfs").and_then(|r| r.get("diff_ids")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let kept: Vec<crate::Layer> = layers
        .iter()
        .enumerate()
        .map(|(i, l)| crate::Layer {
            digest: l.get("digest").and_then(Value::as_str).unwrap_or_default().into(),
            size: l.get("size").and_then(Value::as_u64).unwrap_or(0),
            media_type: l.get("mediaType").and_then(Value::as_str).unwrap_or_default().into(),
            diff_id: diff_ids.get(i).copied().unwrap_or_default().into(),
        })
        .collect();
    let lineage = crate::Lineage { repository: reference.full_repository(), layers: kept, disks: Vec::new() };
    images.save_with(name, &index, platform, &image_config, &history, &lineage, &format!("pulled from {}", reference.host))
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
    fn digests_and_platforms() {
        let index: Vec<Value> = serde_json::from_str(
            r#"[{"digest":"sha256:a","platform":{"os":"linux","architecture":"amd64"}},
                {"digest":"sha256:b","platform":{"os":"linux","architecture":"arm64","variant":"v8"}}]"#,
        )
        .unwrap();
        assert_eq!(choose(&index, &Platform::parse("linux/arm64").unwrap()).as_deref(), Some("sha256:b"));
        assert_eq!(choose(&index, &Platform::parse("windows/amd64").unwrap()), None);
    }
}
