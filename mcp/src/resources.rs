//! Resources: what's here, as a client can attach it to a conversation. The lists of images, machines and volumes; a
//! machine's record, logs and screen; and any file in one (a template).

use serde_json::{Value, json};

use crate::Server;
use crate::shape;

pub(crate) fn templates() -> Vec<Value> {
    vec![
        json!({ "uriTemplate": "carbon://machines/{name}", "name": "machine", "title": "A machine's record", "mimeType": "application/json" }),
        json!({ "uriTemplate": "carbon://machines/{name}/files/{path}", "name": "machine-file", "title": "A file in a machine", "description": "Its path after files/, as /etc/hostname" }),
        json!({ "uriTemplate": "carbon://machines/{name}/logs", "name": "machine-logs", "title": "What a machine wrote", "mimeType": "text/plain" }),
        json!({ "uriTemplate": "carbon://machines/{name}/screen", "name": "machine-screen", "title": "A machine's screen", "mimeType": "image/png" }),
    ]
}

fn text(uri: &str, mime: &str, text: String) -> Vec<Value> {
    vec![json!({ "uri": uri, "mimeType": mime, "text": text })]
}

fn json_text(uri: &str, v: &Value) -> Vec<Value> {
    text(uri, "application/json", serde_json::to_string_pretty(v).unwrap_or_default())
}

impl Server {
    pub(crate) fn resource_list(&self) -> Vec<Value> {
        let mut list = Vec::new();
        if self.options.machine.is_none() {
            list.push(json!({ "uri": "carbon://images", "name": "images", "title": "Images", "mimeType": "application/json" }));
            list.push(json!({ "uri": "carbon://machines", "name": "machines", "title": "Machines", "mimeType": "application/json" }));
            list.push(json!({ "uri": "carbon://volumes", "name": "volumes", "title": "Volumes", "mimeType": "application/json" }));
        }
        for m in self.carbon.machines() {
            if self.options.machine.as_ref().is_some_and(|only| *only != m.name) {
                continue;
            }
            list.push(json!({ "uri": format!("carbon://machines/{}", m.name), "name": m.name, "title": format!("Machine {}", m.name), "mimeType": "application/json" }));
        }
        list
    }

    pub(crate) fn read_resource(&self, uri: &str) -> Result<Vec<Value>, (i64, String)> {
        let bad = |why: String| (-32002, why);
        let rest = uri.strip_prefix("carbon://").ok_or_else(|| bad(format!("{uri} isn't a carbon:// resource")))?;
        let whole = self.options.machine.is_none();
        match rest {
            "images" if whole => {
                let v: Vec<Value> = self.carbon.images().iter().map(|i| json!({ "image": i.short_name(), "platform": i.platform.to_string(), "source": i.source })).collect();
                return Ok(json_text(uri, &json!(v)));
            }
            "machines" if whole => {
                let v: Vec<Value> = self.carbon.machines().iter().map(crate::tools::machine_json).collect();
                return Ok(json_text(uri, &json!(v)));
            }
            "volumes" if whole => {
                let v: Vec<Value> = self.carbon.volumes().iter().map(|v| json!({ "name": v.name })).collect();
                return Ok(json_text(uri, &json!(v)));
            }
            _ => {}
        }
        let after = rest.strip_prefix("machines/").ok_or_else(|| bad(format!("there's no resource {uri}")))?;
        let (name, what) = after.split_once('/').unwrap_or((after, ""));
        if self.options.machine.as_ref().is_some_and(|only| only != name) {
            return Err(bad(format!("this server serves {} alone", self.options.machine.as_deref().unwrap_or(""))));
        }
        let machine = self.carbon.get(name).map_err(|e| bad(e.to_string()))?;
        match what {
            "" => Ok(json_text(uri, &crate::tools::machine_json(&machine.info().map_err(|e| bad(e.to_string()))?))),
            "logs" => {
                let lines = machine.logs(carbon::Logs::new().tail(500)).map_err(|e| bad(e.to_string()))?;
                let bytes: Vec<u8> = lines.flat_map(|l| l.bytes).collect();
                Ok(text(uri, "text/plain", shape::clip(&bytes, shape::OUTPUT_CAP).0))
            }
            "screen" => {
                let png = machine.screen().screenshot().map_err(|e| bad(e.to_string()))?;
                Ok(vec![json!({ "uri": uri, "mimeType": "image/png", "blob": shape::base64(&png) })])
            }
            _ => {
                let path = what.strip_prefix("files").ok_or_else(|| bad(format!("there's no resource {uri}")))?;
                let path = if path.is_empty() { "/" } else { path };
                let bytes = machine.files().read(path, ..(1u64 << 20)).map_err(|e| bad(e.to_string()))?;
                Ok(match String::from_utf8(bytes) {
                    Ok(t) => text(uri, "text/plain", t),
                    Err(e) => vec![json!({ "uri": uri, "mimeType": "application/octet-stream", "blob": shape::base64(e.as_bytes()) })],
                })
            }
        }
    }
}
