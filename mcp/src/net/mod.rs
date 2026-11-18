//! `carbon mcp -H`: the network transports. Each address serves HTTP and WebSocket both, chosen per request by its
//! `Upgrade` header:
//!
//! - `POST /mcp`: MCP's Streamable HTTP, a JSON answer, or server-sent events when the client asks for progress.
//! - `/v1/…`, with `--engine`: the Engine API as REST ([`crate::engine::rest`]).
//! - A WebSocket (at `/` or `/mcp`): one connection carrying everything. Text messages are JSON-RPC, MCP's and
//!   `engine/…` alike; binary messages are streams' bytes, `[stream: u32, big-endian][kind: u8][bytes]`.
//! - `GET /_ping`: `OK`, without a token, for health checks. It doesn't count as a session.
//!
//! Who may connect ([`check`]): a Unix socket's file mode (`0600`) is its boundary, as `docker.sock`'s; loopback needs
//! no token (with a warning at start); anything beyond loopback needs `--token-file`, or `--no-auth` when something in
//! front of Carbon does auth. A request with an `Origin` must come from an allowed origin (`--allow-origin`).
//!
//! `--max-sessions N`: a session is a connection that's made a request (a TCP probe that connects and leaves isn't
//! one). After the Nth begins the server stops accepting, and once the last has closed it returns.

pub mod tls;
pub mod tokens;

use std::collections::HashMap;
use std::convert::Infallible;
use std::io;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Query};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Extension, Json};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};
use tower_service::Service;

use crate::engine::rest::{self, Body};
use crate::{Kind, Outbox, Peer, Scopes, Server};

/// The most a request's body may be: a file written over REST is its body.
const MOST_BODY: usize = 256 << 20;

/// `carbon mcp`'s network flags.
#[derive(Debug, Clone, Default)]
pub struct NetOptions {
    /// `-H`: `host:port`, or a Unix socket's path.
    pub hosts: Vec<String>,
    /// `--max-sessions`: serve this many sessions in all, then return.
    pub max_sessions: Option<u64>,
    /// `--tls`: the certificate's and key's PEM files.
    pub tls: Option<(PathBuf, PathBuf)>,
    pub token_file: Option<PathBuf>,
    pub allow_origins: Vec<String>,
    pub no_auth: bool,
}

/// Where to listen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listen {
    /// `host:port`, as given (`:port` is every address).
    Tcp(String),
    Unix(PathBuf),
}

/// What `-H` names: a path (it has a `/`) is a Unix socket, anything else `host:port`. No scheme.
pub fn parse_host(text: &str) -> Result<Listen, String> {
    if text.contains("://") {
        return Err(format!("-H {text}: no scheme: give host:port, or a Unix socket's path"));
    }
    if text.contains('/') {
        return Ok(Listen::Unix(PathBuf::from(text)));
    }
    let Some((host, port)) = text.rsplit_once(':') else {
        return Err(format!("-H {text}: want host:port, or a Unix socket's path"));
    };
    if port.parse::<u16>().is_err() {
        return Err(format!("-H {text}: {port:?} isn't a port"));
    }
    Ok(Listen::Tcp(if host.is_empty() { format!("0.0.0.0:{port}") } else { text.to_string() }))
}

/// What [`check`] found: where to listen, each with whether only this host can reach it, and what to warn about.
#[derive(Debug)]
pub struct Plan {
    pub listens: Vec<(Listen, bool)>,
    pub warnings: Vec<String>,
}

