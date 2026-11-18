//! Images and volumes, as Docker's commands have them: pull, build, freeze, tag, push, remove, save, load.

use std::time::{Duration, Instant};

use carbon::{Copy, Freeze, Platform, Push};
use serde_json::{Value, json};

use super::{Is, Tool, prop, strings_prop};
use crate::shape::{self, Args, Failure, Outcome};
use crate::{Progress, Server};

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_images",
            title: "List images",
            description: "The images here: name, platform (linux/arm64, windows/amd64…), and where each came from.",
            properties: json!({}),
            required: &[],
            is: Is::ReadOnly,
            on_machine: false,
            host: false,
            run: list,
        },
        Tool {
            name: "inspect_image",
            title: "Inspect an image",
            description: "An image's config (entrypoint, command, environment, user, working folder, exposed ports, volumes, labels) and the history of steps that made it.",
            properties: json!({ "image": prop("string", "Its name: debian:13") }),
            required: &["image"],
            is: Is::ReadOnly,
            on_machine: false,
            host: false,
            run: inspect,
        },
        Tool {
            name: "pull_image",
            title: "Pull an image",
            description: "Pulls an image from its registry, as docker pull. run and start_machine pull what they need themselves; this is for pulling ahead.",
            properties: json!({ "image": prop("string", "Its name: debian:13, ghcr.io/org/app:1"), "platform": prop("string", "linux/arm64, linux/amd64, windows/arm64") }),
            required: &["image"],
            is: Is::OpenWorld,
            on_machine: false,
            host: false,
            run: pull,
        },
        Tool {
            name: "build_image",
            title: "Build an image",
            description: "Builds an image from a Carbonfile, which is a Dockerfile exactly, as docker build. Its context is one of: `files` (a map of paths to text, with the Dockerfile as `carbonfile`), a folder in a machine (`machine` and `path`: write the code in a machine, then build it), or a shared host folder (`context`). Each step's output comes back; the server's rules hold on every RUN.",
            properties: json!({
                "tag": prop("string", "Its name: app:1"),
                "carbonfile": prop("string", "The Dockerfile's text (else the context's Carbonfile, or its Dockerfile)"),
                "files": { "type": "object", "additionalProperties": { "type": "string" }, "description": "The context, as paths and their text: {\"app.py\": \"print('hi')\\n\"}" },
                "machine": prop("string", "A machine whose folder is the context"),
                "path": prop("string", "That folder, in the machine"),
                "context": prop("string", "A host folder as the context, under a shared host folder"),
                "build_args": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Values for the ARGs it declares" },
                "target": prop("string", "The stage to build; the last by default"),
                "platform": prop("string", "What to build for: linux/arm64, linux/amd64"),
                "no_cache": prop("boolean", "Run every step again, finding nothing in the cache"),
            }),
            required: &[],
            is: Is::OpenWorld,
            on_machine: false,
            host: false,
            run: build,
        },
        Tool {
            name: "freeze_machine",
            title: "Make an image of a machine",
            description: "Makes an image of a stopped machine's disk, as docker commit: to start other machines from, or to keep a known-good state. Stop the machine first.",
            properties: json!({ "machine": prop("string", "The machine, by name"), "name": prop("string", "The image's name: dev-snapshot:1") }),
            required: &["machine", "name"],
            is: Is::Changes,
            on_machine: false,
            host: false,
            run: freeze,
        },
        Tool {
            name: "tag_image",
            title: "Tag an image",
            description: "Gives an image another name, as docker tag: the same image, under both.",
            properties: json!({ "image": prop("string", "Its name now"), "name": prop("string", "Its other name: registry.example.com/team/app:2") }),
            required: &["image", "name"],
            is: Is::Idempotent,
            on_machine: false,
            host: false,
            run: tag,
        },
        Tool {
            name: "push_image",
            title: "Push an image",
            description: "Pushes an image to its registry, as docker push, a layer per step, with this host's login for that registry. Tag it with the registry's name first.",
            properties: json!({ "image": prop("string", "Its name: someone/app:1, ghcr.io/team/app:2"), "squash": prop("boolean", "Push it as one layer") }),
            required: &["image"],
            is: Is::OpenWorld,
            on_machine: false,
            host: false,
            run: push,
        },
        Tool {
            name: "remove_image",
            title: "Remove an image",
            description: "Forgets an image's name, as docker rmi. Machines made from it keep their files.",
            properties: json!({ "image": prop("string", "Its name") }),
            required: &["image"],
            is: Is::Destructive,
            on_machine: false,
            host: false,
            run: remove,
        },
        Tool {
            name: "save_image",
            title: "Save images to an archive",
            description: "Writes images as one archive, as docker save (docker load reads it), to a file under a shared host folder.",
            properties: json!({ "images": strings_prop("Their names"), "host_path": prop("string", "The archive to write: /…/images.tar") }),
            required: &["images", "host_path"],
            is: Is::Changes,
            on_machine: false,
            host: true,
            run: save,
        },
        Tool {
            name: "load_image",
            title: "Load images from an archive",
            description: "Loads the images an archive has (docker save's, or an OCI layout's, gzipped or not), from a file under a shared host folder.",
            properties: json!({ "host_path": prop("string", "The archive") }),
            required: &["host_path"],
            is: Is::Changes,
            on_machine: false,
            host: true,
            run: load,
        },
        Tool {
            name: "list_volumes",
            title: "List volumes",
            description: "The volumes here: folders Carbon keeps that machines mount and outlive.",
            properties: json!({}),
            required: &[],
            is: Is::ReadOnly,
            on_machine: false,
            host: false,
            run: volumes,
        },
        Tool {
            name: "create_volume",
            title: "Create a volume",
            description: "Makes a volume, as docker volume create. Mounting one makes it too: mounts: [\"type=volume,source=NAME,target=/data\"].",
            properties: json!({ "name": prop("string", "Its name: letters, digits, _ . and -") }),
            required: &["name"],
            is: Is::Idempotent,
            on_machine: false,
            host: false,
            run: create_volume,
        },
        Tool {
            name: "remove_volume",
            title: "Remove a volume",
            description: "Removes a volume and its files, as docker volume rm. Not while a running machine mounts it.",
            properties: json!({ "name": prop("string", "Its name") }),
            required: &["name"],
            is: Is::Destructive,
            on_machine: false,
            host: false,
            run: remove_volume,
        },
    ]
}

