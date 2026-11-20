//! The server calls go to: `GET /v1/` to see what it serves, and `POST /v1/{service}/{op}` for each call, with the
//! token. Nothing else is ever sent to it.

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use crate::call::Call;

/// What a server serves: each service, and its ops.
pub type Served = Vec<(String, Vec<String>)>;

/// What the server answered.
pub enum Answer {
    /// A stream of bytes, with its size if the server said, and where in the file it starts.
    Bytes { reader: Box<dyn Read + Send>, size: Option<u64>, offset: u64 },
    Json(Value),
    /// A refusal: `invalid`, `not_found`, `forbidden`, `unauthorized`, `too_large`, or `unavailable`, and why.
    Refused { code: String, message: String },
}

impl Answer {
    pub fn refused(code: &str, message: impl Into<String>) -> Answer {
        Answer::Refused { code: code.to_string(), message: message.into() }
    }
}

impl std::fmt::Debug for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Answer::Bytes { size, offset, .. } => write!(f, "Bytes({size:?} from {offset})"),
            Answer::Json(v) => write!(f, "Json({v})"),
            Answer::Refused { code, message } => write!(f, "Refused({code}: {message})"),
        }
    }
}

/// The client every call is sent with.
#[derive(Clone)]
pub struct Upstream {
    agent: ureq::Agent,
    token: Option<String>,
    meta: Vec<(String, String)>,
}

impl Upstream {
    /// A client over `tls` (the host's trust, from the caller), sending `token` and `meta` with every call.
    pub fn new(tls: std::sync::Arc<rustls::ClientConfig>, token: Option<String>, meta: Vec<(String, String)>) -> Upstream {
        let agent = ureq::AgentBuilder::new()
            .tls_config(tls)
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(120))
            .timeout_write(Duration::from_secs(120))
            .redirects(0)
            .user_agent(concat!("carbon/", env!("CARGO_PKG_VERSION")))
            .build();
        Upstream { agent, token, meta }
    }

    fn auth(&self, request: ureq::Request) -> ureq::Request {
        match &self.token {
            Some(t) => request.set("Authorization", &format!("Bearer {t}")),
            None => request,
        }
    }

    /// What `base` serves: each service and its ops, and the whole answer, for what a service says of itself there
    /// (git's hosts).
    pub fn discover(&self, base: &str) -> Result<(Served, Value), String> {
        let response = self.auth(self.agent.get(&format!("{base}/v1/"))).call().map_err(|e| format!("{base}: {}", describe(e)))?;
        let v: Value = json(response, 1 << 20).map_err(|e| format!("{base}/v1/: {e}"))?;
        let services = v.get("services").and_then(Value::as_object).ok_or_else(|| format!("{base}/v1/ doesn't list its services"))?;
        let served = services.iter().map(|(s, ops)| (s.clone(), ops.as_array().map(|l| l.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default())).collect();
        Ok((served, v))
    }

    /// Sends `call` to `base`, and gives what it answered. A server that can't be reached is a refusal, `unavailable`.
    pub fn send(&self, base: &str, call: &Call) -> Answer {
        let url = format!("{base}/v1/{}/{}", call.service, call.op);
        let body = call.body(&self.meta).to_string();
        let sent = match &call.data {
            // The call's JSON on one line, then its data.
            Some(data) => {
                let mut wire = Vec::with_capacity(body.len() + 1 + data.len());
                wire.extend_from_slice(body.as_bytes());
                wire.push(b'\n');
                wire.extend_from_slice(data);
                self.auth(self.agent.post(&url)).set("Content-Type", "application/vnd.carbon.stream").send_bytes(&wire)
            }
            None => self.auth(self.agent.post(&url)).set("Content-Type", "application/json").send_string(&body),
        };
        let response = match sent {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => return refusal(r),
            Err(e) => return Answer::refused("unavailable", format!("the package server didn't answer: {}", describe(e))),
        };
        if response.content_type() == "application/json" {
            return match json(response, 64 << 20) {
                Ok(v) => Answer::Json(v),
                Err(e) => Answer::refused("unavailable", format!("the package server's answer: {e}")),
            };
        }
        let number = |name: &str| response.header(name).and_then(|v| v.parse::<u64>().ok());
        let size = number("Carbon-Size").or_else(|| number("Content-Length"));
        let offset = number("Carbon-Offset").unwrap_or(0);
        Answer::Bytes { reader: Box::new(response.into_reader()), size, offset }
    }
}

/// A refusal from the server's `{"error": {"code", "message"}}`, or its status if it didn't send one.
fn refusal(r: ureq::Response) -> Answer {
    let status = r.status();
    let said: Option<Value> = json(r, 64 << 10).ok();
    let field = |k: &str| said.as_ref().and_then(|v| v.get("error")?.get(k)?.as_str().map(String::from));
    let code = field("code").unwrap_or_else(|| {
        match status {
            400 => "invalid",
            401 => "unauthorized",
            403 => "forbidden",
            404 => "not_found",
            413 => "too_large",
            _ => "unavailable",
        }
        .to_string()
    });
    Answer::Refused { code, message: field("message").unwrap_or_else(|| format!("the package server answered {status}")) }
}

/// A response's body as JSON, if it's at most `most` bytes.
fn json(r: ureq::Response, most: u64) -> Result<Value, String> {
    let bytes = crate::read_all(&mut r.into_reader(), most)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("not JSON: {e}"))
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => format!("answered {code} {}", r.status_text()),
        ureq::Error::Transport(t) => t.to_string(),
    }
}
