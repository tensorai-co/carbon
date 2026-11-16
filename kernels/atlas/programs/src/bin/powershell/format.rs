//! How output shows, as PowerShell's default formatting does it: files and folders as Get-ChildItem's table under each
//! folder's `Directory:` line, hashtables as Name and Value, objects as a table (four properties or fewer) or a list,
//! and anything else as its text, a line each.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use programs::*;

use crate::value::Value;

fn is_file(v: &Value) -> bool {
    matches!(v, Value::Object(o) if o.type_name == "System.IO.FileInfo" || o.type_name == "System.IO.DirectoryInfo") && !v.property("Mode").is_null()
}

/// A value in a table's cell: an array as `{a, b}`.
pub fn cell(v: &Value) -> String {
    match v {
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().map(cell).collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Object(o) if o.text.is_none() && !o.type_name.starts_with("System.IO") => {
            let parts: Vec<String> = o.props.iter().map(|(k, x)| format!("{k}={}", cell(x))).collect();
            format!("@{{{}}}", parts.join("; "))
        }
        other => other.to_string(),
    }
}

pub fn name_of(v: &Value) -> String {
    match v.property("Name") {
        Value::Null => v.to_string(),
        n => n.to_string(),
    }
}

/// The properties shown for an object.
fn props(v: &Value) -> Vec<(String, Value)> {
    match v {
        Value::Object(o) => o.props.iter().filter(|(k, _)| !k.starts_with("PS")).cloned().collect(),
        Value::Hash(h) => h.borrow().clone(),
        _ => Vec::new(),
    }
}

fn files(rows: &[Value]) -> String {
    let mut s = String::new();
    let mut dir = String::new();
    for r in rows {
        let parent = r.property("PSParentPath").to_string();
        if parent != dir || s.is_empty() {
            dir = parent;
            s.push_str(&format!("\r\n\r\n    Directory: {dir}\r\n\r\n\r\n"));
            s.push_str(&format!("{:<6}{:>28}{:>15} {}\r\n", "Mode", "LastWriteTime", "Length", "Name"));
            s.push_str(&format!("{:<6}{:>28}{:>15} {}\r\n", "----", "-------------", "------", "----"));
        }
        let t = r.property("LastWriteTime");
        let (y, mo, d, h, mi) = (t.property("Year").int().unwrap_or(1970), t.property("Month").int().unwrap_or(1), t.property("Day").int().unwrap_or(1), t.property("Hour").int().unwrap_or(0), t.property("Minute").int().unwrap_or(0));
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        let when = format!("{:>10} {:>8}", format!("{mo}/{d}/{y}"), format!("{h12}:{mi:02} {}", if h < 12 { "AM" } else { "PM" }));
        let len = match r.property("Length") {
            Value::Null => String::new(),
            l => l.to_string(),
        };
        s.push_str(&format!("{:<6}{:>28}{:>15} {}\r\n", r.property("Mode"), when, len, r.property("Name")));
    }
    s.push_str("\r\n\r\n");
    s
}