fn image_row(i: &carbon::Image) -> Value {
    json!({ "image": i.short_name(), "platform": i.platform.to_string(), "source": i.source, "created": shape::time(std::time::UNIX_EPOCH + Duration::from_secs(i.created)) })
}

fn list(server: &Server, _: &Args, _: &Progress) -> Outcome {
    Ok(shape::data(json!({ "images": server.carbon.images().iter().map(image_row).collect::<Vec<_>>() })))
}

fn inspect(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let name = args.str("image")?;
    let image = server.carbon.image(&name).ok_or_else(|| Failure(format!("there's no image {name} here: pull_image pulls it")))?;
    let v: Value = serde_json::from_str(&carbon::image_json(&image)).unwrap_or(Value::Null);
    Ok(shape::data(v))
}

fn pull(server: &Server, args: &Args, progress: &Progress) -> Outcome {
    let name = args.str("image")?;
    let platform = args.opt_str("platform").map(|p| Platform::parse(&p).ok_or_else(|| Failure(format!("{p} isn't a platform")))).transpose()?;
    let mut said = Instant::now();
    let image = server.carbon.pull_with(&name, platform, |p| {
        if said.elapsed() >= Duration::from_millis(500) {
            said = Instant::now();
            progress.say(&p.to_string(), None);
        }
    })?;
    Ok(shape::data(image_row(&image)))
}

