//! Running what's parsed: scopes and variables (with `$env:` the process's environment), statements and their flow,
//! pipelines of objects, PowerShell's operators, casts, members and methods (strings', arrays', hashtables', and
//! `[math]`, `[string]`, `[System.IO.Path]`, `[Environment]`, `[regex]` and the like's static ones), and commands:
//! functions, aliases, cmdlets, scripts and programs.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use programs::*;

use crate::parse::{Arg, Block, Command, Element, Expr, Parser, Part, Pipeline, Stmt};
use crate::regex::{self, Regex};
use crate::value::{self, Value, compare, equal, floor, format_double, like, round_even};

/// What stops statements running in order.
pub enum Flow {
    Break,
    Continue,
    Return(Vec<Value>),
    Exit(i32),
    /// A terminating error.
    Throw(Value),
}

pub type Out = Result<Vec<Value>, Flow>;

pub struct Shell {
    scopes: Vec<BTreeMap<String, Value>>,
    pub functions: BTreeMap<String, Rc<Block>>,
    pub aliases: BTreeMap<String, String>,
    /// Whether the last command worked: `$?`.
    pub ok: bool,
    pub last_exit: i32,
    pub locations: Vec<String>,
    /// How deep in something that takes output as values (an assignment, `$()`, a pipe): programs' output is
    /// captured there, and goes straight to the console otherwise.
    pub capturing: usize,
}

/// A terminating error's value: its message, as an ErrorRecord shows it.
pub fn error(message: &str) -> Flow {
    Flow::Throw(Value::object("System.Management.Automation.ErrorRecord", vec![("Exception", Value::str(message)), ("Message", Value::str(message))]))
}

/// An error's message.
pub fn message(v: &Value) -> String {
    match v.property("Message") {
        Value::Null => v.to_string(),
        m => m.to_string(),
    }
}

impl Shell {
    pub fn new() -> Shell {
        let mut sh = Shell { scopes: vec![BTreeMap::new()], functions: BTreeMap::new(), aliases: BTreeMap::new(), ok: true, last_exit: 0, locations: Vec::new(), capturing: 0 };
        crate::cmdlets::aliases(&mut sh.aliases);
        sh
    }

    // ---- Variables ----

