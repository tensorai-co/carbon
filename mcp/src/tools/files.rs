//! A machine's files, running or stopped, through the library: never `cat` through `exec`, so binary files, long
//! ones and Windows machines' come back right.

use carbon::{Copy, FileInfo, FileKind, Files, Remove, Write};
use serde_json::{Value, json};

use super::{Is, Tool, prop, strings_prop};
use crate::shape::{self, Args, Failure, Outcome};
use crate::{Progress, Server};

/// The most of a file that comes back in one read.
const READ_CAP: u64 = 256 << 10;
/// The most a whole-file read (by lines, or an edit) takes.
const WHOLE_CAP: u64 = 16 << 20;
/// Pictures this small come back as images, for a model to see.
const IMAGE_CAP: u64 = 5 << 20;
const LIST_CAP: usize = 2000;

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "read_file",
            title: "Read a file",
            description: "Reads a file in a machine, running or stopped: text as it is, a picture (png, jpeg, gif, webp) as an image, anything else as base64. Up to 256 KiB at a time: give offset and length in bytes, or start_line and line_count, for more.",
            properties: json!({
                "path": prop("string", "Its absolute path in the machine (C:\\... in a Windows machine)"),
                "offset": prop("integer", "Where to start, in bytes"),
                "length": prop("integer", "How many bytes"),
                "start_line": prop("integer", "The first line to read, from 1 (text files)"),
                "line_count": prop("integer", "How many lines from start_line"),
            }),
            required: &["path"],
            is: Is::ReadOnly,
            on_machine: true,
            host: false,
            run: read,
        },
        Tool {
            name: "write_file",
            title: "Write a file",
            description: "Writes a file in a machine whole (or appends to it), making the folders it's in. Text in content, or bytes in content_base64.",
            properties: json!({
                "path": prop("string", "Its absolute path in the machine"),
                "content": prop("string", "Its text"),
                "content_base64": prop("string", "Its bytes, as base64, instead of content"),
                "mode": prop("string", "Its permission bits in octal: 644, 755 (Linux machines)"),
                "owner": prop("string", "Its owner, uid:gid (Linux machines)"),
                "append": prop("boolean", "Add to its end, rather than replacing it"),
            }),
            required: &["path"],
            is: Is::Changes,
            on_machine: true,
            host: false,
            run: write,
        },
        Tool {
            name: "edit_file",
            title: "Edit a file",
            description: "Replaces an exact string in a text file in a machine with another, keeping the file's mode and owner. It fails if the string isn't there, or is there more than once without replace_all: give enough around it to make it one.",
            properties: json!({
                "path": prop("string", "Its absolute path in the machine"),
                "old_string": prop("string", "The text to replace, exactly, whitespace and all"),
                "new_string": prop("string", "What to put in its place"),
                "replace_all": prop("boolean", "Replace every one, not just the one"),
            }),
            required: &["path", "old_string", "new_string"],
            is: Is::Changes,
            on_machine: true,
            host: false,
            run: edit,
        },
        Tool {
            name: "list_files",
            title: "List a folder",
            description: "A folder's entries in a machine, each with its kind, size, mode, owner, time and link target; depth to look into folders within.",
            properties: json!({
                "path": prop("string", "The folder's absolute path in the machine"),
                "depth": prop("integer", "How many levels: 1 (the default) is the folder's own entries, at most 5"),
            }),
            required: &["path"],
            is: Is::ReadOnly,
            on_machine: true,
            host: false,
            run: list,
        },
        Tool {
            name: "move_file",
            title: "Move a file",
            description: "Moves or renames a file or folder in a machine, as mv.",
            properties: json!({ "from": prop("string", "Its absolute path"), "to": prop("string", "Where it goes") }),
            required: &["from", "to"],
            is: Is::Changes,
            on_machine: true,
            host: false,
            run: rename,
        },
        Tool {
            name: "remove_file",
            title: "Remove a file",
            description: "Removes a file, a link or an empty folder in a machine; with recursive, a folder and everything in it, as rm -r.",
            properties: json!({ "path": prop("string", "Its absolute path"), "recursive": prop("boolean", "A folder and all in it") }),
            required: &["path"],
            is: Is::Destructive,
            on_machine: true,
            host: false,
            run: remove,
        },
        Tool {
            name: "make_folder",
            title: "Make a folder",
            description: "Makes a folder in a machine, and the folders it's in, as mkdir -p.",
            properties: json!({ "path": prop("string", "Its absolute path") }),
            required: &["path"],
            is: Is::Idempotent,
            on_machine: true,
            host: false,
            run: mkdir,
        },
        Tool {
            name: "copy_to_machine",
            title: "Copy from the host into a machine",
            description: "Copies a file or folder from a shared host folder into a machine, as docker cp: kinds, modes, times and links kept.",
            properties: json!({
                "host_path": prop("string", "What to copy, under a host folder this server shares"),
                "path": prop("string", "Where it goes in the machine: a folder it goes into, or a new name"),
                "exclude": strings_prop("Patterns to leave out, as an ignore file's: node_modules, **/*.log"),
            }),
            required: &["host_path", "path"],
            is: Is::Changes,
            on_machine: true,
            host: true,
            run: copy_in,
        },
        Tool {
            name: "copy_from_machine",
            title: "Copy from a machine to the host",
            description: "Copies a file or folder from a machine to a shared host folder, as docker cp: kinds, modes, times and links kept.",
            properties: json!({
                "path": prop("string", "What to copy, in the machine"),
                "host_path": prop("string", "Where it goes, under a host folder this server shares"),
                "exclude": strings_prop("Patterns to leave out, as an ignore file's"),
            }),
            required: &["path", "host_path"],
            is: Is::Changes,
            on_machine: true,
            host: true,
            run: copy_out,
        },
        Tool {
            name: "copy_between_machines",
            title: "Copy between machines",
            description: "Copies a file or folder from one machine's disk to another's, nothing through the host: kinds, modes, owners, times and links kept.",
            properties: json!({
                "from_machine": prop("string", "The machine it's in"),
                "path": prop("string", "What to copy"),
                "to_machine": prop("string", "The machine it goes to"),
                "into": prop("string", "Where it goes there"),
            }),
            required: &["from_machine", "path", "to_machine", "into"],
            is: Is::Changes,
            on_machine: false,
            host: false,
            run: copy_between,
        },
    ]
}

