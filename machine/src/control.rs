//! A running machine's control socket, for commands from other `carbon` processes: `carbon display screenshot`,
//! `windows`, and the input an agent drives a machine with (`move`, `click`, `drag`, `scroll`, `type`, `key`). It's a
//! Unix socket in the host's temporary folder (a socket's path has to be short), named in the machine's folder as
//! `control`. One request per connection: a line, then the answer: `ok <length>` and that many bytes, or `error <why>`.

use std::path::{Path, PathBuf};

/// The socket, removed when the machine ends.
pub struct Control {
    #[cfg_attr(not(unix), allow(dead_code))]
    paths: Vec<PathBuf>,
}

impl Drop for Control {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(unix)]
fn answer(stream: &mut std::os::unix::net::UnixStream, desktop: &Option<display::desktop::Shared>) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader, Write};
    let mut line = String::new();
    BufReader::new(&mut *stream).read_line(&mut line)?;
    // Only the line's end goes: text typed, copied or dropped keeps its own spaces at either end.
    let line = line.strip_suffix('\n').unwrap_or(&line);
    match (line.strip_suffix('\r').unwrap_or(line), desktop) {
        ("screenshot", Some(desktop)) => {
            let png = display::desktop::lock(desktop).screen().png();
            stream.write_all(format!("ok {}\n", png.len()).as_bytes())?;
            stream.write_all(&png)
        }
        ("windows", Some(desktop)) => {
            let text = windows(&display::desktop::lock(desktop));
            stream.write_all(format!("ok {}\n", text.len()).as_bytes())?;
            stream.write_all(text.as_bytes())
        }
        ("clipboard", Some(desktop)) => {
            let text = display::desktop::paste(desktop, display::selection::Selection::Clipboard, display::selection::TEXT, std::time::Duration::from_secs(2));
            match text {
                Some(bytes) => {
                    stream.write_all(format!("ok {}\n", bytes.len()).as_bytes())?;
                    stream.write_all(&bytes)
                }
                None => stream.write_all(b"error the clipboard has no text\n"),
            }
        }
        (line, Some(desktop)) if line.starts_with("clipboard ") => {
            // From outside the machine: an offer with no window, its text given at once. Whoever had it hears so.
            let text = unescape(&line["clipboard ".len()..]);
            let waker = {
                let mut d = display::desktop::lock(desktop);
                let source = d.offer(display::selection::Selection::Clipboard, None, Vec::new());
                d.offer_data(source, display::selection::TEXT, text.into_bytes());
                if d.has_events() { d.waker() } else { None }
            };
            if let Some(wake) = waker {
                wake();
            }
            stream.write_all(b"ok 0\n")
        }
        (line, Some(desktop)) if line.starts_with("drop ") => {
            // From outside the machine, as the host's drag brings it in: `drop X Y text TEXT` or `drop X Y files PATHS`
            // (one a line). What the window there did with it comes back.
            let mut parts = line["drop ".len()..].splitn(4, ' ');
            let (x, y, kind, body) = (parts.next(), parts.next(), parts.next(), parts.next().map(unescape).unwrap_or_default());
            let (Some(Ok(x)), Some(Ok(y))) = (x.map(str::parse::<i32>), y.map(str::parse::<i32>)) else {
                return stream.write_all(b"error a drop needs a point: X Y\n");
            };
            use display::desktop::action;
            use display::selection::{TEXT, URI_LIST, uri_list};
            let data = match kind {
                Some("text") => vec![(TEXT.to_string(), body.into_bytes())],
                Some("files") => vec![(URI_LIST.to_string(), uri_list(&body.lines().collect::<Vec<_>>()))],
                _ => return stream.write_all(b"error a drop is of text or files\n"),
            };
            match display::desktop::drop_from_outside(desktop, x, y, data, action::COPY, std::time::Duration::from_secs(2)) {
                Ok(done) => {
                    let what = match done {
                        action::COPY => "copied",
                        action::MOVE => "moved",
                        action::LINK => "linked",
                        action::ASK => "asked",
                        _ => "nothing there took it",
                    };
                    stream.write_all(format!("ok {}\n{what}", what.len()).as_bytes())
                }
                Err(e) => stream.write_all(format!("error {e}\n").as_bytes()),
            }
        }
        (line, Some(desktop)) if line.starts_with("resize ") => match display::output::parse_size(&line["resize ".len()..]) {
            Some((width, height, scale)) => {
                let waker = {
                    let mut d = display::desktop::lock(desktop);
                    let output = display::output::Output { width, height, scale, ..d.output().clone() };
                    d.set_output(output);
                    d.waker()
                };
                // Its kernel reads what that made, without the desktop's lock: configured windows, new scales.
                if let Some(wake) = waker {
                    wake();
                }
                stream.write_all(b"ok 0\n")
            }
            None => stream.write_all(b"error resize takes WIDTHxHEIGHT or WIDTHxHEIGHT@SCALE\n"),
        },
        (line, Some(desktop)) if INPUT.iter().any(|v| line.split(' ').next() == Some(*v)) => match input(desktop, line) {
            Ok(()) => stream.write_all(b"ok 0\n"),
            Err(why) => stream.write_all(format!("error {why}\n").as_bytes()),
        },
        ("screenshot" | "windows", None) => stream.write_all(b"error this machine has no desktop to show (lambda's windows come with its Wayland and X11 servers)\n"),
        (other, _) => stream.write_all(format!("error {other:?} isn't something a machine is asked\n").as_bytes()),
    }
}