    pub fn get(&self, name: &str) -> Value {
        let lower = name.to_ascii_lowercase();
        let lower = ["global:", "script:", "local:", "private:", "variable:"].iter().find_map(|p| lower.strip_prefix(p)).unwrap_or(&lower).to_string();
        if let Some(env) = lower.strip_prefix("env:") {
            return get_env(env).map_or(Value::Null, Value::Str);
        }
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(&lower) {
                return v.clone();
            }
        }
        match lower.as_str() {
            "?" => Value::Bool(self.ok),
            "lastexitcode" => Value::Int(i64::from(self.last_exit)),
            "pwd" => Value::str(current_dir()),
            "home" => Value::str(get_env("USERPROFILE").unwrap_or_else(|| String::from("C:\\Users\\carbon"))),
            "pshome" => Value::str("C:\\Windows\\System32\\WindowsPowerShell\\v1.0"),
            "pid" => Value::Int(0),
            "psversiontable" => Value::hash(vec![
                (String::from("PSVersion"), Value::str("5.1.19041.1")),
                (String::from("PSEdition"), Value::str("Desktop")),
                (String::from("BuildVersion"), Value::str("10.0.19041.1")),
                (String::from("CLRVersion"), Value::str("4.0.30319.42000")),
                (String::from("WSManStackVersion"), Value::str("3.0")),
                (String::from("PSRemotingProtocolVersion"), Value::str("2.3")),
                (String::from("SerializationVersion"), Value::str("1.1.0.1")),
            ]),
            "erroractionpreference" => Value::str("Continue"),
            "ofs" => Value::str(" "),
            "psitem" => self.get("_"),
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::Null,
        }
    }

    pub fn set(&mut self, name: &str, value: Value) {
        let lower = name.to_ascii_lowercase();
        if let Some(global) = lower.strip_prefix("global:") {
            self.scopes[0].insert(global.to_string(), value);
            return;
        }
        let lower = ["script:", "local:", "private:", "variable:"].iter().find_map(|p| lower.strip_prefix(p)).unwrap_or(&lower).to_string();
        if let Some(env) = lower.strip_prefix("env:") {
            let text = value.to_string();
            set_env(&name[name.len() - env.len()..], if value.is_null() || text.is_empty() { None } else { Some(&text) });
            return;
        }
        if lower == "lastexitcode" {
            self.last_exit = value.int().unwrap_or(0) as i32;
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(lower, value);
        }
    }

    pub fn remove(&mut self, name: &str) {
        let lower = name.to_ascii_lowercase();
        for scope in self.scopes.iter_mut().rev() {
            if scope.remove(&lower).is_some() {
                return;
            }
        }
    }

    pub fn variables(&self) -> Vec<(String, Value)> {
        let mut all: BTreeMap<String, Value> = BTreeMap::new();
        for scope in &self.scopes {
            for (k, v) in scope {
                all.insert(k.clone(), v.clone());
            }
        }
        all.into_iter().collect()
    }

    // ---- Statements ----

    pub fn run_block(&mut self, block: &Block) -> Out {
        let mut out = Vec::new();
        for stmt in &block.stmts {
            out.extend(self.stmt(stmt)?);
        }
        Ok(out)
    }

    /// A script block, called: in a scope of its own (unless `dot`), with `$args` and its `param()`s bound.
    pub fn call(&mut self, block: &Block, args: Vec<Value>, named: Vec<(String, Value)>, input: Option<Value>, dot: bool) -> Out {
        if !dot {
            self.scopes.push(BTreeMap::new());
        }
        let mut positional = args.into_iter();
        let mut rest = Vec::new();
        for (name, default) in &block.params {
            let given = named.iter().find(|(n, _)| n.eq_ignore_ascii_case(name) || name.to_ascii_lowercase().starts_with(&n.to_ascii_lowercase())).map(|(_, v)| v.clone());
            let value = match given {
                Some(v) => v,
                None => match positional.next() {
                    Some(v) => v,
                    None => match default {
                        Some(e) => match self.eval(e) {
                            Ok(v) => v,
                            Err(f) => {
                                if !dot {
                                    self.scopes.pop();
                                }
                                return Err(f);
                            }
                        },
                        None => Value::Null,
                    },
                },
            };
            self.set(name, value);
        }
        rest.extend(positional);
        self.set("args", Value::array(rest));
        if let Some(input) = input {
            self.set("_", input.clone());
            self.set("input", input);
        }
        let result = self.run_block(block);
        if !dot {
            self.scopes.pop();
        }
        match result {
            Err(Flow::Return(out)) => Ok(out),
            other => other,
        }
    }

    pub fn stmt(&mut self, stmt: &Stmt) -> Out {
        match stmt {
            Stmt::Pipeline(p) => self.pipeline(p),
            Stmt::If(arms, other) => {
                for (cond, body) in arms {
                    if self.condition(cond)? {
                        return self.run_block(body);
                    }
                }
                match other {
                    Some(body) => self.run_block(body),
                    None => Ok(Vec::new()),
                }
            }
            Stmt::While(cond, body) => {
                let mut out = Vec::new();
                while self.condition(cond)? {
                    match self.run_block(body) {
                        Ok(o) => out.extend(o),
                        Err(Flow::Break) => break,
                        Err(Flow::Continue) => continue,
                        Err(f) => return Err(f),
                    }
                }
                Ok(out)
            }
            Stmt::Do(body, cond, until) => {
                let mut out = Vec::new();
                loop {
                    match self.run_block(body) {
                        Ok(o) => out.extend(o),
                        Err(Flow::Break) => break,
                        Err(Flow::Continue) => {}
                        Err(f) => return Err(f),
                    }
                    if self.condition(cond)? == *until {
                        break;
                    }
                }
                Ok(out)
            }
            Stmt::For(init, cond, step, body) => {
                let mut out = Vec::new();
                if let Some(init) = init {
                    out.extend(self.pipeline(init)?);
                }
                loop {
                    if let Some(cond) = cond
                        && !self.condition(cond)?
                    {
                        break;
                    }
                    match self.run_block(body) {
                        Ok(o) => out.extend(o),
                        Err(Flow::Break) => break,
                        Err(Flow::Continue) => {}
                        Err(f) => return Err(f),
                    }
                    if let Some(step) = step {
                        self.pipeline(step)?;
                    }
                }
                Ok(out)
            }
            Stmt::Foreach(var, collection, body) => {
                self.capturing += 1;
                let items = self.pipeline(collection);
                self.capturing -= 1;
                let mut out = Vec::new();
                for item in items? {
                    self.set(var, item);
                    match self.run_block(body) {
                        Ok(o) => out.extend(o),
                        Err(Flow::Break) => break,
                        Err(Flow::Continue) => continue,
                        Err(f) => return Err(f),
                    }
                }
                Ok(out)
            }
            Stmt::Switch(value, cases) => {
                self.capturing += 1;
                let items = self.pipeline(value);
                self.capturing -= 1;
                let mut out = Vec::new();
                'items: for item in items? {
                    let saved = self.get("_");
                    self.set("_", item.clone());
                    let mut matched = false;
                    for (test, body) in cases {
                        let hit = match test {
                            None => !matched,
                            Some(Expr::Block(b)) => {
                                let r = self.call(b, Vec::new(), Vec::new(), None, true)?;
                                Value::from_items(r).truthy()
                            }
                            Some(e) => {
                                let t = self.eval(e)?;
                                match &t {
                                    Value::Str(s) if s.contains(['*', '?']) => like(&item.to_string(), s, false),
                                    _ => equal(&item, &t, false),
                                }
                            }
                        };
                        if hit {
                            matched = true;
                            match self.run_block(body) {
                                Ok(o) => out.extend(o),
                                Err(Flow::Break) => {
                                    self.set("_", saved);
                                    break 'items;
                                }
                                Err(Flow::Continue) => break,
                                Err(f) => return Err(f),
                            }
                        }
                    }
                    self.set("_", saved);
                }
                Ok(out)
            }
            Stmt::Function(name, body) => {
                self.functions.insert(name.to_ascii_lowercase(), body.clone());
                Ok(Vec::new())
            }
            Stmt::Return(p) => {
                let out = match p {
                    Some(p) => self.pipeline(p)?,
                    None => Vec::new(),
                };
                Err(Flow::Return(out))
            }
            Stmt::Break => Err(Flow::Break),
            Stmt::Continue => Err(Flow::Continue),
            Stmt::Exit(p) => {
                let code = match p {
                    Some(p) => Value::from_items(self.pipeline(p)?).int().unwrap_or(0) as i32,
                    None => self.last_exit,
                };
                Err(Flow::Exit(code))
            }
            Stmt::Throw(p) => {
                let v = match p {
                    Some(p) => Value::from_items(self.pipeline(p)?),
                    None => Value::str("ScriptHalted"),
                };
                Err(match v {
                    Value::Object(_) => Flow::Throw(v),
                    other => error(&other.to_string()),
                })
            }
            Stmt::Try(body, catch, finally) => {
                let result = self.run_block(body);
                let result = match (result, catch) {
                    (Err(Flow::Throw(e)), Some(catch)) => {
                        let saved = self.get("_");
                        self.set("_", e);
                        let r = self.run_block(catch);
                        self.set("_", saved);
                        r
                    }
                    (r, _) => r,
                };
                if let Some(finally) = finally {
                    self.run_block(finally)?;
                }
                result
            }
        }
    }

    fn condition(&mut self, p: &Pipeline) -> Result<bool, Flow> {
        self.capturing += 1;
        let r = self.pipeline(p);
        self.capturing -= 1;
        Ok(Value::from_items(r?).truthy())
    }

    // ---- Pipelines ----

    pub fn pipeline(&mut self, p: &Pipeline) -> Out {
        if let Some((target, op)) = &p.assign {
            self.capturing += 1;
            let r = self.elements(&p.elements);
            self.capturing -= 1;
            let value = Value::from_items(r?);
            let value = if op == "=" {
                value
            } else {
                let old = self.eval(target)?;
                self.binary(&op[..1], old, value)?
            };
            self.assign(target, value)?;
            return Ok(Vec::new());
        }
        self.elements(&p.elements)
    }

    fn elements(&mut self, elements: &[Element]) -> Out {
        let mut input: Option<Vec<Value>> = None;
        let count = elements.len();
        for (n, element) in elements.iter().enumerate() {
            let last = n + 1 == count;
            if !last {
                self.capturing += 1;
            }
            let result = match element {
                Element::Expr(e) => {
                    if input.is_some() {
                        Err(error("Expressions are only allowed as the first element of a pipeline."))
                    } else {
                        // A value as a statement is its items; `$x = 1, 2` keeps the array as one.
                        self.eval(e).map(|v| match e {
                            Expr::Paren(_) | Expr::Sub(_) | Expr::ArrayOf(_) | Expr::Array(_) | Expr::Var(_) | Expr::Binary(..) | Expr::Method(..) | Expr::Member(..) | Expr::Index(..) | Expr::Cast(..) => v.items(),
                            _ => vec![v],
                        })
                    }
                }
                Element::Command(c) => self.command(c, input.take()),
            };
            if !last {
                self.capturing -= 1;
            }
            input = Some(result?);
        }
        Ok(input.unwrap_or_default())
    }

    fn assign(&mut self, target: &Expr, value: Value) -> Result<(), Flow> {
        match target {
            Expr::Var(name) => {
                self.set(name, value);
                Ok(())
            }
            Expr::Cast(ty, inner) => {
                let v = cast(ty, value)?;
                self.assign(inner, v)
            }
            Expr::Member(obj, name, false) => {
                let o = self.eval(obj)?;
                match o {
                    Value::Hash(h) => {
                        let mut h = h.borrow_mut();
                        match h.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
                            Some(slot) => slot.1 = value,
                            None => h.push((name.clone(), value)),
                        }
                        Ok(())
                    }
                    _ => Err(error(&format!("The property '{name}' cannot be found on this object."))),
                }
            }
            Expr::Index(obj, index) => {
                let o = self.eval(obj)?;
                let i = self.eval(index)?;
                match o {
                    Value::Hash(h) => {
                        let key = i.to_string();
                        let mut h = h.borrow_mut();
                        match h.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&key)) {
                            Some(slot) => slot.1 = value,
                            None => h.push((key, value)),
                        }
                        Ok(())
                    }
                    Value::Array(a) => {
                        let mut items = a.as_ref().clone();
                        let len = items.len() as i64;
                        let at = i.int().unwrap_or(0);
                        let at = if at < 0 { len + at } else { at };
                        if at < 0 || at >= len {
                            return Err(error("Index was outside the bounds of the array."));
                        }
                        items[at as usize] = value;
                        self.assign(obj, Value::array(items))
                    }
                    _ => Err(error("Cannot index into a null array.")),
                }
            }
            _ => Err(error("The assignment expression is not valid.")),
        }
    }

    // ---- Expressions ----

    pub fn eval(&mut self, e: &Expr) -> Result<Value, Flow> {
        Ok(match e {
            Expr::Value(v) => v.clone(),
            Expr::Template(parts) => {
                let mut s = String::new();
                for part in parts {
                    match part {
                        Part::Lit(l) => s.push_str(l),
                        Part::Expr(e) => s.push_str(&self.eval(e)?.to_string()),
                    }
                }
                Value::Str(s)
            }
            Expr::Var(name) => self.get(name),
            Expr::Array(items) => {
                let mut out = Vec::new();
                for i in items {
                    out.push(self.eval(i)?);
                }
                Value::array(out)
            }
            Expr::ArrayOf(stmts) => {
                self.capturing += 1;
                let r = self.stmts(stmts);
                self.capturing -= 1;
                Value::array(r?)
            }
            Expr::Sub(stmts) => {
                self.capturing += 1;
                let r = self.stmts(stmts);
                self.capturing -= 1;
                Value::from_items(r?)
            }
            Expr::Paren(p) => {
                self.capturing += 1;
                let r = self.pipeline(p);
                self.capturing -= 1;
                Value::from_items(r?)
            }
            Expr::Hash(pairs) => {
                let mut h = Vec::new();
                for (k, v) in pairs {
                    let key = self.eval(k)?.to_string();
                    let value = self.eval(v)?;
                    h.retain(|(x, _): &(String, Value)| !x.eq_ignore_ascii_case(&key));
                    h.push((key, value));
                }
                Value::hash(h)
            }
            Expr::Block(b) => Value::Block(b.clone()),
            Expr::Unary(op, inner) => {
                let v = self.eval(inner)?;
                match op.as_str() {
                    "-" => match v {
                        Value::Int(n) => Value::Int(-n),
                        other => Value::Double(-other.number().ok_or_else(|| error("Cannot negate that."))?),
                    },
                    "+" => v,
                    "not" => Value::Bool(!v.truthy()),
                    "bnot" => Value::Int(!v.int().unwrap_or(0)),
                    "split" => Value::array(v.to_string().split_whitespace().map(Value::str).collect()),
                    "join" => Value::str(v.items().iter().map(|i| i.to_string()).collect::<Vec<_>>().join("")),
                    _ => v,
                }
            }
            Expr::Binary(op, a, b) => {
                let lower = op.to_ascii_lowercase();
                match lower.as_str() {
                    "and" => return Ok(Value::Bool(self.eval(a)?.truthy() && self.eval(b)?.truthy())),
                    "or" => return Ok(Value::Bool(self.eval(a)?.truthy() || self.eval(b)?.truthy())),
                    "is" | "isnot" => {
                        let v = self.eval(a)?;
                        let ty = match &**b {
                            Expr::Type(t) => t.clone(),
                            other => self.eval(other)?.to_string(),
                        };
                        let is = is_type(&v, &ty);
                        return Ok(Value::Bool(if lower == "is" { is } else { !is }));
                    }
                    "as" => {
                        let v = self.eval(a)?;
                        let ty = match &**b {
                            Expr::Type(t) => t.clone(),
                            other => self.eval(other)?.to_string(),
                        };
                        return Ok(cast(&ty, v).unwrap_or(Value::Null));
                    }
                    _ => {}
                }
                let (x, y) = (self.eval(a)?, self.eval(b)?);
                self.binary(&lower, x, y)?
            }
            Expr::Member(target, name, is_static) => {
                if *is_static {
                    let ty = match &**target {
                        Expr::Type(t) => t.clone(),
                        other => self.eval(other)?.to_string(),
                    };
                    return static_member(&ty, name);
                }
                let v = self.eval(target)?;
                member(&v, name)?
            }
            Expr::Method(target, name, args, is_static) => {
                let mut values = Vec::new();
                for a in args {
                    values.push(self.eval(a)?);
                }
                if *is_static {
                    let ty = match &**target {
                        Expr::Type(t) => t.clone(),
                        other => self.eval(other)?.to_string(),
                    };
                    return self.static_method(&ty, name, values);
                }
                let v = self.eval(target)?;
                self.method(v, name, values)?
            }
            Expr::Index(target, index) => {
                let v = self.eval(target)?;
                let i = self.eval(index)?;
                index_into(&v, &i)
            }
            Expr::Type(t) => Value::str(t.clone()),
            Expr::Cast(ty, inner) => {
                let v = self.eval(inner)?;
                cast(ty, v)?
            }
            Expr::Step(target, by, before) => {
                let old = self.eval(target)?;
                let new = self.binary("+", if old.is_null() { Value::Int(0) } else { old.clone() }, Value::Int(*by))?;
                self.assign(target, new.clone())?;
                if *before { new } else { old }
            }
        })
    }

    fn stmts(&mut self, stmts: &[Stmt]) -> Out {
        let mut out = Vec::new();
        for s in stmts {
            out.extend(self.stmt(s)?);
        }
        Ok(out)
    }

    pub fn binary(&mut self, op: &str, x: Value, y: Value) -> Result<Value, Flow> {
        // `-ceq` and the like are case sensitive; `-ieq` and plain ones aren't.
        let (case, base) = if op.len() > 1 && (op.starts_with('c') || op.starts_with('i')) && !["contains", "in", "is", "isnot"].contains(&op) && !op.starts_with("in") {
            (op.starts_with('c'), &op[1..])
        } else if op.starts_with("cnot") || op.starts_with("ccontains") || op.starts_with("cin") {
            (true, &op[1..])
        } else {
            (false, op)
        };
        let base = if ["notcontains", "notin", "notlike", "notmatch", "contains", "in", "like", "match"].contains(&op) { op } else { base };
        // An array on the left filters, for the comparisons.
        if let Value::Array(items) = &x
            && ["eq", "ne", "gt", "ge", "lt", "le", "like", "notlike", "match", "notmatch"].contains(&base)
        {
            let mut out = Vec::new();
            for item in items.iter() {
                if self.binary(op, item.clone(), y.clone())?.truthy() {
                    out.push(item.clone());
                }
            }
            return Ok(Value::array(out));
        }
        let cmp = |o: core::cmp::Ordering| compare(&x, &y, case).is_some_and(|c| c == o);
        Ok(match base {
            "+" => add(x, y)?,
            "-" | "*" | "/" | "%" => arith(base, x, y)?,
            "eq" => Value::Bool(equal(&x, &y, case)),
            "ne" => Value::Bool(!equal(&x, &y, case)),
            "gt" => Value::Bool(cmp(core::cmp::Ordering::Greater)),
            "lt" => Value::Bool(cmp(core::cmp::Ordering::Less)),
            "ge" => Value::Bool(!cmp(core::cmp::Ordering::Less)),
            "le" => Value::Bool(!cmp(core::cmp::Ordering::Greater)),
            "like" => Value::Bool(like(&x.to_string(), &y.to_string(), case)),
            "notlike" => Value::Bool(!like(&x.to_string(), &y.to_string(), case)),
            "match" | "notmatch" => {
                let re = Regex::new(&y.to_string(), !case).map_err(|e| error(&e))?;
                let text: Vec<char> = x.to_string().chars().collect();
                let found = re.find_at(&text, 0);
                if let Some(caps) = &found {
                    let mut m = Vec::new();
                    for (n, c) in caps.iter().enumerate() {
                        if let Some((a, b)) = c {
                            let name = re.names.iter().find(|(_, i)| *i == n).map(|(k, _)| k.clone()).unwrap_or_else(|| format!("{n}"));
                            m.push((name, Value::str(text[*a..*b].iter().collect::<String>())));
                        }
                    }
                    self.set("matches", Value::hash(m));
                }
                Value::Bool(found.is_some() == (base == "match"))
            }
            "contains" | "notcontains" => {
                let hit = x.items().iter().any(|i| equal(i, &y, case));
                Value::Bool(hit == (base == "contains"))
            }
            "in" | "notin" => {
                let hit = y.items().iter().any(|i| equal(i, &x, case));
                Value::Bool(hit == (base == "in"))
            }
            "replace" => {
                let parts = y.items();
                let (pattern, with) = (parts.first().map(|p| p.to_string()).unwrap_or_default(), parts.get(1).map(|w| w.to_string()).unwrap_or_default());
                let re = Regex::new(&pattern, !case).map_err(|e| error(&e))?;
                let items: Vec<Value> = x.items().iter().map(|i| Value::str(regex::replace(&re, &i.to_string(), &with))).collect();
                if matches!(x, Value::Array(_)) { Value::array(items) } else { Value::from_items(items) }
            }
            "split" => {
                let parts = y.items();
                let pattern = parts.first().map(|p| p.to_string()).unwrap_or_else(|| String::from(" "));
                let re = Regex::new(&pattern, !case).map_err(|e| error(&e))?;
                let mut out = Vec::new();
                for i in x.items() {
                    out.extend(regex::split(&re, &i.to_string()).into_iter().map(Value::Str));
                }
                Value::array(out)
            }
            "join" => Value::str(x.items().iter().map(|i| i.to_string()).collect::<Vec<_>>().join(&y.to_string())),
            "f" => Value::str(format_operator(&x.to_string(), &y.items())),
            "xor" => Value::Bool(x.truthy() != y.truthy()),
            "band" => Value::Int(x.int().unwrap_or(0) & y.int().unwrap_or(0)),
            "bor" => Value::Int(x.int().unwrap_or(0) | y.int().unwrap_or(0)),
            "bxor" => Value::Int(x.int().unwrap_or(0) ^ y.int().unwrap_or(0)),
            "shl" => Value::Int(x.int().unwrap_or(0).wrapping_shl(y.int().unwrap_or(0) as u32)),
            "shr" => Value::Int(x.int().unwrap_or(0).wrapping_shr(y.int().unwrap_or(0) as u32)),
            ".." => {
                let (a, b) = (x.int().unwrap_or(0), y.int().unwrap_or(0));
                if (a - b).abs() > 10_000_000 {
                    return Err(error("The range is too big."));
                }
                Value::array(if a <= b { (a..=b).map(Value::Int).collect() } else { (b..=a).rev().map(Value::Int).collect() })
            }
            other => return Err(error(&format!("Unknown operator -{other}."))),
        })
    }

    fn method(&mut self, v: Value, name: &str, args: Vec<Value>) -> Result<Value, Flow> {
        let lower = name.to_ascii_lowercase();
        let arg = |n: usize| args.get(n).cloned().unwrap_or(Value::Null);
        let text = |n: usize| args.get(n).map(|a| a.to_string()).unwrap_or_default();
        if lower == "gettype" {
            let full = v.type_name();
            let short = full.rsplit('.').next().unwrap_or(&full).to_string();
            return Ok(Value::Object(Rc::new(value::Object { type_name: String::from("System.RuntimeType"), props: vec![(String::from("Name"), Value::str(short.clone())), (String::from("FullName"), Value::str(full.clone()))], text: Some(full) })));
        }
        if lower == "tostring" {
            if let (Some(fmt), Some(n)) = (args.first(), v.number()) {
                return Ok(Value::str(format_number(n, &fmt.to_string(), matches!(v, Value::Int(_)))));
            }
            return Ok(Value::str(v.to_string()));
        }
        if lower == "equals" {
            return Ok(Value::Bool(equal(&v, &arg(0), false)));
        }
        match &v {
            Value::Str(s) => {
                let s = s.clone();
                return Ok(match lower.as_str() {
                    "toupper" | "toupperinvariant" => Value::str(s.to_uppercase()),
                    "tolower" | "tolowerinvariant" => Value::str(s.to_lowercase()),
                    "trim" | "trimstart" | "trimend" => {
                        let chars: Vec<char> = if args.is_empty() { Vec::new() } else { args.iter().flat_map(|a| a.to_string().chars().collect::<Vec<_>>()).collect() };
                        let strip = |c: char| if chars.is_empty() { c.is_whitespace() } else { chars.contains(&c) };
                        Value::str(match lower.as_str() {
                            "trim" => s.trim_matches(strip),
                            "trimstart" => s.trim_start_matches(strip),
                            _ => s.trim_end_matches(strip),
                        })
                    }
                    "split" => {
                        let seps: Vec<char> = args.iter().flat_map(|a| a.to_string().chars().collect::<Vec<_>>()).collect();
                        let parts: Vec<Value> = if seps.is_empty() { s.split(char::is_whitespace).map(Value::str).collect() } else { s.split(|c| seps.contains(&c)).map(Value::str).collect() };
                        Value::array(parts)
                    }
                    "replace" => Value::str(s.replace(&text(0), &text(1))),
                    "contains" => Value::Bool(s.contains(&text(0))),
                    "startswith" => Value::Bool(s.starts_with(&text(0))),
                    "endswith" => Value::Bool(s.ends_with(&text(0))),
                    "indexof" => Value::Int(s.find(&text(0)).map_or(-1, |i| s[..i].chars().count() as i64)),
                    "lastindexof" => Value::Int(s.rfind(&text(0)).map_or(-1, |i| s[..i].chars().count() as i64)),
                    "substring" => {
                        let chars: Vec<char> = s.chars().collect();
                        let start = (arg(0).int().unwrap_or(0).max(0) as usize).min(chars.len());
                        let len = args.get(1).and_then(Value::int).map_or(chars.len() - start, |n| (n.max(0) as usize).min(chars.len() - start));
                        Value::str(chars[start..start + len].iter().collect::<String>())
                    }
                    "padleft" | "padright" => {
                        let width = arg(0).int().unwrap_or(0).max(0) as usize;
                        let fill = text(1).chars().next().unwrap_or(' ');
                        let n = width.saturating_sub(s.chars().count());
                        let pad: String = core::iter::repeat_n(fill, n).collect();
                        Value::str(if lower == "padleft" { format!("{pad}{s}") } else { format!("{s}{pad}") })
                    }
                    "insert" => {
                        let mut chars: Vec<char> = s.chars().collect();
                        let at = (arg(0).int().unwrap_or(0).max(0) as usize).min(chars.len());
                        for (i, c) in text(1).chars().enumerate() {
                            chars.insert(at + i, c);
                        }
                        Value::str(chars.into_iter().collect::<String>())
                    }
                    "remove" => {
                        let chars: Vec<char> = s.chars().collect();
                        let at = (arg(0).int().unwrap_or(0).max(0) as usize).min(chars.len());
                        let n = args.get(1).and_then(Value::int).map_or(chars.len() - at, |n| n.max(0) as usize).min(chars.len() - at);
                        Value::str(chars[..at].iter().chain(&chars[at + n..]).collect::<String>())
                    }
                    "tochararray" => Value::array(s.chars().map(|c| Value::str(c.to_string())).collect()),
                    "compareto" => Value::Int(match compare(&v, &arg(0), false) {
                        Some(core::cmp::Ordering::Less) => -1,
                        Some(core::cmp::Ordering::Greater) => 1,
                        _ => 0,
                    }),
                    "normalize" | "clone" => v.clone(),
                    _ => return Err(error(&format!("Method invocation failed because [System.String] does not contain a method named '{name}'."))),
                });
            }
            Value::Array(items) => {
                return Ok(match lower.as_str() {
                    "contains" => Value::Bool(items.iter().any(|i| equal(i, &arg(0), true))),
                    "indexof" => Value::Int(items.iter().position(|i| equal(i, &arg(0), true)).map_or(-1, |p| p as i64)),
                    "clone" => Value::array(items.as_ref().clone()),
                    "foreach" => {
                        let mut out = Vec::new();
                        if let Value::Block(b) = arg(0) {
                            for item in items.iter() {
                                out.extend(self.call(&b, Vec::new(), Vec::new(), Some(item.clone()), true)?);
                            }
                        }
                        Value::array(out)
                    }
                    "where" => {
                        let mut out = Vec::new();
                        if let Value::Block(b) = arg(0) {
                            for item in items.iter() {
                                if Value::from_items(self.call(&b, Vec::new(), Vec::new(), Some(item.clone()), true)?).truthy() {
                                    out.push(item.clone());
                                }
                            }
                        }
                        Value::array(out)
                    }
                    _ => return Err(error(&format!("Method invocation failed because [System.Object[]] does not contain a method named '{name}'."))),
                });
            }
            Value::Hash(h) => {
                return Ok(match lower.as_str() {
                    "containskey" | "contains" => Value::Bool(h.borrow().iter().any(|(k, _)| k.eq_ignore_ascii_case(&text(0)))),
                    "containsvalue" => Value::Bool(h.borrow().iter().any(|(_, x)| equal(x, &arg(0), false))),
                    "add" => {
                        let key = text(0);
                        if h.borrow().iter().any(|(k, _)| k.eq_ignore_ascii_case(&key)) {
                            return Err(error(&format!("Item has already been added. Key in dictionary: '{key}'")));
                        }
                        h.borrow_mut().push((key, arg(1)));
                        return Ok(Value::Null);
                    }
                    "remove" => {
                        let key = text(0);
                        h.borrow_mut().retain(|(k, _)| !k.eq_ignore_ascii_case(&key));
                        return Ok(Value::Null);
                    }
                    "clear" => {
                        h.borrow_mut().clear();
                        return Ok(Value::Null);
                    }
                    "clone" => Value::hash(h.borrow().clone()),
                    "getenumerator" => Value::array(h.borrow().iter().map(|(k, x)| Value::object("System.Collections.DictionaryEntry", vec![("Name", Value::str(k.clone())), ("Key", Value::str(k.clone())), ("Value", x.clone())])).collect()),
                    _ => return Err(error(&format!("Method invocation failed because [System.Collections.Hashtable] does not contain a method named '{name}'."))),
                });
            }
            Value::Block(b) if lower == "invoke" || lower == "invokereturnasis" => {
                let b = b.clone();
                return Ok(Value::from_items(self.call(&b, args, Vec::new(), None, false)?));
            }
            _ => {}
        }
        if let Some(n) = v.number() {
            return Ok(match lower.as_str() {
                "compareto" => Value::Int(match compare(&v, &arg(0), false) {
                    Some(core::cmp::Ordering::Less) => -1,
                    Some(core::cmp::Ordering::Greater) => 1,
                    _ => 0,
                }),
                _ => return Err(error(&format!("Method invocation failed because [{}] does not contain a method named '{name}'.", v.type_name()))),
            }
            .clone())
            .map(|r| if n.is_nan() { Value::Null } else { r });
        }
        Err(error(&format!("Method invocation failed because [{}] does not contain a method named '{name}'.", v.type_name())))
    }

    fn static_method(&mut self, ty: &str, name: &str, args: Vec<Value>) -> Result<Value, Flow> {
        let ty = ty.to_ascii_lowercase();
        let ty = ty.strip_prefix("system.").unwrap_or(&ty);
        let lower = name.to_ascii_lowercase();
        let num = |n: usize| args.get(n).and_then(Value::number).unwrap_or(0.0);
        let text = |n: usize| args.get(n).map(|a| a.to_string()).unwrap_or_default();
        Ok(match (ty, lower.as_str()) {
            ("math", "round") => {
                let digits = args.get(1).and_then(Value::int).unwrap_or(0).clamp(0, 15);
                let scale = (0..digits).fold(1.0, |s, _| s * 10.0);
                let r = round_even(num(0) * scale) / scale;
                if digits == 0 && matches!(args.first(), Some(Value::Int(_))) { Value::Int(r as i64) } else { Value::Double(r) }
            }
            ("math", "floor") => Value::Double(floor(num(0))),
            ("math", "ceiling") => Value::Double(-floor(-num(0))),
            ("math", "truncate") => Value::Double(num(0) as i64 as f64),
            ("math", "abs") => match args.first() {
                Some(Value::Int(n)) => Value::Int(n.abs()),
                _ => Value::Double(num(0).abs()),
            },
            ("math", "max") | ("math", "min") => {
                let (a, b) = (args.first().cloned().unwrap_or(Value::Null), args.get(1).cloned().unwrap_or(Value::Null));
                let a_bigger = compare(&a, &b, false) == Some(core::cmp::Ordering::Greater);
                if (lower == "max") == a_bigger { a } else { b }
            }
            ("math", "pow") => Value::Double(pow(num(0), num(1))),
            ("math", "sqrt") => Value::Double(sqrt(num(0))),
            ("string", "join") => Value::str(args.iter().skip(1).flat_map(Value::items).map(|i| i.to_string()).collect::<Vec<_>>().join(&text(0))),
            ("string", "isnullorempty") => Value::Bool(args.first().is_none_or(|a| a.to_string().is_empty())),
            ("string", "isnullorwhitespace") => Value::Bool(args.first().is_none_or(|a| a.to_string().trim().is_empty())),
            ("string", "format") => Value::str(format_operator(&text(0), &args[1.min(args.len())..].iter().flat_map(Value::items).collect::<Vec<_>>())),
            ("string", "concat") => Value::str(args.iter().map(|a| a.to_string()).collect::<String>()),
            ("io.path", "combine") => {
                let mut path = String::new();
                for a in &args {
                    let part = a.to_string();
                    if part.starts_with('\\') || part.contains(':') || path.is_empty() {
                        path = part;
                    } else {
                        if !path.ends_with('\\') {
                            path.push('\\');
                        }
                        path.push_str(&part);
                    }
                }
                Value::str(path)
            }
            ("io.path", "getfilename") => Value::str(text(0).rsplit(['\\', '/']).next().unwrap_or("").to_string()),
            ("io.path", "getdirectoryname") => Value::str(text(0).rsplit_once(['\\', '/']).map_or(String::new(), |(d, _)| d.to_string())),
            ("io.path", "getextension") => {
                let f = text(0);
                let name = f.rsplit(['\\', '/']).next().unwrap_or("").to_string();
                Value::str(name.rfind('.').map_or(String::new(), |i| name[i..].to_string()))
            }
            ("io.path", "getfilenamewithoutextension") => {
                let f = text(0);
                let name = f.rsplit(['\\', '/']).next().unwrap_or("").to_string();
                Value::str(name.rfind('.').map_or(name.clone(), |i| name[..i].to_string()))
            }
            ("io.path", "getfullpath") => Value::str(crate::cmdlets::full_path(&text(0))),
            ("io.path", "gettemppath") => Value::str(get_env("TEMP").map_or(String::from("C:\\Windows\\Temp\\"), |t| format!("{t}\\"))),
            ("io.file", "exists") => Value::Bool(crate::cmdlets::attributes(&text(0)).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY == 0)),
            ("io.directory", "exists") => Value::Bool(crate::cmdlets::attributes(&text(0)).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0)),
            ("io.directory", "getcurrentdirectory") => Value::str(current_dir()),
            ("io.file", "readalltext") => Value::str(String::from_utf8_lossy(&read_file(&text(0)).ok_or_else(|| error(&format!("Could not find file '{}'.", text(0))))?).into_owned()),
            ("io.file", "readalllines") => Value::array(lines(&read_file(&text(0)).ok_or_else(|| error(&format!("Could not find file '{}'.", text(0))))?).into_iter().map(Value::Str).collect()),
            ("io.file", "writealltext") => {
                crate::cmdlets::write_file(&text(0), text(1).as_bytes(), false).map_err(|e| error(&e))?;
                Value::Null
            }
            ("environment", "getenvironmentvariable") => get_env(&text(0)).map_or(Value::Null, Value::Str),
            ("environment", "setenvironmentvariable") => {
                let (name, value) = (text(0), text(1));
                set_env(&name, if value.is_empty() { None } else { Some(&value) });
                Value::Null
            }
            ("environment", "exit") => return Err(Flow::Exit(num(0) as i32)),
            ("regex", "escape") => Value::str(text(0).chars().flat_map(|c| if "\\*+?|{[()^$.# ".contains(c) { vec!['\\', c] } else { vec![c] }).collect::<String>()),
            ("regex", "ismatch") => Value::Bool(Regex::new(&text(1), false).map_err(|e| error(&e))?.is_match(&text(0))),
            ("regex", "replace") => Value::str(regex::replace(&Regex::new(&text(1), false).map_err(|e| error(&e))?, &text(0), &text(2))),
            ("regex", "split") => Value::array(regex::split(&Regex::new(&text(1), false).map_err(|e| error(&e))?, &text(0)).into_iter().map(Value::Str).collect()),
            ("int" | "int32" | "int64" | "long", "parse") => Value::Int(text(0).trim().parse().map_err(|_| error(&format!("Input string was not in a correct format: '{}'.", text(0))))?),
            ("double", "parse") => Value::Double(text(0).trim().parse().map_err(|_| error("Input string was not in a correct format."))?),
            ("convert", "toint32" | "toint64") => {
                let base = args.get(1).and_then(Value::int).unwrap_or(10) as u32;
                Value::Int(i64::from_str_radix(text(0).trim().trim_start_matches("0x"), base).map_err(|_| error("Could not convert."))?)
            }
            ("convert", "tostring") => {
                let n = args.first().and_then(Value::int).unwrap_or(0);
                Value::str(match args.get(1).and_then(Value::int) {
                    Some(2) => format!("{n:b}"),
                    Some(8) => format!("{n:o}"),
                    Some(16) => format!("{n:x}"),
                    _ => format!("{n}"),
                })
            }
            ("datetime", "now") => crate::cmdlets::now_value(),
            _ => return Err(error(&format!("Method invocation failed because [{ty}] does not contain a method named '{name}'."))),
        })
    }

    // ---- Commands ----

    /// A command's arguments, evaluated: `-Name` (with its value if it had one after `:`), and values.
    fn arguments(&mut self, args: &[Arg]) -> Result<Vec<(Option<String>, Option<Value>)>, Flow> {
        let mut out = Vec::new();
        for a in args {
            match a {
                Arg::Named(n, v) => {
                    let v = match v {
                        Some(e) => Some(self.eval(e)?),
                        None => None,
                    };
                    out.push((Some(n.clone()), v));
                }
                Arg::Value(e) => {
                    let v = self.eval(e)?;
                    out.push((None, Some(v)));
                }
            }
        }
        Ok(out)
    }

    fn command(&mut self, c: &Command, input: Option<Vec<Value>>) -> Out {
        let target = self.eval(&c.name)?;
        let args = self.arguments(&c.args)?;
        // Redirections: output to a file (or nowhere), or errors into output.
        let to_file = c.redirects.iter().find(|(fd, t, _)| (*fd == 1 || *fd == 9) && !t.starts_with('&')).cloned();
        if to_file.is_some() {
            self.capturing += 1;
        }
        let result = match &target {
            Value::Block(b) => {
                let (positional, named) = split_named(args);
                let b = b.clone();
                self.call(&b, positional, named, input.map(Value::from_items), false)
            }
            _ => {
                let name = target.to_string();
                self.invoke(&name, args, input, c.redirects.iter().any(|(fd, t, _)| *fd == 2 && t == "&1"))
            }
        };
        if let Some((_, path, append)) = to_file {
            self.capturing -= 1;
            let out = result?;
            if !path.eq_ignore_ascii_case("$null") && !path.eq_ignore_ascii_case("nul") {
                let text = crate::format::to_text(&out);
                crate::cmdlets::write_file(&path, text.as_bytes(), append).map_err(|e| error(&e))?;
            }
            return Ok(Vec::new());
        }
        result
    }

    /// Runs a command by name: a function, an alias's, a cmdlet, a script, or a program.
    pub fn invoke(&mut self, name: &str, args: Vec<(Option<String>, Option<Value>)>, input: Option<Vec<Value>>, merge_errors: bool) -> Out {
        let lower = name.to_ascii_lowercase();
        if let Some(f) = self.functions.get(&lower).cloned() {
            let (positional, named) = split_named(args);
            let mut out = Vec::new();
            // A function with input runs once, with `$input` (and `$_` as the last item).
            let input = input.map(Value::from_items);
            out.extend(self.call(&f, positional, named, input, false)?);
            return Ok(out);
        }
        let resolved = self.aliases.get(&lower).cloned().unwrap_or(lower.clone());
        if crate::cmdlets::exists(&resolved) {
            let result = crate::cmdlets::run(self, &resolved, args, input.unwrap_or_default());
            return match result {
                Ok(out) => {
                    self.ok = true;
                    Ok(out)
                }
                Err(Flow::Throw(e)) if !matches!(&e, Value::Object(o) if o.type_name == "terminating") => {
                    // A cmdlet's error doesn't stop the script: it's said, and `$?` is false.
                    let shown = crate::cmdlets::display_name(&resolved);
                    err(&format!("{shown} : {}\r\n", message(&e)));
                    self.ok = false;
                    Ok(Vec::new())
                }
                Err(f) => Err(f),
            };
        }
        // A script, or a program.
        match crate::cmdlets::find_program(name) {
            Some(path) if path.to_ascii_lowercase().ends_with(".ps1") => {
                let text = read_file(&path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
                let block = Parser::script(&text).map_err(|e| error(&e.message))?;
                let (positional, named) = split_named(args);
                self.call(&block, positional, named, input.map(Value::from_items), false)
            }
            Some(path) => {
                let words: Vec<String> = args
                    .into_iter()
                    .flat_map(|(n, v)| {
                        let mut w = Vec::new();
                        match (n, v) {
                            (Some(n), Some(v)) => w.push(format!("-{n}:{v}")),
                            (Some(n), None) => w.push(format!("-{n}")),
                            (None, Some(v)) => w.extend(v.items().iter().map(|i| i.to_string())),
                            (None, None) => {}
                        }
                        w
                    })
                    .collect();
                crate::cmdlets::run_program(self, &path, name, &words, input, merge_errors)
            }
            None => Err(Flow::Throw(Value::object(
                "terminating",
                vec![("Message", Value::str(format!("The term '{name}' is not recognized as the name of a cmdlet, function, script file, or operable program. Check the spelling of the name, or if a path was included, verify that the path is correct and try again.")))],
            ))),
        }
    }
}

