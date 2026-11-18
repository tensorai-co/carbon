//! Running a build: stages in order, each instruction a library call, each step's image found again by its cache key.
//!
//! A stage's state is the image its last disk step made, and the config and history the instructions since have
//! changed. A `RUN` or `COPY` saves that state as an unnamed image, makes a machine from it, changes its disk, and
//! freezes it with the config and the step: the next state. Every instruction feeds the stage's cache key, so a key
//! names exactly what a step starts from and what it does.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use images::{Config, History, Image, Platform};
use serde_json::json;

use super::{Args, Build, Built, Carbonfile, Instruction, Lexer, Patterns, Step};
use crate::error::{Error, ErrorKind, Result};
use crate::store::{now, rfc3339};
use crate::{Copy, Freeze, Logs, Machine, Network, Write};

/// Build arguments every build has without an `ARG` for them to reach `RUN`.
const PROXY_ARGS: &[&str] = &["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "FTP_PROXY", "ftp_proxy", "NO_PROXY", "no_proxy", "ALL_PROXY", "all_proxy"];

/// Part of every cache key: changed when what a step does changes, so what older builds cached isn't found.
const CACHE_VERSION: &str = "carbon-build-2";

fn sha(parts: &[&str]) -> String {
    images::store::hex(&images::store::sha256(parts.join("\0").as_bytes()))
}

/// A stage as it's built.
#[derive(Clone)]
struct State {
    /// The image the last disk step made (or the base).
    image: Image,
    config: Config,
    history: Vec<History>,
    key: String,
    platform: Platform,
    /// ARGs declared in the stage, with their values.
    args: Vec<(String, Option<String>)>,
    /// Folders WORKDIR named, made by the next disk step.
    pending_dirs: Vec<String>,
    /// CMD was set in this stage (so ENTRYPOINT doesn't reset it).
    cmd_set: bool,
}

impl State {
    fn windows(&self) -> bool {
        self.platform.os == "windows"
    }

    fn get(&self, name: &str) -> Option<String> {
        // ENV wins over ARG.
        for var in self.config.env.iter().rev() {
            if let Some(value) = var.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
                return Some(value.to_string());
            }
        }
        self.args.iter().rev().find(|(n, _)| n == name).and_then(|(_, v)| v.clone())
    }

    fn shell(&self) -> Vec<String> {
        if !self.config.shell.is_empty() {
            self.config.shell.clone()
        } else if self.windows() {
            vec!["cmd".into(), "/S".into(), "/C".into()]
        } else {
            vec!["/bin/sh".into(), "-c".into()]
        }
    }

    fn set_env(&mut self, name: &str, value: &str) {
        self.config.env.retain(|e| e.split('=').next() != Some(name));
        self.config.env.push(format!("{name}={value}"));
    }

    /// A path in the machine, from the working folder if it's relative.
    fn path(&self, path: &str) -> String {
        if self.windows() {
            let absolute = path.starts_with(['\\', '/']) || path.get(1..3).is_some_and(|s| s.starts_with(':'));
            if absolute { path.to_string() } else { format!("{}\\{path}", self.config.workdir.trim_end_matches('\\')) }
        } else if path.starts_with('/') {
            clean(path)
        } else {
            let base = if self.config.workdir.is_empty() { "/" } else { &self.config.workdir };
            clean(&format!("{base}/{path}"))
        }
    }

    fn config_step(&mut self, created_by: String) {
        self.key = sha(&[&self.key, &created_by]);
        self.history.push(History { created: rfc3339(now()), created_by, empty_layer: true, comment: String::new() });
    }
}

/// A Unix path cleaned: no `.`, `..` taken back, no doubled or trailing `/`.
fn clean(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// A Go duration (`30s`, `1m30s`, `500ms`), in nanoseconds.
fn duration(text: &str) -> Option<u64> {
    let mut total = 0f64;
    let mut rest = text.trim();
    if rest == "0" {
        return Some(0);
    }
    while !rest.is_empty() {
        let n = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).filter(|&n| n > 0)?;
        let value: f64 = rest[..n].parse().ok()?;
        rest = &rest[n..];
        let u = rest.find(|c: char| c.is_ascii_digit() || c == '.').unwrap_or(rest.len());
        let unit = match &rest[..u] {
            "ns" => 1.0,
            "us" | "µs" => 1e3,
            "ms" => 1e6,
            "s" => 1e9,
            "m" => 60e9,
            "h" => 3600e9,
            _ => return None,
        };
        total += value * unit;
        rest = &rest[u..];
    }
    Some(total as u64)
}

struct Run<'a> {
    build: &'a Build,
    file: Carbonfile,
    lex: Lexer,
    ignore: Patterns,
    progress: &'a mut dyn FnMut(&Step),
    id: String,
    machines: usize,
    ran: usize,
    cached: usize,
    used_args: HashSet<String>,
    meta: Vec<(String, Option<String>)>,
    stages: Vec<Option<State>>,
    target_platform: Platform,
    cache: PathBuf,
    /// The folder secrets are put in for `RUN --mount=type=secret`, if any was.
    secrets: Option<PathBuf>,
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        if let Some(dir) = self.secrets.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// A machine a build made, removed however the step ends.
struct Temp(Option<Machine>);

impl Temp {
    fn get(&self) -> &Machine {
        self.0.as_ref().expect("a build's machine")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if let Some(m) = self.0.take() {
            if m.is_running() {
                let _ = m.kill();
                m.wait(Some(Duration::from_secs(10)));
            }
            let _ = m.remove();
        }
    }
}

