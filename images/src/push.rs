//! Pushing an image to a registry, a layer per step, and the layers themselves, which `save` writes too.
//!
//! An image's disks are its lineage's, then its own. The first is sent as the registry's own layers when it was
//! pulled (mounted from the repository it came from on the same registry, or copied from it to another), else made
//! here as one layer; each disk after it is one layer, a tar of what differs from the disk before (`layers::tar::
//! export`, with whiteouts). A step that changed nothing makes no layer, and its history says so. `squash` makes the
//! whole disk one layer. Layers made here are written to files as they're made, hashed both as tars (their diff IDs)
//! and compressed (their digests), so nothing is held in memory but a piece at a time, and the same disks always make
//! the same bytes.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use layers::Persist;
use layers::tar::Entry;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::auth::credentials;
use crate::registry::{Client, Reference, scope};
use crate::{History, Image, Images, Layer, rfc3339, store};

/// What a push is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Making a layer: the tar of what a step changed.
    Making,
    /// Sending a layer.
    Sending,
    /// A layer the registry had already, or took from the repository it came from, with nothing sent.
    Present,
    /// Sending the config and the manifest.
    Manifest,
}

/// Where a push is: what it's doing, on which layer (from 1) of how many, and how many of that layer's bytes are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub stage: Stage,
    pub layer: usize,
    pub layers: usize,
    pub done: u64,
    pub total: u64,
}

impl std::fmt::Display for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mib = |b: u64| b as f64 / f64::from(1 << 20);
        match self.stage {
            Stage::Making => write!(f, "making the layers: step {} of {}", self.layer, self.layers),
            Stage::Sending => write!(f, "layer {} of {}: {:.1} of {:.1} MiB", self.layer, self.layers, mib(self.done), mib(self.total)),
            Stage::Present => write!(f, "layer {} of {}: already there", self.layer, self.layers),
            Stage::Manifest => f.write_str("sending the manifest"),
        }
    }
}

/// What a push did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    /// Where, in full: `docker.io/someone/app:1`.
    pub name: String,
    /// The manifest's digest, `sha256:…`: the image by what it is, as `name@digest`.
    pub digest: String,
    pub layers: usize,
    /// How many layers' bytes were sent (the rest the registry had, or took from another of its repositories).
    pub sent: usize,
}

/// A file that's removed when it's dropped.
pub(crate) struct Temporary(pub PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A layer as pushed or saved.
pub(crate) enum Part {
    /// The registry's own, as pulled.
    Pulled(Layer),
    /// Made here, in a file until it's sent.
    Made { file: Temporary, layer: Layer },
}

impl Part {
    pub fn layer(&self) -> &Layer {
        match self {
            Part::Pulled(l) | Part::Made { layer: l, .. } => l,
        }
    }
}

/// An image's layers and its history as they make it, each non-empty entry a layer.
pub(crate) struct Plan {
    pub parts: Vec<Part>,
    pub history: Vec<History>,
    /// Whether its layers are Docker's kind (a pulled base's are), so the manifest and config say so too.
    pub docker: bool,
}

/// How to make the layers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct How {
    pub squash: bool,
    /// gzip the layers made here (a push), or leave them tars (a save).
    pub gzip: bool,
    /// Send a pulled first disk as the registry's layers; else it's made here as one.
    pub reuse_pulled: bool,
}

const OCI_LAYER_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
const OCI_LAYER: &str = "application/vnd.oci.image.layer.v1.tar";
const DOCKER_LAYER_GZIP: &str = "application/vnd.docker.image.rootfs.diff.tar.gzip";
pub(crate) const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
pub(crate) const OCI_CONFIG: &str = "application/vnd.oci.image.config.v1+json";
const DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";
const DOCKER_CONFIG: &str = "application/vnd.docker.container.image.v1+json";

