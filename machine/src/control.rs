//! A running machine's control socket, for commands from other `carbon` processes: `carbon display screenshot`. It's a
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
    match (line.trim(), desktop) {
        ("screenshot", Some(desktop)) => {
            let png = display::desktop::lock(desktop).screen().png();
            stream.write_all(format!("ok {}\n", png.len()).as_bytes())?;
            stream.write_all(&png)
        }
        ("screenshot", None) => stream.write_all(b"error this machine has no desktop to show (lambda's windows come with its Wayland and X11 servers)\n"),
        (other, _) => stream.write_all(format!("error {other:?} isn't something a machine is asked\n").as_bytes()),
    }
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