pub(super) fn build(build: Build, progress: &mut dyn FnMut(&Step)) -> Result<Built> {
    let started = Instant::now();
    let context = build.context.clone();
    if !context.is_dir() {
        return Err(Error::not_found(format!("{}: the build's context must be a folder", context.display())));
    }
    let file_path = match &build.file {
        Some(f) => f.clone(),
        None => ["Carbonfile", "Dockerfile"].iter().map(|n| context.join(n)).find(|p| p.is_file()).ok_or_else(|| Error::not_found(format!("{}: no Carbonfile or Dockerfile", context.display())))?,
    };
    let text = std::fs::read_to_string(&file_path).map_err(|e| Error::not_found(format!("{}: {e}", file_path.display())))?;
    let file = Carbonfile::parse(&text).map_err(|e| Error::invalid(format!("{}: {e}", file_path.display())))?;
    if let Some(syntax) = &file.directives.syntax {
        let name = syntax.strip_prefix("docker.io/").unwrap_or(syntax);
        let ours = name.strip_prefix("docker/dockerfile").is_some_and(|tag| tag.is_empty() || tag == ":latest" || tag.starts_with(":1") || tag.starts_with("@"));
        if !ours {
            return Err(Error::unsupported(format!("# syntax={syntax}: Carbon builds the Dockerfile's own grammar (docker/dockerfile:1), and doesn't run other frontends")));
        }
    }
    for warning in &file.warnings {
        progress(&Step::Warning(format!("{}: {warning}", file_path.display())));
    }
    let ignore = ignore_file(&file_path, &context);
    let platform = build.platform.clone().unwrap_or_else(Platform::linux_here);
    let cache = build.carbon.home().join("build").join("cache");
    std::fs::create_dir_all(&cache).map_err(|e| Error::io(format!("{}: {e}", cache.display())))?;
    let lex = Lexer::new(file.directives.escape);
    let id = format!("{:08x}", (now() as u32) ^ (std::process::id() << 16) ^ (Instant::now().elapsed().subsec_nanos()));
    let stages = vec![None; file.stages.len()];
    let mut run = Run { build: &build, file, lex, ignore, progress, id, machines: 0, ran: 0, cached: 0, used_args: HashSet::new(), meta: Vec::new(), stages, target_platform: platform, cache, secrets: None };

    run.meta_args()?;
    let target = match &build.target {
        Some(t) => run.file.stage(t).ok_or_else(|| Error::not_found(format!("--target {t}: there's no such stage")))?,
        None => run.file.stages.len() - 1,
    };
    let needed = run.needed(target);
    for index in 0..=target {
        if needed.contains(&index) {
            run.stage(index)?;
        }
    }
    let mut state = run.stages[target].clone().ok_or_else(|| Error::new(ErrorKind::Failed, "the target stage wasn't built"))?;
    run.flush_dirs(&mut state, target)?;
    let image = run.materialize(&state)?;
    for name in build.args.keys() {
        if !run.used_args.contains(name) && !PROXY_ARGS.contains(&name.as_str()) {
            (run.progress)(&Step::Warning(format!("build argument {name} isn't used by any ARG")));
        }
    }
    let mut named = image.clone();
    for (i, tag) in build.tags.iter().enumerate() {
        let tagged = build.carbon.tag(&image.name, tag)?;
        if i == 0 {
            named = tagged;
        }
    }
    if let Some(dir) = &build.output {
        run.output(&image, dir)?;
    }
    Ok(Built { image: named, ran: run.ran, cached: run.cached, took: started.elapsed() })
}

/// The ignore file: `<file>.carbonignore` or `<file>.dockerignore` next to the file, else the context's.
fn ignore_file(file: &Path, context: &Path) -> Patterns {
    let mut candidates = Vec::new();
    if let Some(name) = file.file_name().and_then(|n| n.to_str()) {
        let dir = file.parent().unwrap_or(Path::new("."));
        candidates.push(dir.join(format!("{name}.carbonignore")));
        candidates.push(dir.join(format!("{name}.dockerignore")));
    }
    candidates.push(context.join(".carbonignore"));
    candidates.push(context.join(".dockerignore"));
    candidates.iter().find_map(|p| std::fs::read_to_string(p).ok()).map(|t| Patterns::parse(&t)).unwrap_or_default()
}

