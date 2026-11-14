//! GLSL's preprocessor and lexer, as one pass: the source cut into tokens (comments and line continuations gone),
//! directives obeyed line by line (`#version`, `#define` with and without parameters, `#undef`, the conditionals with
//! `defined`, `#extension`, `#pragma`, `#line`, `#error`), and macros expanded in what's left. The parser gets tokens,
//! each with the line it came from.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use super::{Error, Version, error};

/// A token, as the parser sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    /// An integer literal: its value (wrapped to 32 bits as GLSL does) and whether it's unsigned (`u`).
    Int(u32, bool),
    /// A floating-point literal; `true` if it's a double (`lf`).
    Float(f64, bool),
    /// Punctuation: one of [`PUNCT`].
    Punct(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
}

/// Every punctuator, longest first so the lexer takes the longest that fits.
const PUNCT: &[&str] = &[
    "<<=", ">>=", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "^^", "*=", "/=", "%=", "+=", "-=", "&=", "^=", "|=", "##", "(", ")",
    "[", "]", "{", "}", ".", ",", ";", ":", "?", "=", "+", "-", "*", "/", "%", "<", ">", "&", "|", "^", "!", "~", "#",
];

/// A token before preprocessing: the same, but its spacing kept, for function-like macros and `#define`'s body.
#[derive(Debug, Clone, PartialEq)]
struct PpTok {
    tok: Tok,
    /// Whether white space came before it (a `(` right after a macro's name makes it function-like).
    space: bool,
}

/// What `#extension` said for an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    Require,
    Enable,
    Warn,
    Disable,
}

/// The preprocessor's result: the tokens, the version, the extensions asked for, and warnings for the info log.
pub struct Output {
    pub tokens: Vec<Token>,
    pub version: Version,
    pub extensions: BTreeMap<String, Behavior>,
    pub warnings: Vec<String>,
    /// Whether the compatibility profile's built-ins are there: before GLSL 1.40, or asked for by name.
    pub compatibility: bool,
}

/// The extensions this compiler has, besides what's core in the version: `#extension ... : require` of another fails.
pub const EXTENSIONS: &[&str] = &[
    "GL_ARB_explicit_attrib_location",
    "GL_ARB_explicit_uniform_location",
    "GL_ARB_separate_shader_objects",
    "GL_ARB_shading_language_420pack",
    "GL_ARB_uniform_buffer_object",
    "GL_ARB_shader_storage_buffer_object",
    "GL_ARB_compute_shader",
    "GL_ARB_shader_bit_encoding",
    "GL_ARB_texture_rectangle",
    "GL_EXT_shader_texture_lod",
    "GL_OES_standard_derivatives",
    "GL_ARB_shader_texture_lod",
    "GL_EXT_frag_depth",
    "GL_EXT_draw_buffers",
];

#[derive(Clone)]
struct Macro {
    /// `None` for an object-like macro.
    params: Option<Vec<String>>,
    body: Vec<PpTok>,
}