/// The rules on who may connect, before anything listens.
pub fn check(net: &NetOptions) -> Result<Plan, String> {
    if net.hosts.is_empty() {
        return Err("no -H: nothing to listen on".into());
    }
    if net.no_auth && net.token_file.is_some() {
        return Err("--no-auth and --token-file: one or the other".into());
    }
    if net.max_sessions == Some(0) {
        return Err("--max-sessions 0 would serve nothing".into());
    }
    let mut plan = Plan { listens: Vec::new(), warnings: Vec::new() };
    for h in &net.hosts {
        let listen = parse_host(h)?;
        let local = match &listen {
            Listen::Unix(_) => true,
            Listen::Tcp(addr) => {
                let addrs: Vec<_> = addr.to_socket_addrs().map_err(|e| format!("-H {h}: {e}"))?.collect();
                let loopback = !addrs.is_empty() && addrs.iter().all(|a| a.ip().is_loopback());
                if !loopback && net.token_file.is_none() && !net.no_auth {
                    return Err(format!(
                        "-H {h} can be reached from other hosts: give --token-file, or --no-auth when something in front of Carbon does auth"
                    ));
                }
                if loopback && net.token_file.is_none() && !net.no_auth {
                    plan.warnings.push(format!("-H {h}: no --token-file, so anything on this host can connect"));
                }
                loopback
            }
        };
        plan.listens.push((listen, local));
    }
    Ok(plan)
}

/// How many sessions have begun and how many are open; when `--max-sessions` have begun, the listeners stop, and when
/// they've all closed, the server's done.
struct Sessions {
    max: Option<u64>,
    counts: Mutex<(u64, u64)>,
    stop: watch::Sender<bool>,
    done: watch::Sender<bool>,
}

impl Sessions {
    fn begin(&self) -> bool {
        let mut c = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if self.max.is_some_and(|m| c.0 >= m) {
            return false;
        }
        c.0 += 1;
        c.1 += 1;
        if self.max == Some(c.0) {
            self.stop.send_replace(true);
        }
        true
    }

    fn end(&self) {
        let mut c = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        c.1 = c.1.saturating_sub(1);
        if self.max.is_some_and(|m| c.0 >= m) && c.1 == 0 {
            self.done.send_replace(true);
        }
    }
}

/// What every connection shares.
struct Net {
    server: Arc<Server>,
    tokens: Option<tokens::Tokens>,
    no_auth: bool,
    allow_origins: Vec<String>,
    sessions: Sessions,
}

/// One connection: where it's from, and whether it's begun a session. The session ends when the last reference goes:
/// the connection's, and its WebSocket's if it became one.
struct Conn {
    net: Arc<Net>,
    address: String,
    local: bool,
    counted: AtomicBool,
}

impl Drop for Conn {
    fn drop(&mut self) {
        if self.counted.load(Ordering::SeqCst) {
            self.net.sessions.end();
        }
    }
}

/// Who a request is from: what its token reaches, and its name for the logs.
struct Who {
    scopes: Scopes,
    name: String,
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "message": message.into() }))).into_response()
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let auth = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = auth.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim().to_string())
}

/// Lets a request in, or answers why not: its origin, its token, and the session it begins.
fn admit(conn: &Conn, headers: &HeaderMap) -> Result<Who, Box<Response>> {
    let net = &conn.net;
    if let Some(origin) = headers.get(header::ORIGIN) {
        let origin = origin.to_str().unwrap_or("");
        if !net.allow_origins.iter().any(|a| a == "*" || a == origin) {
            return Err(Box::new(error(StatusCode::FORBIDDEN, format!("origin {origin:?} isn't allowed (--allow-origin)"))));
        }
    }
    let anyone = Who { scopes: Scopes::ALL, name: conn.address.clone() };
    let who = if net.no_auth {
        anyone
    } else {
        match (&net.tokens, bearer(headers)) {
            (None, _) => anyone,
            (Some(tokens), Some(token)) => match tokens.check(&token) {
                Some((scopes, name)) => Who { scopes, name },
                None => return Err(Box::new(unauthorized("the token isn't one --token-file lets in"))),
            },
            (Some(_), None) if conn.local => anyone,
            (Some(_), None) => return Err(Box::new(unauthorized("a token is needed: Authorization: Bearer <token>"))),
        }
    };
    if !conn.counted.swap(true, Ordering::SeqCst) {
        if !net.sessions.begin() {
            conn.counted.store(false, Ordering::SeqCst);
            return Err(Box::new(error(StatusCode::SERVICE_UNAVAILABLE, "this server has served its --max-sessions")));
        }
        if who.name == conn.address {
            eprintln!("carbon mcp: {}: connected", who.name);
        } else {
            eprintln!("carbon mcp: {}: connected from {}", who.name, conn.address);
        }
    }
    Ok(who)
}

