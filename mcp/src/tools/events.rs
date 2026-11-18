//! What a machine does, as `carbon watch` sees it: the programs it starts, its connections and lookups, and what the
//! rules decided. A machine's events are seen from when the server first watched it (it watches each machine it
//! starts from its start), and kept, the last [`KEPT`], for a cursor to read.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Is, Tool, prop};
use crate::shape::{self, Args, Failure, Outcome};
use crate::{Progress, Server};

const KEPT: usize = 5000;

pub(crate) fn tools() -> Vec<Tool> {
    vec![Tool {
        name: "machine_events",
        title: "A machine's events",
        description: "What a running machine did since a cursor: each program it started (and its arguments), each connection and name it looked up, and what the rules decided (allowed, denied). Seen from when the server began watching it: from its start for machines this server started, else from the first call.",
        properties: json!({
            "cursor": prop("integer", "The cursor a call before gave back: only what came after it"),
            "wait_secs": prop("integer", "Wait up to this long for something to happen: 0 by default, 60 at most"),
        }),
        required: &[],
        is: Is::ReadOnly,
        on_machine: true,
        host: false,
        run: events,
    }]
}

/// One machine's events as they come, numbered from its first.
pub(crate) struct Feed {
    state: Mutex<FeedState>,
    changed: Condvar,
}

#[derive(Default)]
struct FeedState {
    lines: VecDeque<String>,
    /// The number of the first line kept.
    first: u64,
    ended: bool,
}

/// Starts watching a machine, if this server isn't already.
pub(crate) fn watch(server: &Server, name: &str) -> Arc<Feed> {
    let mut feeds = server.events.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(feed) = feeds.get(name).filter(|f| !f.state.lock().map_or(true, |s| s.ended)) {
        return feed.clone();
    }
    let feed = Arc::new(Feed { state: Mutex::new(FeedState::default()), changed: Condvar::new() });
    feeds.insert(name.to_string(), feed.clone());
    let watching = server.carbon.get(name).and_then(|m| m.watch(carbon::hooks::Interest::all()));
    let writer = feed.clone();
    std::thread::spawn(move || {
        if let Ok(mut w) = watching {
            while let Some(line) = w.next_line() {
                let mut s = writer.state.lock().unwrap_or_else(|e| e.into_inner());
                s.lines.push_back(line);
                if s.lines.len() > KEPT {
                    s.lines.pop_front();
                    s.first += 1;
                }
                writer.changed.notify_all();
            }
        }
        writer.state.lock().unwrap_or_else(|e| e.into_inner()).ended = true;
        writer.changed.notify_all();
    });
    feed
}

fn events(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let machine = server.machine(args)?;
    if !machine.is_running() {
        return Err(Failure(format!("{} isn't running", machine.name())));
    }
    let feed = watch(server, machine.name());
    let cursor = args.u64("cursor").unwrap_or(0);
    let deadline = Instant::now() + Duration::from_secs(args.u64("wait_secs").unwrap_or(0).min(60));
    let mut s = feed.state.lock().unwrap_or_else(|e| e.into_inner());
    while s.first + s.lines.len() as u64 <= cursor && !s.ended {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        s = feed.changed.wait_timeout(s, left).unwrap_or_else(|e| e.into_inner()).0;
    }
    let end = s.first + s.lines.len() as u64;
    let from = cursor.max(s.first);
    let events: Vec<Value> = s.lines.iter().skip((from - s.first) as usize).filter_map(|l| serde_json::from_str(l).ok()).collect();
    let mut text = format!("{} events (cursor {end})", events.len());
    if from > cursor {
        text.push_str(&format!(", {} older ones no longer kept", from - cursor));
    }
    let lines: Vec<&String> = s.lines.iter().skip((from - s.first) as usize).collect();
    let body: Vec<u8> = lines.iter().flat_map(|l| format!("\n{l}").into_bytes()).collect();
    let (body, cut) = shape::clip(&body, shape::OUTPUT_CAP);
    text.push_str(&body);
    Ok(shape::text_and(text, json!({ "cursor": end, "events": events, "truncated": cut })))
}