/// Runs the preprocessor over a shader's source (or several, as `glShaderSource` takes, joined).
pub fn run(source: &str, es_default: bool) -> Result<Output, Error> {
    let lines = logical_lines(source)?;
    let mut pp = Pp { macros: BTreeMap::new(), version: Version { number: if es_default { 100 } else { 110 }, es: es_default }, extensions: BTreeMap::new(), warnings: Vec::new(), out: Vec::new(), line: 0, seen_code: false };
    // Each conditional's state: whether the group it's in is on, whether its own group is, whether one of its groups
    // was taken, and whether its `#else` was seen.
    let mut conds: Vec<Cond> = Vec::new();
    let mut version_seen = false;
    let mut compatibility_profile = false;
    // A function-like macro's call may go on over several lines: those tokens wait here, with their lines.
    let mut pending: Vec<(PpTok, u32)> = Vec::new();
    for (number, text) in lines {
        pp.line = number;
        let toks = lex(&text, number)?;
        let active = conds.last().is_none_or(|c| c.on);
        if toks.first().is_some_and(|t| t.tok == Tok::Punct("#")) {
            let name = match toks.get(1) {
                Some(PpTok { tok: Tok::Ident(n), .. }) => n.clone(),
                None => continue,
                Some(_) => {
                    if active {
                        return error(number, "a directive without a name");
                    }
                    continue;
                }
            };
            let args = &toks[2..];
            match name.as_str() {
                "if" | "ifdef" | "ifndef" => {
                    // A conditional inside a group that's off isn't evaluated.
                    let on = active
                        && match name.as_str() {
                            "if" => pp.condition(args)? != 0,
                            "ifdef" => pp.defined(args)?,
                            _ => !pp.defined(args)?,
                        };
                    conds.push(Cond { parent: active, on, taken: on, otherwise: false });
                }
                "elif" => {
                    let Some(c) = conds.last() else { return error(number, "an #elif without an #if") };
                    if c.otherwise {
                        return error(number, "an #elif after #else");
                    }
                    let on = c.parent && !c.taken && pp.condition(args)? != 0;
                    let c = conds.last_mut().expect("there");
                    c.on = on;
                    c.taken |= on;
                }
                "else" => {
                    let Some(c) = conds.last_mut() else { return error(number, "an #else without an #if") };
                    if c.otherwise {
                        return error(number, "a second #else");
                    }
                    c.otherwise = true;
                    c.on = c.parent && !c.taken;
                    c.taken = true;
                }
                "endif" => {
                    if conds.pop().is_none() {
                        return error(number, "an #endif without an #if");
                    }
                }
                _ if !active => {}
                "version" => {
                    if version_seen || pp.seen_code {
                        return error(number, "#version must come first, once");
                    }
                    version_seen = true;
                    let Some(Tok::Int(v, _)) = args.first().map(|t| t.tok.clone()) else { return error(number, "#version without a number") };
                    let profile = match args.get(1).map(|t| &t.tok) {
                        Some(Tok::Ident(p)) => p.as_str(),
                        _ => "",
                    };
                    let es = profile == "es" || v == 100;
                    let known = if es { matches!(v, 100 | 300 | 310 | 320) } else { matches!(v, 110 | 120 | 130 | 140 | 150 | 330 | 400 | 410 | 420 | 430 | 440 | 450 | 460) };
                    if !known || (es && v != 100 && profile != "es") {
                        return error(number, &format!("GLSL {v}{} isn't a version", if es { " es" } else { "" }));
                    }
                    if !matches!(profile, "" | "es" | "core" | "compatibility") {
                        return error(number, &format!("{profile} isn't a profile"));
                    }
                    pp.version = Version { number: v, es };
                    compatibility_profile = profile == "compatibility";
                }
                "define" => pp.define(args)?,
                "undef" => {
                    let Some(Tok::Ident(n)) = args.first().map(|t| &t.tok) else { return error(number, "#undef without a name") };
                    pp.macros.remove(n);
                }
                "extension" => {
                    let (Some(Tok::Ident(ext)), Some(Tok::Punct(":")), Some(Tok::Ident(b))) = (args.first().map(|t| &t.tok), args.get(1).map(|t| &t.tok), args.get(2).map(|t| &t.tok)) else {
                        return error(number, "#extension's form is `name : behavior`");
                    };
                    let behavior = match b.as_str() {
                        "require" => Behavior::Require,
                        "enable" => Behavior::Enable,
                        "warn" => Behavior::Warn,
                        "disable" => Behavior::Disable,
                        _ => return error(number, &format!("{b} isn't an extension's behavior")),
                    };
                    if ext == "all" {
                        if matches!(behavior, Behavior::Require | Behavior::Enable) {
                            return error(number, "`all` can only be warned of or disabled");
                        }
                    } else if !EXTENSIONS.contains(&ext.as_str()) {
                        if behavior == Behavior::Require {
                            return error(number, &format!("{ext} isn't supported"));
                        }
                        pp.warnings.push(format!("0:{number}: warning: {ext} isn't supported"));
                    }
                    pp.extensions.insert(ext.clone(), behavior);
                }
                "pragma" => {}
                "line" => {}
                "error" => {
                    let text: Vec<String> = args.iter().map(|t| spell(&t.tok)).collect();
                    return error(number, &format!("#error {}", text.join(" ")));
                }
                other => return error(number, &format!("#{other} isn't a directive")),
            }
            continue;
        }
        if !active {
            continue;
        }
        if !toks.is_empty() {
            pp.seen_code = true;
        }
        pending.extend(toks.into_iter().map(|t| (t, number)));
        // Expanded once no call is left open at the line's end.
        if pp.open_call(&pending) {
            continue;
        }
        let line_toks = core::mem::take(&mut pending);
        let first_line = line_toks.first().map_or(number, |t| t.1);
        let toks: Vec<PpTok> = line_toks.into_iter().map(|t| t.0).collect();
        let expanded = pp.expand(&toks, &mut Vec::new(), first_line)?;
        pp.out.extend(expanded.into_iter().map(|t| Token { tok: t.tok, line: first_line }));
    }
    if !conds.is_empty() {
        return error(pp.line, "an #if without its #endif");
    }
    if !pending.is_empty() {
        return error(pp.line, "a macro's arguments go on past the end");
    }
    if !version_seen && es_default {
        pp.version = Version { number: 100, es: true };
    }
    let compatibility = !pp.version.es && (pp.version.number < 140 || compatibility_profile);
    Ok(Output { tokens: pp.out, version: pp.version, extensions: pp.extensions, warnings: pp.warnings, compatibility })
}