fn unauthorized(why: &str) -> Response {
    let mut r = error(StatusCode::UNAUTHORIZED, why);
    r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    r
}

/// Where nothing's sent: a REST call's peer, or a JSON answer's.
struct Discard;

impl Outbox for Discard {
    fn message(&self, _: &Value) {}
}

/// A Streamable HTTP request's events: notifications as they come, then the answer, which ends the stream.
struct Events(mpsc::UnboundedSender<(bool, Value)>);

impl Outbox for Events {
    fn message(&self, message: &Value) {
        let _ = self.0.send((false, message.clone()));
    }
}

/// A WebSocket's messages and frames, to the task that writes them.
struct WsOutbox(mpsc::UnboundedSender<WsMessage>);

impl Outbox for WsOutbox {
    fn message(&self, message: &Value) {
        let _ = self.0.send(WsMessage::Text(serde_json::to_string(message).unwrap_or_default().into()));
    }

    fn has_frames(&self) -> bool {
        true
    }

    fn frame(&self, stream: u32, kind: Kind, bytes: &[u8]) -> bool {
        self.0.send(WsMessage::Binary(stream_frame(stream, kind as u8, bytes).into())).is_ok()
    }
}

/// A stream's frame: `[stream][kind][bytes]`.
pub fn stream_frame(stream: u32, kind: u8, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + bytes.len());
    out.extend_from_slice(&stream.to_be_bytes());
    out.push(kind);
    out.extend_from_slice(bytes);
    out
}

/// A binary message's stream, kind and bytes, if it's a frame.
pub fn parse_stream_frame(bytes: &[u8]) -> Option<(u32, u8, &[u8])> {
    if bytes.len() < 5 {
        return None;
    }
    Some((u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]), bytes[4], &bytes[5..]))
}

fn router() -> Router {
    Router::new()
        .route("/_ping", get(ping).head(ping))
        .route("/", get(root))
        .route("/mcp", post(mcp_post).get(mcp_get).delete(mcp_other))
        .route("/v1/{*rest}", any(engine_rest))
        .fallback(|| async { error(StatusCode::NOT_FOUND, "not found: MCP is at /mcp, the Engine API under /v1") })
        .layer(DefaultBodyLimit::max(MOST_BODY))
}

async fn ping() -> &'static str {
    "OK"
}

async fn root(Extension(conn): Extension<Arc<Conn>>, headers: HeaderMap, ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>) -> Response {
    match ws {
        Ok(ws) => upgrade(conn, &headers, ws),
        Err(_) => error(StatusCode::NOT_FOUND, "not found: MCP is at /mcp, the Engine API under /v1, and a WebSocket here"),
    }
}

async fn mcp_get(Extension(conn): Extension<Arc<Conn>>, headers: HeaderMap, ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>) -> Response {
    match ws {
        Ok(ws) => upgrade(conn, &headers, ws),
        Err(_) => mcp_other().await,
    }
}

/// No stream of the server's own (GET), and no sessions to end (DELETE): every request stands alone.
async fn mcp_other() -> Response {
    let mut r = error(StatusCode::METHOD_NOT_ALLOWED, "POST a JSON-RPC message to /mcp");
    r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("POST"));
    r
}

