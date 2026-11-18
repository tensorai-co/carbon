//! `carbon mcp -H` on loopback: MCP over HTTP and WebSocket, the Engine API's scopes, tokens, origins, and
//! `--max-sessions`. No machines: what's called here needs none.

use std::path::PathBuf;
use std::time::Duration;

use mcp::net::{NetOptions, tokens};
use mcp::{Options, Scopes, Server};
use serde_json::{Value, json};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("carbon-mcp-net-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Serves on a port of its own, on a thread; the thread's handle ends when `serve` returns.
fn start(name: &str, engine: bool, net: impl FnOnce(&PathBuf) -> NetOptions) -> (String, std::thread::JoinHandle<()>) {
    let dir = home(name);
    let port = free_port();
    let mut net = net(&dir);
    net.hosts = vec![format!("127.0.0.1:{port}")];
    let carbon = carbon::Carbon::open_at(dir.join("home")).unwrap();
    let server = Server::new(carbon, Options { engine, ..Options::default() }).unwrap();
    let handle = std::thread::spawn(move || mcp::net::serve(server, net).unwrap());
    let base = format!("127.0.0.1:{port}");
    for _ in 0..200 {
        if std::net::TcpStream::connect(&base).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    (base, handle)
}

/// A request's status and body; `ureq` makes 4xx and 5xx errors, which are answers here.
fn call(req: ureq::Request, body: Option<&Value>) -> (u16, String) {
    let res = match body {
        Some(b) => req.set("Content-Type", "application/json").send_string(&b.to_string()),
        None => req.call(),
    };
    match res {
        Ok(r) => (r.status(), r.into_string().unwrap()),
        Err(ureq::Error::Status(code, r)) => (code, r.into_string().unwrap()),
        Err(e) => panic!("{e}"),
    }
}

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn post_mcp(base: &str, token: Option<&str>, message: &Value) -> (u16, Value) {
    let mut req = ureq::post(&format!("http://{base}/mcp")).set("Accept", "application/json, text/event-stream");
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let (status, body) = call(req, Some(message));
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

fn ws(base: &str, token: Option<&str>) -> tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>> {
    let mut req = format!("ws://{base}/").into_client_request().unwrap();
    if let Some(t) = token {
        req.headers_mut().insert("Authorization", format!("Bearer {t}").parse().unwrap());
    }
    tungstenite::connect(req).unwrap().0
}

/// Sends a request on a WebSocket, and reads until its answer.
fn ws_call(socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>, message: &Value) -> Value {
    socket.send(Message::Text(message.to_string().into())).unwrap();
    loop {
        if let Message::Text(t) = socket.read().unwrap() {
            let v: Value = serde_json::from_str(t.as_str()).unwrap();
            if v.get("id") == message.get("id") {
                return v;
            }
        }
    }
}

#[test]
fn mcp_over_http_and_websocket_without_the_engine() {
    let (base, _) = start("plain", false, |_| NetOptions::default());
    assert_eq!(call(ureq::get(&format!("http://{base}/_ping")), None), (200, "OK".into()));

    let (status, reply) = post_mcp(&base, None, &rpc(1, "tools/list", json!({})));
    assert_eq!(status, 200);
    let tools = reply["result"]["tools"].as_array().unwrap();
    assert!(!tools.is_empty());
    assert!(tools.iter().all(|t| !t["name"].as_str().unwrap().starts_with("engine")));

    // A client that takes only events gets the answer as one.
    let req = ureq::post(&format!("http://{base}/mcp")).set("Accept", "text/event-stream");
    let (status, body) = call(req, Some(&rpc(9, "tools/list", json!({}))));
    assert_eq!(status, 200);
    let data = body.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
    assert_eq!(serde_json::from_str::<Value>(data).unwrap()["id"], 9);

    // A notification gets 202; GET without an upgrade, 405.
    let (status, _) = post_mcp(&base, None, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    assert_eq!(status, 202);
    assert_eq!(call(ureq::get(&format!("http://{base}/mcp")), None).0, 405);

    // Without --engine, there's no Engine API at all.
    let (_, reply) = post_mcp(&base, None, &rpc(2, "engine/system.version", json!({})));
    assert_eq!(reply["error"]["code"], -32601);
    assert_eq!(call(ureq::get(&format!("http://{base}/v1/version")), None).0, 404);

    // A browser page's origin isn't allowed.
    let req = ureq::post(&format!("http://{base}/mcp")).set("Origin", "https://evil.example");
    assert_eq!(call(req, Some(&rpc(3, "tools/list", json!({})))).0, 403);

    let mut socket = ws(&base, None);
    let reply = ws_call(&mut socket, &rpc(4, "tools/list", json!({})));
    assert!(reply["result"]["tools"].as_array().is_some_and(|t| !t.is_empty()));
    socket.close(None).unwrap();
}

#[test]
fn tokens_scopes_and_the_engine() {
    let (agent, agent_line) = tokens::new_token(Scopes { mcp: true, engine: false }, "agent").unwrap();
    let (gateway, gateway_line) = tokens::new_token(Scopes::ALL, "gateway").unwrap();
    let (base, _) = start("tokens", true, |dir| {
        let file = dir.join("tokens");
        std::fs::write(&file, format!("{agent_line}\n{gateway_line}\n")).unwrap();
        NetOptions { token_file: Some(file), ..NetOptions::default() }
    });

    // A token the file doesn't have is refused, even on loopback.
    let (status, _) = post_mcp(&base, Some("carbon_nope"), &rpc(1, "tools/list", json!({})));
    assert_eq!(status, 401);

    // The agent's token reaches MCP, never the Engine API.
    let (status, reply) = post_mcp(&base, Some(&agent), &rpc(2, "tools/list", json!({})));
    assert_eq!(status, 200);
    assert!(reply["result"]["tools"].as_array().unwrap().iter().all(|t| !t["name"].as_str().unwrap().starts_with("engine")));
    let (_, reply) = post_mcp(&base, Some(&agent), &rpc(3, "engine/system.version", json!({})));
    assert_eq!(reply["error"]["code"], -32001);
    let req = ureq::get(&format!("http://{base}/v1/version")).set("Authorization", &format!("Bearer {agent}"));
    assert_eq!(call(req, None).0, 403);

    // The gateway's reaches both, over REST and a WebSocket.
    let req = ureq::get(&format!("http://{base}/v1/version")).set("Authorization", &format!("Bearer {gateway}"));
    let (status, body) = call(req, None);
    assert_eq!(status, 200);
    assert!(serde_json::from_str::<Value>(&body).unwrap()["Version"].is_string());
    let req = ureq::get(&format!("http://{base}/v1/machines")).set("Authorization", &format!("Bearer {gateway}"));
    let (status, body) = call(req, None);
    assert_eq!((status, serde_json::from_str::<Value>(&body).unwrap()), (200, json!([])));
    let req = ureq::get(&format!("http://{base}/v1/machines/nope")).set("Authorization", &format!("Bearer {gateway}"));
    assert_eq!(call(req, None).0, 404);
    let req = ureq::get(&format!("http://{base}/v1/images")).set("Authorization", &format!("Bearer {gateway}"));
    assert_eq!(call(req, None), (200, "[]".into()));
    let req = ureq::get(&format!("http://{base}/v1/images/docker.io/library/nope:1")).set("Authorization", &format!("Bearer {gateway}"));
    assert_eq!(call(req, None).0, 404);
    let req = ureq::post(&format!("http://{base}/v1/machines")).set("Authorization", &format!("Bearer {gateway}"));
    let (status, body) = call(req, Some(&json!({ "Image": "debian:13", "Labels": { "a": "b" } })));
    assert_eq!(status, 501, "{body}");

    let mut socket = ws(&base, Some(&gateway));
    let reply = ws_call(&mut socket, &rpc(4, "engine/system.version", json!({})));
    assert!(reply["result"]["Version"].is_string(), "{reply}");
    let reply = ws_call(&mut socket, &rpc(5, "engine/machines.inspect", json!({ "Id": "nope" })));
    assert_eq!(reply["error"]["code"], -32004);
    let reply = ws_call(&mut socket, &rpc(6, "tools/list", json!({})));
    assert!(reply["result"]["tools"].is_array());
    socket.close(None).unwrap();

    let mut socket = ws(&base, Some(&agent));
    let reply = ws_call(&mut socket, &rpc(7, "engine/machines.list", json!({})));
    assert_eq!(reply["error"]["code"], -32001);
    socket.close(None).unwrap();
}

#[test]
fn max_sessions_then_exit() {
    let (base, done) = start("max", true, |_| NetOptions { max_sessions: Some(2), ..NetOptions::default() });
    // A probe that connects and leaves isn't a session, nor is a health check.
    drop(std::net::TcpStream::connect(&base).unwrap());
    assert_eq!(call(ureq::get(&format!("http://{base}/_ping")), None).0, 200);

    let mut first = ws(&base, None);
    assert!(ws_call(&mut first, &rpc(1, "engine/system.ping", json!({})))["result"].is_string());
    let agent = ureq::AgentBuilder::new().build();
    let (status, _) = call(agent.post(&format!("http://{base}/mcp")), Some(&rpc(2, "tools/list", json!({}))));
    assert_eq!(status, 200);
    drop(agent);

    // Both sessions are open; the server stopped accepting when the second began.
    assert!(!done.is_finished());
    first.close(None).unwrap();
    let _ = first.read();
    for _ in 0..500 {
        if done.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done.is_finished(), "the server's still serving after its --max-sessions closed");
    done.join().unwrap();
}