/// One `#if`'s state.
struct Cond {
    /// Whether the group it's in is on.
    parent: bool,
    on: bool,
    taken: bool,
    otherwise: bool,
}

struct Pp {
    macros: BTreeMap<String, Macro>,
    version: Version,
    extensions: BTreeMap<String, Behavior>,
    warnings: Vec<String>,
    out: Vec<Token>,
    line: u32,
    seen_code: bool,
}

impl Pp {
    fn define(&mut self, args: &[PpTok]) -> Result<(), Error> {
        let Some(Tok::Ident(name)) = args.first().map(|t| &t.tok) else { return error(self.line, "#define without a name") };
        if name.starts_with("GL_") || name.contains("__") {
            return error(self.line, &format!("{name} is reserved"));
        }
        let mut rest = &args[1..];
        let params = if rest.first().is_some_and(|t| t.tok == Tok::Punct("(") && !t.space) {
            let mut params = Vec::new();
            rest = &rest[1..];
            loop {
                match rest.first().map(|t| &t.tok) {
                    Some(Tok::Punct(")")) if params.is_empty() => {
                        rest = &rest[1..];
                        break;
                    }
                    Some(Tok::Ident(p)) => {
                        params.push(p.clone());
                        rest = &rest[1..];
                        match rest.first().map(|t| &t.tok) {
                            Some(Tok::Punct(",")) => rest = &rest[1..],
                            Some(Tok::Punct(")")) => {
                                rest = &rest[1..];
                                break;
                            }
                            _ => return error(self.line, &format!("{name}'s parameters")),
                        }
                    }
                    _ => return error(self.line, &format!("{name}'s parameters")),
                }
            }
            Some(params)
        } else {
            None
        };
        let m = Macro { params, body: rest.to_vec() };
        if let Some(old) = self.macros.get(name)
            && (old.params != m.params || old.body.iter().map(|t| &t.tok).ne(m.body.iter().map(|t| &t.tok)))
        {
            return error(self.line, &format!("{name} is defined again, differently"));
        }
        self.macros.insert(name.clone(), m);
        Ok(())
    }

    fn defined(&self, args: &[PpTok]) -> Result<bool, Error> {
        match args.first().map(|t| &t.tok) {
            Some(Tok::Ident(n)) => Ok(self.is_defined(n)),
            _ => error(self.line, "#ifdef without a name"),
        }
    }

    fn is_defined(&self, name: &str) -> bool {
        self.macros.contains_key(name) || self.builtin(name).is_some()
    }

    /// The predefined macros.
    fn builtin(&self, name: &str) -> Option<Tok> {
        Some(match name {
            "__LINE__" => Tok::Int(self.line, false),
            "__FILE__" => Tok::Int(0, false),
            "__VERSION__" => Tok::Int(self.version.number, false),
            "GL_ES" if self.version.es => Tok::Int(1, false),
            "GL_FRAGMENT_PRECISION_HIGH" if self.version.es => Tok::Int(1, false),
            "GL_core_profile" if !self.version.es && self.version.number >= 150 => Tok::Int(1, false),
            _ if EXTENSIONS.contains(&name) => Tok::Int(1, false),
            _ => return None,
        })
    }