/// The requests that are input.
const INPUT: [&str; 6] = ["move", "click", "drag", "scroll", "type", "key"];

/// Input for the desktop, as a user's would come: each event through `display::desktop::send`, which wakes the
/// kernel. Checked whole before any of it's sent, so a bad request does nothing.
#[cfg_attr(not(unix), allow(dead_code))]
fn input(desktop: &display::desktop::Shared, line: &str) -> Result<(), String> {
    use display::desktop::{Button, Input, send};
    use display::keys;
    let mut words = line.splitn(2, ' ');
    let verb = words.next().unwrap_or_default();
    let rest = words.next().unwrap_or_default();
    let numbers = |want: std::ops::RangeInclusive<usize>| -> Result<Vec<i32>, String> {
        let n: Vec<i32> = rest.split_whitespace().map_while(|w| w.parse().ok()).collect();
        if want.contains(&n.len()) { Ok(n) } else { Err(format!("{verb} takes {}", usage(verb))) }
    };
    let mut events = Vec::new();
    match verb {
        "move" => {
            let n = numbers(2..=2)?;
            events.push(Input::PointerTo(n[0], n[1]));
        }
        "click" => {
            let n = numbers(2..=2)?;
            let words: Vec<&str> = rest.split_whitespace().skip(2).collect();
            let button = match words.first().copied() {
                None | Some("left") => Button::Left,
                Some("right") => Button::Right,
                Some("middle") => Button::Middle,
                Some(other) => return Err(format!("{other} isn't a button: left, right, middle")),
            };
            let count = match words.get(1) {
                Some(c) => c.parse::<u32>().ok().filter(|c| (1..=3).contains(c)).ok_or("a click's count is 1, 2 or 3")?,
                None => 1,
            };
            events.push(Input::PointerTo(n[0], n[1]));
            for _ in 0..count {
                events.push(Input::Button(button, true));
                events.push(Input::Button(button, false));
            }
        }
        "drag" => {
            let n = numbers(4..=4)?;
            events.push(Input::PointerTo(n[0], n[1]));
            events.push(Input::Button(Button::Left, true));
            // On the way, so it moves as a hand would.
            for step in 1..=8 {
                events.push(Input::PointerTo(n[0] + (n[2] - n[0]) * step / 8, n[1] + (n[3] - n[1]) * step / 8));
            }
            events.push(Input::Button(Button::Left, false));
        }
        "scroll" => {
            let n = numbers(3..=4)?;
            events.push(Input::PointerTo(n[0], n[1]));
            events.push(Input::Scroll { x: n.get(3).copied().unwrap_or(0) * 120, y: n[2] * 120 });
        }
        "type" => {
            // Keys for what the keyboard types (US English), as a user's; anything else as text, as an input method
            // commits it.
            let text = unescape(rest);
            let mut other = String::new();
            for c in text.chars() {
                let Some((code, shift)) = keys::for_char(c) else {
                    other.push(c);
                    continue;
                };
                if !other.is_empty() {
                    events.push(Input::Text(std::mem::take(&mut other)));
                }
                if shift {
                    events.push(Input::Key(keys::LEFT_SHIFT, true));
                }
                events.push(Input::Key(code, true));
                events.push(Input::Key(code, false));
                if shift {
                    events.push(Input::Key(keys::LEFT_SHIFT, false));
                }
            }
            if !other.is_empty() {
                events.push(Input::Text(other));
            }
        }
        "key" => {
            if rest.trim().is_empty() {
                return Err(format!("key takes {}", usage("key")));
            }
            for combo in rest.split_whitespace() {
                let codes: Vec<u16> = combo.split('+').map(|k| keys::by_name(k).ok_or_else(|| format!("{k:?} isn't a key's name"))).collect::<Result<_, _>>()?;
                events.extend(codes.iter().map(|&c| Input::Key(c, true)));
                events.extend(codes.iter().rev().map(|&c| Input::Key(c, false)));
            }
        }
        _ => return Err(format!("{verb:?} isn't input")),
    }
    for e in events {
        send(desktop, e);
    }
    Ok(())
}

