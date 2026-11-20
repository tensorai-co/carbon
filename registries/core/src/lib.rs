//! `registries-core`: a machine's package managers answered in Carbon's own process, and each of their requests sent
//! upstream as one checked call, never as itself.
//!
//! A tool in the machine (`apt`, later `npm` and `git`) connects to the gateway, which `net` hands to this crate's
//! [`Server`]. It reads each HTTP request, and the service's [`Adapter`] turns it into a [`Call`]: an op and its
//! fields, `apt deb {archive: debian, name: jq, …}`. The call is checked against the service's schema, the machine's
//! hooks decide it ([`hooks::Registry`]), and only then is it sent: `POST {upstream}/v1/{service}/{op}` with a JSON
//! body and the token. What the server answers, the adapter turns back into what the tool expects.
//!
//! Nothing the tool sends goes anywhere: not its method, path, headers or body. A request that doesn't parse into a
//! valid call is refused here, and the server never hears of it.

mod call;
mod http;
mod settings;
mod upstream;
mod verify;

use std::io::{self, BufReader, Read, Write};
use std::sync::Arc;

pub use call::{Call, Expect, Field, Op, check};
pub use http::{Body, Native, Request, clean, decode_segment};
pub use settings::Registries;
pub use upstream::{Answer, Upstream};
pub use verify::{Verified, base64_decode, hex};

/// One service's translation between a tool's protocol and calls.
pub trait Adapter: Send + Sync {
    /// `apt`: the first segment of its paths, and its calls' service.
    fn service(&self) -> &'static str;
    /// Its ops, and each one's fields: every call is checked against them before it's sent.
    fn ops(&self) -> &'static [Op];
    /// A conversation for one connection, where an adapter keeps what it needs between requests.
    fn conversation(&self) -> Box<dyn Conversation>;
    /// The most a request's body may be: package managers' reads have none, so 1 MiB; git's pushes are packs.
    fn most_body(&self) -> u64 {
        1 << 20
    }
    /// What the machine's environment needs for its tool to use the registry at `base`
    /// (`http://registry.carbon/npm/`): `NPM_CONFIG_REGISTRY`, say. `details` is what the server's discovery says under
    /// the service's name, if anything (git's hosts). None by default: apt reads only files.
    fn env(&self, _base: &str, _details: &serde_json::Value) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Files the machine's tool needs to use the registry at `base`, where no environment variable will do (cargo's
    /// source replacement): absolute paths, or under `~/` (the home of the user the machine runs as), and their bytes. Carbon puts each in a tmpfs at its folder, for the run
    /// only, so it's never on the machine's disk or in an image committed from it. None by default.
    fn files(&self, _base: &str, _details: &serde_json::Value) -> Vec<(String, Vec<u8>)> {
        Vec::new()
    }
}

/// One connection's requests and answers, for one service.
pub trait Conversation: Send {
    /// A request (its path without the service's prefix) as a call, or the tool's own refusal. No I/O.
    fn parse(&mut self, req: &Request, path: &str) -> Result<Call, Native>;
    /// What the server answered, or why there's no answer, as the tool reads it.
    fn reply(&mut self, call: &Call, answer: Answer) -> Native;
}

/// A machine's registries, answering its connections to the gateway.
pub struct Server {
    adapters: Vec<(Arc<dyn Adapter>, String)>,
    /// What each server's discovery said of each service, for its env.
    details: Vec<serde_json::Value>,
    upstream: Upstream,
    hub: hooks::Hub,
    /// The most a request's body may be, for the adapters it has.
    most_body: u64,
}

impl Server {
    /// The registries `settings` names, from `adapters` (what this Carbon has). Each server is asked what it serves
    /// first, so a service it doesn't have fails here, loudly, not in the machine.
    pub fn open(settings: &Registries, adapters: Vec<Arc<dyn Adapter>>, tls: Arc<rustls::ClientConfig>, hub: hooks::Hub) -> Result<Server, String> {
        let known: Vec<&str> = adapters.iter().map(|a| a.service()).collect();
        settings.check(&known)?;
        let upstream = Upstream::new(tls, settings.token.clone(), settings.meta.clone());
        let mut chosen = Vec::new();
        let mut details = Vec::new();
        let mut asked: Vec<(String, (upstream::Served, serde_json::Value))> = Vec::new();
        for service in &settings.services {
            let adapter = adapters.iter().find(|a| a.service() == service).cloned().ok_or_else(|| format!("no registry {service}"))?;
            let base = settings.upstream_for(service).unwrap_or_default().to_string();
            if !asked.iter().any(|(b, _)| *b == base) {
                let served = upstream.discover(&base).map_err(|e| format!("--registries {service}: {e}"))?;
                asked.push((base.clone(), served));
            }
            let (served, whole) = asked.iter().find(|(b, _)| *b == base).map(|(_, s)| s.clone()).unwrap_or_default();
            let ops = served.iter().find(|(s, _)| s == service).map(|(_, ops)| ops).ok_or_else(|| format!("--registries {service}: {base} doesn't serve {service}"))?;
            if let Some(missing) = adapter.ops().iter().find(|op| !ops.iter().any(|o| o == op.name)) {
                return Err(format!("--registries {service}: {base} doesn't answer {service} {}", missing.name));
            }
            details.push(whole.get(service.as_str()).cloned().unwrap_or(serde_json::Value::Null));
            chosen.push((adapter, base));
        }
        let most_body = chosen.iter().map(|(a, _)| a.most_body()).max().unwrap_or(1 << 20);
        Ok(Server { adapters: chosen, details, upstream, hub, most_body })
    }

