//! Just enough HTTP/1.1 to answer a package manager: requests one after another on a connection (apt pipelines
//! them), each read whole before it's answered, and answers with a length or chunked.

use std::io::{self, BufRead, Read, Write};

/// The most a request's line and headers may be.
const MOST_HEAD: usize = 32 << 10;

/// A request, as the tool sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// The path, without the query, as it was sent (still percent-encoded).
    pub path: String,
    /// The query, without its `?`, as it was sent.
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The connection stays open after the answer.
    pub keep_alive: bool,
}

impl Request {
    /// A header's value, its name in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// An answer in the tool's own protocol.
pub struct Native {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl std::fmt::Debug for Native {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Native({} {})", self.status, self.reason)
    }
}

pub enum Body {
    Empty,
    Bytes(Vec<u8>),
    /// Read as it's written: with its length if it's known, else chunked.
    Stream(Box<dyn Read + Send>, Option<u64>),
}

impl Native {
    pub fn new(status: u16, reason: impl Into<String>) -> Native {
        Native { status, reason: reason.into(), headers: Vec::new(), body: Body::Empty }
    }
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Native {
        self.headers.push((name.to_string(), value.into()));
        self
    }
    pub fn bytes(mut self, bytes: impl Into<Vec<u8>>) -> Native {
        self.body = Body::Bytes(bytes.into());
        self
    }
    pub fn stream(mut self, reader: Box<dyn Read + Send>, len: Option<u64>) -> Native {
        self.body = Body::Stream(reader, len);
        self
    }
    /// A plain answer with no body worth reading: `404 Not Found`, and why.
    pub fn plain(status: u16, reason: &str, why: &str) -> Native {
        Native::new(status, clean(reason)).header("Content-Type", "text/plain; charset=utf-8").bytes(format!("{why}\n"))
    }
}

/// Text safe for a status line: printable ASCII, at most 200 bytes.
pub fn clean(text: &str) -> String {
    let mut out: String = text.chars().map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { ' ' }).collect();
    out.truncate(200);
    out
}

/// The next request, None at the connection's end, its body at most `most_body` bytes, sent with a length or
/// chunked. An error is a request that can't be read, and ends the connection.
pub fn read_request(r: &mut impl BufRead, most_body: u64) -> io::Result<Option<Request>> {
    let mut head = Vec::new();
    loop {
        let start = head.len();
        let n = r.by_ref().take((MOST_HEAD - start.min(MOST_HEAD)) as u64 + 1).read_until(b'\n', &mut head)?;
        if n == 0 {
            return if head.iter().all(u8::is_ascii_whitespace) { Ok(None) } else { Err(bad("the connection ended in a request")) };
        }
        if head.len() > MOST_HEAD {
            return Err(bad("a request's head is too long"));
        }
        let line = &head[start..];
        // Blank lines before a request are skipped; one after its headers ends them.
        if line == b"\r\n" || line == b"\n" {
            if head[..start].iter().all(u8::is_ascii_whitespace) {
                head.clear();
                continue;
            }
            break;
        }
    }
    let text = std::str::from_utf8(&head).map_err(|_| bad("a request's head isn't text"))?;
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty());
    let first = lines.next().ok_or_else(|| bad("no request line"))?;
    let mut parts = first.split(' ');
    let (method, target, version) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(t), Some(v), None) if !m.is_empty() && t.starts_with('/') && v.starts_with("HTTP/1.") => (m, t, v),
        _ => return Err(bad("not an HTTP/1 request line")),
    };
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(|| bad("a header without a ':'"))?;
        if name.is_empty() || name.contains([' ', '\t']) {
            return Err(bad("a bad header name"));
        }
        headers.push((name.to_string(), value.trim().to_string()));
        if headers.len() > 100 {
            return Err(bad("too many headers"));
        }
    }
    let target = target.split('#').next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut req = Request { method: method.to_string(), path: path.to_string(), query: query.to_string(), headers, body: Vec::new(), keep_alive: false };
    let connection = req.header("Connection").unwrap_or("").to_ascii_lowercase();
    req.keep_alive = if version == "HTTP/1.0" { connection.contains("keep-alive") } else { !connection.contains("close") };
    if let Some(te) = req.header("Transfer-Encoding") {
        if !te.eq_ignore_ascii_case("chunked") {
            return Err(bad("a request body sent with a transfer encoding other than chunked"));
        }
        req.body = read_chunked(r, most_body)?;
        return Ok(Some(req));
    }
    if let Some(len) = req.header("Content-Length") {
        let len: u64 = len.parse().map_err(|_| bad("a bad Content-Length"))?;
        if len > most_body {
            return Err(bad("a request body too big"));
        }
        r.by_ref().take(len).read_to_end(&mut req.body)?;
        if req.body.len() as u64 != len {
            return Err(bad("the connection ended in a request's body"));
        }
    }
    Ok(Some(req))
}

/// A chunked body, whole, if it's at most `most` bytes.
fn read_chunked(r: &mut impl BufRead, most: u64) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut line = Vec::new();
        r.by_ref().take(1024).read_until(b'\n', &mut line)?;
        let text = std::str::from_utf8(&line).map_err(|_| bad("a bad chunk size"))?.trim();
        let size = u64::from_str_radix(text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| bad("a bad chunk size"))?;
        if size == 0 {
            // Trailers, if any, to the blank line.
            loop {
                let mut trailer = Vec::new();
                let n = r.by_ref().take(8192).read_until(b'\n', &mut trailer)?;
                if n == 0 || trailer == b"\r\n" || trailer == b"\n" {
                    return Ok(body);
                }
            }
        }
        if body.len() as u64 + size > most {
            return Err(bad("a request body too big"));
        }
        let start = body.len();
        r.by_ref().take(size).read_to_end(&mut body)?;
        if (body.len() - start) as u64 != size {
            return Err(bad("the connection ended in a chunk"));
        }
        let mut end = [0u8; 2];
        r.read_exact(&mut end)?;
        if &end != b"\r\n" {
            return Err(bad("a chunk without its CRLF"));
        }
    }
}