fn upgrade(conn: Arc<Conn>, headers: &HeaderMap, ws: WebSocketUpgrade) -> Response {
    let who = match admit(&conn, headers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    ws.protocols(["mcp"]).on_upgrade(move |socket| websocket(socket, conn, who))
}

/// A WebSocket, until it closes: each text message dispatched (answered on a thread of its own), each binary one to
/// the stream it names.
async fn websocket(socket: WebSocket, conn: Arc<Conn>, who: Who) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<WsMessage>();
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            let closing = matches!(m, WsMessage::Close(_));
            if sink.send(m).await.is_err() || closing {
                break;
            }
        }
    });
    let peer = Arc::new(Peer::new(Arc::new(WsOutbox(tx.clone())), who.scopes, who.name));
    let server = conn.net.server.clone();
    while let Some(Ok(message)) = stream.next().await {
        match message {
            WsMessage::Text(text) => server.dispatch(&peer, text.as_str()),
            WsMessage::Binary(bytes) => match parse_stream_frame(&bytes) {
                Some((id, kind, payload)) => match Kind::from_byte(kind) {
                    Some(kind) => peer.deliver(id, kind, payload.to_vec()),
                    None => peer.log(&format!("a frame of unknown kind {kind} on stream {id}, dropped")),
                },
                None => peer.log("a binary message too short to be a frame, dropped"),
            },
            WsMessage::Close(_) => break,
            WsMessage::Ping(_) | WsMessage::Pong(_) => {}
        }
    }
    peer.hang_up();
    peer.log("disconnected");
    let _ = tx.send(WsMessage::Close(None));
    let _ = tokio::time::timeout(Duration::from_secs(5), writer).await;
    drop(conn);
}

/// Whether a request asks for progress: a `progressToken` in its `_meta`.
fn wants_progress(message: &Value) -> bool {
    message.pointer("/params/_meta/progressToken").is_some_and(|t| !t.is_null())
}

/// MCP's Streamable HTTP: one JSON-RPC message per POST. A request's answer is JSON, or server-sent events when the
/// client takes them and asked for progress (its notifications, then the answer). Anything else gets 202.
async fn mcp_post(Extension(conn): Extension<Arc<Conn>>, headers: HeaderMap, body: Bytes) -> Response {
    let who = match admit(&conn, &headers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    let message: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(crate::protocol::error(Value::Null, -32700, &format!("not JSON: {e}"), None))).into_response(),
    };
    if !message.is_object() {
        return (StatusCode::BAD_REQUEST, Json(crate::protocol::error(Value::Null, -32600, "one JSON-RPC message per request, an object", None))).into_response();
    }
    let server = conn.net.server.clone();
    let is_request = message.get("method").is_some() && message.get("id").is_some_and(|id| !id.is_null());
    let accept = headers.get(header::ACCEPT).and_then(|a| a.to_str().ok()).unwrap_or("");
    let events = accept.contains("text/event-stream") && (wants_progress(&message) || !accept.contains("application/json"));
    if !is_request {
        let peer = Arc::new(Peer::new(Arc::new(Discard), who.scopes, who.name));
        let _ = tokio::task::spawn_blocking(move || server.handle(&peer, message)).await;
        return StatusCode::ACCEPTED.into_response();
    }
    if !events {
        let peer = Arc::new(Peer::new(Arc::new(Discard), who.scopes, who.name));
        return match tokio::task::spawn_blocking(move || server.handle(&peer, message)).await {
            Ok(Some(reply)) => Json(reply).into_response(),
            Ok(None) => StatusCode::ACCEPTED.into_response(),
            Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
    }
    let (tx, rx) = mpsc::unbounded_channel::<(bool, Value)>();
    let peer = Arc::new(Peer::new(Arc::new(Events(tx.clone())), who.scopes, who.name));
    tokio::task::spawn_blocking(move || {
        let reply = server.handle(&peer, message).unwrap_or(Value::Null);
        let _ = tx.send((true, reply));
    });
    let stream = futures_util::stream::unfold((rx, false), |(mut rx, done)| async move {
        if done {
            return None;
        }
        let (last, message) = rx.recv().await?;
        if message.is_null() {
            return None;
        }
        Some((Ok::<_, Infallible>(Event::default().event("message").data(message.to_string())), (rx, last)))
    });
    Sse::new(stream).into_response()
}