    /// The services it answers.
    pub fn services(&self) -> Vec<&'static str> {
        self.adapters.iter().map(|(a, _)| a.service()).collect()
    }

    /// The environment its tools need, with the registries at `http://{host}/`: `NAME=value`s.
    pub fn env(&self, host: &str) -> Vec<String> {
        self.adapters
            .iter()
            .zip(&self.details)
            .flat_map(|((a, _), details)| a.env(&format!("http://{host}/{}/", a.service()), details))
            .map(|(k, v)| format!("{k}={v}"))
            .collect()
    }

    /// The files its tools need, with the registries at `http://{host}/`: absolute paths and their bytes.
    pub fn files(&self, host: &str) -> Vec<(String, Vec<u8>)> {
        self.adapters.iter().zip(&self.details).flat_map(|((a, _), details)| a.files(&format!("http://{host}/{}/", a.service()), details)).collect()
    }

    /// Answers one connection's requests until it ends, on this thread.
    pub fn serve(&self, conn: impl Read + Write) {
        let mut conn = BufReader::new(conn);
        let mut conversations: Vec<(&'static str, Box<dyn Conversation>)> = Vec::new();
        loop {
            let req = match http::read_request(&mut conn, self.most_body) {
                Ok(Some(req)) => req,
                Ok(None) => return,
                Err(e) => {
                    let _ = http::write_answer(conn.get_mut(), Native::plain(400, "Bad Request", &e.to_string()), false, false);
                    return;
                }
            };
            let keep_alive = req.keep_alive;
            let head_only = req.method == "HEAD";
            let answer = self.answer(&req, &mut conversations);
            if http::write_answer(conn.get_mut(), answer, keep_alive, head_only).is_err() || !keep_alive {
                return;
            }
        }
    }

    fn answer(&self, req: &Request, conversations: &mut Vec<(&'static str, Box<dyn Conversation>)>) -> Native {
        let rest = req.path.strip_prefix('/').unwrap_or("");
        let (first, path) = rest.split_once('/').map_or((rest, ""), |(f, p)| (f, p));
        let Some((adapter, base)) = self.adapters.iter().find(|(a, _)| a.service() == first) else {
            return Native::plain(404, "Not Found", &format!("this machine's registries are {}", self.services().join(", ")));
        };
        let service = adapter.service();
        let index = match conversations.iter().position(|(s, _)| *s == service) {
            Some(i) => i,
            None => {
                conversations.push((service, adapter.conversation()));
                conversations.len() - 1
            }
        };
        let conv = &mut conversations[index].1;
        let call = match conv.parse(req, &format!("/{path}")) {
            Ok(call) => call,
            Err(native) => return native,
        };
        if let Err(why) = call.check(adapter.ops()) {
            return conv.reply(&call, Answer::refused("invalid", why));
        }
        let decided = if self.hub.wants(hooks::Kind::Registry) { self.hub.registry(call.event()) } else { hooks::Decision::Allow };
        match decided {
            hooks::Decision::Pass | hooks::Decision::Allow => {}
            // There's no rewriting a call or answering one yet: either is a refusal, so nothing slips through.
            hooks::Decision::Deny | hooks::Decision::Rewrite(_) | hooks::Decision::Answer(_) => {
                return conv.reply(&call, Answer::refused("forbidden", "this machine's rules refuse it"));
            }
        }
        let answer = match self.upstream.send(base, &call) {
            // What's known about the file is checked here too, as it streams: one that doesn't match is cut short,
            // so the tool never has it whole, whatever the server did.
            Answer::Bytes { reader, size, offset: 0 } if call.offset == 0 && call.expect.is_some() => {
                let expect = call.expect.clone().unwrap_or_default();
                if let (Some(want), Some(got)) = (expect.size, size)
                    && want != got
                {
                    return conv.reply(&call, Answer::refused("unavailable", format!("the package server's file is {got} bytes, not {want}")));
                }
                Answer::Bytes { reader: Box::new(Verified::new(reader, &expect)), size, offset: 0 }
            }
            other => other,
        };
        conv.reply(&call, answer)
    }
}

/// Reads an [`Answer::Bytes`] whole, for an adapter that rewrites what it's answered (npm's packument); an error, or
/// more than `most`, is an `unavailable` refusal.
pub fn read_all(reader: &mut dyn Read, most: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    reader.take(most + 1).read_to_end(&mut out).map_err(|e| format!("reading the package server's answer: {e}"))?;
    if out.len() as u64 > most {
        return Err("the package server's answer is too big".into());
    }
    Ok(out)
}

/// An `io::Error` for a stream that ended wrong.
pub fn broken(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}