fn files(server: &Server, args: &Args) -> Result<Files, Failure> {
    Ok(server.machine(args)?.files())
}

fn kind_letter(k: FileKind) -> &'static str {
    match k {
        FileKind::File => "file",
        FileKind::Dir => "dir",
        FileKind::Symlink => "link",
    }
}

fn info_json(path: &str, i: &FileInfo) -> Value {
    json!({
        "path": path,
        "kind": kind_letter(i.kind),
        "size": i.size,
        "mode": format!("{:o}", i.mode & 0o7777),
        "owner": format!("{}:{}", i.uid, i.gid),
        "modified": shape::time(i.modified),
        "link": i.link,
    })
}

fn picture(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    [(".png", "image/png"), (".jpg", "image/jpeg"), (".jpeg", "image/jpeg"), (".gif", "image/gif"), (".webp", "image/webp")].into_iter().find(|(e, _)| lower.ends_with(e)).map(|(_, m)| m)
}

fn read(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let files = files(server, args)?;
    let path = args.str("path")?;
    let info = files.stat(&path)?;
    if info.kind == FileKind::Dir {
        return Err(Failure(format!("{path} is a folder: list_files lists it")));
    }
    if let Some(mime) = picture(&path).filter(|_| info.size <= IMAGE_CAP && args.get("offset").is_none()) {
        let bytes = files.read(&path, ..)?;
        return Ok(json!({ "content": [{ "type": "image", "data": shape::base64(&bytes), "mimeType": mime }], "isError": false }));
    }
    if let Some(start) = args.u64("start_line") {
        let bytes = files.read(&path, ..WHOLE_CAP)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| Failure(format!("{path} isn't text: read it by offset and length")))?;
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let first = start.max(1) as usize - 1;
        let count = args.u64("line_count").unwrap_or(lines.len() as u64) as usize;
        let mut out = String::new();
        let mut last = first;
        for line in lines.iter().skip(first).take(count) {
            if out.len() + line.len() > READ_CAP as usize {
                break;
            }
            out.push_str(line);
            last += 1;
        }
        let head = format!("lines {} to {last} of {} ({} bytes)\n", first + 1, lines.len(), info.size);
        return Ok(shape::text_and(head + &out, json!({ "path": path, "start_line": first + 1, "end_line": last, "lines": lines.len(), "size": info.size, "text": out })));
    }
    let offset = args.u64("offset").unwrap_or(0);
    let length = args.u64("length").unwrap_or(READ_CAP).min(READ_CAP);
    let bytes = files.read(&path, offset..offset.saturating_add(length))?;
    let end = offset + bytes.len() as u64;
    let more = if end < info.size { format!("; {} bytes more: read from offset {end}", info.size - end) } else { String::new() };
    // Text, unless it isn't (a cut through a character at the end isn't a reason to think so).
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => Some(t.to_string()),
        Err(e) if e.error_len().is_none() && bytes.len() - e.valid_up_to() < 4 => Some(String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()),
        Err(_) => None,
    };
    Ok(match text {
        Some(t) if offset == 0 && end >= info.size => shape::text_and(t.clone(), json!({ "path": path, "size": info.size, "text": t })),
        Some(t) => shape::text_and(format!("bytes {offset} to {end} of {}{more}\n{t}", info.size), json!({ "path": path, "offset": offset, "end": end, "size": info.size, "text": t })),
        None => {
            let b64 = shape::base64(&bytes);
            shape::text_and(format!("{path} isn't text: bytes {offset} to {end} of {} as base64{more}\n{b64}", info.size), json!({ "path": path, "offset": offset, "end": end, "size": info.size, "base64": b64 }))
        }
    })
}