/// Writes to a file, hashing and counting what's written.
struct Hashing<W: Write> {
    inner: W,
    hash: Sha256,
    size: u64,
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(bytes)?;
        self.hash.update(&bytes[..n]);
        self.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// A disk by its index's name, to read.
fn open<M: Persist>(images: &Images, hex: &str, root: &M) -> Result<layers::Fs<M>, String> {
    let index = images.index_named(hex).map_err(|e| format!("{e}: push or save it with --squash"))?;
    layers::Fs::open(Some(&index), Box::new(images.store()?), root.clone(), false).map_err(|e| format!("opening the disk {hex}: {e}"))
}

/// A layer of what `disk` changed from `base` (all of it, without one), in a file in `dir`; None if nothing did.
fn make<M: Persist>(images: &Images, base: Option<&str>, disk: &str, entry: &mut impl FnMut(&M) -> Entry, root: &M, gzip: bool, dir: &Path) -> Result<Option<(Temporary, Layer)>, String> {
    let fs = open(images, disk, root)?;
    let base = base.map(|b| open(images, b, root)).transpose()?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("layer-{}-{n}", std::process::id()));
    let file = Temporary(path);
    let out = File::create(&file.0).map_err(|e| format!("{}: {e}", file.0.display()))?;
    let mut tar_hash = Sha256::new();
    let mut failed = None;
    let hashing = Hashing { inner: BufWriter::new(out), hash: Sha256::new(), size: 0 };
    let (entries, written) = if gzip {
        let mut gz = GzEncoder::new(hashing, Compression::default());
        let entries = layers::tar::export(&fs, base.as_ref(), entry, &mut |b| {
            tar_hash.update(b);
            gz.write_all(b).map_err(|e| {
                failed = Some(e.to_string());
                layers::Error::Io
            })
        });
        (entries, gz.finish())
    } else {
        let mut plain = hashing;
        let entries = layers::tar::export(&fs, base.as_ref(), entry, &mut |b| {
            tar_hash.update(b);
            plain.write_all(b).map_err(|e| {
                failed = Some(e.to_string());
                layers::Error::Io
            })
        });
        (entries, Ok(plain))
    };
    let entries = entries.map_err(|e| format!("making a layer: {}", failed.unwrap_or_else(|| e.to_string())))?;
    let mut written = written.map_err(|e| format!("making a layer: {e}"))?;
    written.flush().map_err(|e| format!("making a layer: {e}"))?;
    if entries == 0 {
        return Ok(None);
    }
    let diff_id = format!("sha256:{}", store::hex(&tar_hash.finalize()));
    let digest = format!("sha256:{}", store::hex(&written.hash.finalize()));
    let media_type = if gzip { OCI_LAYER_GZIP } else { OCI_LAYER };
    Ok(Some((file, Layer { digest, size: written.size, media_type: media_type.into(), diff_id })))
}

/// One step's history entries made to say they made `layers` layers: as they are if they do, else all empty but the
/// last that made a layer (or the last), and entries of Carbon's own if there are too few.
fn fix_history(entries: &mut Vec<History>, layers: usize, what: &str) {
    let made = entries.iter().filter(|h| !h.empty_layer).count();
    if made == layers {
        return;
    }
    let last = entries.iter().rposition(|h| !h.empty_layer).or(entries.len().checked_sub(1));
    for h in entries.iter_mut() {
        h.empty_layer = true;
    }
    match (layers, last) {
        (0, _) => {}
        (1, Some(i)) => entries[i].empty_layer = false,
        _ => {
            for k in 1..=layers {
                let created_by = if layers == 1 { what.to_string() } else { format!("{what}: layer {k} of {layers}") };
                entries.push(History { created: entries.last().map(|h| h.created.clone()).unwrap_or_default(), created_by, empty_layer: false, comment: "carbon".into() });
            }
        }
    }
}

