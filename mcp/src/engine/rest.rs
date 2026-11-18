//! The Engine API as REST, under `/v1`: each route the same method `engine/…` calls, its path's and query's parts
//! merged into the method's params with the JSON body. Machines are `/v1/machines/{name}`, images `/v1/images/{name}`
//! (whose names have slashes). Bytes (a file's) go as the body, raw.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use serde_json::{Value, json};

use super::{EngineError, call};
use crate::{Peer, Server};

/// What a route answers with.
pub enum Body {
    Json(Value),
    Bytes(Vec<u8>),
    Text(String),
}

/// A REST request's answer: its status and body.
pub struct Answer {
    pub status: u16,
    pub body: Body,
}

impl Answer {
    fn json(value: Value) -> Answer {
        Answer { status: 200, body: Body::Json(value) }
    }
    fn error(e: &EngineError) -> Answer {
        Answer { status: e.status(), body: Body::Json(json!({ "message": e.message, "kind": e.kind })) }
    }
}

/// Query names, as the params' names: `?path=` is `Path`.
fn param_name(query_name: &str) -> String {
    match query_name {
                "repo" => "Repo".into(),
        "tag" => "Tag".into(),
        "name" => "Name".into(),
        "force" => "Force".into(),
        "all" => "All".into(),
        "tail" => "Tail".into(),
        "platform" => "Platform".into(),
        "comment" => "Comment".into(),
        "squash" => "Squash".into(),
        "path" => "Path".into(),
        "recursive" => "Recursive".into(),
        "mode" => "Mode".into(),
        "parents" => "Parents".into(),
        "timeout" | "t" => "Timeout".into(),
        "to" => "To".into(),
        other => other.into(),
    }
}

/// Serves one REST request under `/v1`: `path` without its query, `query` decoded, `body` as sent.
pub fn route(server: &Server, peer: &Arc<Peer>, method: &str, path: &str, query: &HashMap<String, String>, body: &[u8]) -> Answer {
    let Some(rest) = path.strip_prefix("/v1/") else {
        return Answer::error(&EngineError::new("no_method", "the Engine API is under /v1"));
    };
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    let mut params = serde_json::Map::new();
    for (k, v) in query {
        params.insert(param_name(k), json!(v));
    }
    let take_body = |params: &mut serde_json::Map<String, Value>| -> Result<(), EngineError> {
        if body.iter().all(u8::is_ascii_whitespace) {
            return Ok(());
        }
        match serde_json::from_slice::<Value>(body) {
            Ok(Value::Object(o)) => {
                params.extend(o);
                Ok(())
            }
            Ok(_) => Err(EngineError::invalid("the body must be a JSON object")),
            Err(e) => Err(EngineError::invalid(format!("the body isn't JSON: {e}"))),
        }
    };
    // Which method, and how its params are made.
    let routed: Result<(&str, bool), EngineError> = (|| {
        Ok(match (method, parts.as_slice()) {
            ("GET", ["version"]) => ("system.version", false),
            ("GET" | "HEAD", ["_ping"]) => ("system.ping", false),
            ("GET", ["machines"]) => ("machines.list", false),
            ("POST", ["machines"]) => ("machines.create", true),
            ("GET", ["machines", id]) => id_route(&mut params, id, "machines.inspect", false),
            ("DELETE", ["machines", id]) => id_route(&mut params, id, "machines.remove", false),
            ("POST", ["machines", id, "start"]) => id_route(&mut params, id, "machines.start", false),
            ("POST", ["machines", id, "stop"]) => id_route(&mut params, id, "machines.stop", false),
            ("POST", ["machines", id, "kill"]) => id_route(&mut params, id, "machines.kill", false),
            ("POST", ["machines", id, "wait"]) => id_route(&mut params, id, "machines.wait", false),
            ("GET", ["machines", id, "logs"]) => id_route(&mut params, id, "machines.logs", false),
            ("POST", ["machines", id, "commit"]) => id_route(&mut params, id, "machines.commit", true),
            ("POST", ["machines", id, "exec"]) => id_route(&mut params, id, "machines.exec", true),
            ("GET", ["machines", id, "files"]) => id_route(&mut params, id, "files.read", false),
            ("PUT", ["machines", id, "files"]) => id_route(&mut params, id, "files.write", false),
            ("DELETE", ["machines", id, "files"]) => id_route(&mut params, id, "files.remove", false),
            ("GET", ["machines", id, "files", "list"]) => id_route(&mut params, id, "files.list", false),
            ("GET", ["machines", id, "files", "stat"]) => id_route(&mut params, id, "files.stat", false),
            ("GET", ["images"]) => ("images.list", false),
            ("POST", ["images", "pull"]) => ("images.pull", true),
            // An image's name has slashes: everything after `images/`, less a last part that's an action.
            ("POST", ["images", name @ .., "push"]) if !name.is_empty() => name_route(&mut params, name, "images.push", true),
            ("POST", ["images", name @ .., "tag"]) if !name.is_empty() => {
                params.insert("Source".into(), json!(name.join("/")));
                ("images.tag", true)
            }
            ("GET", ["images", name @ ..]) if !name.is_empty() => name_route(&mut params, name, "images.inspect", false),
            ("DELETE", ["images", name @ ..]) if !name.is_empty() => name_route(&mut params, name, "images.remove", false),
            _ => return Err(EngineError::new("no_method", format!("{method} {path} isn't a route of the Engine API"))),
        })
    })();
    let (name, json_params) = match routed {
        Ok(r) => r,
        Err(e) => return Answer::error(&e),
    };
    if json_params && let Err(e) = take_body(&mut params) {
        return Answer::error(&e);
    }
    if name == "files.write" {
        params.insert("Data".into(), json!(base64::engine::general_purpose::STANDARD.encode(body)));
    }
    peer.log(&format!("called engine/{name} ({method} {path})"));
    match call(server, peer, name, &Value::Object(params)) {
        Ok(value) if name == "system.ping" => Answer { status: 200, body: Body::Text(value.as_str().unwrap_or("OK").to_string()) },
        Ok(value) if name == "files.read" => {
            let data = value.get("Data").and_then(Value::as_str).unwrap_or("");
            match base64::engine::general_purpose::STANDARD.decode(data) {
                Ok(bytes) => Answer { status: 200, body: Body::Bytes(bytes) },
                Err(e) => Answer::error(&EngineError::new("failed", e.to_string())),
            }
        }
        Ok(value) if name == "machines.create" || name == "machines.commit" => Answer { status: 201, body: Body::Json(value) },
        Ok(value) if matches!(name, "machines.start" | "machines.stop" | "machines.kill" | "machines.remove" | "files.remove") => {
            let _ = value;
            Answer { status: 204, body: Body::Text(String::new()) }
        }
        Ok(value) => Answer::json(value),
        Err(e) => Answer::error(&e),
    }
}

fn id_route<'a>(params: &mut serde_json::Map<String, Value>, id: &str, method: &'a str, json: bool) -> (&'a str, bool) {
    params.insert("Id".into(), json!(id));
    (method, json)
}

fn name_route<'a>(params: &mut serde_json::Map<String, Value>, name: &[&str], method: &'a str, json: bool) -> (&'a str, bool) {
    params.insert("Name".into(), json!(name.join("/")));
    (method, json)
}