/// Positional values, and named ones (a name with no value is `$true`, a switch).
pub fn split_named(args: Vec<(Option<String>, Option<Value>)>) -> (Vec<Value>, Vec<(String, Value)>) {
    let mut positional = Vec::new();
    let mut named = Vec::new();
    let mut it = args.into_iter().peekable();
    while let Some((n, v)) = it.next() {
        match (n, v) {
            (Some(n), Some(v)) => named.push((n, v)),
            (Some(n), None) => {
                // The value after it, if it's one, is its value.
                if let Some((None, Some(_))) = it.peek() {
                    let (_, v) = it.next().unwrap_or((None, None));
                    named.push((n, v.unwrap_or(Value::Bool(true))));
                } else {
                    named.push((n, Value::Bool(true)));
                }
            }
            (None, Some(v)) => positional.push(v),
            (None, None) => {}
        }
    }
    (positional, named)
}

fn add(x: Value, y: Value) -> Result<Value, Flow> {
    Ok(match (&x, &y) {
        (Value::Null, _) => match y {
            Value::Array(_) | Value::Str(_) | Value::Hash(_) => y,
            other => other,
        },
        (Value::Str(s), _) => Value::str(format!("{s}{y}")),
        (Value::Array(a), _) => {
            let mut items = a.as_ref().clone();
            items.extend(y.items());
            Value::array(items)
        }
        (Value::Hash(a), Value::Hash(b)) => {
            let mut h = a.borrow().clone();
            for (k, v) in b.borrow().iter() {
                if h.iter().any(|(x, _)| x.eq_ignore_ascii_case(k)) {
                    return Err(error(&format!("Item has already been added. Key in dictionary: '{k}'")));
                }
                h.push((k.clone(), v.clone()));
            }
            Value::hash(h)
        }
        (Value::Int(a), Value::Int(b)) => a.checked_add(*b).map_or(Value::Double(*a as f64 + *b as f64), Value::Int),
        _ => match (x.number(), y.number()) {
            (Some(a), Some(b)) if matches!(x, Value::Int(_)) && matches!(value::parse_number(&y.to_string()), Some(Value::Int(_))) => Value::Int((a + b) as i64),
            (Some(a), Some(b)) => Value::Double(a + b),
            _ => return Err(error(&format!("Cannot convert value \"{y}\" to type \"System.Int32\". Error: \"Input string was not in a correct format.\""))),
        },
    })
}

