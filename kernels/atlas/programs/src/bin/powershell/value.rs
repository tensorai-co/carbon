//! PowerShell's values: what pipelines carry and variables hold. Strings, numbers, booleans, `$null`, arrays,
//! hashtables (ordered, keys case folded), objects with properties (a file's or folder's, `Select-Object`'s), and
//! script blocks. Conversions follow PowerShell's: the left operand decides an operator's type.

use alloc::format;
use alloc::rc::Rc;
use core::cell::RefCell;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::parse::Block;

#[derive(Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    Str(String),
    Array(Rc<Vec<Value>>),
    Hash(Rc<RefCell<Vec<(String, Value)>>>),
    Object(Rc<Object>),
    Block(Rc<Block>),
}

/// An object: its type's name (`System.IO.FileInfo`) and properties, in order.
pub struct Object {
    pub type_name: String,
    pub props: Vec<(String, Value)>,
    /// What it shows as, as a string, if not its properties (a date's text).
    pub text: Option<String>,
}

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    pub fn array(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(items))
    }

    pub fn object(type_name: &str, props: Vec<(&str, Value)>) -> Value {
        Value::Object(Rc::new(Object { type_name: type_name.to_string(), props: props.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), text: None }))
    }

    pub fn hash(pairs: Vec<(String, Value)>) -> Value {
        Value::Hash(Rc::new(RefCell::new(pairs)))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// As a condition: `$null`, `$false`, 0, `""` and an empty array are false; an array of one is that one's truth.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Double(d) => *d != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Array(a) => match a.len() {
                0 => false,
                1 => a[0].truthy(),
                _ => true,
            },
            _ => true,
        }
    }

    /// The items a pipeline takes from it: an array's, or itself (nothing for `$null`).
    pub fn items(&self) -> Vec<Value> {
        match self {
            Value::Array(a) => a.as_ref().clone(),
            Value::Null => Vec::new(),
            v => alloc::vec![v.clone()],
        }
    }

    /// A pipeline's output as one value: nothing is `$null`, one item is itself, more are an array.
    pub fn from_items(mut items: Vec<Value>) -> Value {
        match items.len() {
            0 => Value::Null,
            1 => items.pop().unwrap_or(Value::Null),
            _ => Value::array(items),
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Int(n) => Some(*n as f64),
            Value::Double(d) => Some(*d),
            Value::Bool(b) => Some(f64::from(u8::from(*b))),
            Value::Null => Some(0.0),
            Value::Str(s) => parse_number(s.trim()).and_then(|v| v.number()),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            Value::Double(d) => Some(round_even(*d) as i64),
            other => other.number().map(|d| round_even(d) as i64),
        }
    }

    /// A property's value, case folded; an array's is each item's (`$files.Name`).
    pub fn property(&self, name: &str) -> Value {
        let lower = name.to_ascii_lowercase();
        match self {
            Value::Object(o) => o.props.iter().find(|(k, _)| k.to_ascii_lowercase() == lower).map(|(_, v)| v.clone()).unwrap_or(Value::Null),
            Value::Hash(h) => {
                let h = h.borrow();
                match lower.as_str() {
                "count" if !h.iter().any(|(k, _)| k == "count") => Value::Int(h.len() as i64),
                "keys" => Value::array(h.iter().map(|(k, _)| Value::str(k.clone())).collect()),
                "values" => Value::array(h.iter().map(|(_, v)| v.clone()).collect()),
                _ => h.iter().find(|(k, _)| k.to_ascii_lowercase() == lower).map(|(_, v)| v.clone()).unwrap_or(Value::Null),
                }
            }
            Value::Str(s) if lower == "length" => Value::Int(s.chars().count() as i64),
            Value::Array(a) if lower == "count" || lower == "length" => Value::Int(a.len() as i64),
            Value::Array(a) => Value::from_items(a.iter().map(|v| v.property(name)).filter(|v| !v.is_null()).collect()),
            Value::Null => Value::Null,
            _ if lower == "count" || lower == "length" => Value::Int(1),
            _ => Value::Null,
        }
    }

    pub fn type_name(&self) -> String {
        String::from(match self {
            Value::Null => "null",
            Value::Bool(_) => "System.Boolean",
            Value::Int(n) if i32::try_from(*n).is_ok() => "System.Int32",
            Value::Int(_) => "System.Int64",
            Value::Double(_) => "System.Double",
            Value::Str(_) => "System.String",
            Value::Array(_) => "System.Object[]",
            Value::Hash(_) => "System.Collections.Hashtable",
            Value::Object(o) => return o.type_name.clone(),
            Value::Block(_) => "System.Management.Automation.ScriptBlock",
        })
    }
}

/// Rounding as .NET's to an integer: halves to even.
pub fn round_even(d: f64) -> f64 {
    let r = libm_round(d);
    if (d - trunc(d)).abs() == 0.5 && (r as i64) % 2 != 0 { r - d.signum() } else { r }
}

fn trunc(d: f64) -> f64 {
    if d.abs() >= 9.007_199_254_740_992e15 { d } else { (d as i64) as f64 }
}

fn libm_round(d: f64) -> f64 {
    let t = trunc(d);
    if (d - t).abs() >= 0.5 { t + d.signum() } else { t }
}

pub fn floor(d: f64) -> f64 {
    let t = trunc(d);
    if t > d { t - 1.0 } else { t }
}