    /// Whether a function-like macro's name is followed by `(` whose `)` hasn't come yet.
    fn open_call(&self, toks: &[(PpTok, u32)]) -> bool {
        let mut i = 0;
        while i < toks.len() {
            if let Tok::Ident(n) = &toks[i].0.tok
                && self.macros.get(n).is_some_and(|m| m.params.is_some())
            {
                if i + 1 == toks.len() {
                    // The name last on the line: its `(` may be on the next.
                    return true;
                }
                if toks[i + 1].0.tok == Tok::Punct("(") {
                    let mut depth = 0;
                    let mut j = i + 1;
                    while j < toks.len() {
                        match toks[j].0.tok {
                            Tok::Punct("(") => depth += 1,
                            Tok::Punct(")") => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if j == toks.len() {
                        return true;
                    }
                    i = j;
                }
            }
            i += 1;
        }
        false
    }

    /// Macros expanded, and the result rescanned, never expanding a macro inside its own expansion.
    fn expand(&self, toks: &[PpTok], hidden: &mut Vec<String>, line: u32) -> Result<Vec<PpTok>, Error> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            let t = &toks[i];
            let Tok::Ident(name) = &t.tok else {
                out.push(t.clone());
                i += 1;
                continue;
            };
            if hidden.contains(name) {
                out.push(t.clone());
                i += 1;
                continue;
            }
            if let Some(b) = self.builtin(name).filter(|_| !self.macros.contains_key(name)) {
                let b = if name == "__LINE__" { Tok::Int(line, false) } else { b };
                out.push(PpTok { tok: b, space: t.space });
                i += 1;
                continue;
            }
            let Some(m) = self.macros.get(name) else {
                out.push(t.clone());
                i += 1;
                continue;
            };
            let body = match &m.params {
                None => {
                    i += 1;
                    m.body.clone()
                }
                Some(params) => {
                    if toks.get(i + 1).map(|t| &t.tok) != Some(&Tok::Punct("(")) {
                        // A function-like macro's name alone isn't a call.
                        out.push(t.clone());
                        i += 1;
                        continue;
                    }
                    let (args, end) = arguments(&toks[i + 2..]).ok_or_else(|| Error { message: format!("0:{line}: error: {name}'s arguments aren't closed") })?;
                    i += 2 + end + 1;
                    let args = if params.is_empty() && args.len() == 1 && args[0].is_empty() { Vec::new() } else { args };
                    if args.len() != params.len() {
                        return error(line, &format!("{name} takes {} arguments, given {}", params.len(), args.len()));
                    }
                    // Each argument expanded on its own first, then put in place.
                    let mut body = Vec::new();
                    for b in &m.body {
                        match &b.tok {
                            Tok::Ident(p) if params.contains(p) => {
                                let k = params.iter().position(|q| q == p).expect("there");
                                body.extend(self.expand(&args[k], hidden, line)?);
                            }
                            _ => body.push(b.clone()),
                        }
                    }
                    body
                }
            };
            let body = paste(body, line)?;
            hidden.push(name.clone());
            let again = self.expand(&body, hidden, line)?;
            hidden.pop();
            out.extend(again);
        }
        Ok(out)
    }

    /// `#if`'s expression: macros expanded (`defined` first), then evaluated as integers.
    fn condition(&self, args: &[PpTok]) -> Result<i64, Error> {
        let mut resolved = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if args[i].tok == Tok::Ident("defined".into()) {
                let (name, used) = match (args.get(i + 1).map(|t| &t.tok), args.get(i + 2).map(|t| &t.tok), args.get(i + 3).map(|t| &t.tok)) {
                    (Some(Tok::Ident(n)), _, _) => (n.clone(), 2),
                    (Some(Tok::Punct("(")), Some(Tok::Ident(n)), Some(Tok::Punct(")"))) => (n.clone(), 4),
                    _ => return error(self.line, "`defined` without a name"),
                };
                resolved.push(PpTok { tok: Tok::Int(u32::from(self.is_defined(&name)), false), space: true });
                i += used;
            } else {
                resolved.push(args[i].clone());
                i += 1;
            }
        }
        let toks = self.expand(&resolved, &mut Vec::new(), self.line)?;
        let mut e = Eval { toks: &toks, at: 0, line: self.line };
        let v = e.expr(0)?;
        if e.at != toks.len() {
            return error(self.line, "#if's expression goes on past its end");
        }
        Ok(v)
    }
}

/// A call's arguments, split at top-level commas, and where its `)` is.
fn arguments(toks: &[PpTok]) -> Option<(Vec<Vec<PpTok>>, usize)> {
    let mut args = vec![Vec::new()];
    let mut depth = 0;
    for (i, t) in toks.iter().enumerate() {
        match t.tok {
            Tok::Punct("(") => depth += 1,
            Tok::Punct(")") if depth == 0 => return Some((args, i)),
            Tok::Punct(")") => depth -= 1,
            Tok::Punct(",") if depth == 0 => {
                args.push(Vec::new());
                continue;
            }
            _ => {}
        }
        args.last_mut().expect("one").push(t.clone());
    }
    None
}