/// An image's layers and history, made as `how` says: in files in `dir` for those made here.
pub(crate) fn plan<M: Persist>(images: &Images, image: &Image, how: How, mut entry: impl FnMut(&M) -> Entry, root: M, dir: &Path, progress: &mut dyn FnMut(&Progress)) -> Result<Plan, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let lineage = &image.lineage;
    // Each disk and where its history ends; the image's own last.
    let mut disks: Vec<(String, usize)> = lineage.disks.iter().map(|d| (d.index.clone(), d.history)).collect();
    disks.push((image.index.clone(), image.history.len()));
    let mut parts = Vec::new();
    let mut history = Vec::new();
    let mut docker = false;
    if how.squash {
        progress(&Progress { stage: Stage::Making, layer: 1, layers: 1, done: 0, total: 0 });
        let made = make(images, None, &image.index, &mut entry, &root, how.gzip, dir)?;
        let mut entries = image.history.clone();
        for h in &mut entries {
            h.empty_layer = true;
        }
        if made.is_some() {
            entries.push(History { created: rfc3339(image.created), created_by: format!("carbon: {} squashed into one layer", image.short_name()), empty_layer: false, comment: "carbon".into() });
        }
        parts.extend(made.map(|(file, layer)| Part::Made { file, layer }));
        return Ok(Plan { parts, history: entries, docker });
    }
    let count = disks.len();
    let mut start = 0;
    for (i, (disk, end)) in disks.iter().enumerate() {
        let end = (*end).clamp(start, image.history.len());
        let mut entries = image.history[start..end].to_vec();
        progress(&Progress { stage: Stage::Making, layer: i + 1, layers: count, done: 0, total: 0 });
        let made = if i == 0 && how.reuse_pulled && lineage.pulled() {
            docker = lineage.layers.iter().any(|l| l.media_type.starts_with("application/vnd.docker"));
            parts.extend(lineage.layers.iter().cloned().map(Part::Pulled));
            lineage.layers.len()
        } else {
            let base = if i == 0 { None } else { Some(disks[i - 1].0.as_str()) };
            let unchanged = base == Some(disk.as_str());
            match if unchanged { None } else { make(images, base, disk, &mut entry, &root, how.gzip, dir)? } {
                Some((file, layer)) => {
                    parts.push(Part::Made { file, layer });
                    1
                }
                None => 0,
            }
        };
        let what = if i == 0 { format!("carbon: the files {} began with", image.short_name()) } else { "carbon: a step's changes".into() };
        fix_history(&mut entries, made, &what);
        history.extend(entries);
        start = end;
    }
    // Entries past the last disk's (there shouldn't be), as they changed nothing.
    history.extend(image.history[start..].iter().cloned().map(|mut h| {
        h.empty_layer = true;
        h
    }));
    if docker {
        for part in &mut parts {
            if let Part::Made { layer, .. } = part {
                layer.media_type = DOCKER_LAYER_GZIP.into();
            }
        }
    }
    Ok(Plan { parts, history, docker })
}

/// An image's OCI config, for its layers and history as planned.
pub(crate) fn config_json(image: &Image, plan: &Plan) -> Vec<u8> {
    let history: Vec<Value> = plan
        .history
        .iter()
        .map(|h| {
            let mut v = json!({ "created": h.created, "created_by": h.created_by });
            if !h.comment.is_empty() {
                v["comment"] = json!(h.comment);
            }
            if h.empty_layer {
                v["empty_layer"] = json!(true);
            }
            v
        })
        .collect();
    let mut v = json!({
        "created": rfc3339(image.created),
        "architecture": image.platform.arch,
        "os": image.platform.os,
        "config": image.config.to_oci(),
        "rootfs": { "type": "layers", "diff_ids": plan.parts.iter().map(|p| p.layer().diff_id.clone()).collect::<Vec<_>>() },
        "history": history,
    });
    if image.platform.arch == "arm64" {
        v["variant"] = json!("v8");
    }
    serde_json::to_vec(&v).unwrap_or_default()
}

/// The manifest, for a config and layers; and its media type.
pub(crate) fn manifest_json(plan: &Plan, config: &[u8]) -> (Vec<u8>, &'static str) {
    let (manifest_type, config_type) = if plan.docker { (DOCKER_MANIFEST, DOCKER_CONFIG) } else { (OCI_MANIFEST, OCI_CONFIG) };
    let layers: Vec<Value> = plan.parts.iter().map(|p| p.layer()).map(|l| json!({ "mediaType": l.media_type, "digest": l.digest, "size": l.size })).collect();
    let v = json!({
        "schemaVersion": 2,
        "mediaType": manifest_type,
        "config": { "mediaType": config_type, "digest": format!("sha256:{}", store::hex(&Sha256::digest(config))), "size": config.len() },
        "layers": layers,
    });
    (serde_json::to_vec(&v).unwrap_or_default(), manifest_type)
}

/// Reads a file, telling `sent` how much is read as it goes.
struct Counting<'p, R: Read> {
    inner: R,
    done: u64,
    told: u64,
    sent: &'p mut dyn FnMut(u64),
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        if self.done - self.told >= crate::registry::REPORT_EVERY || n == 0 {
            self.told = self.done;
            (self.sent)(self.done);
        }
        Ok(n)
    }
}

