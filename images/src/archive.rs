//! Images as one archive, as `docker save` writes them and `docker load` reads them: an OCI image layout
//! (`oci-layout`, `index.json`, `blobs/sha256/…`) with Docker's `manifest.json` beside it, as Docker 25 and later
//! write, so either reads it. Saving makes a layer per step, as a push does, but each a plain tar and every one made
//! here (an archive has nowhere to send a reader for a registry's). Loading takes either kind, a layout or Docker's
//! older archive (`<id>/layer.tar`), gzipped or not.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use layers::Persist;
use layers::tar::Entry;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::push::{How, OCI_MANIFEST, Part, Progress, Temporary, config_json, manifest_json, plan};
use crate::{Config, History, Image, Images, Lineage, Platform, store};

const OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";

/// Writes an archive a piece at a time: each file's header, its bytes, its padding.
struct Writer<'o> {
    out: &'o mut dyn Write,
    written: BTreeSet<String>,
}

impl Writer<'_> {
    fn bytes(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.from(path, bytes.len() as u64, &mut &bytes[..])
    }

    fn from(&mut self, path: &str, size: u64, body: &mut dyn Read) -> Result<(), String> {
        let fail = |e: std::io::Error| format!("writing the archive: {e}");
        self.out.write_all(&layers::tar::file_header(path, size)).map_err(fail)?;
        let copied = std::io::copy(&mut body.take(size), self.out).map_err(fail)?;
        if copied != size {
            return Err(format!("writing the archive: {path} was shorter than it said"));
        }
        let pad = (size.next_multiple_of(512) - size) as usize;
        self.out.write_all(&[0u8; 512][..pad]).map_err(fail)?;
        self.written.insert(path.to_string());
        Ok(())
    }

    /// A blob, unless it's in already.
    fn blob(&mut self, digest: &str, size: u64, body: &mut dyn Read) -> Result<(), String> {
        let path = blob_path(digest);
        if self.written.contains(&path) { Ok(()) } else { self.from(&path, size, body) }
    }
}

fn blob_path(digest: &str) -> String {
    format!("blobs/{}", digest.replacen(':', "/", 1))
}