fn arith(op: &str, x: Value, y: Value) -> Result<Value, Flow> {
    if op == "*" {
        if let Value::Str(s) = &x {
            return Ok(Value::str(s.repeat(y.int().unwrap_or(0).max(0) as usize)));
        }
        if let Value::Array(a) = &x {
            let n = y.int().unwrap_or(0).max(0) as usize;
            let mut out = Vec::new();
            for _ in 0..n {
                out.extend(a.iter().cloned());
            }
            return Ok(Value::array(out));
        }
    }
    let (Some(a), Some(b)) = (x.number(), y.number()) else {
        return Err(error(&format!("Method invocation failed: the operator '{op}' doesn't take [{}] and [{}].", x.type_name(), y.type_name())));
    };
    let ints = matches!(x, Value::Int(_) | Value::Null | Value::Bool(_)) && matches!(y, Value::Int(_) | Value::Bool(_)) || (matches!(x, Value::Int(_)) && matches!(value::parse_number(&y.to_string()), Some(Value::Int(_))));
    Ok(match op {
        "-" if ints => Value::Int(a as i64 - b as i64),
        "*" if ints => (a as i64).checked_mul(b as i64).map_or(Value::Double(a * b), Value::Int),
        "/" => {
            if b == 0.0 {
                return Err(error("Attempted to divide by zero."));
            }
            if ints && (a as i64) % (b as i64) == 0 { Value::Int(a as i64 / b as i64) } else { Value::Double(a / b) }
        }
        "%" => {
            if b == 0.0 {
                return Err(error("Attempted to divide by zero."));
            }
            if ints { Value::Int(a as i64 % b as i64) } else { Value::Double(a - b * ((a / b) as i64) as f64) }
        }
        "-" => Value::Double(a - b),
        _ => Value::Double(a * b),
    })
}