pub fn push<M: Persist>(images: &Images, image: &Image, target: &str, squash: bool, entry: impl FnMut(&M) -> Entry, root: M, progress: &mut dyn FnMut(&Progress)) -> Result<Pushed, String> {
    let reference = Reference::parse(target);
    let dir = images.home().join("tmp");
    let plan = plan(images, image, How { squash, gzip: true, reuse_pulled: !squash }, entry, root, &dir, progress)?;
    let source = Reference::repository(&image.lineage.repository, "latest");
    let from_here = image.lineage.pulled() && source.registry == reference.registry && source.repository != reference.repository;
    let mut scopes = vec![scope(&reference.repository, "pull,push")];
    if from_here {
        scopes.push(scope(&source.repository, "pull"));
    }
    let login = credentials(images.home(), &reference.registry);
    let client = Client::connect(&reference, &scopes, login.as_ref())?;
    let mut origin: Option<Client> = None;
    let count = plan.parts.len();
    let mut sent = 0;
    for (i, part) in plan.parts.iter().enumerate() {
        let layer = part.layer();
        let at = |stage, done| Progress { stage, layer: i + 1, layers: count, done, total: layer.size };
        if client.has_blob(&layer.digest)? || (from_here && matches!(part, Part::Pulled(_)) && client.mount(&layer.digest, &source.repository)?) {
            progress(&at(Stage::Present, layer.size));
            continue;
        }
        let file = match part {
            Part::Made { file, .. } => File::open(&file.0).map_err(|e| format!("{}: {e}", file.0.display()))?,
            Part::Pulled(_) => {
                // From another registry, or one that wouldn't mount it: fetched from where it came from, then sent.
                if origin.is_none() {
                    let login = credentials(images.home(), &source.registry);
                    origin = Some(Client::connect(&source, &[scope(&source.repository, "pull")], login.as_ref())?);
                }
                let fetched = Temporary(dir.join(format!("fetched-{}-{i}", std::process::id())));
                let mut out = BufWriter::new(File::create(&fetched.0).map_err(|e| format!("{}: {e}", fetched.0.display()))?);
                origin.as_ref().ok_or("no source")?.blob_into(&layer.digest, &mut out, &mut |_| {})?;
                out.flush().map_err(|e| format!("{}: {e}", fetched.0.display()))?;
                let file = File::open(&fetched.0).map_err(|e| format!("{}: {e}", fetched.0.display()))?;
                drop(fetched);
                file
            }
        };
        progress(&at(Stage::Sending, 0));
        let mut tell = |done| progress(&at(Stage::Sending, done));
        client.upload(&layer.digest, layer.size, Counting { inner: std::io::BufReader::new(file), done: 0, told: 0, sent: &mut tell })?;
        sent += 1;
    }
    progress(&Progress { stage: Stage::Manifest, layer: count, layers: count, done: 0, total: 0 });
    let config = config_json(image, &plan);
    let config_digest = format!("sha256:{}", store::hex(&Sha256::digest(&config)));
    if !client.has_blob(&config_digest)? {
        client.upload(&config_digest, config.len() as u64, config.as_slice())?;
    }
    let (manifest, media_type) = manifest_json(&plan, &config);
    let digest = client.put_manifest(&reference.tag, media_type, &manifest)?;
    Ok(Pushed { name: format!("{}:{}", reference.full_repository(), reference.tag), digest, layers: count, sent })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(created_by: &str, empty: bool) -> History {
        History { created: String::new(), created_by: created_by.into(), empty_layer: empty, comment: String::new() }
    }

    fn made(entries: &[History]) -> Vec<&str> {
        entries.iter().filter(|e| !e.empty_layer).map(|e| e.created_by.as_str()).collect()
    }

    #[test]
    fn history_says_what_made_layers() {
        // A RUN and the config steps after it: as they are.
        let mut e = vec![h("RUN make", false), h("CMD x", true)];
        fix_history(&mut e, 1, "-");
        assert_eq!(made(&e), ["RUN make"]);
        // A RUN that changed nothing makes no layer.
        fix_history(&mut e, 0, "-");
        assert_eq!(made(&e), Vec::<&str>::new());
        // A base of three layers made here as one: its last.
        let mut base = vec![h("ADD rootfs", false), h("RUN a", false), h("CMD sh", true), h("RUN b", false)];
        fix_history(&mut base, 1, "-");
        assert_eq!(made(&base), ["RUN b"]);
        // No entries at all: Carbon's own.
        let mut none = Vec::new();
        fix_history(&mut none, 1, "carbon: the files");
        assert_eq!(made(&none), ["carbon: the files"]);
        // Pulled layers the history doesn't count right: Carbon's, one each.
        let mut odd = vec![h("FROM", false)];
        fix_history(&mut odd, 2, "pulled");
        assert_eq!(made(&odd), ["pulled: layer 1 of 2", "pulled: layer 2 of 2"]);
    }
}
