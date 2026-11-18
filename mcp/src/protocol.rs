//! MCP's messages, in both eras. A request whose `_meta` names a protocol version is 2026-07-28's (stateless: each
//! carries its version and the client's capabilities); one that begins with `initialize` is the older handshake's, as
//! clients of 2025-11-25 and before speak it. The tools and resources answer the same either way.

use serde_json::{Value, json};

use crate::Server;

/// The stateless revision.
pub const VERSION: &str = "2026-07-28";
/// The handshake's revisions, newest first.
pub const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";

pub(crate) fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut e = json!({ "code": code, "message": message });
    if let Some(d) = data {
        e["data"] = d;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": e })
}

fn server_info() -> Value {
    json!({ "name": "carbon", "title": "Carbon", "version": env!("CARGO_PKG_VERSION") })
}

fn capabilities() -> Value {
    json!({ "tools": { "listChanged": false }, "resources": { "subscribe": false, "listChanged": false } })
}

/// A request's answer in its era: a stateless one's says it's complete and who answered.
fn complete(id: Value, mut result: Value, modern: bool) -> Value {
    if modern {
        result["resultType"] = json!("complete");
        if result.get("_meta").is_none() {
            result["_meta"] = json!({});
        }
        result["_meta"][META_SERVER] = server_info();
    }
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub(crate) fn handle(server: &Server, message: Value) -> Option<Value> {
    // A batch (2025-03-26 had them): each, and the answers together.
    if let Some(batch) = message.as_array() {
        let answers: Vec<Value> = batch.iter().filter_map(|m| handle(server, m.clone())).collect();
        return (!answers.is_empty()).then_some(Value::Array(answers));
    }
    let method = message.get("method").and_then(Value::as_str).unwrap_or("").to_string();
    let id = message.get("id").cloned();
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    // A notification, or an answer to something this never asks: nothing to say back.
    let id = id?;
    if method.is_empty() {
        return None;
    }
    let meta = params.get("_meta");
    let version = meta.and_then(|m| m.get(META_VERSION)).and_then(Value::as_str);
    let modern = version.is_some();
    if let Some(v) = version
        && v != VERSION
    {
        return Some(error(id, -32022, "Unsupported protocol version", Some(json!({ "supported": [VERSION], "requested": v }))));
    }
    let progress = server.progress(meta.and_then(|m| m.get("progressToken")).cloned());
    let result = match method.as_str() {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = LEGACY_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(LEGACY_VERSIONS[0]);
            Ok(json!({ "protocolVersion": version, "capabilities": capabilities(), "serverInfo": server_info(), "instructions": server.instructions() }))
        }
        "server/discover" => Ok(json!({
            "supportedVersions": [VERSION],
            "capabilities": capabilities(),
            "instructions": server.instructions(),
            "ttlMs": 60_000,
            "cacheScope": "private",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": server.tool_list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            server.call_tool(name, &args, &progress)
        }
        "resources/list" => Ok(json!({ "resources": server.resource_list() })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": crate::resources::templates() })),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            server.read_resource(uri).map(|contents| json!({ "contents": contents }))
        }
        "prompts/list" => Ok(json!({ "prompts": [] })),
        "logging/setLevel" => Ok(json!({})),
        _ => Err((-32601, format!("{method} isn't a method this server has"))),
    };
    Some(match result {
        Ok(r) => complete(id, r, modern),
        Err((code, message)) => error(id, code, &message, None),
    })
}