impl Run<'_> {
    fn say(&mut self, step: Step) {
        (self.progress)(&step);
    }

    /// The build arguments Docker defines for every build.
    fn predefined(&self, name: &str) -> Option<String> {
        let here = Platform::linux_here();
        let t = &self.target_platform;
        Some(match name {
            "TARGETPLATFORM" => t.to_string(),
            "TARGETOS" => t.os.clone(),
            "TARGETARCH" => t.arch.clone(),
            "TARGETVARIANT" | "BUILDVARIANT" => String::new(),
            "BUILDPLATFORM" => here.to_string(),
            "BUILDOS" => here.os,
            "BUILDARCH" => here.arch,
            _ => return None,
        })
    }

    /// The ARGs before the first FROM, with the build's values.
    fn meta_args(&mut self) -> Result<()> {
        for ins in self.file.meta_args.clone() {
            let Args::Words(words) = &ins.args else { continue };
            for word in words {
                let (name, default) = match word.split_once('=') {
                    Some((n, d)) => (n.to_string(), Some(d.to_string())),
                    None => (word.clone(), None),
                };
                let meta = self.meta.clone();
                let lookup = |n: &str| meta.iter().rev().find(|(k, _)| k == n).and_then(|(_, v)| v.clone()).or_else(|| self.predefined(n));
                let default = match default {
                    Some(d) => Some(self.lex.word(&d, &lookup).map_err(|e| self.bad(&ins, e))?),
                    None => None,
                };
                let value = self.build.args.get(&name).cloned().or(default).or_else(|| self.predefined(&name));
                self.used_args.insert(name.clone());
                self.meta.push((name, value));
            }
        }
        Ok(())
    }

    fn meta_lookup(&self, name: &str) -> Option<String> {
        self.meta.iter().rev().find(|(k, _)| k == name).and_then(|(_, v)| v.clone()).or_else(|| self.predefined(name))
    }

    fn bad(&self, ins: &Instruction, why: impl std::fmt::Display) -> Error {
        Error::invalid(format!("line {}: {}: {why}", ins.line, ins.name()))
    }

    /// The stages `target` needs, itself included: those it starts FROM and copies --from, and theirs.
    fn needed(&self, target: usize) -> HashSet<usize> {
        let mut needed = HashSet::new();
        let mut todo = vec![target];
        while let Some(i) = todo.pop() {
            if !needed.insert(i) {
                continue;
            }
            let stage = &self.file.stages[i];
            let base = self.lex.word(&stage.base, &|n| self.meta_lookup(n)).unwrap_or_default();
            let mut refs = vec![base];
            refs.extend(stage.instructions.iter().filter_map(|ins| ins.flag("from").map(String::from)));
            for r in refs {
                if let Some(j) = self.file.stage(&r).filter(|&j| j < i) {
                    todo.push(j);
                }
            }
        }
        needed
    }

    fn cached(&self, key: &str) -> Option<Image> {
        if self.build.no_cache {
            return None;
        }
        let text = std::fs::read_to_string(self.cache.join(format!("{key}.json"))).ok()?;
        let name = serde_json::from_str::<serde_json::Value>(&text).ok()?.get("image")?.as_str()?.to_string();
        let image = self.build.carbon.image(&name)?;
        self.build.carbon.0.store.images.touch_unnamed(&image.name);
        Some(image)
    }

    fn remember(&self, key: &str, image: &Image) {
        let _ = std::fs::write(self.cache.join(format!("{key}.json")), json!({ "image": image.name, "made": now() }).to_string());
    }

    /// The state as an image, saved if its config or history moved on from its disk step's.
    fn materialize(&self, state: &State) -> Result<Image> {
        if state.image.config == state.config && state.image.history == state.history {
            return Ok(state.image.clone());
        }
        let images = &self.build.carbon.0.store.images;
        let index = images.index(&state.image).map_err(crate::error::image)?;
        images.save_unnamed(&index, &state.platform, &state.config, &state.history, &state.image.lineage, "built").map_err(crate::error::image)
    }

    /// An empty disk, for `FROM scratch`.
    fn scratch(&self, platform: &Platform) -> Result<Image> {
        let images = &self.build.carbon.0.store.images;
        let store = Box::new(images.store().map_err(crate::error::image)?);
        let index = if platform.os == "windows" {
            layers::Fs::building(atlas::Meta { attributes: 0x10 }, true, store).freeze()
        } else {
            layers::Fs::building(lambda::Meta::dir(0o755), false, store).freeze()
        }
        .map_err(|e| Error::io(format!("an empty disk: {e}")))?;
        images.save_unnamed(&index, platform, &Config::default(), &[], &images::Lineage::default(), "scratch").map_err(crate::error::image)
    }

    fn stage(&mut self, index: usize) -> Result<()> {
        let stage = self.file.stages[index].clone();
        let base = self.lex.word(&stage.base, &|n| self.meta_lookup(n)).map_err(|e| self.bad(&stage.from, e))?;
        let platform = match &stage.platform {
            Some(p) => {
                let text = self.lex.word(p, &|n| self.meta_lookup(n)).map_err(|e| self.bad(&stage.from, e))?;
                Platform::parse(&text).ok_or_else(|| self.bad(&stage.from, format!("--platform={text}: not a platform Carbon runs")))?
            }
            None => self.target_platform.clone(),
        };
        self.say(Step::Stage { stage: index, name: stage.name.clone(), base: base.clone() });
        let mut state = match self.file.stage(&base).filter(|&j| j < index) {
            Some(j) => {
                let mut s = self.stages[j].clone().ok_or_else(|| self.bad(&stage.from, format!("stage {base} wasn't built")))?;
                self.flush_dirs(&mut s, j)?;
                State { args: Vec::new(), cmd_set: false, ..s }
            }
            None => {
                let image = if base.eq_ignore_ascii_case("scratch") { self.scratch(&platform)? } else { self.image(&base, &platform)? };
                let key = sha(&[CACHE_VERSION, "from", &image.index, &image.config.to_json().to_string(), &image.platform.to_string()]);
                State { config: image.config.clone(), history: image.history.clone(), key, platform: image.platform.clone(), image, args: Vec::new(), pending_dirs: Vec::new(), cmd_set: false }
            }
        };
        // What the base said to do when built on, first.
        let triggers = std::mem::take(&mut state.config.onbuild);
        if !triggers.is_empty() {
            state.key = sha(&[&state.key, "onbuild cleared"]);
        }
        for (i, trigger) in triggers.iter().enumerate() {
            let parsed = Carbonfile::parse(&format!("FROM scratch\n{trigger}")).map_err(|e| self.bad(&stage.from, format!("ONBUILD {trigger}: {e}")))?;
            let ins = parsed.stages[0].instructions.first().cloned().ok_or_else(|| self.bad(&stage.from, "an empty ONBUILD trigger"))?;
            self.instruction(&mut state, index, i + 1, &ins, true)?;
        }
        for (i, ins) in stage.instructions.iter().enumerate() {
            self.instruction(&mut state, index, triggers.len() + i + 1, ins, false)?;
        }
        self.stages[index] = Some(state);
        Ok(())
    }

    /// An image by name for `platform`, pulled if it isn't here.
    fn image(&mut self, name: &str, platform: &Platform) -> Result<Image> {
        if let Some(image) = self.build.carbon.image(name).filter(|i| i.platform == *platform) {
            return Ok(image);
        }
        self.say(Step::Note(format!("pulling {} for {platform}", images::full_name(name))));
        let mut last = None;
        let carbon = self.build.carbon.clone();
        let progress = &mut *self.progress;
        carbon.pull_with(name, Some(platform.clone()), |p| {
            if last != Some((p.stage, p.layer)) {
                last = Some((p.stage, p.layer));
                progress(&Step::Note(p.to_string()));
            }
        })
    }

    fn instruction(&mut self, state: &mut State, stage: usize, n: usize, ins: &Instruction, trigger: bool) -> Result<()> {
        let started = Instant::now();
        let shown = if trigger { format!("ONBUILD {}", ins.original) } else { ins.original.clone() };
        self.say(Step::Started { stage, n, instruction: shown });
        let cached = match ins.keyword.as_str() {
            "run" => self.run(state, stage, n, ins)?,
            "copy" | "add" => self.copy(state, stage, n, ins)?,
            _ => {
                let created_by = self.config(state, ins, &lookup_owned(state))?;
                state.config_step(created_by);
                false
            }
        };
        if cached {
            self.cached += 1;
            self.say(Step::Cached { stage, n });
        } else {
            self.say(Step::Done { stage, n, took: started.elapsed() });
        }
        Ok(())
    }

    /// An instruction that only changes the config; what its history says it was.
    fn config(&mut self, state: &mut State, ins: &Instruction, lookup: &dyn Fn(&str) -> Option<String>) -> Result<String> {
        let lex = self.lex;
        let (line, name) = (ins.line, ins.name());
        let word = |text: &str| lex.word(text, lookup).map_err(|e| Error::invalid(format!("line {line}: {name}: {e}")));
        let argv = |args: &Args, state: &State| -> Result<Vec<String>> {
            Ok(match args {
                Args::Json(list) => list.clone(),
                Args::Text(text) => state.shell().into_iter().chain([text.clone()]).collect(),
                _ => Vec::new(),
            })
        };
        let keyword = ins.keyword.as_str();
        Ok(match (keyword, &ins.args) {
            ("arg", Args::Words(words)) => {
                let mut shown = Vec::new();
                for w in words {
                    let (name, default) = match w.split_once('=') {
                        Some((n, d)) => (n.to_string(), Some(word(d)?)),
                        None => (w.clone(), None),
                    };
                    let meta = self.meta.iter().rev().find(|(k, _)| *k == name).and_then(|(_, v)| v.clone());
                    let value = self.build.args.get(&name).cloned().or(default).or(meta).or_else(|| self.predefined(&name));
                    self.used_args.insert(name.clone());
                    shown.push(match &value {
                        Some(v) => format!("{name}={v}"),
                        None => name.clone(),
                    });
                    state.args.retain(|(n, _)| *n != name);
                    state.args.push((name, value));
                }
                format!("ARG {}", shown.join(" "))
            }
            ("env", Args::Pairs(pairs)) | ("label", Args::Pairs(pairs)) => {
                let mut shown = Vec::new();
                for (k, v) in pairs {
                    let (k, v) = (word(k)?, word(v)?);
                    shown.push(format!("{k}={v}"));
                    if keyword == "env" {
                        state.set_env(&k, &v);
                    } else {
                        state.config.labels.insert(k, v);
                    }
                }
                format!("{} {}", ins.name(), shown.join(" "))
            }
            ("workdir", Args::Text(text)) => {
                let path = state.path(&word(text)?);
                state.config.workdir = path.clone();
                state.pending_dirs.push(path.clone());
                format!("WORKDIR {path}")
            }
            ("user", Args::Text(text)) => {
                let user = word(text)?;
                state.config.user = user.clone();
                format!("USER {user}")
            }
            ("stopsignal", Args::Text(text)) => {
                let signal = word(text)?;
                state.config.stop_signal = signal.clone();
                format!("STOPSIGNAL {signal}")
            }
            ("maintainer", Args::Text(text)) => format!("MAINTAINER {text}"),
            ("expose", Args::Words(words)) => {
                let mut ports = Vec::new();
                for w in words {
                    for port in self.lex.words(w, lookup).map_err(|e| self.bad(ins, e))? {
                        let port = if port.contains('/') { port.to_ascii_lowercase() } else { format!("{port}/tcp") };
                        if !state.config.exposed_ports.contains(&port) {
                            state.config.exposed_ports.push(port.clone());
                        }
                        ports.push(port);
                    }
                }
                format!("EXPOSE {}", ports.join(" "))
            }
            ("volume", Args::Json(list) | Args::Words(list)) => {
                let mut shown = Vec::new();
                for v in list {
                    let v = word(v)?;
                    if v.is_empty() {
                        return Err(self.bad(ins, "a volume can't be empty"));
                    }
                    if !state.config.volumes.contains(&v) {
                        state.config.volumes.push(v.clone());
                    }
                    shown.push(v);
                }
                format!("VOLUME [{}]", shown.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>().join(" "))
            }
            ("cmd", args) => {
                state.config.cmd = argv(args, state)?;
                state.cmd_set = true;
                format!("CMD {}", serde_json::to_string(&state.config.cmd).unwrap_or_default())
            }
            ("entrypoint", args) => {
                state.config.entrypoint = argv(args, state)?;
                if !state.cmd_set {
                    state.config.cmd.clear();
                }
                format!("ENTRYPOINT {}", serde_json::to_string(&state.config.entrypoint).unwrap_or_default())
            }
            ("shell", Args::Json(list)) => {
                if list.is_empty() {
                    return Err(self.bad(ins, "SHELL needs a shell"));
                }
                state.config.shell = list.clone();
                format!("SHELL {}", serde_json::to_string(list).unwrap_or_default())
            }
            ("shell", _) => return Err(self.bad(ins, "SHELL requires the arguments to be in JSON form")),
            ("healthcheck", Args::Health(None)) => {
                state.config.healthcheck = Some(images::Healthcheck { test: vec!["NONE".into()], ..images::Healthcheck::default() });
                "HEALTHCHECK NONE".into()
            }
            ("healthcheck", Args::Health(Some(command))) => {
                let test = match command.as_ref() {
                    Args::Json(list) => std::iter::once("CMD".to_string()).chain(list.iter().cloned()).collect(),
                    Args::Text(text) => vec!["CMD-SHELL".to_string(), text.clone()],
                    _ => return Err(self.bad(ins, "a health check needs a command")),
                };
                let time = |name: &str| -> Result<u64> {
                    match ins.flag(name) {
                        Some(v) => duration(v).ok_or_else(|| self.bad(ins, format!("--{name}={v}: not a duration"))),
                        None => Ok(0),
                    }
                };
                let retries = match ins.flag("retries") {
                    Some(v) => v.parse().map_err(|_| self.bad(ins, format!("--retries={v}: not a number")))?,
                    None => 0,
                };
                for flag in &ins.flags {
                    let key = flag.split('=').next().unwrap_or("");
                    if !matches!(key, "interval" | "timeout" | "start-period" | "start-interval" | "retries") {
                        return Err(self.bad(ins, format!("unknown flag: {key}")));
                    }
                }
                state.config.healthcheck = Some(images::Healthcheck { test: test.clone(), interval: time("interval")?, timeout: time("timeout")?, start_period: time("start-period")?, retries });
                format!("HEALTHCHECK {}", serde_json::to_string(&test).unwrap_or_default())
            }
            ("onbuild", Args::OnBuild(inner)) => {
                state.config.onbuild.push(inner.original.clone());
                format!("ONBUILD {}", inner.original)
            }
            (_, Args::None) => return Err(self.bad(ins, "requires an argument")),
            _ => return Err(self.bad(ins, "these arguments don't fit it")),
        })
    }

    fn next_name(&mut self) -> String {
        self.machines += 1;
        format!("build-{}-{}", self.id, self.machines)
    }

    /// A machine made from the state, not running, with the folders WORKDIR asked for.
    fn create(&mut self, state: &State) -> Result<Temp> {
        let base = self.materialize(state)?;
        let name = self.next_name();
        let machine = self.build.carbon.machine(&base.name).name(&name).platform(state.platform.clone()).create()?;
        let temp = Temp(Some(machine));
        for dir in &state.pending_dirs {
            temp.get().files().mkdir(dir)?;
        }
        Ok(temp)
    }

    /// Freezes a step's machine, and moves the state on to it.
    fn finish(&mut self, state: &mut State, temp: Temp, created_by: String, key: String) -> Result<()> {
        let image = temp.get().freeze_with(&Freeze::new().source("built").config(state.config.clone()).step(created_by))?;
        drop(temp);
        self.remember(&key, &image);
        state.history = image.history.clone();
        state.image = image;
        state.key = key;
        state.pending_dirs.clear();
        self.ran += 1;
        Ok(())
    }

    fn restore_cached(&self, state: &mut State, key: &str) -> bool {
        match self.cached(key) {
            Some(image) => {
                state.history = image.history.clone();
                state.config = image.config.clone();
                state.image = image;
                state.key = key.to_string();
                state.pending_dirs.clear();
                true
            }
            None => false,
        }
    }

    /// Makes the folders WORKDIR named that no disk step has made yet, at a stage's end.
    fn flush_dirs(&mut self, state: &mut State, stage: usize) -> Result<()> {
        if state.pending_dirs.is_empty() {
            return Ok(());
        }
        let created_by = format!("WORKDIR {}", state.pending_dirs.join(" "));
        let key = sha(&[&state.key, "mkdir", &created_by]);
        if !self.restore_cached(state, &key) {
            let temp = self.create(state)?;
            // The history already has WORKDIR's own entries: they're this step's, now that it holds their folders.
            let mut history = state.history.clone();
            // Only those since the last step that changed the disk.
            for entry in history.iter_mut().rev().take_while(|h| h.empty_layer).filter(|h| h.created_by.starts_with("WORKDIR ")) {
                entry.empty_layer = false;
            }
            let image = temp.get().freeze_with(&Freeze::new().source("built").config(state.config.clone()).history(history))?;
            drop(temp);
            self.remember(&key, &image);
            state.history = image.history.clone();
            state.image = image;
            state.key = key;
            state.pending_dirs.clear();
        }
        self.stages[stage] = Some(state.clone());
        Ok(())
    }

    /// `RUN`: a machine launched from the state, run to its end, frozen. Whether it was found in the cache.
    fn run(&mut self, state: &mut State, stage: usize, n: usize, ins: &Instruction) -> Result<bool> {
        let mut network = self.build.network;
        for flag in &ins.flags {
            let (key, value) = flag.split_once('=').unwrap_or((flag.as_str(), ""));
            match key {
                "network" => {
                    network = Some(match value {
                        "default" => self.build.network.unwrap_or(Network::Nat),
                        "none" => Network::None,
                        "host" => Network::Host,
                        _ => return Err(self.bad(ins, format!("--network={value}: default, none or host"))),
                    })
                }
                "mount" => {}
                "security" => {}
                _ => return Err(self.bad(ins, format!("unknown flag: {key}"))),
            }
        }
        // What runs: the exec form as it is; the shell form under the stage's shell; here-documents as scripts.
        let shell = state.shell();
        let mut script_file: Option<(String, String)> = None;
        let (argv, created_by) = match &ins.args {
            Args::Json(list) => (list.clone(), format!("RUN {}", serde_json::to_string(list).unwrap_or_default())),
            Args::Text(text) if !ins.heredocs.is_empty() => {
                let only = ins.heredocs.len() == 1 && text.trim().trim_start_matches(|c: char| c.is_ascii_digit()).starts_with("<<") && !text.trim().contains(char::is_whitespace);
                if only && ins.heredocs[0].content.starts_with("#!") {
                    let path = format!("/.carbon-run-{}", self.id);
                    script_file = Some((path.clone(), ins.heredocs[0].content.clone()));
                    (vec![path], format!("RUN <<{}\n{}{}", ins.heredocs[0].name, ins.heredocs[0].content, ins.heredocs[0].name))
                } else if only {
                    let content = ins.heredocs[0].content.clone();
                    (shell.iter().cloned().chain([content.clone()]).collect(), format!("RUN {} {content}", shell.join(" ")))
                } else {
                    let mut script = format!("{text}\n");
                    for doc in &ins.heredocs {
                        script.push_str(&doc.content);
                        script.push_str(&doc.name);
                        script.push('\n');
                    }
                    (shell.iter().cloned().chain([script.clone()]).collect(), format!("RUN {} {script}", shell.join(" ")))
                }
            }
            Args::Text(text) => (shell.iter().cloned().chain([text.clone()]).collect(), format!("RUN {} {text}", shell.join(" "))),
            _ => return Err(self.bad(ins, "RUN needs a command")),
        };
        // ARGs reach RUN as variables (unless ENV has them), and the proxy variables always.
        let mut env: Vec<(String, String)> = state.args.iter().filter_map(|(k, v)| v.clone().map(|v| (k.clone(), v))).filter(|(k, _)| !state.config.env.iter().any(|e| e.split('=').next() == Some(k))).collect();
        for (k, v) in &self.build.args {
            if PROXY_ARGS.contains(&k.as_str()) && !env.iter().any(|(n, _)| n == k) {
                env.push((k.clone(), v.clone()));
            }
        }
        env.sort();
        let env_text = env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\0");
        let net = network.map(crate::network_name).unwrap_or("default");
        // Its mounts: caches, parts of the context, tmpfs, secrets. What a bound part of the context holds is in the key.
        let (mounts, secret_env, mount_key) = self.run_mounts(state, ins)?;
        let key = sha(&[&state.key, &created_by, &env_text, net, &state.platform.to_string(), &mount_key]);
        if self.restore_cached(state, &key) {
            return Ok(true);
        }
        let temp = self.create(state)?;
        if let Some((path, content)) = &script_file {
            temp.get().files().write(path, content, Write::new().mode(0o755))?;
        }
        let name = temp.get().name().to_string();
        let mut spec = self.build.carbon.again(&name).isolation(self.build.isolation).rules(self.build.rules.clone());
        for mount in mounts {
            spec = spec.mount(mount);
        }
        for (k, v) in &secret_env {
            spec = spec.env(k, v);
        }
        for (k, v) in &env {
            spec = spec.env(k, v);
        }
        if let Some(network) = network {
            spec = spec.network(network);
        }
        if let Some(memory) = &self.build.memory {
            spec = spec.memory(memory.as_str());
        }
        if let Some(timeout) = self.build.timeout {
            spec = spec.timeout(timeout);
        }
        for hooks in &self.build.hooks {
            spec = spec.hooks(hooks.clone());
        }
        for (filter, f) in &self.build.exec_hooks {
            let f = f.clone();
            spec = spec.on_exec(filter.clone(), move |e| f(e));
        }
        for (filter, f) in &self.build.connect_hooks {
            let f = f.clone();
            spec = spec.on_connect(filter.clone(), move |e| f(e));
        }
        let machine = spec.launch(argv)?;
        for line in machine.logs(Logs::new().follow())? {
            self.say(Step::Output { stage, n, line });
        }
        machine.wait(None);
        let exit = machine.info()?.exit;
        if exit != Some(0) {
            let code = exit.map_or("no exit code".to_string(), |c| format!("exit code {c}"));
            return Err(Error::new(ErrorKind::Failed, format!("line {}: {} failed: {code}", ins.line, ins.original)));
        }
        if let Some((path, _)) = &script_file {
            temp.get().files().remove(path, crate::Remove::new())?;
        }
        self.finish(state, temp, created_by, key)?;
        Ok(false)
    }

    /// A `RUN`'s `--mount`s, as the machine's mounts: `type=cache` a volume kept between builds (by its `id`, else
    /// its target), `bind` part of the context read-only, `tmpfs`, and `secret` a file the build was given, in a
    /// folder of its own at `/run/secrets` (or as a variable, with `env=`). Also the variables secrets give, and what
    /// goes in the step's key: a bound part of the context's contents, a cache's and a secret's names (not their
    /// contents, as Docker's).
    fn run_mounts(&mut self, state: &State, ins: &Instruction) -> Result<RunMounts> {
        let lookup = lookup_owned(state);
        let (mut mounts, mut env, mut key) = (Vec::new(), Vec::new(), Vec::new());
        for text in ins.flags("mount") {
            let text = self.lex.word(text, &lookup).map_err(|e| self.bad(ins, e))?;
            let mut o: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
            for part in text.split(',') {
                let (k, v) = part.split_once('=').unwrap_or((part, "true"));
                let k = match k {
                    "dst" | "destination" => "target",
                    "src" => "source",
                    "readonly" => "ro",
                    k => k,
                };
                o.insert(k.to_string(), v.trim_matches('"').to_string());
            }
            let kind = o.get("type").map_or("bind", String::as_str).to_string();
            let target = o.get("target").map(|t| state.path(t));
            match kind.as_str() {
                "cache" => {
                    let target = target.ok_or_else(|| self.bad(ins, "--mount=type=cache needs a target"))?;
                    let id = o.get("id").cloned().unwrap_or_else(|| target.clone());
                    let name = format!("build-cache-{}", &sha(&[&id])[..16]);
                    key.push(format!("cache {id} {target}"));
                    let mount = crate::Mount::volume(name, target);
                    mounts.push(if o.contains_key("ro") { mount.read_only() } else { mount });
                }
                "tmpfs" => {
                    let target = target.ok_or_else(|| self.bad(ins, "--mount=type=tmpfs needs a target"))?;
                    key.push(format!("tmpfs {target}"));
                    mounts.push(crate::Mount::tmpfs(target));
                }
                "bind" => {
                    let target = target.ok_or_else(|| self.bad(ins, "--mount=type=bind needs a target"))?;
                    if o.contains_key("from") {
                        return Err(Error::unsupported(format!("line {}: --mount=type=bind,from= (a stage's or image's files) isn't there yet: COPY --from them", ins.line)));
                    }
                    if o.contains_key("rw") {
                        return Err(Error::unsupported(format!("line {}: --mount=type=bind,rw (writes thrown away after) isn't there yet", ins.line)));
                    }
                    let source = o.get("source").cloned().unwrap_or_else(|| ".".into());
                    let rel = self.context_matches(&source, ins)?.into_iter().next().unwrap_or_default();
                    key.push(format!("bind {rel} {target} {}", self.hash_context(&rel)?));
                    mounts.push(crate::Mount::bind(self.build.context.join(&rel), target).read_only());
                }
                "secret" => {
                    let id = o.get("id").cloned().or_else(|| o.get("target").and_then(|t| t.rsplit('/').next().map(String::from))).ok_or_else(|| self.bad(ins, "--mount=type=secret needs an id"))?;
                    let required = o.get("required").is_some_and(|v| v != "false");
                    let Some(source) = self.build.secrets.get(&id).cloned() else {
                        if required {
                            return Err(self.bad(ins, format!("secret {id} is required, and the build wasn't given it (--secret id={id},src=FILE)")));
                        }
                        continue;
                    };
                    key.push(format!("secret {id}"));
                    if let Some(var) = o.get("env") {
                        let value = std::fs::read_to_string(&source).map_err(|e| Error::io(format!("secret {id}: {e}")))?;
                        env.push((var.clone(), value.trim_end_matches('\n').to_string()));
                        if !o.contains_key("target") {
                            continue;
                        }
                    }
                    let target = o.get("target").map_or_else(|| format!("/run/secrets/{id}"), |t| state.path(t));
                    let (dir, file) = target.rsplit_once('/').unwrap_or(("", &target));
                    if dir != "/run/secrets" {
                        return Err(Error::unsupported(format!("line {}: a secret's target must be under /run/secrets for now (it was {target})", ins.line)));
                    }
                    // Each secret in a folder of the build's own, bound read-only; gone when the build is.
                    let folder = self.secrets_dir()?;
                    std::fs::copy(&source, folder.join(file)).map_err(|e| Error::io(format!("secret {id}: {e}")))?;
                    if !mounts.iter().any(|m: &crate::Mount| m.target == "/run/secrets") {
                        mounts.push(crate::Mount::bind(folder, "/run/secrets").read_only());
                    }
                }
                "ssh" => return Err(Error::unsupported(format!("line {}: --mount=type=ssh isn't there yet", ins.line))),
                other => return Err(self.bad(ins, format!("--mount=type={other}: bind, cache, tmpfs, secret or ssh"))),
            }
        }
        Ok((mounts, env, key.join("\n")))
    }

    /// This build's folder for secrets, made the first time it's wanted, and removed when the build ends.
    fn secrets_dir(&mut self) -> Result<PathBuf> {
        let dir = self.build.carbon.home().join("build").join("secrets").join(&self.id);
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(format!("{}: {e}", dir.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        self.secrets = Some(dir.clone());
        Ok(dir)
    }

    /// `COPY` and `ADD`: into a machine made from the state, never run, then frozen. Whether it was found in the cache.
    fn copy(&mut self, state: &mut State, _stage: usize, _n: usize, ins: &Instruction) -> Result<bool> {
        let add = ins.keyword == "add";
        let lookup = lookup_owned(state);
        let word = |text: &str| self.lex.word(text, &lookup).map_err(|e| self.bad(ins, e));
        let mut how = Copy::new();
        let (mut from, mut chmod, mut excludes, mut chown) = (None, None, Vec::new(), None::<String>);
        for flag in &ins.flags {
            let (key, value) = flag.split_once('=').unwrap_or((flag.as_str(), "true"));
            match key {
                "from" if !add => from = Some(word(value)?),
                "chown" => {
                    let owner = word(value)?;
                    how = how.chown(&owner);
                    chown = Some(owner);
                }
                "chmod" => {
                    let v = word(value)?;
                    let mode = u32::from_str_radix(&v, 8).map_err(|_| self.bad(ins, format!("--chmod={v}: only octal modes (0755) for now")))?;
                    chmod = Some(mode);
                    how = how.chmod(mode);
                }
                "exclude" => excludes.push(word(value)?),
                "link" => {}
                "parents" => return Err(Error::unsupported(format!("line {}: COPY --parents isn't there yet", ins.line))),
                "checksum" | "keep-git-dir" | "unpack" if add => {}
                _ => return Err(self.bad(ins, format!("unknown flag: {key}"))),
            }
        }
        if !excludes.is_empty() {
            how = how.exclude(Patterns::new(&excludes));
        }
        let items = match &ins.args {
            Args::Json(list) | Args::Words(list) => list.clone(),
            _ => return Err(self.bad(ins, format!("{} needs a source and a destination", ins.name()))),
        };
        if items.len() < 2 {
            return Err(self.bad(ins, format!("{} needs a source and a destination", ins.name())));
        }
        let dest_word = word(&items[items.len() - 1])?;
        let dest = state.path(&dest_word);
        // A folder if it says so (`/` at its end, `.`), if there's more than one source, or if it's one already.
        let last_part = dest_word.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
        let mut dest_dir = dest_word.ends_with('/') || dest_word.ends_with('\\') || matches!(last_part, "." | "..") || dest_word.is_empty() || items.len() > 2;
        let mut sources = Vec::new();
        for item in &items[..items.len() - 1] {
            match ins.heredocs.iter().find(|d| item.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches("<<").trim_start_matches('-').trim_matches(['"', '\'']) == d.name && item.contains("<<")) {
                Some(doc) => {
                    let content = if doc.expand {
                        let lex = Lexer { skip_quotes: true, raw_escapes: true, ..self.lex };
                        lex.word(&doc.content, &lookup).map_err(|e| self.bad(ins, e))?
                    } else {
                        doc.content.clone()
                    };
                    sources.push(Source::Heredoc(doc.name.clone(), content));
                }
                None => {
                    let src = word(item)?;
                    if add && (src.starts_with("http://") || src.starts_with("https://") || src.starts_with("git@")) {
                        return Err(Error::unsupported(format!("line {}: ADD of a URL or a git repository isn't there yet: RUN curl or git in the build", ins.line)));
                    }
                    sources.push(Source::Path(src));
                }
            }
        }
        let shown = format!("{} {}{} {}", ins.name(), ins.flags.iter().map(|f| format!("--{f} ")).collect::<String>(), items[..items.len() - 1].join(" "), dest);
        // What it copies decides its key: the files' contents and what they are, or the stage or image they're from.
        let mut inputs = Vec::new();
        let from_state = match &from {
            Some(f) => {
                let state = match self.file.stage(f) {
                    Some(j) => {
                        let mut s = self.stages[j].clone().ok_or_else(|| self.bad(ins, format!("--from={f}: that stage isn't built before this one")))?;
                        self.flush_dirs(&mut s, j)?;
                        s
                    }
                    None => {
                        let image = self.image(f, &state.platform.clone())?;
                        let key = sha(&[CACHE_VERSION, "from", &image.index]);
                        State { config: image.config.clone(), history: image.history.clone(), key, platform: image.platform.clone(), image, args: Vec::new(), pending_dirs: Vec::new(), cmd_set: false }
                    }
                };
                inputs.push(state.key.clone());
                Some(state)
            }
            None => {
                let mut count = 0;
                for source in &sources {
                    match source {
                        Source::Heredoc(name, content) => {
                            count += 1;
                            inputs.push(sha(&[name, content]));
                        }
                        Source::Path(src) => {
                            let matches = self.context_matches(src, ins)?;
                            count += matches.len();
                            for rel in &matches {
                                inputs.push(self.hash_context(rel)?);
                            }
                        }
                    }
                }
                if count > 1 && !dest_dir {
                    return Err(self.bad(ins, "with more than one source file, the destination must be a folder and end with a /"));
                }
                None
            }
        };
        let key = sha(&[&state.key, &shown, &inputs.join(","), &format!("{chmod:?}")]);
        if self.restore_cached(state, &key) {
            return Ok(true);
        }
        let temp = self.create(state)?;
        let files = temp.get().files();
        dest_dir |= files.stat(&dest).is_ok_and(|i| i.kind == crate::FileKind::Dir);
        let one_target = |name: &str| if dest_dir { format!("{}/{name}", dest.trim_end_matches('/')) } else { dest.clone() };
        match from_state {
            Some(source_state) => {
                let source = self.create(&source_state)?;
                let source_files = source.get().files();
                for src in &sources {
                    let Source::Path(src) = src else { return Err(self.bad(ins, "COPY --from can't copy a here-document")) };
                    let path = source_state.path(src);
                    let info = source_files.stat(&path).map_err(|e| self.bad(ins, format!("{src}: {e}")))?;
                    if info.kind == crate::FileKind::Dir {
                        files.mkdir(&dest)?;
                        files.copy_from(&source_files, &format!("{}/.", path.trim_end_matches('/')), &dest, &how)?;
                    } else {
                        let name = path.rsplit('/').next().unwrap_or_default().to_string();
                        let target = one_target(&name);
                        let (parent, leaf) = target.rsplit_once('/').unwrap_or(("", &target));
                        let parent = if parent.is_empty() { "/" } else { parent };
                        files.mkdir(parent)?;
                        files.copy_from(&source_files, &path, parent, &how)?;
                        if leaf != name {
                            files.rename(&format!("{}/{name}", parent.trim_end_matches('/')), &target)?;
                        }
                    }
                }
            }
            None => {
                for source in &sources {
                    match source {
                        Source::Heredoc(name, content) => {
                            files.write(&one_target(name), content, Write::new().mode(chmod.unwrap_or(0o644)).parents(true))?;
                        }
                        Source::Path(src) => {
                            for rel in self.context_matches(src, ins)? {
                                let host = self.build.context.join(&rel);
                                let meta = std::fs::symlink_metadata(&host).map_err(|e| Error::io(format!("{}: {e}", host.display())))?;
                                if meta.is_dir() {
                                    files.mkdir(&dest)?;
                                    let mut copy = how.clone().exclude_under(self.ignore.clone(), &rel);
                                    if !excludes.is_empty() {
                                        copy = copy.exclude(Patterns::new(&excludes));
                                    }
                                    temp.get().copy_in(host.join("."), &dest, &copy)?;
                                } else if add && is_archive(&host) {
                                    // A local archive is unpacked into the destination, as Docker's ADD does.
                                    let bytes = std::fs::read(&host).map_err(|e| Error::io(format!("{}: {e}", host.display())))?;
                                    files.mkdir(&dest)?;
                                    super::archive::unpack(&files, &bytes, &dest, chown.as_deref()).map_err(|e| self.bad(ins, format!("{src}: {e}")))?;
                                } else {
                                    let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
                                    let target = one_target(&name);
                                    let (parent, leaf) = target.rsplit_once('/').unwrap_or(("", &target));
                                    let parent = if parent.is_empty() { "/".to_string() } else { parent.to_string() };
                                    files.mkdir(&parent)?;
                                    temp.get().copy_in(&host, &parent, &how)?;
                                    if leaf != name {
                                        files.rename(&format!("{}/{name}", parent.trim_end_matches('/')), &target)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.finish(state, temp, shown, key)?;
        Ok(false)
    }

    /// The context's paths a source names (from the context's root), its wildcards matched, what the ignore file
    /// excludes left out. An error if it names nothing.
    fn context_matches(&self, src: &str, ins: &Instruction) -> Result<Vec<String>> {
        let rel = clean(src).trim_start_matches('/').to_string();
        let mut depth = 0i32;
        for part in src.split('/') {
            match part {
                "" | "." => {}
                ".." => depth -= 1,
                _ => depth += 1,
            }
            if depth < 0 {
                return Err(self.bad(ins, format!("{src}: outside the build's context")));
            }
        }
        let wild = rel.contains(['*', '?', '[']);
        let mut out = Vec::new();
        if wild {
            let pattern: Vec<String> = rel.split('/').map(String::from).collect();
            walk_matches(&self.build.context, &pattern, "", &mut out);
            out.retain(|r| !self.ignore.excludes(r));
            out.sort();
        } else if (rel.is_empty() || self.build.context.join(&rel).symlink_metadata().is_ok()) && (rel.is_empty() || !self.ignore.excludes(&rel) || self.build.context.join(&rel).is_dir() && self.ignore.may_include_inside(&rel)) {
            out.push(rel.clone());
        }
        if out.is_empty() {
            return Err(Error::not_found(format!("line {}: {}: {src}: not found in the build's context, or excluded by its ignore file", ins.line, ins.name())));
        }
        Ok(out)
    }

    /// What a context path is, for its step's key: each file's path, kind, mode and contents, the ignore file and the
    /// copy's exclusions applied.
    fn hash_context(&self, rel: &str) -> Result<String> {
        let mut parts = Vec::new();
        let root = self.build.context.join(rel);
        let mut todo = vec![(root, String::new())];
        while let Some((path, under)) = todo.pop() {
            let full_rel = if under.is_empty() { rel.to_string() } else if rel.is_empty() { under.clone() } else { format!("{rel}/{under}") };
            if !under.is_empty() && self.ignore.excludes(&full_rel) && !self.ignore.may_include_inside(&full_rel) {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path).map_err(|e| Error::io(format!("{}: {e}", path.display())))?;
            #[cfg(unix)]
            let mode = std::os::unix::fs::PermissionsExt::mode(&meta.permissions());
            #[cfg(not(unix))]
            let mode = 0u32;
            if meta.file_type().is_symlink() {
                parts.push(format!("l {under} {mode:o} {}", std::fs::read_link(&path).map(|t| t.to_string_lossy().into_owned()).unwrap_or_default()));
            } else if meta.is_dir() {
                parts.push(format!("d {under} {mode:o}"));
                let mut entries: Vec<_> = std::fs::read_dir(&path).map_err(|e| Error::io(format!("{}: {e}", path.display())))?.flatten().collect();
                entries.sort_by_key(|e| e.file_name());
                for entry in entries.into_iter().rev() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    todo.push((entry.path(), if under.is_empty() { name } else { format!("{under}/{name}") }));
                }
            } else if meta.is_file() {
                let bytes = std::fs::read(&path).map_err(|e| Error::io(format!("{}: {e}", path.display())))?;
                parts.push(format!("f {under} {mode:o} {}", images::store::hex(&images::store::sha256(&bytes))));
            }
        }
        Ok(sha(&[rel, &parts.join("\n")]))
    }

    /// Copies a built image's files to a host folder.
    fn output(&mut self, image: &Image, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(format!("{}: {e}", dir.display())))?;
        let name = self.next_name();
        let temp = Temp(Some(self.build.carbon.machine(&image.name).name(&name).create()?));
        for entry in temp.get().files().list("/")? {
            temp.get().copy_out(&format!("/{}", entry.name), dir, &Copy::new())?;
        }
        Ok(())
    }
}

/// A state's variables, owned, for a lookup that outlives borrowing it.
fn lookup_owned(state: &State) -> impl Fn(&str) -> Option<String> + use<> {
    let state = State { image: state.image.clone(), config: state.config.clone(), history: Vec::new(), key: String::new(), platform: state.platform.clone(), args: state.args.clone(), pending_dirs: Vec::new(), cmd_set: false };
    move |name: &str| state.get(name)
}

/// A `RUN`'s mounts, the variables its secrets give, and what of them goes in its key.
type RunMounts = (Vec<crate::Mount>, Vec<(String, String)>, String);

enum Source {
    Heredoc(String, String),
    Path(String),
}

/// Whether a file holds an archive `ADD` unpacks: by its first bytes, as Docker tells.
fn is_archive(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 512];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    super::archive::detect(&head[..n]).is_some()
}

/// The paths under `dir` that a wildcard pattern's parts match.
fn walk_matches(dir: &Path, pattern: &[String], under: &str, out: &mut Vec<String>) {
    let Some((first, rest)) = pattern.split_first() else { return };
    let p: Vec<char> = first.chars().collect();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let n: Vec<char> = name.chars().collect();
        if !super::lex::glob(&p, &n, '\\', true) {
            continue;
        }
        let rel = if under.is_empty() { name.clone() } else { format!("{under}/{name}") };
        if rest.is_empty() {
            out.push(rel);
        } else if entry.path().is_dir() {
            walk_matches(&entry.path(), rest, &rel, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_as_go_writes_them() {
        assert_eq!(duration("30s"), Some(30_000_000_000));
        assert_eq!(duration("1m30s"), Some(90_000_000_000));
        assert_eq!(duration("500ms"), Some(500_000_000));
        assert_eq!(duration("1.5h"), Some(5_400_000_000_000));
        assert_eq!(duration("0"), Some(0));
        assert_eq!(duration("5"), None);
        assert_eq!(duration("5x"), None);
    }

    #[test]
    fn paths_from_the_working_folder() {
        assert_eq!(clean("/a/./b//c/../d/"), "/a/b/d");
        assert_eq!(clean("/../x"), "/x");
    }

    #[test]
    fn hashes_differ_by_what_they_join() {
        assert_ne!(sha(&["a", "bc"]), sha(&["ab", "c"]));
    }
}