/// How each input request is written.
fn usage(verb: &str) -> &'static str {
    match verb {
        "move" => "X Y",
        "click" => "X Y [left|right|middle] [COUNT]",
        "drag" => "FROM_X FROM_Y TO_X TO_Y",
        "scroll" => "X Y NOTCHES [SIDEWAYS] (up and right positive)",
        "type" => "TEXT",
        _ => "a key or keys held together, as ctrl+s or alt+f4",
    }
}

/// A request's text, with `%0A` for a new line and `%25` for `%` (a request is one line).
fn unescape(text: &str) -> String {
    text.replace("%0A", "\n").replace("%25", "%")
}

/// Text made fit for a one-line request: what `unescape` undoes.
pub fn escape(text: &str) -> String {
    text.replace('%', "%25").replace('\n', "%0A")
}

/// The desktop's windows, top first, as `carbon display windows` shows them: for an agent to find what it's to click.
#[cfg_attr(not(unix), allow(dead_code))]
fn windows(d: &display::Desktop) -> String {
    use display::desktop::state;
    let mut out = format!("{:<8}{:<12}{:<12}{:<28}TITLE\n", "WINDOW", "AT", "SIZE", "STATE");
    for w in d.windows().iter().rev() {
        let mut what = Vec::new();
        if d.active() == Some(w.id) {
            what.push("active".to_string());
        }
        if !w.on_screen {
            what.push(if w.states & state::MINIMIZED != 0 { "minimized" } else { "hidden" }.to_string());
        }
        for (bit, name) in [(state::MAXIMIZED, "maximized"), (state::FULLSCREEN, "fullscreen")] {
            if w.states & bit != 0 {
                what.push(name.to_string());
            }
        }
        if w.attention {
            what.push("attention".to_string());
        }
        match (w.popup, w.parent) {
            (true, Some(p)) => what.push(format!("popup of {p}")),
            (false, Some(p)) => what.push(format!("dialog of {p}")),
            _ => {}
        }
        let state = if what.is_empty() { "-".to_string() } else { what.join(",") };
        let (at, size) = (format!("{},{}", w.rect.left, w.rect.top), format!("{}x{}", w.rect.width(), w.rect.height()));
        out.push_str(&format!("{:<8}{:<12}{:<12}{:<28}{}\n", w.id, at, size, state, w.title));
    }
    out
}

/// Starts answering on a socket for the machine whose folder is `dir`.
#[cfg(unix)]
pub fn serve(dir: PathBuf, desktop: Option<display::desktop::Shared>) -> Option<Control> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    dir.hash(&mut hasher);
    let socket = std::env::temp_dir().join(format!("carbon-{:016x}.sock", hasher.finish()));
    let _ = std::fs::remove_file(&socket);
    let listener = std::os::unix::net::UnixListener::bind(&socket).ok()?;
    let named = dir.join("control");
    std::fs::write(&named, socket.to_string_lossy().as_bytes()).ok()?;
    let _ = std::thread::Builder::new().name("carbon control".into()).spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = answer(&mut stream, &desktop);
        }
    });
    Some(Control { paths: vec![socket, named] })
}

#[cfg(not(unix))]
pub fn serve(_dir: PathBuf, _desktop: Option<display::desktop::Shared>) -> Option<Control> {
    None
}

/// Asks the running machine whose folder is `dir`, and gives its answer's bytes.
#[cfg(unix)]
pub fn ask(dir: &Path, request: &str) -> Result<Vec<u8>, String> {
    use std::io::{BufRead, BufReader, Read, Write};
    let socket = std::fs::read_to_string(dir.join("control")).map_err(|_| "the machine isn't running".to_string())?;
    let mut stream = std::os::unix::net::UnixStream::connect(socket.trim()).map_err(|e| format!("reaching the machine: {e}"))?;
    stream.write_all(format!("{request}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let line = line.trim_end();
    if let Some(why) = line.strip_prefix("error ") {
        return Err(why.to_string());
    }
    let len: usize = line.strip_prefix("ok ").and_then(|n| n.parse().ok()).ok_or("the machine's answer made no sense")?;
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[cfg(not(unix))]
pub fn ask(_dir: &Path, _request: &str) -> Result<Vec<u8>, String> {
    Err("asking a running machine isn't there yet on Windows hosts".into())
}
