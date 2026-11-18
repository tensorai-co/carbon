//! A session as sandbox-gateway runs one, on one WebSocket, against real machines: create from an image with a
//! hostname, env, rules and a tmpfs; start; write a token into the tmpfs with frames; exec with streams (output, and
//! stdin); read the file back as frames; commit, and the token isn't in the image; clean up.
//!
//! It runs `carbon mcp` itself (on macOS a machine's worker is the `carbon` binary run again, signed for the
//! hypervisor): `$CARBON`, else `target/release/carbon`. And it needs `debian:latest` in this host's Carbon home:
//!
//!     cargo build --release -p cli && codesign --force --sign - --entitlements carbon.entitlements target/release/carbon
//!     cargo test -p mcp --test session -- --ignored

use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::Message;

type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

struct Session {
    socket: Socket,
    next: u64,
}

impl Session {
    fn send_frame(&mut self, stream: u32, kind: u8, bytes: &[u8]) {
        self.socket.send(Message::Binary(mcp::net::stream_frame(stream, kind, bytes).into())).unwrap();
    }

    /// Sends a request; `frames` go right behind it. Collects its stream's frames until its answer.
    fn call(&mut self, method: &str, params: Value, frames: &[(u32, u8, &[u8])]) -> (Value, Vec<(u8, Vec<u8>)>) {
        self.next += 1;
        let id = self.next;
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.socket.send(Message::Text(message.to_string().into())).unwrap();
        for (stream, kind, bytes) in frames {
            self.send_frame(*stream, *kind, bytes);
        }
        let mut got = Vec::new();
        loop {
            match self.socket.read().unwrap() {
                Message::Text(t) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if v["id"] == id {
                        return (v, got);
                    }
                }
                Message::Binary(b) => {
                    let (_, kind, bytes) = mcp::net::parse_stream_frame(&b).unwrap();
                    got.push((kind, bytes.to_vec()));
                }
                _ => {}
            }
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (reply, _) = self.call(method, params, &[]);
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }
}

fn joined(frames: &[(u8, Vec<u8>)], kind: u8) -> String {
    String::from_utf8_lossy(&frames.iter().filter(|f| f.0 == kind).flat_map(|f| f.1.clone()).collect::<Vec<u8>>()).into_owned()
}

