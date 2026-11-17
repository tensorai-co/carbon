//! What a launched machine writes, kept: its stdout and stderr in `<home>/machines/<name>/output`, in order, each
//! write a record (a stream byte, a 4-byte little-endian length, the bytes), so the two streams stay apart and in the
//! order they were written. Past [`CAP`] the file becomes `output.1` (dropping the one before) and a new one starts.
//! [`crate::Machine::logs`] reads them back, as lines.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Error, Result};

/// How big `output` grows before it's rotated.
pub const CAP: u64 = 8 << 20;
const OUT: u8 = 1;
const ERR: u8 = 2;

/// Which stream a line was written to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// A line a machine wrote, with its line end, or the end of what it wrote without one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub stream: LogStream,
    pub bytes: Vec<u8>,
}

impl LogLine {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

/// What [`crate::Machine::logs`] reads: everything (the default), the last lines, and whether to keep reading as the
/// machine writes more.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Logs {
    follow: bool,
    tail: Option<usize>,
}

impl Logs {
    pub fn new() -> Logs {
        Logs::default()
    }

    /// Keeps reading what it writes, until it stops (`docker logs -f`).
    pub fn follow(mut self) -> Logs {
        self.follow = true;
        self
    }

    /// Only the last `lines` lines of what's kept, then (with [`Logs::follow`]) what comes.
    pub fn tail(mut self, lines: usize) -> Logs {
        self.tail = Some(lines);
        self
    }
}

pub(crate) fn path(dir: &Path) -> PathBuf {
    dir.join("output")
}

/// Appends a launched machine's output to its log, as it comes.
pub(crate) struct Writer {
    dir: PathBuf,
    file: File,
    size: u64,
}

impl Writer {
    pub fn create(dir: &Path) -> Result<Writer> {
        let file = OpenOptions::new().create(true).append(true).open(path(dir)).map_err(|e| Error::io(format!("opening the machine's log: {e}")))?;
        let size = file.metadata().map_or(0, |m| m.len());
        Ok(Writer { dir: dir.to_path_buf(), file, size })
    }

    pub fn write(&mut self, stream: LogStream, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if self.size >= CAP {
            let _ = std::fs::rename(path(&self.dir), self.dir.join("output.1"));
            if let Ok(file) = OpenOptions::new().create(true).append(true).open(path(&self.dir)) {
                (self.file, self.size) = (file, 0);
            }
        }
        let mut record = Vec::with_capacity(5 + bytes.len());
        record.push(if stream == LogStream::Stdout { OUT } else { ERR });
        record.extend((bytes.len() as u32).to_le_bytes());
        record.extend_from_slice(bytes);
        if self.file.write_all(&record).is_ok() {
            self.size += record.len() as u64;
        }
    }
}

/// The records in `bytes`, and how many bytes they took (a record cut short at the end is left for later).
fn records(bytes: &[u8]) -> (Vec<(LogStream, &[u8])>, usize) {
    let (mut out, mut at) = (Vec::new(), 0);
    while bytes.len() - at >= 5 {
        let len = u32::from_le_bytes([bytes[at + 1], bytes[at + 2], bytes[at + 3], bytes[at + 4]]) as usize;
        if bytes.len() - at - 5 < len {
            break;
        }
        let stream = if bytes[at] == ERR { LogStream::Stderr } else { LogStream::Stdout };
        out.push((stream, &bytes[at + 5..at + 5 + len]));
        at += 5 + len;
    }
    (out, at)
}

/// Splits records into lines, each stream's apart: a line not yet ended waits for the rest.
#[derive(Default)]
struct Lines {
    partial: [Vec<u8>; 2],
}

impl Lines {
    fn push(&mut self, stream: LogStream, mut bytes: &[u8], out: &mut std::collections::VecDeque<LogLine>) {
        let i = usize::from(stream == LogStream::Stderr);
        while let Some(end) = bytes.iter().position(|&b| b == b'\n') {
            let mut line = std::mem::take(&mut self.partial[i]);
            line.extend_from_slice(&bytes[..=end]);
            out.push_back(LogLine { stream, bytes: line });
            bytes = &bytes[end + 1..];
        }
        self.partial[i].extend_from_slice(bytes);
    }

    /// What's left without a line end, once nothing more is coming.
    fn finish(&mut self, out: &mut std::collections::VecDeque<LogLine>) {
        for (i, stream) in [LogStream::Stdout, LogStream::Stderr].into_iter().enumerate() {
            if !self.partial[i].is_empty() {
                out.push_back(LogLine { stream, bytes: std::mem::take(&mut self.partial[i]) });
            }
        }
    }
}