/// `##` joins the tokens on either side.
fn paste(body: Vec<PpTok>, line: u32) -> Result<Vec<PpTok>, Error> {
    if !body.iter().any(|t| t.tok == Tok::Punct("##")) {
        return Ok(body);
    }
    let mut out: Vec<PpTok> = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if body[i].tok == Tok::Punct("##") {
            let (Some(left), Some(right)) = (out.pop(), body.get(i + 1)) else { return error(line, "`##` at a macro's edge") };
            let joined = format!("{}{}", spell(&left.tok), spell(&right.tok));
            let mut t = lex(&joined, line)?;
            if t.len() != 1 {
                return error(line, &format!("`##` makes {joined}, which isn't a token"));
            }
            let mut t = t.remove(0);
            t.space = left.space;
            out.push(t);
            i += 2;
        } else {
            out.push(body[i].clone());
            i += 1;
        }
    }
    Ok(out)
}

/// A token as source text.
fn spell(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => s.clone(),
        Tok::Int(v, u) => format!("{v}{}", if *u { "u" } else { "" }),
        Tok::Float(v, d) => format!("{v:?}{}", if *d { "lf" } else { "" }),
        Tok::Punct(p) => (*p).to_string(),
    }
}

/// `#if`'s integer expressions, C's precedence.
struct Eval<'a> {
    toks: &'a [PpTok],
    at: usize,
    line: u32,
}

impl Eval<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at).map(|t| &t.tok)
    }

    fn unary(&mut self) -> Result<i64, Error> {
        let t = self.peek().cloned();
        self.at += 1;
        match t {
            Some(Tok::Int(v, _)) => Ok(i64::from(v)),
            // A name left after expansion is 0, as C has it; GLSL calls it an error, which this forgives.
            Some(Tok::Ident(_)) => Ok(0),
            Some(Tok::Punct("(")) => {
                let v = self.expr(0)?;
                if self.peek() != Some(&Tok::Punct(")")) {
                    return error(self.line, "#if's parentheses don't close");
                }
                self.at += 1;
                Ok(v)
            }
            Some(Tok::Punct("-")) => Ok(self.unary()?.wrapping_neg()),
            Some(Tok::Punct("+")) => self.unary(),
            Some(Tok::Punct("!")) => Ok(i64::from(self.unary()? == 0)),
            Some(Tok::Punct("~")) => Ok(!self.unary()?),
            _ => error(self.line, "#if's expression isn't integers"),
        }
    }

    fn expr(&mut self, min: u8) -> Result<i64, Error> {
        let mut left = self.unary()?;
        loop {
            let Some(Tok::Punct(op)) = self.peek().cloned() else { return Ok(left) };
            let prec = match op {
                "||" => 1,
                "&&" => 2,
                "|" => 3,
                "^" => 4,
                "&" => 5,
                "==" | "!=" => 6,
                "<" | ">" | "<=" | ">=" => 7,
                "<<" | ">>" => 8,
                "+" | "-" => 9,
                "*" | "/" | "%" => 10,
                "?" if min == 0 => {
                    self.at += 1;
                    let a = self.expr(0)?;
                    if self.peek() != Some(&Tok::Punct(":")) {
                        return error(self.line, "#if's ?: without its :");
                    }
                    self.at += 1;
                    let b = self.expr(0)?;
                    left = if left != 0 { a } else { b };
                    continue;
                }
                _ => return Ok(left),
            };
            if prec <= min {
                return Ok(left);
            }
            self.at += 1;
            let right = self.expr(prec)?;
            left = match op {
                "||" => i64::from(left != 0 || right != 0),
                "&&" => i64::from(left != 0 && right != 0),
                "|" => left | right,
                "^" => left ^ right,
                "&" => left & right,
                "==" => i64::from(left == right),
                "!=" => i64::from(left != right),
                "<" => i64::from(left < right),
                ">" => i64::from(left > right),
                "<=" => i64::from(left <= right),
                ">=" => i64::from(left >= right),
                "<<" => left.wrapping_shl(right as u32),
                ">>" => left.wrapping_shr(right as u32),
                "+" => left.wrapping_add(right),
                "-" => left.wrapping_sub(right),
                "*" => left.wrapping_mul(right),
                "/" | "%" if right == 0 => return error(self.line, "#if divides by zero"),
                "/" => left.wrapping_div(right),
                _ => left.wrapping_rem(right),
            };
        }
    }
}