/// Octal permission bits as a model gives them: "755", "0644", or 755 as a number.
fn mode(args: &Args) -> Result<Option<u32>, Failure> {
    let Some(text) = args.opt_str("mode") else { return Ok(None) };
    let digits = text.trim_start_matches("0o").trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    u32::from_str_radix(digits, 8).map(Some).map_err(|_| Failure(format!("mode {text}: octal, as 644 or 755")))
}

fn write(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let files = files(server, args)?;
    let path = args.str("path")?;
    let data = match (args.opt_str("content"), args.opt_str("content_base64")) {
        (_, Some(b)) => shape::unbase64(&b)?,
        (Some(t), None) => t.into_bytes(),
        (None, None) => Vec::new(),
    };
    let mut how = Write::new().parents(true).append(args.bool("append"));
    if let Some(m) = mode(args)? {
        how = how.mode(m);
    }
    if let Some(o) = args.opt_str("owner") {
        how = how.owner(&o);
    }
    files.write(&path, &data, how)?;
    Ok(shape::text(format!("wrote {} bytes to {path}", data.len())))
}

fn edit(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let files = files(server, args)?;
    let path = args.str("path")?;
    let (old, new) = (args.str("old_string")?, args.str("new_string")?);
    if old.is_empty() {
        return Err(Failure("old_string is empty: write_file writes a file whole".into()));
    }
    let info = files.stat(&path)?;
    if info.size > WHOLE_CAP {
        return Err(Failure(format!("{path} is {} bytes, too big to edit here", info.size)));
    }
    let text = String::from_utf8(files.read(&path, ..)?).map_err(|_| Failure(format!("{path} isn't text")))?;
    let count = text.matches(&old).count();
    let replaced = match count {
        0 => return Err(Failure(format!("old_string isn't in {path}: check its whitespace and line ends, or read the file again"))),
        1 => text.replacen(&old, &new, 1),
        n if args.bool("replace_all") => {
            let _ = n;
            text.replace(&old, &new)
        }
        n => return Err(Failure(format!("old_string is in {path} {n} times: give more around it to make it one, or replace_all"))),
    };
    let how = Write::new().mode(info.mode & 0o7777).owner(&format!("{}:{}", info.uid, info.gid));
    files.write(&path, replaced.as_bytes(), how)?;
    Ok(shape::text(format!("replaced {} in {path}", if count == 1 { "it".to_string() } else { format!("all {count}") })))
}