fn pow(a: f64, b: f64) -> f64 {
    if b == (b as i64) as f64 && b.abs() < 1e6 {
        let mut r = 1.0;
        for _ in 0..(b.abs() as i64) {
            r *= a;
        }
        return if b < 0.0 { 1.0 / r } else { r };
    }
    // exp(b * ln a), by series: enough for a script's needs.
    exp(b * ln(a))
}

fn sqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return if x == 0.0 { 0.0 } else { f64::NAN };
    }
    let mut r = if x > 1.0 { x / 2.0 } else { 1.0 };
    for _ in 0..60 {
        r = 0.5 * (r + x / r);
    }
    r
}

fn ln(x: f64) -> f64 {
    if x <= 0.0 {
        return f64::NAN;
    }
    let (mut m, mut e) = (x, 0i32);
    while m > 2.0 {
        m /= 2.0;
        e += 1;
    }
    while m < 1.0 {
        m *= 2.0;
        e -= 1;
    }
    let y = (m - 1.0) / (m + 1.0);
    let (mut term, mut sum) = (y, 0.0);
    let mut n = 1.0;
    while n < 200.0 {
        sum += term / n;
        term *= y * y;
        n += 2.0;
    }
    2.0 * sum + f64::from(e) * core::f64::consts::LN_2
}