/// The Engine API as REST, with `--engine` and a token with `carbon:engine`.
async fn engine_rest(
    Extension(conn): Extension<Arc<Conn>>,
    method: Method,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let who = match admit(&conn, &headers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    let server = conn.net.server.clone();
    if !server.engine() {
        return error(StatusCode::NOT_FOUND, "the Engine API is off: start carbon mcp with --engine");
    }
    if !who.scopes.engine {
        return error(StatusCode::FORBIDDEN, "this token doesn't reach the Engine API (carbon:engine)");
    }
    let peer = Arc::new(Peer::new(Arc::new(Discard), who.scopes, who.name));
    let (method, path) = (method.as_str().to_string(), uri.path().to_string());
    let answer = match tokio::task::spawn_blocking(move || rest::route(&server, &peer, &method, &path, &query, &body)).await {
        Ok(a) => a,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let status = StatusCode::from_u16(answer.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    match answer.body {
        _ if status == StatusCode::NO_CONTENT => status.into_response(),
        Body::Json(v) => (status, Json(v)).into_response(),
        Body::Bytes(b) => (status, [(header::CONTENT_TYPE, "application/octet-stream")], b).into_response(),
        Body::Text(t) => (status, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], t).into_response(),
    }
}

/// Serves one connection: HTTP/1.1, which may become a WebSocket.
async fn connection<I>(io: I, conn: Arc<Conn>, router: Router)
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |mut request: hyper::Request<hyper::body::Incoming>| {
        request.extensions_mut().insert(conn.clone());
        let mut router = router.clone();
        async move { router.call(request).await }
    });
    let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(hyper_util::rt::TokioIo::new(io), service)
        .with_upgrades()
        .await;
}

enum Listener {
    Tcp(tokio::net::TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener),
}