/// Writes `answer`. An error reading its body ends the connection with the answer cut short, so the tool sees a
/// broken download, never a whole one.
pub fn write_answer(w: &mut impl Write, answer: Native, keep_alive: bool, head_only: bool) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", answer.status, clean(&answer.reason));
    for (name, value) in &answer.headers {
        head.push_str(&format!("{name}: {}\r\n", clean(value)));
    }
    let length = match &answer.body {
        Body::Empty => Some(0),
        Body::Bytes(b) => Some(b.len() as u64),
        Body::Stream(_, len) => *len,
    };
    match length {
        Some(n) => head.push_str(&format!("Content-Length: {n}\r\n")),
        None => head.push_str("Transfer-Encoding: chunked\r\n"),
    }
    head.push_str(if keep_alive { "Connection: keep-alive\r\n\r\n" } else { "Connection: close\r\n\r\n" });
    w.write_all(head.as_bytes())?;
    if head_only {
        return w.flush();
    }
    match answer.body {
        Body::Empty => {}
        Body::Bytes(b) => w.write_all(&b)?,
        Body::Stream(mut reader, Some(len)) => {
            let copied = io::copy(&mut reader.by_ref().take(len), w)?;
            if copied != len {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the answer ended early"));
            }
        }
        Body::Stream(mut reader, None) => {
            let mut buf = vec![0u8; 64 << 10];
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                write!(w, "{n:x}\r\n")?;
                w.write_all(&buf[..n])?;
                w.write_all(b"\r\n")?;
            }
            w.write_all(b"0\r\n\r\n")?;
        }
    }
    w.flush()
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

/// `%XX` decoded, for a path's segment; None if what it decodes to isn't UTF-8, or has a `/` or a NUL in it.
pub fn decode_segment(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let text = String::from_utf8(out).ok()?;
    (!text.contains(['/', '\0'])).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn pipelined_requests_are_read_one_by_one() {
        let wire = b"GET /apt/debian/dists/trixie/InRelease HTTP/1.1\r\nHost: registry.carbon\r\nUser-Agent: Debian APT-HTTP/1.3\r\n\r\nGET /a?b=c HTTP/1.1\r\nConnection: close\r\n\r\n";
        let mut r = BufReader::new(&wire[..]);
        let first = read_request(&mut r, 1 << 20).unwrap().unwrap();
        assert_eq!((first.method.as_str(), first.path.as_str(), first.keep_alive), ("GET", "/apt/debian/dists/trixie/InRelease", true));
        assert_eq!(first.header("user-agent"), Some("Debian APT-HTTP/1.3"));
        let second = read_request(&mut r, 1 << 20).unwrap().unwrap();
        assert_eq!((second.path.as_str(), second.query.as_str(), second.keep_alive), ("/a", "b=c", false));
        assert!(read_request(&mut r, 1 << 20).unwrap().is_none());
        for bad in [&b"GET http://x/ HTTP/1.1\r\n\r\n"[..], b"GET / SPDY\r\n\r\n", b"GET /\r\n\r\n", b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n", b"GET / HTTP/1.1\r\nHost"] {
            assert!(read_request(&mut BufReader::new(bad), 1 << 20).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        let huge = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MOST_HEAD));
        assert!(read_request(&mut BufReader::new(huge.as_bytes()), 1 << 20).is_err());
        // A chunked body, as git sends a big one, with its trailers; and one over the limit.
        let chunked = b"POST /git/a/b/git-receive-pack HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;x=y\r\n world\r\n0\r\nX-T: 1\r\n\r\nGET / HTTP/1.1\r\n\r\n";
        let mut r = BufReader::new(&chunked[..]);
        assert_eq!(read_request(&mut r, 1 << 20).unwrap().unwrap().body, b"hello world");
        assert_eq!(read_request(&mut r, 1 << 20).unwrap().unwrap().path, "/");
        assert!(read_request(&mut BufReader::new(&chunked[..]), 8).is_err());
    }

    #[test]
    fn answers_with_a_length_or_chunked() {
        let mut out = Vec::new();
        write_answer(&mut out, Native::new(200, "OK").stream(Box::new(&b"hello"[..]), None), true, false).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n5\r\nhello\r\n0\r\n\r\n");
        let mut out = Vec::new();
        write_answer(&mut out, Native::plain(404, "Not Found - no\r\nX: y", "why"), false, false).unwrap();
        assert!(String::from_utf8(out).unwrap().starts_with("HTTP/1.1 404 Not Found - no  X: y\r\n"));
        // A body that ends short of its length is an error, and the connection ends.
        assert!(write_answer(&mut Vec::new(), Native::new(200, "OK").stream(Box::new(&b"hi"[..]), Some(5)), true, false).is_err());
    }

    #[test]
    fn segments_decode() {
        assert_eq!(decode_segment("libstdc%2b%2b6").as_deref(), Some("libstdc++6"));
        assert_eq!(decode_segment("a%2fb"), None);
        assert_eq!(decode_segment("%zz"), None);
    }
}