fn exp(x: f64) -> f64 {
    let (mut term, mut sum) = (1.0, 1.0);
    for n in 1..200 {
        term *= x / f64::from(n);
        sum += term;
    }
    sum
}

/// `[type]` as PowerShell converts to it.
pub fn cast(ty: &str, v: Value) -> Result<Value, Flow> {
    let lower = ty.to_ascii_lowercase();
    let lower = lower.strip_prefix("system.").unwrap_or(&lower);
    let bad = || error(&format!("Cannot convert value \"{v}\" to type \"System.{ty}\"."));
    Ok(match lower {
        "int" | "int32" | "int64" | "long" | "byte" | "int16" | "uint32" | "uint64" => match &v {
            Value::Str(s) if s.trim().is_empty() => Value::Int(0),
            _ => Value::Int(v.int().ok_or_else(bad)?),
        },
        "double" | "float" | "single" | "decimal" => Value::Double(v.number().ok_or_else(bad)?),
        "string" => if v.is_null() { Value::str("") } else { Value::str(v.to_string()) },
        "bool" | "boolean" => Value::Bool(v.truthy()),
        "char" => match &v {
            Value::Int(n) => Value::str(char::from_u32(*n as u32).map(String::from).unwrap_or_default()),
            other => Value::str(other.to_string().chars().next().map(String::from).unwrap_or_default()),
        },
        "array" | "object[]" | "string[]" | "int[]" => Value::array(v.items().into_iter().map(|i| if lower == "string[]" { Value::str(i.to_string()) } else { i }).collect()),
        "hashtable" | "ordered" => match v {
            Value::Hash(_) => v,
            _ => return Err(bad()),
        },
        "pscustomobject" | "psobject" => match &v {
            Value::Hash(h) => Value::Object(Rc::new(value::Object { type_name: String::from("System.Management.Automation.PSCustomObject"), props: h.borrow().clone(), text: None })),
            _ => v,
        },
        "void" => Value::Null,
        "regex" | "scriptblock" | "object" | "psobject[]" => v,
        "datetime" => v,
        _ => v,
    })
}