/// A machine's kept output, as lines: [`crate::Machine::logs`].
pub struct LogReader {
    dir: PathBuf,
    follow: Option<Box<dyn Fn() -> bool + Send>>,
    file: Option<File>,
    pending: Vec<u8>,
    lines: Lines,
    ready: std::collections::VecDeque<LogLine>,
    done: bool,
}

impl LogReader {
    /// `running` says whether the machine still runs, for following.
    pub(crate) fn open(dir: &Path, logs: Logs, running: Box<dyn Fn() -> bool + Send>) -> Result<LogReader> {
        let mut reader = LogReader { dir: dir.to_path_buf(), follow: logs.follow.then_some(running), file: None, pending: Vec::new(), lines: Lines::default(), ready: Default::default(), done: false };
        // What's kept: the rotated file, then the current one.
        let mut kept = std::fs::read(dir.join("output.1")).unwrap_or_default();
        let mut file = match File::open(path(dir)) {
            Ok(file) => Some(file),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && reader.follow.is_some() => None,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if kept.is_empty() {
                    return Err(Error::not_found("it has no logs: only a launched machine's output is kept"));
                }
                None
            }
            Err(e) => return Err(Error::io(format!("reading the machine's log: {e}"))),
        };
        if let Some(file) = file.as_mut() {
            let _ = file.read_to_end(&mut kept);
        }
        let (records, used) = records(&kept);
        for (stream, bytes) in records {
            reader.lines.push(stream, bytes, &mut reader.ready);
        }
        reader.pending = kept[used..].to_vec();
        if reader.follow.is_none() {
            reader.lines.finish(&mut reader.ready);
        }
        if let Some(n) = logs.tail {
            while reader.ready.len() > n {
                reader.ready.pop_front();
            }
        }
        reader.file = file;
        Ok(reader)
    }

    /// Reads what's been written since, if following. Whether anything came.
    fn more(&mut self) -> bool {
        if self.file.is_none() {
            self.file = File::open(path(&self.dir)).ok();
        }
        let Some(file) = self.file.as_mut() else { return false };
        // Rotated meanwhile: the new file, from its start.
        let here = file.stream_position().unwrap_or(0);
        if std::fs::metadata(path(&self.dir)).is_ok_and(|m| m.len() < here) {
            self.file = File::open(path(&self.dir)).ok();
            self.pending.clear();
            return self.more();
        }
        let mut fresh = Vec::new();
        let _ = file.read_to_end(&mut fresh);
        if fresh.is_empty() {
            return false;
        }
        self.pending.extend(fresh);
        let pending = std::mem::take(&mut self.pending);
        let (records, used) = records(&pending);
        for (stream, bytes) in records {
            self.lines.push(stream, bytes, &mut self.ready);
        }
        self.pending = pending[used..].to_vec();
        true
    }
}

impl Iterator for LogReader {
    type Item = LogLine;

    fn next(&mut self) -> Option<LogLine> {
        loop {
            if let Some(line) = self.ready.pop_front() {
                return Some(line);
            }
            if self.done {
                return None;
            }
            let running = self.follow.as_ref()?();
            if self.more() {
                continue;
            }
            if !running {
                // One last read for what it wrote as it ended.
                self.more();
                self.lines.finish(&mut self.ready);
                self.done = true;
                continue;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_come_back_apart_and_in_order() {
        let dir = std::env::temp_dir().join(format!("carbon-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = Writer::create(&dir).unwrap();
        w.write(LogStream::Stdout, b"one\ntw");
        w.write(LogStream::Stderr, b"oops\n");
        w.write(LogStream::Stdout, b"o\nthree");
        drop(w);
        let lines: Vec<(LogStream, String)> = LogReader::open(&dir, Logs::new(), Box::new(|| false)).unwrap().map(|l| (l.stream, l.text())).collect();
        assert_eq!(lines, vec![
            (LogStream::Stdout, "one\n".to_string()),
            (LogStream::Stderr, "oops\n".to_string()),
            (LogStream::Stdout, "two\n".to_string()),
            (LogStream::Stdout, "three".to_string()),
        ]);
        let last: Vec<String> = LogReader::open(&dir, Logs::new().tail(2), Box::new(|| false)).unwrap().map(|l| l.text()).collect();
        assert_eq!(last, vec!["two\n", "three"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_log_is_rotated_and_both_are_read() {
        let dir = std::env::temp_dir().join(format!("carbon-logs-rot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = Writer::create(&dir).unwrap();
        w.size = CAP; // as if full
        w.write(LogStream::Stdout, b"after\n");
        std::fs::write(dir.join("output.1"), [&[OUT][..], &5u32.to_le_bytes(), b"befor"].concat()).unwrap();
        let lines: Vec<String> = LogReader::open(&dir, Logs::new(), Box::new(|| false)).unwrap().map(|l| l.text()).collect();
        assert_eq!(lines, vec!["beforafter\n"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