/// A number written as PowerShell reads one: `12`, `0x1F`, `1.5`, `1e3`, with `kb`, `mb`, `gb`, `tb` suffixes.
pub fn parse_number(s: &str) -> Option<Value> {
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let (body, mult) = [("kb", 1i64 << 10), ("mb", 1 << 20), ("gb", 1 << 30), ("tb", 1 << 40), ("pb", 1 << 50)]
        .iter()
        .find_map(|(suffix, m)| lower.strip_suffix(suffix).map(|b| (b.to_string(), *m)))
        .unwrap_or((lower.clone(), 1));
    let (neg, body) = match body.strip_prefix('-') {
        Some(b) => (true, b.to_string()),
        None => (false, body.strip_prefix('+').unwrap_or(&body).to_string()),
    };
    let sign = if neg { -1 } else { 1 };
    if let Some(hex) = body.strip_prefix("0x") {
        return i64::from_str_radix(hex, 16).ok().map(|n| Value::Int(sign * n * mult));
    }
    if body.chars().all(|c| c.is_ascii_digit()) {
        return body.parse::<i64>().ok().map(|n| Value::Int(sign * n * mult));
    }
    let valid = body.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | '-' | '+')) && body.chars().any(|c| c.is_ascii_digit());
    if valid {
        return body.parse::<f64>().ok().map(|d| Value::Double(sign as f64 * d * mult as f64));
    }
    None
}

/// A number as PowerShell shows one: integers plainly, doubles with up to 15 significant digits and no trailing zeros.
pub fn format_double(d: f64) -> String {
    if d.is_nan() {
        return String::from("NaN");
    }
    if d.is_infinite() {
        return String::from(if d > 0.0 { "Infinity" } else { "-Infinity" });
    }
    if d == trunc(d) && d.abs() < 1e15 {
        return format!("{}", d as i64);
    }
    let s = format!("{d:.14}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    // At most 15 significant digits.
    let mut digits = 0;
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            if digits >= 15 {
                if out.contains('.') {
                    continue;
                }
                out.push('0');
                continue;
            }
            if c != '0' || digits > 0 {
                digits += 1;
            }
        }
        out.push(c);
    }
    String::from(out.trim_end_matches('0').trim_end_matches('.'))
}

impl core::fmt::Display for Value {
    /// As a string, as `"$x"` makes one: an array's items joined with spaces.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Value::Null => Ok(()),
            Value::Bool(b) => f.write_str(if *b { "True" } else { "False" }),
            Value::Int(n) => write!(f, "{n}"),
            Value::Double(d) => f.write_str(&format_double(*d)),
            Value::Str(s) => f.write_str(s),
            Value::Array(a) => {
                let parts: Vec<String> = a.iter().map(|v| v.to_string()).collect();
                f.write_str(&parts.join(" "))
            }
            Value::Hash(_) => f.write_str("System.Collections.Hashtable"),
            Value::Object(o) if o.text.is_some() => f.write_str(o.text.as_deref().unwrap_or("")),
            Value::Object(o) => match o.props.iter().find(|(k, _)| k == "Name") {
                Some((_, v)) if o.type_name.starts_with("System.IO") => write!(f, "{v}"),
                _ => {
                    let parts: Vec<String> = o.props.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    write!(f, "@{{{}}}", parts.join("; "))
                }
            },
            Value::Block(b) => f.write_str(&b.text),
        }
    }
}

/// Compares two values as PowerShell's operators do: by the left one's type, strings case folded unless `case`.
pub fn compare(a: &Value, b: &Value, case: bool) -> Option<Ordering> {
    match a {
        Value::Null => Some(if b.is_null() { Ordering::Equal } else { Ordering::Less }),
        Value::Int(_) | Value::Double(_) => {
            let x = a.number()?;
            let y = b.number()?;
            x.partial_cmp(&y)
        }
        Value::Bool(x) => Some(x.cmp(&b.truthy())),
        Value::Str(s) => {
            let t = b.to_string();
            Some(if case { s.as_str().cmp(t.as_str()) } else { s.to_ascii_lowercase().cmp(&t.to_ascii_lowercase()) })
        }
        _ => {
            let (x, y) = (a.to_string(), b.to_string());
            Some(x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase()))
        }
    }
}

pub fn equal(a: &Value, b: &Value, case: bool) -> bool {
    if b.is_null() {
        return a.is_null();
    }
    compare(a, b, case) == Some(Ordering::Equal)
}

/// `-like`'s wildcards: `*`, `?`, `[abc]`, `[a-z]`, case folded unless `case`.
pub fn like(text: &str, pattern: &str, case: bool) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => (0..=t.len()).any(|i| go(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && go(&p[1..], &t[1..]),
            Some('[') => {
                let Some(end) = p.iter().position(|&c| c == ']') else { return !t.is_empty() && t[0] == '[' && go(&p[1..], &t[1..]) };
                let set = &p[1..end];
                let Some(&c) = t.first() else { return false };
                let mut hit = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        hit |= set[i] <= c && c <= set[i + 2];
                        i += 3;
                    } else {
                        hit |= set[i] == c;
                        i += 1;
                    }
                }
                hit && go(&p[end + 1..], &t[1..])
            }
            Some('`') if p.len() > 1 => !t.is_empty() && t[0] == p[1] && go(&p[2..], &t[1..]),
            Some(&c) => !t.is_empty() && t[0] == c && go(&p[1..], &t[1..]),
        }
    }
    let (t, p) = if case { (text.to_string(), pattern.to_string()) } else { (text.to_ascii_lowercase(), pattern.to_ascii_lowercase()) };
    go(&p.chars().collect::<Vec<_>>(), &t.chars().collect::<Vec<_>>())
}