fn list(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let files = files(server, args)?;
    let root = args.str("path")?;
    let depth = args.u64("depth").unwrap_or(1).clamp(1, 5) as usize;
    let windows = root.contains('\\') || root.get(1..2) == Some(":");
    let sep = if windows { '\\' } else { '/' };
    let (mut entries, mut lines, mut cut) = (Vec::new(), Vec::new(), false);
    let mut stack = vec![(root.trim_end_matches(sep).to_string(), 1usize)];
    while let Some((dir, level)) = stack.pop() {
        let dir_path = if dir.is_empty() { sep.to_string() } else { dir.clone() };
        let mut listed = files.list(&dir_path)?;
        listed.sort_by(|a, b| a.name.cmp(&b.name));
        for e in listed.into_iter().rev() {
            if entries.len() >= LIST_CAP {
                cut = true;
                break;
            }
            let path = format!("{dir}{sep}{}", e.name);
            let i = &e.info;
            let mut line = format!("{:4} {:>4} {:>9} {:>10} {}  {}", kind_letter(i.kind), format!("{:o}", i.mode & 0o7777), format!("{}:{}", i.uid, i.gid), i.size, shape::time(i.modified), path);
            if let Some(l) = &i.link {
                line.push_str(&format!(" -> {l}"));
            }
            lines.push(line);
            entries.push(info_json(&path, i));
            if i.kind == FileKind::Dir && level < depth {
                stack.push((path, level + 1));
            }
        }
    }
    let mut text = lines.join("\n");
    if cut {
        text.push_str(&format!("\n[… only the first {LIST_CAP} entries]"));
    }
    Ok(shape::text_and(text, json!({ "entries": entries, "truncated": cut })))
}

fn rename(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let (from, to) = (args.str("from")?, args.str("to")?);
    files(server, args)?.rename(&from, &to)?;
    Ok(shape::text(format!("moved {from} to {to}")))
}

fn remove(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let path = args.str("path")?;
    let how = if args.bool("recursive") { Remove::recursive() } else { Remove::new() };
    files(server, args)?.remove(&path, how)?;
    Ok(shape::text(format!("removed {path}")))
}

fn mkdir(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let path = args.str("path")?;
    files(server, args)?.mkdir(&path)?;
    Ok(shape::text(format!("made {path}")))
}

fn copy_how(args: &Args) -> Copy {
    let patterns = args.strings("exclude");
    if patterns.is_empty() { Copy::new() } else { Copy::new().exclude(carbon::build::Patterns::parse(&patterns.join("\n"))) }
}

fn copy_in(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let host = shape::host_path(&server.host_dirs, &args.str("host_path")?, true)?;
    let into = args.str("path")?;
    let n = server.machine(args)?.copy_in(&host, &into, &copy_how(args))?;
    Ok(shape::text(format!("copied {n} files and folders into {into}")))
}

fn copy_out(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let host = shape::host_path(&server.host_dirs, &args.str("host_path")?, false)?;
    let path = args.str("path")?;
    let n = server.machine(args)?.copy_out(&path, &host, &copy_how(args))?;
    Ok(shape::text(format!("copied {n} files and folders to {}", host.display())))
}

fn copy_between(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let from = server.carbon.get(&args.str("from_machine")?)?;
    let to = server.carbon.get(&args.str("to_machine")?)?;
    let (path, into) = (args.str("path")?, args.str("into")?);
    let n = to.files().copy_from(&from.files(), &path, &into, &Copy::new())?;
    Ok(shape::text(format!("copied {n} files and folders from {}:{path} to {}:{into}", from.name(), to.name())))
}