/// Writes `images` (each with the names it's saved as) to `out`, as one archive.
pub fn save<M: Persist>(images: &Images, saving: &[(Image, Vec<String>)], out: &mut dyn Write, mut entry: impl FnMut(&M) -> Entry, root: M, progress: &mut dyn FnMut(&Progress)) -> Result<(), String> {
    let mut w = Writer { out, written: BTreeSet::new() };
    w.bytes("oci-layout", br#"{"imageLayoutVersion":"1.0.0"}"#)?;
    let dir = images.home().join("tmp");
    let (mut index, mut docker) = (Vec::new(), Vec::new());
    for (image, names) in saving {
        let plan = plan(images, image, How { squash: false, gzip: false, reuse_pulled: false }, &mut entry, root.clone(), &dir, progress)?;
        for part in &plan.parts {
            if let Part::Made { file, layer } = part {
                let mut f = BufReader::new(File::open(&file.0).map_err(|e| format!("{}: {e}", file.0.display()))?);
                w.blob(&layer.digest, layer.size, &mut f)?;
            }
        }
        let config = config_json(image, &plan);
        let config_digest = format!("sha256:{}", store::hex(&Sha256::digest(&config)));
        w.blob(&config_digest, config.len() as u64, &mut config.as_slice())?;
        let (manifest, _) = manifest_json(&plan, &config);
        let manifest_digest = format!("sha256:{}", store::hex(&Sha256::digest(&manifest)));
        w.blob(&manifest_digest, manifest.len() as u64, &mut manifest.as_slice())?;
        let descriptor = json!({ "mediaType": OCI_MANIFEST, "digest": manifest_digest, "size": manifest.len() });
        if names.is_empty() {
            index.push(descriptor.clone());
        }
        for name in names {
            let full = crate::full_name(name);
            let tag = full.rsplit_once(':').map_or("latest", |(_, t)| t);
            let mut d = descriptor.clone();
            d["annotations"] = json!({ "io.containerd.image.name": full, "org.opencontainers.image.ref.name": tag });
            index.push(d);
        }
        let short: Vec<String> = names.iter().map(|n| short_name(&crate::full_name(n))).collect();
        docker.push(json!({
            "Config": blob_path(&config_digest),
            "RepoTags": if short.is_empty() { Value::Null } else { json!(short) },
            "Layers": plan.parts.iter().map(|p| blob_path(&p.layer().digest)).collect::<Vec<_>>(),
        }));
    }
    let index = serde_json::to_vec(&json!({ "schemaVersion": 2, "mediaType": OCI_INDEX, "manifests": index })).unwrap_or_default();
    w.bytes("index.json", &index)?;
    w.bytes("manifest.json", &serde_json::to_vec(&docker).unwrap_or_default())?;
    w.out.write_all(&[0u8; 1024]).map_err(|e| format!("writing the archive: {e}"))?;
    w.out.flush().map_err(|e| format!("writing the archive: {e}"))
}

fn short_name(full: &str) -> String {
    full.strip_prefix("docker.io/library/").or_else(|| full.strip_prefix("docker.io/")).unwrap_or(full).to_string()
}

/// An archive being loaded: its files, by path, each where it is in it.
pub struct Archive {
    file: RefCell<File>,
    files: BTreeMap<String, (u64, u64)>,
    /// A gzipped archive, unpacked to here first.
    _unpacked: Option<Temporary>,
}

/// An image an archive has: the names it was saved as, its platform, and where its config and layers are.
#[derive(Debug, Clone)]
pub struct Found {
    pub names: Vec<String>,
    pub platform: Platform,
    config: Value,
    layers: Vec<String>,
}

impl Found {
    /// How many layers it comes as.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }
}

fn octal(field: &[u8]) -> u64 {
    field.iter().skip_while(|&&b| b == b' ').take_while(|&&b| (b'0'..=b'7').contains(&b)).fold(0, |n, &b| n * 8 + u64::from(b - b'0'))
}

fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

impl Archive {
    /// Opens an archive and finds its images; `scratch` is where a gzipped one is unpacked.
    pub fn open(path: &Path, scratch: &Path) -> Result<(Archive, Vec<Found>), String> {
        let fail = |e: std::io::Error| format!("{}: {e}", path.display());
        let mut file = File::open(path).map_err(fail)?;
        let mut magic = [0u8; 2];
        let n = file.read(&mut magic).map_err(fail)?;
        file.seek(SeekFrom::Start(0)).map_err(fail)?;
        let mut unpacked = None;
        if n == 2 && magic == [0x1f, 0x8b] {
            std::fs::create_dir_all(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
            let temporary = Temporary(scratch.join(format!("load-{}", std::process::id())));
            let mut out = BufWriter::new(File::create(&temporary.0).map_err(|e| format!("{}: {e}", temporary.0.display()))?);
            std::io::copy(&mut flate2::read::GzDecoder::new(BufReader::new(file)), &mut out).map_err(|e| format!("unpacking {}: {e}", path.display()))?;
            out.flush().map_err(|e| format!("unpacking {}: {e}", path.display()))?;
            file = File::open(&temporary.0).map_err(|e| format!("{}: {e}", temporary.0.display()))?;
            unpacked = Some(temporary);
        }
        let files = Self::list(&mut file).map_err(|e| format!("{}: {e}", path.display()))?;
        let archive = Archive { file: RefCell::new(file), files, _unpacked: unpacked };
        let found = archive.find()?;
        Ok((archive, found))
    }

    /// Every plain file in it, and where its bytes are.
    fn list(file: &mut File) -> Result<BTreeMap<String, (u64, u64)>, String> {
        let mut files = BTreeMap::new();
        let mut at = 0u64;
        let (mut long, mut pax_size): (Option<String>, Option<u64>) = (None, None);
        let mut h = [0u8; 512];
        loop {
            file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
            if file.read_exact(&mut h).is_err() || h.iter().all(|&b| b == 0) {
                break;
            }
            let size = pax_size.take().unwrap_or_else(|| octal(&h[124..136]));
            let data = at + 512;
            at = data + size.next_multiple_of(512);
            let read = |file: &mut File| -> Result<String, String> {
                let mut bytes = vec![0u8; usize::try_from(size).map_err(|_| "a header too big")?.min(1 << 20)];
                file.seek(SeekFrom::Start(data)).map_err(|e| e.to_string())?;
                file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
                Ok(text(&bytes))
            };
            match h[156] {
                b'L' => long = Some(read(file)?),
                b'x' => {
                    for record in read(file)?.split('\n') {
                        if let Some((_, kv)) = record.split_once(' ') {
                            if let Some(p) = kv.strip_prefix("path=") {
                                long = Some(p.to_string());
                            } else if let Some(v) = kv.strip_prefix("size=") {
                                pax_size = v.parse().ok();
                            }
                        }
                    }
                }
                b'0' | 0 => {
                    let name = long.take().unwrap_or_else(|| {
                        let prefix = if &h[257..262] == b"ustar" { text(&h[345..500]) } else { String::new() };
                        let name = text(&h[0..100]);
                        if prefix.is_empty() { name } else { format!("{prefix}/{name}") }
                    });
                    files.insert(name.trim_start_matches("./").to_string(), (data, size));
                }
                _ => long = None,
            }
        }
        Ok(files)
    }

    /// A file's bytes.
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let &(at, size) = self.files.get(path.trim_start_matches("./")).ok_or_else(|| format!("the archive has no {path}"))?;
        let mut bytes = vec![0u8; usize::try_from(size).map_err(|_| format!("{path} is too big"))?];
        let mut file = self.file.borrow_mut();
        file.seek(SeekFrom::Start(at)).map_err(|e| format!("reading {path}: {e}"))?;
        file.read_exact(&mut bytes).map_err(|e| format!("reading {path}: {e}"))?;
        Ok(bytes)
    }

    fn json(&self, path: &str) -> Result<Value, String> {
        serde_json::from_slice(&self.read(path)?).map_err(|e| format!("reading {path}: {e}"))
    }

    /// Its images: from Docker's `manifest.json` if it has one, else its OCI index.
    fn find(&self) -> Result<Vec<Found>, String> {
        let mut found = Vec::new();
        if self.files.contains_key("manifest.json") {
            for item in self.json("manifest.json")?.as_array().ok_or("manifest.json isn't a list")? {
                let path = |v: &Value| v.as_str().map(String::from);
                let config = self.json(&item.get("Config").and_then(path).ok_or("an image in manifest.json has no Config")?)?;
                let layers = item.get("Layers").and_then(Value::as_array).map(|a| a.iter().filter_map(path).collect()).unwrap_or_default();
                let names = item.get("RepoTags").and_then(Value::as_array).map(|a| a.iter().filter_map(path).collect()).unwrap_or_default();
                found.push(Found { names, platform: platform_of(&config)?, config, layers });
            }
            return Ok(found);
        }
        let index = self.json("index.json").map_err(|_| "this isn't an image archive: it has neither manifest.json nor index.json".to_string())?;
        let mut manifests: Vec<Value> = index.get("manifests").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        while let Some(d) = manifests.pop() {
            let digest = d.get("digest").and_then(Value::as_str).ok_or("index.json lists a manifest with no digest")?;
            let manifest = self.json(&blob_path(digest))?;
            // An index within: every manifest it lists.
            if let Some(more) = manifest.get("manifests").and_then(Value::as_array) {
                manifests.extend(more.iter().cloned());
                continue;
            }
            let annotations = d.get("annotations");
            let name = annotations
                .and_then(|a| a.get("io.containerd.image.name"))
                .and_then(Value::as_str)
                .or_else(|| annotations.and_then(|a| a.get("org.opencontainers.image.ref.name")).and_then(Value::as_str).filter(|n| n.contains(['/', ':'])));
            if let Some(&i) = seen.get(digest) {
                found[i].names.extend(name.map(String::from));
                continue;
            }
            let config_digest = manifest.get("config").and_then(|c| c.get("digest")).and_then(Value::as_str).ok_or("a manifest has no config")?;
            let config = self.json(&blob_path(config_digest))?;
            let layers = manifest.get("layers").and_then(Value::as_array).map(|a| a.iter().filter_map(|l| l.get("digest").and_then(Value::as_str)).map(blob_path).collect()).unwrap_or_default();
            seen.insert(digest.to_string(), found.len());
            found.push(Found { names: name.map(String::from).into_iter().collect(), platform: platform_of(&config)?, config, layers });
        }
        found.reverse();
        Ok(found)
    }
}

fn platform_of(config: &Value) -> Result<Platform, String> {
    let os = config.get("os").and_then(Value::as_str).unwrap_or("linux");
    let arch = config.get("architecture").and_then(Value::as_str).unwrap_or("");
    Platform::parse(&format!("{os}/{arch}")).ok_or_else(|| format!("an image in the archive is for {os}/{arch}, which Carbon doesn't run"))
}

/// Makes an image the archive has: its layers stacked, as a pull stacks them, and named as it was saved (or by its id
/// if it had no name). Each tar entry's `M` is made by `meta`, the root's is `root`. The disk after each layer but the
/// last is kept as its lineage, so it's pushed and saved again a layer per layer it came as.
pub fn load<M: Persist>(images: &Images, archive: &Archive, found: &Found, mut meta: impl FnMut(Entry) -> M, root: M) -> Result<Vec<Image>, String> {
    let history: Vec<History> = found.config.get("history").and_then(Value::as_array).map(|h| h.iter().map(History::from_json).collect()).unwrap_or_default();
    // Where in the history each layer's entry is: the k-th that isn't empty, and so the disk after it ends there.
    let ends: Vec<usize> = history.iter().enumerate().filter(|(_, h)| !h.empty_layer).map(|(i, _)| i + 1).collect();
    let mut lineage = Lineage::default();
    let mut disk = layers::Fs::building(root, found.platform.os == "windows", Box::new(images.store()?));
    for (k, path) in found.layers.iter().enumerate() {
        if k > 0 {
            let index = disk.freeze().map_err(|e| format!("writing an index: {e}"))?;
            let hash = layers::Store::put(&mut images.store()?, &index).map_err(|_| "putting an index in the store".to_string())?;
            let made_by = ends.get(k - 1).copied().unwrap_or(history.len()).max(lineage.disks.last().map_or(0, |d| d.history));
            lineage.disks.push(crate::Disk { index: store::hex(&hash), history: made_by });
        }
        let bytes = archive.read(path)?;
        let tar = match bytes.get(..4) {
            Some([0x1f, 0x8b, ..]) => crate::gunzip(&bytes)?,
            Some([0x28, 0xb5, 0x2f, 0xfd]) => return Err(format!("{path} is compressed with zstd, which isn't supported yet")),
            _ => bytes,
        };
        layers::tar::import(&mut disk, &tar, &mut meta).map_err(|e| format!("{path}: {e}"))?;
    }
    let index = disk.freeze().map_err(|e| format!("writing the index: {e}"))?;
    let config = Config::from_oci(found.config.get("config"));
    if found.names.is_empty() {
        return Ok(vec![images.save_unnamed(&index, &found.platform, &config, &history, &lineage, "loaded")?]);
    }
    found.names.iter().map(|name| images.save_with(name, &index, &found.platform, &config, &history, &lineage, "loaded")).collect()
}