fn is_type(v: &Value, ty: &str) -> bool {
    let lower = ty.to_ascii_lowercase();
    let lower = lower.strip_prefix("system.").unwrap_or(&lower);
    match lower {
        "int" | "int32" => matches!(v, Value::Int(n) if i32::try_from(*n).is_ok()),
        "int64" | "long" => matches!(v, Value::Int(_)),
        "double" => matches!(v, Value::Double(_)),
        "string" => matches!(v, Value::Str(_)),
        "bool" | "boolean" => matches!(v, Value::Bool(_)),
        "array" | "object[]" => matches!(v, Value::Array(_)),
        "hashtable" | "collections.hashtable" => matches!(v, Value::Hash(_)),
        "scriptblock" | "management.automation.scriptblock" => matches!(v, Value::Block(_)),
        "object" | "psobject" => !v.is_null(),
        other => v.type_name().to_ascii_lowercase().ends_with(other),
    }
}

fn member(v: &Value, name: &str) -> Result<Value, Flow> {
    Ok(v.property(name))
}

fn static_member(ty: &str, name: &str) -> Result<Value, Flow> {
    let ty = ty.to_ascii_lowercase();
    let ty = ty.strip_prefix("system.").unwrap_or(&ty);
    Ok(match (ty, name.to_ascii_lowercase().as_str()) {
        ("math", "pi") => Value::Double(core::f64::consts::PI),
        ("math", "e") => Value::Double(core::f64::consts::E),
        ("int" | "int32", "maxvalue") => Value::Int(i64::from(i32::MAX)),
        ("int" | "int32", "minvalue") => Value::Int(i64::from(i32::MIN)),
        ("int64" | "long", "maxvalue") => Value::Int(i64::MAX),
        ("string", "empty") => Value::str(""),
        ("environment", "newline") => Value::str("\r\n"),
        ("environment", "machinename") => Value::str(computer_name()),
        ("environment", "username") => Value::str(user_name()),
        ("environment", "currentdirectory") => Value::str(current_dir()),
        ("environment", "is64bitprocess") | ("environment", "is64bitoperatingsystem") => Value::Bool(cfg!(target_pointer_width = "64")),
        ("environment", "osversion") => Value::str("Microsoft Windows NT 10.0.19045.0"),
        ("environment", "processorcount") => Value::Int(get_env("NUMBER_OF_PROCESSORS").and_then(|n| n.parse().ok()).unwrap_or(1)),
        ("io.path", "directoryseparatorchar") => Value::str("\\"),
        ("io.path", "pathseparator") => Value::str(";"),
        ("datetime", "now") => crate::cmdlets::now_value(),
        ("datetime", "today") => crate::cmdlets::now_value(),
        _ => Value::Null,
    })
}