/// A folder removed when it's dropped.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build(server: &Server, args: &Args, progress: &Progress) -> Outcome {
    let scratch = Scratch(server.carbon.home().join("tmp").join(format!("mcp-build-{}", shape::new_id())));
    let context = if let Some(dir) = args.opt_str("context") {
        shape::host_path(&server.host_dirs, &dir, true)?
    } else if let Some(machine) = args.opt_str("machine") {
        let path = args.str("path")?;
        if let Some(only) = &server.options.machine
            && *only != machine
        {
            return Err(Failure(format!("this server serves {only} alone")));
        }
        std::fs::create_dir_all(&scratch.0)?;
        let m = server.carbon.get(&machine)?;
        // The folder, copied into the scratch folder as docker cp copies one into a folder there: the context.
        let trimmed = path.trim_end_matches('/');
        let base = trimmed.rsplit('/').next().filter(|b| !b.is_empty()).unwrap_or("context");
        m.copy_out(trimmed, &scratch.0, &Copy::new())?;
        let dir = scratch.0.join(base);
        if !dir.is_dir() {
            return Err(Failure(format!("{path} in {machine} isn't a folder")));
        }
        dir
    } else {
        std::fs::create_dir_all(&scratch.0)?;
        if let Some(Value::Object(files)) = args.get("files") {
            for (rel, text) in files {
                let rel = std::path::Path::new(rel);
                if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                    return Err(Failure(format!("{}: a context file's path must be relative, inside the context", rel.display())));
                }
                let at = scratch.0.join(rel);
                if let Some(parent) = at.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&at, text.as_str().unwrap_or_default())?;
            }
        }
        scratch.0.clone()
    };
    let mut b = server.carbon.build(&context).rules(server.rules.clone());
    if let Some(text) = args.opt_str("carbonfile") {
        std::fs::create_dir_all(&scratch.0)?;
        let file = scratch.0.join(".carbon-mcp-Carbonfile");
        std::fs::write(&file, text)?;
        b = b.file(file);
    }
    if let Some(t) = args.opt_str("tag") {
        b = b.tag(t);
    }
    for (k, v) in args.env("build_args") {
        b = b.arg(k, v);
    }
    if let Some(t) = args.opt_str("target") {
        b = b.target(t);
    }
    if let Some(p) = args.opt_str("platform") {
        b = b.platform(Platform::parse(&p).ok_or_else(|| Failure(format!("{p} isn't a platform")))?);
    }
    if let Some(n) = server.options.network {
        b = b.network(n);
    }
    if let Some(m) = &server.options.memory {
        b = b.memory(m.clone());
    }
    b = b.no_cache(args.bool("no_cache"));
    let mut log = Vec::new();
    let built = b.run_with(|step| {
        let line = match step {
            carbon::build::Step::Output { line, .. } => format!("    {}", line.text()),
            carbon::build::Step::Done { .. } => return,
            other => {
                progress.say(&other.to_string(), None);
                format!("{other}\n")
            }
        };
        log.extend_from_slice(line.as_bytes());
    });
    let (text, _) = shape::clip(&log, shape::OUTPUT_CAP);
    match built {
        Ok(built) => {
            let summary = format!("built {} in {:.1}s: {} ran, {} cached\n", built.image.short_name(), built.took.as_secs_f64(), built.ran, built.cached);
            Ok(shape::text_and(summary + &text, json!({ "image": built.image.short_name(), "ran": built.ran, "cached": built.cached, "log": text })))
        }
        Err(e) => Ok(shape::failed(&format!("the build failed: {e}\n{text}"))),
    }
}

fn freeze(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let machine = server.carbon.get(&args.str("machine")?)?;
    if machine.is_running() {
        return Err(Failure(format!("{} is running: stop_machine first (an image of a running machine isn't there yet)", machine.name())));
    }
    let image = machine.freeze_with(&Freeze::new().name(args.str("name")?))?;
    Ok(shape::data(image_row(&image)))
}

fn tag(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let image = server.carbon.tag(&args.str("image")?, &args.str("name")?)?;
    Ok(shape::data(image_row(&image)))
}

fn push(server: &Server, args: &Args, progress: &Progress) -> Outcome {
    let name = args.str("image")?;
    let mut said = Instant::now();
    let pushed = server.carbon.push_with(&name, &Push::new().squash(args.bool("squash")), |p| {
        if said.elapsed() >= Duration::from_millis(500) {
            said = Instant::now();
            progress.say(&p.to_string(), None);
        }
    })?;
    Ok(shape::data(json!({ "pushed": format!("{}@{}", pushed.name, pushed.digest), "layers": pushed.layers, "sent": pushed.sent })))
}

fn remove(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let name = args.str("image")?;
    server.carbon.remove_image(&name)?;
    Ok(shape::text(format!("removed {name}")))
}

fn save(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let names = args.strings("images");
    if names.is_empty() {
        return Err(Failure("`images` is needed: the images to save".into()));
    }
    let path = shape::host_path(&server.host_dirs, &args.str("host_path")?, false)?;
    let partial = path.with_extension("partial");
    let file = std::fs::File::create(&partial)?;
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    if let Err(e) = server.carbon.save(&names, std::io::BufWriter::with_capacity(1 << 20, file)) {
        let _ = std::fs::remove_file(&partial);
        return Err(e.into());
    }
    std::fs::rename(&partial, &path)?;
    let size = std::fs::metadata(&path).map_or(0, |m| m.len());
    Ok(shape::data(json!({ "saved": path.display().to_string(), "bytes": size, "images": names })))
}

fn load(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let path = shape::host_path(&server.host_dirs, &args.str("host_path")?, true)?;
    let loaded = server.carbon.load(&path)?;
    Ok(shape::data(json!({ "loaded": loaded.iter().map(image_row).collect::<Vec<_>>() })))
}

fn volumes(server: &Server, _: &Args, _: &Progress) -> Outcome {
    let list: Vec<Value> = server.carbon.volumes().iter().map(|v| json!({ "name": v.name, "created": shape::time(std::time::UNIX_EPOCH + Duration::from_secs(v.created)) })).collect();
    Ok(shape::data(json!({ "volumes": list })))
}

fn create_volume(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let name = args.str("name")?;
    server.carbon.volume(&name).create()?;
    Ok(shape::text(format!("volume {name}")))
}

fn remove_volume(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let name = args.str("name")?;
    server.carbon.remove_volume(&name)?;
    Ok(shape::text(format!("removed volume {name}")))
}