/// The source as lines, each with its first line's number: comments gone (a block comment's lines kept as empty
/// lines), and a backslash at a line's end joining the next to it.
fn logical_lines(source: &str) -> Result<Vec<(u32, String)>, Error> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut start = 1u32;
    let mut line = 1u32;
    let mut chars = source.chars().peekable();
    let mut in_block = false;
    while let Some(c) = chars.next() {
        if in_block {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block = false;
                current.push(' ');
            } else if c == '\n' {
                line += 1;
            }
            continue;
        }
        match c {
            '/' if chars.peek() == Some(&'/') => {
                while chars.peek().is_some_and(|&c| c != '\n') {
                    chars.next();
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                in_block = true;
            }
            '\\' if matches!(chars.peek(), Some('\n') | Some('\r')) => {
                if chars.next() == Some('\r') && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                line += 1;
            }
            '\r' => {}
            '\n' => {
                lines.push((start, core::mem::take(&mut current)));
                line += 1;
                start = line;
            }
            c => current.push(c),
        }
    }
    if in_block {
        return error(line, "a comment that isn't closed");
    }
    lines.push((start, current));
    Ok(lines)
}

/// One line's tokens.
fn lex(text: &str, line: u32) -> Result<Vec<PpTok>, Error> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut space = true;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            space = true;
            i += 1;
            continue;
        }
        let start = i;
        let tok = if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            Tok::Ident(text[start..i].to_string())
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let (t, n) = number(&text[i..], line)?;
            i += n;
            t
        } else if let Some(p) = PUNCT.iter().find(|p| text[i..].starts_with(**p)) {
            i += p.len();
            Tok::Punct(p)
        } else {
            return error(line, &format!("`{}` isn't GLSL", text[i..].chars().next().map(String::from).unwrap_or_default()));
        };
        out.push(PpTok { tok, space });
        space = false;
    }
    Ok(out)
}

/// A number at the start of `s`: the token and how many bytes it took.
fn number(s: &str, line: u32) -> Result<(Tok, usize), Error> {
    let b = s.as_bytes();
    let mut i = 0;
    if b.len() > 1 && b[0] == b'0' && (b[1] == b'x' || b[1] == b'X') {
        i = 2;
        while i < b.len() && b[i].is_ascii_hexdigit() {
            i += 1;
        }
        let v = u64::from_str_radix(&s[2..i], 16).map_err(|_| Error { message: format!("0:{line}: error: {} isn't a number", &s[..i]) })?;
        return int(v, s, i, line);
    }
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut float = false;
    if i < b.len() && b[i] == b'.' {
        float = true;
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            float = true;
            i = j;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    if float || (i < b.len() && (b[i] == b'f' || b[i] == b'F')) {
        let v: f64 = parse_float(&s[..i]).ok_or_else(|| Error { message: format!("0:{line}: error: {} isn't a number", &s[..i]) })?;
        let mut double = false;
        if s[i..].starts_with("lf") || s[i..].starts_with("LF") {
            double = true;
            i += 2;
        } else if i < b.len() && (b[i] == b'f' || b[i] == b'F') {
            i += 1;
        }
        return Ok((Tok::Float(v, double), i));
    }
    let digits = &s[..i];
    let v = if digits.len() > 1 && digits.starts_with('0') {
        if digits.bytes().any(|d| d > b'7') {
            return error(line, &format!("{digits} isn't an octal number"));
        }
        u64::from_str_radix(&digits[1..], 8)
    } else {
        digits.parse::<u64>()
    }
    .map_err(|_| Error { message: format!("0:{line}: error: {digits} is too large") })?;
    int(v, s, i, line)
}

fn int(v: u64, s: &str, mut i: usize, line: u32) -> Result<(Tok, usize), Error> {
    let unsigned = s[i..].starts_with('u') || s[i..].starts_with('U');
    if unsigned {
        i += 1;
    }
    if v > u64::from(u32::MAX) {
        return error(line, &format!("{} is too large for 32 bits", &s[..i]));
    }
    if s.as_bytes().get(i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
        return error(line, &format!("{}… isn't a number", &s[..=i]));
    }
    Ok((Tok::Int(v as u32, unsigned), i))
}

/// A decimal float, as Rust's `str::parse` reads it once the leading or trailing `.` C allows is made explicit.
fn parse_float(s: &str) -> Option<f64> {
    let mut t = String::new();
    if s.starts_with('.') {
        t.push('0');
    }
    for (k, c) in s.char_indices() {
        t.push(c);
        if c == '.' && !s[k + 1..].starts_with(|d: char| d.is_ascii_digit()) {
            t.push('0');
        }
    }
    t.parse().ok()
}