fn index_into(v: &Value, i: &Value) -> Value {
    match (v, i) {
        (_, Value::Array(idx)) => Value::array(idx.iter().map(|x| index_into(v, x)).filter(|x| !x.is_null()).collect()),
        (Value::Array(a), _) => {
            let len = a.len() as i64;
            let n = i.int().unwrap_or(0);
            let n = if n < 0 { len + n } else { n };
            if n >= 0 && n < len { a[n as usize].clone() } else { Value::Null }
        }
        (Value::Str(s), _) => {
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len() as i64;
            let n = i.int().unwrap_or(0);
            let n = if n < 0 { len + n } else { n };
            if n >= 0 && n < len { Value::str(chars[n as usize].to_string()) } else { Value::Null }
        }
        (Value::Hash(_), _) => v.property(&i.to_string()),
        (Value::Null, _) => Value::Null,
        (other, _) if i.int() == Some(0) => other.clone(),
        _ => Value::Null,
    }
}

/// .NET's composite formatting, as `-f` does it: `{0}`, `{1,8}`, `{0:N2}`, `{0:D3}`, `{0:X}`, `{0:P1}`, `{{`.
pub fn format_operator(fmt: &str, args: &[Value]) -> String {
    let c: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '{' if c.get(i + 1) == Some(&'{') => {
                out.push('{');
                i += 2;
            }
            '}' if c.get(i + 1) == Some(&'}') => {
                out.push('}');
                i += 2;
            }
            '{' => {
                let Some(end) = c[i..].iter().position(|&x| x == '}') else {
                    out.push('{');
                    i += 1;
                    continue;
                };
                let spec: String = c[i + 1..i + end].iter().collect();
                let (head, format) = match spec.split_once(':') {
                    Some((h, f)) => (h.to_owned(), Some(f.to_owned())),
                    None => (spec.clone(), None),
                };
                let (index, width) = match head.split_once(',') {
                    Some((n, w)) => (n.trim().parse::<usize>().unwrap_or(0), w.trim().parse::<i64>().unwrap_or(0)),
                    None => (head.trim().parse::<usize>().unwrap_or(0), 0),
                };
                let value = args.get(index).cloned().unwrap_or(Value::Null);
                let text = match (&format, value.number()) {
                    (Some(f), Some(n)) if !matches!(value, Value::Str(_)) => format_number(n, f, matches!(value, Value::Int(_))),
                    _ => value.to_string(),
                };
                let n = text.chars().count() as i64;
                if width > n {
                    out.push_str(&" ".repeat((width - n) as usize));
                    out.push_str(&text);
                } else if -width > n {
                    out.push_str(&text);
                    out.push_str(&" ".repeat((-width - n) as usize));
                } else {
                    out.push_str(&text);
                }
                i += end + 1;
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// A number in a .NET standard format: `N2`, `F1`, `D4`, `X`, `P0`, `C`, or a custom one of `0`, `#`, `.` and `,`.
pub fn format_number(n: f64, f: &str, integer: bool) -> String {
    let mut chars = f.chars();
    let kind = chars.next().unwrap_or('G').to_ascii_uppercase();
    let digits: Option<usize> = chars.as_str().parse().ok();
    let fixed = |d: usize, group: bool| -> String {
        let scale = (0..d).fold(1.0, |s, _| s * 10.0);
        let r = round_even(n.abs() * scale) / scale;
        let whole = r as u64;
        let frac = r - whole as f64;
        let mut s = if group { grouped(whole) } else { format!("{whole}") };
        if d > 0 {
            let f = round_even(frac * scale) as u64;
            s.push('.');
            s.push_str(&format!("{f:0>d$}"));
        }
        if n < 0.0 && r != 0.0 { format!("-{s}") } else { s }
    };
    match kind {
        'N' if f.len() <= 3 => fixed(digits.unwrap_or(2), true),
        'F' if f.len() <= 3 => fixed(digits.unwrap_or(2), false),
        'C' if f.len() <= 3 => format!("${}", fixed(digits.unwrap_or(2), true)),
        'P' if f.len() <= 3 => format!("{} %", fixed_of(n * 100.0, digits.unwrap_or(2))),
        'D' if integer && f.len() <= 3 => {
            let s = format!("{}", (n as i64).abs());
            let w = digits.unwrap_or(0);
            format!("{}{s:0>w$}", if n < 0.0 { "-" } else { "" })
        }
        'X' if integer && f.len() <= 3 => {
            let w = digits.unwrap_or(0);
            let s = if f.starts_with('x') { format!("{:x}", n as i64) } else { format!("{:X}", n as i64) };
            format!("{s:0>w$}")
        }
        'G' if f.len() <= 3 => format_double(n),
        _ => {
            // A custom pattern: zeros and hashes before and after a point, commas for grouping.
            let (int_part, frac_part) = f.split_once('.').unwrap_or((f, ""));
            let decimals = frac_part.chars().filter(|c| *c == '0' || *c == '#').count();
            let min_decimals = frac_part.chars().filter(|c| *c == '0').count();
            let min_int = int_part.chars().filter(|c| *c == '0').count();
            let group = int_part.contains(',');
            let mut s = fixed(decimals, group);
            if let Some(dot) = s.find('.') {
                while s.len() > dot + 1 + min_decimals && s.ends_with('0') {
                    s.pop();
                }
                if s.ends_with('.') {
                    s.pop();
                }
            }
            let int_len = s.find('.').unwrap_or(s.len()) - usize::from(s.starts_with('-'));
            if int_len < min_int {
                let pad = "0".repeat(min_int - int_len);
                s = if let Some(rest) = s.strip_prefix('-') { format!("-{pad}{rest}") } else { format!("{pad}{s}") };
            }
            s
        }
    }
}

fn fixed_of(n: f64, d: usize) -> String {
    format_number(n, &format!("N{d}"), false)
}

#[allow(dead_code)]
fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}

#[allow(dead_code)]
fn cell<T>(t: T) -> Rc<RefCell<T>> {
    Rc::new(RefCell::new(t))
}