/// Objects (or hashtables) as a table: the first one's properties as columns.
pub fn table(rows: &[Value], hide_headers: bool) -> String {
    if rows.is_empty() {
        return String::new();
    }
    if rows.iter().all(is_file) {
        return files(rows);
    }
    // Hashtables: a row per entry.
    if rows.iter().all(|r| matches!(r, Value::Hash(_))) {
        let mut entries: Vec<(String, String)> = Vec::new();
        for r in rows {
            if let Value::Hash(h) = r {
                entries.extend(h.borrow().iter().map(|(k, v)| (k.clone(), cell(v))));
            }
        }
        let w = entries.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(4).max(4);
        let mut s = String::from("\r\n");
        if !hide_headers {
            s.push_str(&format!("{:<w$} {}\r\n{:<w$} {}\r\n", "Name", "Value", "----", "-----"));
        }
        for (k, v) in entries {
            s.push_str(&format!("{k:<w$} {v}\r\n"));
        }
        s.push_str("\r\n");
        return s;
    }
    let columns: Vec<String> = props(&rows[0]).into_iter().map(|(k, _)| k).collect();
    if columns.is_empty() {
        let mut s = String::new();
        for r in rows {
            s.push_str(&format!("{}\r\n", r));
        }
        return s;
    }
    let cells: Vec<Vec<(String, bool)>> = rows.iter().map(|r| columns.iter().map(|c| {
        let v = r.property(c);
        let numeric = matches!(v, Value::Int(_) | Value::Double(_));
        (cell(&v), numeric)
    }).collect()).collect();
    let widths: Vec<usize> = columns.iter().enumerate().map(|(i, c)| cells.iter().map(|row| row[i].0.chars().count()).max().unwrap_or(0).max(c.chars().count())).collect();
    let numeric: Vec<bool> = (0..columns.len()).map(|i| cells.iter().all(|row| row[i].1 || row[i].0.is_empty())).collect();
    let line = |items: Vec<(String, bool)>| -> String {
        let n = items.len();
        let mut s = String::new();
        for (i, (text, _)) in items.into_iter().enumerate() {
            let w = widths[i];
            if numeric[i] {
                s.push_str(&format!("{text:>w$}"));
            } else if i + 1 == n {
                s.push_str(&text);
            } else {
                s.push_str(&format!("{text:<w$}"));
            }
            if i + 1 < n {
                s.push(' ');
            }
        }
        String::from(s.trim_end())
    };
    let mut s = String::from("\r\n");
    if !hide_headers {
        s.push_str(&line(columns.iter().map(|c| (c.clone(), false)).collect()));
        s.push_str("\r\n");
        s.push_str(&line(columns.iter().map(|c| ("-".repeat(c.chars().count()), false)).collect()));
        s.push_str("\r\n");
    }
    for row in cells {
        s.push_str(&line(row));
        s.push_str("\r\n");
    }
    s.push_str("\r\n");
    s
}

/// Objects as `Name : value` lines, one block each.
pub fn list(rows: &[Value]) -> String {
    let mut s = String::new();
    for r in rows {
        let ps = props(r);
        let w = ps.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
        s.push_str("\r\n");
        if ps.is_empty() {
            s.push_str(&format!("{r}\r\n"));
            continue;
        }
        for (k, v) in ps {
            s.push_str(&format!("{k:<w$} : {}\r\n", cell(&v)));
        }
    }
    s.push_str("\r\n");
    s
}

/// Output as text, the way the console shows it.
pub fn to_text(values: &[Value]) -> String {
    let mut s = String::new();
    let mut i = 0;
    let flat: Vec<Value> = values.iter().flat_map(|v| if matches!(v, Value::Array(_)) { v.items() } else { alloc::vec![v.clone()] }).collect();
    while i < flat.len() {
        let v = &flat[i];
        let structured = |x: &Value| matches!(x, Value::Hash(_)) || matches!(x, Value::Object(o) if o.text.is_none());
        if structured(v) {
            // A run of the same kind of thing, formatted together.
            let ty = v.type_name();
            let mut j = i;
            while j < flat.len() && structured(&flat[j]) && flat[j].type_name() == ty {
                j += 1;
            }
            let run = &flat[i..j];
            let wide = props(v).len() > 4 && !is_file(v) && !matches!(v, Value::Hash(_));
            s.push_str(&if wide { list(run) } else { table(run, false) });
            i = j;
            continue;
        }
        s.push_str(&v.to_string());
        s.push_str("\r\n");
        i += 1;
    }
    s
}

/// What a statement's output does at the prompt: shown.
pub fn out_default(values: &[Value]) {
    if values.is_empty() {
        return;
    }
    out(&to_text(values));
}

#[allow(dead_code)]
fn unused() -> String {
    ToString::to_string("")
}