#[test]
#[ignore = "needs debian:latest and a host that runs machines"]
fn a_gateway_s_session() {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let carbon = std::env::var("CARBON").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../target/release/carbon").to_string());
    let base = format!("127.0.0.1:{port}");
    let mut server = Command::new(&carbon)
        .args(["mcp", "-H", &base, "--engine", "--max-sessions", "1"])
        .stdin(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{carbon}: {e}"));
    for _ in 0..500 {
        if std::net::TcpStream::connect(&base).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let (socket, _) = tungstenite::connect(format!("ws://{base}/")).unwrap();
    let mut s = Session { socket, next: 0 };
    let name = format!("mcp-e2e-{}", std::process::id());
    let repo = format!("mcp-e2e-{}", std::process::id());

    let created = s.ok(
        "engine/machines.create",
        json!({
            "Image": "debian:latest", "Name": name, "Hostname": "sbx-8f3a", "Env": ["TURN=1"], "Cmd": ["sleep", "infinity"],
            "Labels": { "session": "8f3a", "tier": "sandbox" },
            "HostConfig": { "Tmpfs": { "/run/secrets": "" }, "ExtraHosts": ["repo-server.internal:10.8.0.2", "mirror=10.8.0.3"] },
            "Rules": { "Deny": ["0.0.0.0/0"] },
        }),
    );
    let id = created["Id"].as_str().unwrap().to_string();
    s.ok("engine/machines.start", json!({ "Id": id }));

    // The token, as frames right behind the request.
    let (reply, _) = s.call(
        "engine/files.write",
        json!({ "Id": id, "Path": "/run/secrets/token", "Mode": 384, "Stream": 1 }),
        &[(1, 4, b"s3cret"), (1, 5, b"")],
    );
    assert!(reply.get("error").is_none(), "{reply}");

    let script = "hostname; echo turn=$TURN; cat /run/secrets/token; echo; stat -c %a /run/secrets/token; echo oops >&2; exit 3";
    let (reply, frames) = s.call("engine/machines.exec", json!({ "Id": id, "Cmd": ["sh", "-c", script], "Stream": 7 }), &[]);
    assert_eq!(reply["result"]["ExitCode"], 3, "{reply}");
    let out = joined(&frames, 1);
    assert_eq!(out.lines().collect::<Vec<_>>(), ["sbx-8f3a", "turn=1", "s3cret", "600"], "{out}");
    assert_eq!(joined(&frames, 2).trim(), "oops");
    let exit = frames.iter().find(|f| f.0 == 3).expect("an exit frame");
    assert_eq!(i32::from_be_bytes(exit.1[..4].try_into().unwrap()), 3);

    // stdin, as frames.
    let (reply, frames) = s.call(
        "engine/machines.exec",
        json!({ "Id": id, "Cmd": ["cat"], "Stream": 8 }),
        &[(8, 0, b"hello "), (8, 0, b"there"), (8, 5, b"")],
    );
    assert_eq!(reply["result"]["ExitCode"], 0, "{reply}");
    assert_eq!(joined(&frames, 1), "hello there");

    // --add-host's lines, after the machine's own.
    let r = s.ok("engine/machines.exec", json!({ "Id": id, "Cmd": ["sh", "-c", "getent hosts repo-server.internal mirror; tail -2 /etc/hosts"] }));
    let out = r["Stdout"].as_str().unwrap();
    assert!(out.contains("10.8.0.2        repo-server.internal") || out.contains("10.8.0.2\trepo-server.internal") || out.lines().any(|l| l.split_whitespace().collect::<Vec<_>>() == ["10.8.0.2", "repo-server.internal"]), "{out}");
    assert!(out.lines().any(|l| l.split_whitespace().collect::<Vec<_>>() == ["10.8.0.3", "mirror"]), "{out}");

    // Labels: found by them, and shown.
    let found = s.ok("engine/machines.list", json!({ "Label": ["session=8f3a", "tier"] }));
    assert_eq!(found.as_array().unwrap().iter().map(|m| m["Id"].as_str().unwrap()).collect::<Vec<_>>(), [id.as_str()]);
    assert_eq!(s.ok("engine/machines.list", json!({ "Label": "session=other" })), json!([]));
    let inspect = s.ok("engine/machines.inspect", json!({ "Id": id }));
    assert_eq!(inspect["Config"]["Labels"]["tier"], "sandbox", "{inspect}");
    assert_eq!(inspect["Config"]["Hostname"], "sbx-8f3a");
    assert_eq!(inspect["HostConfig"]["ExtraHosts"], json!(["repo-server.internal:10.8.0.2", "mirror:10.8.0.3"]));

    // Without a stream, the output's in the answer.
    let r = s.ok("engine/machines.exec", json!({ "Id": id, "Cmd": ["sh", "-c", "echo plain"] }));
    assert_eq!(r["Stdout"].as_str().unwrap().trim(), "plain");

    let (reply, frames) = s.call("engine/files.read", json!({ "Id": id, "Path": "/run/secrets/token", "Stream": 9 }), &[]);
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(joined(&frames, 4), "s3cret");
    assert!(frames.last().is_some_and(|f| f.0 == 5));

    // The rules held: nothing out.
    let r = s.ok("engine/machines.exec", json!({ "Id": id, "Cmd": ["bash", "-c", "(exec 3<>/dev/tcp/1.1.1.1/80) 2>/dev/null && echo connected || echo refused"], "Timeout": 20 }));
    assert_eq!(r["Stdout"].as_str().unwrap().trim(), "refused", "{r}");

    let inspect = s.ok("engine/machines.inspect", json!({ "Id": id }));
    assert_eq!(inspect["State"]["Running"], true, "{inspect}");

    // Commit, running: a tmpfs isn't the disk, so the token isn't in the image.
    s.ok("engine/files.write", json!({ "Id": id, "Path": "/root/work.txt", "Data": "d29yaw==" }));
    let committed = s.ok("engine/machines.commit", json!({ "Id": id, "Repo": repo, "Tag": "t1" }));
    assert!(committed["Id"].is_string(), "{committed}");
    s.ok("engine/machines.stop", json!({ "Id": id }));
    s.ok("engine/machines.remove", json!({ "Id": id }));

    let again = s.ok("engine/machines.create", json!({ "Image": format!("{repo}:t1"), "Name": format!("{name}-2") }));
    let again = again["Id"].as_str().unwrap().to_string();
    let image = s.ok("engine/images.inspect", json!({ "Name": format!("{repo}:t1") }));
    assert_eq!(image["Config"]["Labels"]["session"], "8f3a", "the labels are in the image: {image}");
    let r = s.ok("engine/files.stat", json!({ "Id": again, "Path": "/root/work.txt" }));
    assert_eq!(r["Size"], 4);
    let (reply, _) = s.call("engine/files.stat", json!({ "Id": again, "Path": "/run/secrets/token" }), &[]);
    assert_eq!(reply["error"]["code"], -32004, "the token was committed: {reply}");
    s.ok("engine/machines.remove", json!({ "Id": again }));
    s.ok("engine/images.remove", json!({ "Name": format!("{repo}:t1") }));

    // The one session closes, and the server's done.
    s.socket.close(None).unwrap();
    let _ = s.socket.read();
    for _ in 0..500 {
        if let Some(status) = server.try_wait().unwrap() {
            assert!(status.success(), "carbon mcp: {status}");
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = server.kill();
    panic!("carbon mcp still serving after its --max-sessions 1");
}