#[cfg(unix)]
fn bind_unix(path: &PathBuf) -> io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            return Err(io::Error::other(format!("-H {}: something that isn't a socket is there", path.display())));
        }
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(io::Error::other(format!("-H {}: another server is listening there", path.display())));
        }
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Serves on every `-H` until `--max-sessions` have been served, or a signal (SIGINT, SIGTERM).
pub fn serve(server: Server, net: NetOptions) -> io::Result<()> {
    let plan = check(&net).map_err(io::Error::other)?;
    for w in &plan.warnings {
        eprintln!("carbon mcp: warning: {w}");
    }
    let tls = match &net.tls {
        Some((cert, key)) => Some(tokio_rustls::TlsAcceptor::from(tls::config(cert, key).map_err(io::Error::other)?)),
        None => None,
    };
    let tokens = net.token_file.as_deref().map(tokens::Tokens::load).transpose().map_err(io::Error::other)?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let (stop, _) = watch::channel(false);
        let (done, _) = watch::channel(false);
        let state = Arc::new(Net {
            server: Arc::new(server),
            tokens,
            no_auth: net.no_auth,
            allow_origins: net.allow_origins.clone(),
            sessions: Sessions { max: net.max_sessions, counts: Mutex::new((0, 0)), stop, done },
        });
        let mut listeners = Vec::new();
        let mut sockets = Vec::new();
        for (listen, local) in &plan.listens {
            match listen {
                Listen::Tcp(addr) => {
                    let l = tokio::net::TcpListener::bind(addr).await.map_err(|e| io::Error::new(e.kind(), format!("-H {addr}: {e}")))?;
                    let scheme = if tls.is_some() { "https" } else { "http" };
                    eprintln!("carbon mcp: listening on {scheme}://{} (MCP at /mcp{})", l.local_addr()?, if state.server.engine() { ", the Engine API under /v1" } else { "" });
                    listeners.push((Listener::Tcp(l), *local));
                }
                #[cfg(unix)]
                Listen::Unix(path) => {
                    let l = bind_unix(path)?;
                    eprintln!("carbon mcp: listening on {}", path.display());
                    sockets.push(path.clone());
                    listeners.push((Listener::Unix(l), *local));
                }
                #[cfg(not(unix))]
                Listen::Unix(path) => return Err(io::Error::other(format!("-H {}: Unix sockets aren't on this host", path.display()))),
            }
        }
        let router = router();
        for (listener, local) in listeners {
            let (state, router, tls) = (state.clone(), router.clone(), tls.clone());
            let mut stop = state.sessions.stop.subscribe();
            tokio::spawn(async move {
                loop {
                    if *stop.borrow() {
                        break;
                    }
                    let conn = |address: String| Arc::new(Conn { net: state.clone(), address, local, counted: AtomicBool::new(false) });
                    tokio::select! {
                        _ = stop.changed() => {}
                        accepted = accept(&listener) => match accepted {
                            Ok(Accepted::Tcp(sock, peer)) => {
                                let _ = sock.set_nodelay(true);
                                let (conn, router, tls) = (conn(format!("tcp {peer}")), router.clone(), tls.clone());
                                tokio::spawn(async move {
                                    match tls {
                                        Some(tls) => match tls.accept(sock).await {
                                            Ok(sock) => connection(sock, conn, router).await,
                                            Err(e) => eprintln!("carbon mcp: tcp {peer}: TLS: {e}"),
                                        },
                                        None => connection(sock, conn, router).await,
                                    }
                                });
                            }
                            #[cfg(unix)]
                            Ok(Accepted::Unix(sock)) => {
                                tokio::spawn(connection(sock, conn("unix".into()), router.clone()));
                            }
                            Err(e) => {
                                eprintln!("carbon mcp: accepting: {e}");
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            });
        }
        let mut done = state.sessions.done.subscribe();
        let finished = async move {
            if state.sessions.max.is_some() {
                let _ = done.wait_for(|d| *d).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            _ = finished => eprintln!("carbon mcp: served --max-sessions {}; exiting", net.max_sessions.unwrap_or(0)),
            _ = signal() => eprintln!("carbon mcp: stopping"),
        }
        for path in sockets {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    })
}

enum Accepted {
    Tcp(tokio::net::TcpStream, std::net::SocketAddr),
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
}

async fn accept(listener: &Listener) -> io::Result<Accepted> {
    match listener {
        Listener::Tcp(l) => l.accept().await.map(|(s, a)| Accepted::Tcp(s, a)),
        #[cfg(unix)]
        Listener::Unix(l) => l.accept().await.map(|(s, _)| Accepted::Unix(s)),
    }
}

async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(parse_host("127.0.0.1:8080"), Ok(Listen::Tcp("127.0.0.1:8080".into())));
        assert_eq!(parse_host(":8080"), Ok(Listen::Tcp("0.0.0.0:8080".into())));
        assert_eq!(parse_host("/run/carbon.sock"), Ok(Listen::Unix("/run/carbon.sock".into())));
        assert_eq!(parse_host("./carbon.sock"), Ok(Listen::Unix("./carbon.sock".into())));
        assert!(parse_host("tcp://0.0.0.0:80").is_err());
        assert!(parse_host("localhost").is_err());
        assert!(parse_host("host:http").is_err());
    }

    #[test]
    fn who_may_listen_where() {
        let net = |hosts: &[&str], tokens: bool, no_auth: bool| NetOptions {
            hosts: hosts.iter().map(|h| h.to_string()).collect(),
            token_file: tokens.then(|| PathBuf::from("tokens")),
            no_auth,
            ..NetOptions::default()
        };
        let plan = check(&net(&["127.0.0.1:0"], false, false)).unwrap();
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.listens[0].1);
        assert!(check(&net(&["0.0.0.0:0"], false, false)).is_err());
        assert!(check(&net(&["0.0.0.0:0"], true, false)).unwrap().warnings.is_empty());
        let plan = check(&net(&["0.0.0.0:0"], false, true)).unwrap();
        assert!(!plan.listens[0].1);
        assert!(check(&net(&["0.0.0.0:0"], true, true)).is_err());
        assert!(check(&net(&["/tmp/x.sock"], false, false)).unwrap().warnings.is_empty());
        assert!(check(&net(&[], false, false)).is_err());
        assert!(check(&NetOptions { max_sessions: Some(0), ..net(&["127.0.0.1:0"], false, false) }).is_err());
    }

    #[test]
    fn stream_frames() {
        let f = stream_frame(7, 1, b"out");
        assert_eq!(parse_stream_frame(&f), Some((7, 1, &b"out"[..])));
        assert_eq!(parse_stream_frame(&[0, 0, 0]), None);
    }
}
