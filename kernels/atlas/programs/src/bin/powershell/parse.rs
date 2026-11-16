//! PowerShell's grammar, enough of it: statements (`if`/`elseif`/`else`, `while`, `do`, `for`, `foreach`, `switch`,
//! `function`, `try`/`catch`/`finally`, `return`, `break`, `continue`, `exit`, `throw`), pipelines of commands and
//! expressions with assignment, commands in argument mode (`-Name value`, `-Name:value`, barewords, `&` and `.`), and
//! expressions with PowerShell's operators by its precedence, strings with `$var` and `$(...)` in them, here-strings,
//! arrays, hashtables, script blocks, casts and static members (`[math]::Round`).

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::value::{Value, parse_number};

pub struct Block {
    pub params: Vec<(String, Option<Expr>)>,
    pub stmts: Vec<Stmt>,
    /// As written, for showing it.
    pub text: String,
}

pub enum Stmt {
    Pipeline(Pipeline),
    If(Vec<(Pipeline, Block)>, Option<Block>),
    While(Pipeline, Block),
    /// `do { } while (...)`, or `until` if true.
    Do(Block, Pipeline, bool),
    For(Option<Pipeline>, Option<Pipeline>, Option<Pipeline>, Block),
    Foreach(String, Pipeline, Block),
    /// Each case: a value to match (None for `default`), and its block.
    Switch(Pipeline, Vec<(Option<Expr>, Block)>),
    Function(String, Rc<Block>),
    Return(Option<Pipeline>),
    Break,
    Continue,
    Exit(Option<Pipeline>),
    Throw(Option<Pipeline>),
    Try(Block, Option<Block>, Option<Block>),
}

pub struct Pipeline {
    /// `$x = ...`, `$x += ...`: what's assigned to, and how.
    pub assign: Option<(Expr, String)>,
    pub elements: Vec<Element>,
}

pub enum Element {
    Expr(Expr),
    Command(Command),
}

pub struct Command {
    pub name: Expr,
    pub args: Vec<Arg>,
    /// Run with `&` (or `.`).
    #[allow(dead_code)]
    pub call: bool,
    /// `>`, `>>`, `2>`, `2>&1`: the stream, the file (or `&1`), and whether to append.
    pub redirects: Vec<(u8, String, bool)>,
}

pub enum Arg {
    /// `-Name`, or `-Name:value`.
    Named(String, Option<Expr>),
    Value(Expr),
}

pub enum Part {
    Lit(String),
    Expr(Expr),
}

pub enum Expr {
    Value(Value),
    Template(Vec<Part>),
    Var(String),
    Array(Vec<Expr>),
    /// `@( ... )`: always an array.
    ArrayOf(Vec<Stmt>),
    Hash(Vec<(Expr, Expr)>),
    /// `$( ... )`.
    Sub(Vec<Stmt>),
    Paren(Box<Pipeline>),
    Block(Rc<Block>),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    /// `a.b`, or `[type]::b` if static.
    Member(Box<Expr>, String, bool),
    Method(Box<Expr>, String, Vec<Expr>, bool),
    Index(Box<Expr>, Box<Expr>),
    Type(String),
    Cast(String, Box<Expr>),
    /// `$x++` (after) or `++$x` (before), by this much.
    Step(Box<Expr>, i64, bool),
}

pub struct Parser {
    c: Vec<char>,
    i: usize,
}

/// An error, and whether more input would finish what's there (a block or string left open).
pub struct Error {
    pub message: String,
    pub incomplete: bool,
}

type R<T> = Result<T, Error>;

fn err<T>(message: &str) -> R<T> {
    Err(Error { message: message.to_string(), incomplete: false })
}

fn incomplete<T>(message: &str) -> R<T> {
    Err(Error { message: message.to_string(), incomplete: true })
}

const OPERATORS: &[&str] = &[
    "eq", "ne", "gt", "ge", "lt", "le", "like", "notlike", "match", "notmatch", "contains", "notcontains", "in", "notin", "replace", "split", "join", "is", "isnot", "as", "and", "or", "xor", "not", "band", "bor", "bxor", "bnot", "f", "shl", "shr",
];

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Parser {
    pub fn script(text: &str) -> R<Block> {
        let mut p = Parser { c: text.chars().collect(), i: 0 };
        let block = p.block_body(None)?;
        if p.i < p.c.len() {
            return err(&alloc::format!("Unexpected token '{}' in expression or statement.", p.c[p.i]));
        }
        Ok(block)
    }

    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn at(&self, n: usize) -> Option<char> {
        self.c.get(self.i + n).copied()
    }

    fn starts(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(n, ch)| self.at(n).is_some_and(|c| c.eq_ignore_ascii_case(&ch)))
    }

    /// Spaces, comments and backtick line continuations; newlines too if `lines`.
    fn skip(&mut self, lines: bool) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\r') => self.i += 1,
                Some('\n') if lines => self.i += 1,
                Some('`') if matches!(self.at(1), Some('\n' | '\r')) => {
                    self.i += 1;
                    while matches!(self.peek(), Some('\r' | '\n')) {
                        self.i += 1;
                        if self.c.get(self.i - 1) == Some(&'\n') {
                            break;
                        }
                    }
                }
                Some('<') if self.at(1) == Some('#') => {
                    while self.i < self.c.len() && !(self.peek() == Some('#') && self.at(1) == Some('>')) {
                        self.i += 1;
                    }
                    self.i += 2;
                }
                Some('#') => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                _ => return,
            }
        }
    }

    /// A keyword here, followed by something that can't continue a word; taken if so.
    fn keyword(&mut self, k: &str) -> bool {
        if self.starts(k) && !self.at(k.len()).is_some_and(|c| word_char(c) || c == '-') {
            self.i += k.len();
            true
        } else {
            false
        }
    }

    /// Statements up to `end` (`}` or `)`), or the end.
    fn block_body(&mut self, end: Option<char>) -> R<Block> {
        let start = self.i;
        let mut params = Vec::new();
        self.skip(true);
        if self.keyword("param") {
            self.skip(true);
            params = self.params()?;
        }
        let mut stmts = Vec::new();
        loop {
            self.skip(true);
            while self.peek() == Some(';') {
                self.i += 1;
                self.skip(true);
            }
            match self.peek() {
                None if end.is_some() => return incomplete("Missing closing '}' in statement block."),
                None => break,
                Some(c) if Some(c) == end => break,
                Some('}' | ')') if end.is_none() => return err(&alloc::format!("Unexpected token '{}' in expression or statement.", self.peek().unwrap_or(' '))),
                _ => {}
            }
            stmts.push(self.statement()?);
            self.skip(false);
            match self.peek() {
                Some(';' | '\n') => self.i += 1,
                None => {}
                Some(c) if Some(c) == end => {}
                Some(c) => return err(&alloc::format!("Unexpected token '{c}' in expression or statement.")),
            }
        }
        Ok(Block { params, stmts, text: self.c[start..self.i].iter().collect() })
    }

    /// `param(...)` or a function's `(...)`: names, and defaults.
    fn params(&mut self) -> R<Vec<(String, Option<Expr>)>> {
        if self.peek() != Some('(') {
            return err("Missing '(' after 'param'.");
        }
        self.i += 1;
        let mut params = Vec::new();
        loop {
            self.skip(true);
            // Attributes and types are taken and set aside: `[Parameter(Mandatory)] [string]$x`.
            while self.peek() == Some('[') {
                let mut depth = 0;
                while let Some(c) = self.peek() {
                    self.i += 1;
                    if c == '[' {
                        depth += 1;
                    } else if c == ']' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                self.skip(true);
            }
            match self.peek() {
                Some(')') => {
                    self.i += 1;
                    return Ok(params);
                }
                Some('$') => {
                    self.i += 1;
                    let name = self.var_name();
                    self.skip(true);
                    let default = if self.peek() == Some('=') {
                        self.i += 1;
                        self.skip(true);
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    params.push((name, default));
                    self.skip(true);
                    if self.peek() == Some(',') {
                        self.i += 1;
                    }
                }
                None => return incomplete("Missing ')' in function parameter list."),
                Some(c) => return err(&alloc::format!("Unexpected token '{c}' in parameter list.")),
            }
        }
    }

    /// `{ ... }`.
    fn braced(&mut self) -> R<Block> {
        self.skip(true);
        if self.peek() != Some('{') {
            return if self.peek().is_none() { incomplete("Missing statement block.") } else { err("Missing opening '{' of statement block.") };
        }
        self.i += 1;
        let block = self.block_body(Some('}'))?;
        self.i += 1;
        Ok(block)
    }

    /// `( pipeline )`, as `if` and `while` take a condition.
    fn condition(&mut self) -> R<Pipeline> {
        self.skip(true);
        if self.peek() != Some('(') {
            return if self.peek().is_none() { incomplete("Missing '(' after keyword.") } else { err("Missing '(' after keyword.") };
        }
        self.i += 1;
        self.skip(true);
        let p = self.pipeline()?;
        self.skip(true);
        if self.peek() != Some(')') {
            return if self.peek().is_none() { incomplete("Missing closing ')'.") } else { err("Missing closing ')' after expression.") };
        }
        self.i += 1;
        Ok(p)
    }

    fn statement(&mut self) -> R<Stmt> {
        if self.keyword("if") {
            let mut arms = Vec::new();
            let cond = self.condition()?;
            let body = self.braced()?;
            arms.push((cond, body));
            let mut other = None;
            loop {
                let save = self.i;
                self.skip(true);
                if self.keyword("elseif") {
                    let cond = self.condition()?;
                    let body = self.braced()?;
                    arms.push((cond, body));
                } else if self.keyword("else") {
                    other = Some(self.braced()?);
                    break;
                } else {
                    self.i = save;
                    break;
                }
            }
            return Ok(Stmt::If(arms, other));
        }
        if self.keyword("while") {
            let cond = self.condition()?;
            return Ok(Stmt::While(cond, self.braced()?));
        }
        if self.keyword("do") {
            let body = self.braced()?;
            self.skip(true);
            let until = if self.keyword("while") {
                false
            } else if self.keyword("until") {
                true
            } else {
                return err("Missing 'while' or 'until' in do loop.");
            };
            let cond = self.condition()?;
            return Ok(Stmt::Do(body, cond, until));
        }
        if self.keyword("for") {
            self.skip(true);
            if self.peek() != Some('(') {
                return err("Missing opening '(' after keyword 'for'.");
            }
            self.i += 1;
            let mut parts: Vec<Option<Pipeline>> = Vec::new();
            for n in 0..3 {
                self.skip(true);
                let close = if n == 2 { ')' } else { ';' };
                if self.peek() == Some(close) {
                    parts.push(None);
                } else {
                    parts.push(Some(self.pipeline()?));
                    self.skip(true);
                }
                if self.peek() != Some(close) {
                    return if self.peek().is_none() { incomplete("Missing closing ')' in for loop.") } else { err("Missing ';' in for loop.") };
                }
                self.i += 1;
            }
            let body = self.braced()?;
            let mut parts = parts.into_iter();
            return Ok(Stmt::For(parts.next().flatten(), parts.next().flatten(), parts.next().flatten(), body));
        }
        if self.keyword("foreach") {
            self.skip(true);
            if self.peek() != Some('(') {
                return err("Missing opening '(' after keyword 'foreach'.");
            }
            self.i += 1;
            self.skip(true);
            if self.peek() != Some('$') {
                return err("Missing variable name after foreach.");
            }
            self.i += 1;
            let var = self.var_name();
            self.skip(true);
            if !self.keyword("in") {
                return err("Missing 'in' after variable in foreach loop.");
            }
            self.skip(true);
            let collection = self.pipeline()?;
            self.skip(true);
            if self.peek() != Some(')') {
                return err("Missing closing ')' in foreach.");
            }
            self.i += 1;
            return Ok(Stmt::Foreach(var, collection, self.braced()?));
        }
        if self.keyword("switch") {
            self.skip(true);
            // Options (`-regex`, `-wildcard`, `-exact`, `-casesensitive`) aren't looked at.
            while self.peek() == Some('-') {
                while self.peek().is_some_and(|c| !c.is_whitespace() && c != '(') {
                    self.i += 1;
                }
                self.skip(true);
            }
            let value = self.condition()?;
            self.skip(true);
            if self.peek() != Some('{') {
                return err("Missing '{' in switch statement.");
            }
            self.i += 1;
            let mut cases = Vec::new();
            loop {
                self.skip(true);
                while self.peek() == Some(';') {
                    self.i += 1;
                    self.skip(true);
                }
                match self.peek() {
                    Some('}') => {
                        self.i += 1;
                        break;
                    }
                    None => return incomplete("Missing '}' in switch."),
                    _ => {}
                }
                let test = if self.keyword("default") {
                    None
                } else if matches!(self.peek(), Some('$' | '(' | '\'' | '"' | '{')) || self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    Some(self.postfix()?)
                } else {
                    let w = self.bareword();
                    Some(Expr::Value(Value::str(w)))
                };
                let body = self.braced()?;
                cases.push((test, body));
            }
            return Ok(Stmt::Switch(value, cases));
        }
        if self.keyword("function") || self.keyword("filter") {
            self.skip(true);
            let name = self.bareword();
            self.skip(true);
            let params = if self.peek() == Some('(') { self.params()? } else { Vec::new() };
            let mut body = self.braced()?;
            if !params.is_empty() {
                body.params = params;
            }
            return Ok(Stmt::Function(name, Rc::new(body)));
        }
        if self.keyword("try") {
            let body = self.braced()?;
            let (mut catch, mut finally) = (None, None);
            loop {
                let save = self.i;
                self.skip(true);
                if self.keyword("catch") {
                    self.skip(true);
                    // An exception type: any is caught.
                    while self.peek() == Some('[') {
                        while self.peek().is_some_and(|c| c != ']') {
                            self.i += 1;
                        }
                        self.i += 1;
                        self.skip(true);
                        if self.peek() == Some(',') {
                            self.i += 1;
                            self.skip(true);
                        }
                    }
                    catch = Some(self.braced()?);
                } else if self.keyword("finally") {
                    finally = Some(self.braced()?);
                } else {
                    self.i = save;
                    break;
                }
            }
            return Ok(Stmt::Try(body, catch, finally));
        }
        let ends = |p: &mut Parser| {
            p.skip(false);
            matches!(p.peek(), None | Some('\n' | ';' | '}' | ')'))
        };
        if self.keyword("return") {
            return Ok(Stmt::Return(if ends(self) { None } else { Some(self.pipeline()?) }));
        }
        if self.keyword("exit") {
            return Ok(Stmt::Exit(if ends(self) { None } else { Some(self.pipeline()?) }));
        }
        if self.keyword("throw") {
            return Ok(Stmt::Throw(if ends(self) { None } else { Some(self.pipeline()?) }));
        }
        if self.keyword("break") {
            return Ok(Stmt::Break);
        }
        if self.keyword("continue") {
            return Ok(Stmt::Continue);
        }
        Ok(Stmt::Pipeline(self.pipeline()?))
    }

    /// An assignment's operator here, taken.
    fn assign_op(&mut self) -> Option<String> {
        self.skip(false);
        for op in ["+=", "-=", "*=", "/=", "%=", "="] {
            if self.starts(op) && !(op == "=" && self.at(1) == Some('=')) {
                self.i += op.len();
                return Some(op.to_string());
            }
        }
        None
    }

    pub fn pipeline(&mut self) -> R<Pipeline> {
        self.skip(false);
        let mut assign = None;
        if self.peek() == Some('$') || self.peek() == Some('[') {
            let save = self.i;
            if let Ok(target) = self.postfix()
                && matches!(target, Expr::Var(_) | Expr::Member(..) | Expr::Index(..) | Expr::Cast(..))
                && let Some(op) = self.assign_op()
            {
                self.skip(true);
                // `$x = if (...) { }` and the like: a statement's output.
                if ["if", "foreach", "switch", "while", "for", "do", "try"].iter().any(|k| self.starts(k) && !self.at(k.len()).is_some_and(word_char)) {
                    let stmt = self.statement()?;
                    return Ok(Pipeline { assign: Some((target, op)), elements: alloc::vec![Element::Expr(Expr::Sub(alloc::vec![stmt]))] });
                }
                assign = Some((target, op));
            } else {
                self.i = save;
            }
        }
        let mut elements = Vec::new();
        loop {
            self.skip(false);
            elements.push(self.element()?);
            self.skip(false);
            if self.peek() == Some('|') && self.at(1) != Some('|') {
                self.i += 1;
                self.skip(true);
                if self.peek().is_none() {
                    return incomplete("An empty pipe element is not allowed.");
                }
                continue;
            }
            break;
        }
        Ok(Pipeline { assign, elements })
    }

    fn element(&mut self) -> R<Element> {
        let c = match self.peek() {
            None => return incomplete("Missing expression."),
            Some(c) => c,
        };
        let starts_expr = matches!(c, '$' | '\'' | '"' | '(' | '@' | '[' | '{' | '!' | '+')
            || c.is_ascii_digit()
            || (c == '.' && self.at(1).is_some_and(|d| d.is_ascii_digit()))
            || (c == '-' && (self.at(1).is_some_and(|d| d.is_ascii_digit() || d == '-') || self.starts("-not") || self.starts("-join") || self.starts("-split") || self.starts("-bnot")));
        if (c == '&' || c == '.') && matches!(self.at(1), Some(' ' | '\t' | '$' | '\'' | '"' | '{' | '(')) {
            self.i += 1;
            self.skip(false);
            let name = self.postfix()?;
            return Ok(Element::Command(self.arguments(name, true)?));
        }
        if starts_expr {
            return Ok(Element::Expr(self.expr()?));
        }
        let name = self.bareword();
        if name.is_empty() {
            return err(&alloc::format!("Unexpected token '{c}' in expression or statement."));
        }
        Ok(Element::Command(self.arguments(Expr::Value(Value::str(name)), false)?))
    }

    /// A word in argument mode: up to a space or something that ends an argument.
    fn bareword(&mut self) -> String {
        let mut w = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || matches!(c, '|' | ';' | ')' | '}' | '(' | '{' | ',' | '&' | '>' | '<') && !(c == '(' && false) {
                break;
            }
            if c == '`' {
                self.i += 1;
                if let Some(n) = self.peek() {
                    w.push(n);
                    self.i += 1;
                }
                continue;
            }
            w.push(c);
            self.i += 1;
        }
        w
    }

    fn arguments(&mut self, name: Expr, call: bool) -> R<Command> {
        let mut args = Vec::new();
        let mut redirects = Vec::new();
        loop {
            self.skip(false);
            let Some(c) = self.peek() else { break };
            if matches!(c, '\n' | ';' | '|' | ')' | '}') {
                break;
            }
            // Redirections.
            let fd = match (c, self.at(1)) {
                ('>', _) => Some(1u8),
                ('2' | '1' | '3' | '*', Some('>')) => {
                    self.i += 1;
                    Some(if c == '1' { 1 } else if c == '*' { 9 } else { c as u8 - b'0' })
                }
                _ => None,
            };
            if let Some(fd) = fd {
                self.i += 1;
                let append = self.peek() == Some('>');
                if append {
                    self.i += 1;
                }
                if self.peek() == Some('&') {
                    self.i += 1;
                    let to = self.bareword();
                    redirects.push((fd, alloc::format!("&{to}"), false));
                    continue;
                }
                self.skip(false);
                let target = match self.peek() {
                    Some('\'' | '"') => match self.primary()? {
                        Expr::Value(v) => v.to_string(),
                        Expr::Template(parts) => parts.iter().map(|p| if let Part::Lit(s) = p { s.as_str() } else { "" }).collect(),
                        _ => String::new(),
                    },
                    _ => self.bareword(),
                };
                redirects.push((fd, target, append));
                continue;
            }
            if c == '-' && self.at(1).is_some_and(|n| n.is_alphabetic() || n == '?') {
                self.i += 1;
                let mut pname = String::new();
                while let Some(n) = self.peek() {
                    if n.is_whitespace() || matches!(n, ':' | '|' | ';' | ')' | '}' | ',') {
                        break;
                    }
                    pname.push(n);
                    self.i += 1;
                }
                if self.peek() == Some(':') {
                    self.i += 1;
                    let v = self.argument()?;
                    args.push(Arg::Named(pname, Some(v)));
                } else {
                    args.push(Arg::Named(pname, None));
                }
                continue;
            }
            if c == '-' && self.at(1) == Some('-') && self.at(2) == Some('%') {
                // --%: the rest as it is.
                self.i += 3;
                let mut rest = String::new();
                while self.peek().is_some_and(|c| c != '\n' && c != '|') {
                    rest.push(self.peek().unwrap_or(' '));
                    self.i += 1;
                }
                for w in rest.split_whitespace() {
                    args.push(Arg::Value(Expr::Value(Value::str(w))));
                }
                continue;
            }
            let v = self.argument()?;
            args.push(Arg::Value(v));
        }
        Ok(Command { name, args, call, redirects })
    }

    /// One argument in argument mode, with `,` making an array of them.
    fn argument(&mut self) -> R<Expr> {
        let first = self.argument_one()?;
        self.skip(false);
        if self.peek() != Some(',') {
            return Ok(first);
        }
        let mut items = alloc::vec![first];
        while self.peek() == Some(',') {
            self.i += 1;
            self.skip(true);
            items.push(self.argument_one()?);
            self.skip(false);
        }
        Ok(Expr::Array(items))
    }

    fn argument_one(&mut self) -> R<Expr> {
        match self.peek() {
            Some('$' | '\'' | '"' | '(' | '@' | '{') => {
                let e = self.postfix()?;
                // A string run on into more, as `"a"b` or `$x\y`: the parts joined.
                if self.peek().is_some_and(|c| !c.is_whitespace() && !matches!(c, '|' | ';' | ')' | '}' | ',' | '&' | '>' | '<')) {
                    let rest = self.bareword();
                    return Ok(Expr::Template(alloc::vec![Part::Expr(e), Part::Lit(rest)]));
                }
                Ok(e)
            }
            _ => {
                let w = self.bareword();
                if let Some(n) = parse_number(&w) {
                    return Ok(Expr::Value(n));
                }
                if w.contains('$') {
                    return Ok(Expr::Template(self.interpolate(&w)?));
                }
                Ok(Expr::Value(Value::str(w)))
            }
        }
    }

    // ---- Expressions ----

    pub fn expr(&mut self) -> R<Expr> {
        self.logical()
    }

    /// An operator `-name` here (with `i` or `c` before it for case), of those `allowed`; taken if so.
    fn operator(&mut self, allowed: &[&str]) -> Option<String> {
        let save = self.i;
        self.skip(false);
        if self.peek() != Some('-') {
            self.i = save;
            return None;
        }
        let mut n = 1;
        let mut name = String::new();
        while let Some(c) = self.at(n) {
            if !c.is_ascii_alphabetic() {
                break;
            }
            name.push(c.to_ascii_lowercase());
            n += 1;
        }
        let base = if (name.starts_with('i') || name.starts_with('c')) && OPERATORS.contains(&&name[1..]) && !OPERATORS.contains(&name.as_str()) { &name[1..] } else { name.as_str() };
        if allowed.contains(&base) {
            self.i += n;
            self.skip(true);
            return Some(name);
        }
        self.i = save;
        None
    }

    fn logical(&mut self) -> R<Expr> {
        let mut left = self.bitwise()?;
        while let Some(op) = self.operator(&["and", "or", "xor"]) {
            let right = self.bitwise()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn bitwise(&mut self) -> R<Expr> {
        let mut left = self.comparison()?;
        while let Some(op) = self.operator(&["band", "bor", "bxor"]) {
            let right = self.comparison()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn comparison(&mut self) -> R<Expr> {
        let mut left = self.additive()?;
        const OPS: &[&str] = &["eq", "ne", "gt", "ge", "lt", "le", "like", "notlike", "match", "notmatch", "contains", "notcontains", "in", "notin", "replace", "split", "join", "is", "isnot", "as", "shl", "shr"];
        while let Some(op) = self.operator(OPS) {
            let right = self.additive()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn symbol(&mut self, ops: &[&str]) -> Option<String> {
        let save = self.i;
        self.skip(false);
        for op in ops {
            if self.starts(op) && !(op.len() == 1 && matches!(self.at(1), Some('=') | Some('+') | Some('-')) && (self.at(1) == Some('=') || self.at(1) == self.peek())) {
                self.i += op.len();
                self.skip(true);
                return Some(op.to_string());
            }
        }
        self.i = save;
        None
    }

    fn additive(&mut self) -> R<Expr> {
        let mut left = self.multiplicative()?;
        while let Some(op) = self.symbol(&["+", "-"]) {
            // `-eq` and the like are operators, not minus.
            if op == "-" && self.peek().is_some_and(|c| c.is_ascii_alphabetic()) && self.c.get(self.i - 1) == Some(&'-') {
                self.i -= 1;
                break;
            }
            let right = self.multiplicative()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn multiplicative(&mut self) -> R<Expr> {
        let mut left = self.format()?;
        while let Some(op) = self.symbol(&["*", "/", "%"]) {
            let right = self.format()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn format(&mut self) -> R<Expr> {
        let mut left = self.range()?;
        while let Some(op) = self.operator(&["f"]) {
            let right = self.range()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn range(&mut self) -> R<Expr> {
        let left = self.comma()?;
        let save = self.i;
        self.skip(false);
        if self.starts("..") {
            self.i += 2;
            self.skip(true);
            let right = self.comma()?;
            return Ok(Expr::Binary(String::from(".."), Box::new(left), Box::new(right)));
        }
        self.i = save;
        Ok(left)
    }

    fn comma(&mut self) -> R<Expr> {
        let first = self.unary()?;
        let save = self.i;
        self.skip(false);
        if self.peek() != Some(',') {
            self.i = save;
            return Ok(first);
        }
        let mut items = alloc::vec![first];
        while self.peek() == Some(',') {
            self.i += 1;
            self.skip(true);
            items.push(self.unary()?);
            let save = self.i;
            self.skip(false);
            if self.peek() != Some(',') {
                self.i = save;
                break;
            }
        }
        Ok(Expr::Array(items))
    }

    fn unary(&mut self) -> R<Expr> {
        self.skip(false);
        if let Some(op) = self.operator(&["not", "bnot", "split", "join"]) {
            let e = self.unary()?;
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        match self.peek() {
            Some('!') => {
                self.i += 1;
                let e = self.unary()?;
                Ok(Expr::Unary(String::from("not"), Box::new(e)))
            }
            Some('+') if self.at(1) == Some('+') => {
                self.i += 2;
                let e = self.postfix()?;
                Ok(Expr::Step(Box::new(e), 1, true))
            }
            Some('-') if self.at(1) == Some('-') => {
                self.i += 2;
                let e = self.postfix()?;
                Ok(Expr::Step(Box::new(e), -1, true))
            }
            Some('-') => {
                self.i += 1;
                let e = self.unary()?;
                Ok(Expr::Unary(String::from("-"), Box::new(e)))
            }
            Some('+') => {
                self.i += 1;
                let e = self.unary()?;
                Ok(Expr::Unary(String::from("+"), Box::new(e)))
            }
            Some(',') => {
                // A unary comma: an array of one.
                self.i += 1;
                let e = self.unary()?;
                Ok(Expr::Array(alloc::vec![e]))
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        loop {
            match self.peek() {
                Some('.') if self.at(1).is_some_and(|c| word_char(c) || c == '$' || c == '\'' || c == '"') => {
                    self.i += 1;
                    e = self.member(e, false)?;
                }
                Some(':') if self.at(1) == Some(':') => {
                    self.i += 2;
                    e = self.member(e, true)?;
                }
                Some('[') => {
                    self.i += 1;
                    self.skip(true);
                    let index = self.expr()?;
                    self.skip(true);
                    if self.peek() != Some(']') {
                        return if self.peek().is_none() { incomplete("Missing ']' after array index.") } else { err("Missing ']' after array index expression.") };
                    }
                    self.i += 1;
                    e = Expr::Index(Box::new(e), Box::new(index));
                }
                Some('+') if self.at(1) == Some('+') => {
                    self.i += 2;
                    e = Expr::Step(Box::new(e), 1, false);
                }
                Some('-') if self.at(1) == Some('-') && !self.at(2).is_some_and(|c| c.is_ascii_alphanumeric()) => {
                    self.i += 2;
                    e = Expr::Step(Box::new(e), -1, false);
                }
                _ => return Ok(e),
            }
        }
    }

    fn member(&mut self, target: Expr, is_static: bool) -> R<Expr> {
        let name = match self.peek() {
            Some('\'' | '"') => self.primary()?.literal_text(),
            _ => {
                let mut n = String::new();
                while self.peek().is_some_and(word_char) {
                    n.push(self.peek().unwrap_or(' '));
                    self.i += 1;
                }
                n
            }
        };
        if self.peek() == Some('(') {
            self.i += 1;
            let mut args = Vec::new();
            loop {
                self.skip(true);
                if self.peek() == Some(')') {
                    self.i += 1;
                    break;
                }
                if self.peek().is_none() {
                    return incomplete("Missing ')' in method call.");
                }
                args.push(self.unary_no_comma()?);
                self.skip(true);
                if self.peek() == Some(',') {
                    self.i += 1;
                }
            }
            return Ok(Expr::Method(Box::new(target), name, args, is_static));
        }
        Ok(Expr::Member(Box::new(target), name, is_static))
    }

    /// A method's argument: an expression with its operators, but not `,`, which separates arguments.
    fn unary_no_comma(&mut self) -> R<Expr> {
        // An expression up to a top-level comma or `)`: parsed whole, then its array taken apart if it's one.
        let start = self.i;
        let mut depth = 0i32;
        let mut quote: Option<char> = None;
        while let Some(c) = self.peek() {
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None => match c {
                    '\'' | '"' => quote = Some(c),
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' if depth == 0 => break,
                    ')' | ']' | '}' => depth -= 1,
                    ',' if depth == 0 => break,
                    _ => {}
                },
            }
            self.i += 1;
        }
        let text: String = self.c[start..self.i].iter().collect();
        let mut p = Parser { c: text.chars().collect(), i: 0 };
        p.skip(true);
        p.expr()
    }

    fn var_name(&mut self) -> String {
        if self.peek() == Some('{') {
            self.i += 1;
            let mut n = String::new();
            while let Some(c) = self.peek() {
                self.i += 1;
                if c == '}' {
                    break;
                }
                n.push(c);
            }
            return n;
        }
        if matches!(self.peek(), Some('_' | '?' | '$' | '^')) && !self.at(1).is_some_and(word_char) {
            let c = self.peek().unwrap_or('_');
            self.i += 1;
            return c.to_string();
        }
        let mut n = String::new();
        while let Some(c) = self.peek() {
            if word_char(c) || (c == ':' && self.at(1).is_some_and(word_char) && !n.is_empty() && !n.contains(':')) {
                n.push(c);
                self.i += 1;
            } else {
                break;
            }
        }
        n
    }

    fn primary(&mut self) -> R<Expr> {
        self.skip(false);
        let Some(c) = self.peek() else { return incomplete("Missing expression.") };
        match c {
            '$' => {
                self.i += 1;
                match self.peek() {
                    Some('(') => {
                        self.i += 1;
                        let block = self.block_body(Some(')'))?;
                        self.i += 1;
                        Ok(Expr::Sub(block.stmts))
                    }
                    _ => {
                        let name = self.var_name();
                        Ok(match name.to_ascii_lowercase().as_str() {
                            "true" => Expr::Value(Value::Bool(true)),
                            "false" => Expr::Value(Value::Bool(false)),
                            "null" => Expr::Value(Value::Null),
                            _ => Expr::Var(name),
                        })
                    }
                }
            }
            '\'' => {
                self.i += 1;
                let mut s = String::new();
                loop {
                    match self.peek() {
                        None => return incomplete("The string is missing the terminator: '."),
                        Some('\'') if self.at(1) == Some('\'') => {
                            s.push('\'');
                            self.i += 2;
                        }
                        Some('\'') => {
                            self.i += 1;
                            break;
                        }
                        Some(c) => {
                            s.push(c);
                            self.i += 1;
                        }
                    }
                }
                Ok(Expr::Value(Value::Str(s)))
            }
            '"' => {
                self.i += 1;
                let mut raw = String::new();
                loop {
                    match self.peek() {
                        None => return incomplete("The string is missing the terminator: \"."),
                        Some('`') => {
                            raw.push('`');
                            self.i += 1;
                            if let Some(n) = self.peek() {
                                raw.push(n);
                                self.i += 1;
                            }
                        }
                        Some('"') if self.at(1) == Some('"') => {
                            raw.push_str("`\"");
                            self.i += 2;
                        }
                        Some('"') => {
                            self.i += 1;
                            break;
                        }
                        Some('$') if self.at(1) == Some('(') => {
                            // A sub-expression's text, with its own quotes and parentheses.
                            let start = self.i;
                            self.i += 2;
                            let mut depth = 1;
                            let mut quote: Option<char> = None;
                            while let Some(c) = self.peek() {
                                self.i += 1;
                                match quote {
                                    Some(q) if c == q => quote = None,
                                    Some(_) => {}
                                    None => match c {
                                        '\'' | '"' => quote = Some(c),
                                        '(' => depth += 1,
                                        ')' => {
                                            depth -= 1;
                                            if depth == 0 {
                                                break;
                                            }
                                        }
                                        _ => {}
                                    },
                                }
                            }
                            raw.extend(&self.c[start..self.i]);
                        }
                        Some(c) => {
                            raw.push(c);
                            self.i += 1;
                        }
                    }
                }
                Ok(Expr::Template(self.interpolate(&raw)?))
            }
            '@' => match self.at(1) {
                Some('(') => {
                    self.i += 2;
                    let block = self.block_body(Some(')'))?;
                    self.i += 1;
                    Ok(Expr::ArrayOf(block.stmts))
                }
                Some('{') => {
                    self.i += 2;
                    let mut pairs = Vec::new();
                    loop {
                        self.skip(true);
                        while self.peek() == Some(';') {
                            self.i += 1;
                            self.skip(true);
                        }
                        match self.peek() {
                            Some('}') => {
                                self.i += 1;
                                break;
                            }
                            None => return incomplete("Missing closing '}' in hash literal."),
                            _ => {}
                        }
                        let key = match self.peek() {
                            Some('\'' | '"' | '$' | '(') => self.postfix()?,
                            _ => {
                                let mut k = String::new();
                                while self.peek().is_some_and(|c| !c.is_whitespace() && c != '=') {
                                    k.push(self.peek().unwrap_or(' '));
                                    self.i += 1;
                                }
                                Expr::Value(Value::str(k))
                            }
                        };
                        self.skip(true);
                        if self.peek() != Some('=') {
                            return err("Missing '=' operator after key in hash literal.");
                        }
                        self.i += 1;
                        self.skip(true);
                        let value = self.pipeline()?;
                        let value = match (value.assign, value.elements.len()) {
                            (None, 1) => match value.elements.into_iter().next() {
                                Some(Element::Expr(e)) => e,
                                Some(other) => Expr::Paren(Box::new(Pipeline { assign: None, elements: alloc::vec![other] })),
                                None => Expr::Value(Value::Null),
                            },
                            (assign, _) => Expr::Paren(Box::new(Pipeline { assign, elements: value.elements })),
                        };
                        pairs.push((key, value));
                    }
                    Ok(Expr::Hash(pairs))
                }
                Some('"') | Some('\'') => {
                    // A here-string: from the next line to a line starting with the quote and `@`.
                    let quote = self.at(1).unwrap_or('"');
                    self.i += 2;
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                    self.i += 1;
                    let start = self.i;
                    loop {
                        if self.i >= self.c.len() {
                            return incomplete("The here-string is missing its end.");
                        }
                        if (self.i == start || self.c[self.i - 1] == '\n') && self.peek() == Some(quote) && self.at(1) == Some('@') {
                            break;
                        }
                        self.i += 1;
                    }
                    let mut body: String = self.c[start..self.i].iter().collect();
                    if body.ends_with('\n') {
                        body.pop();
                        if body.ends_with('\r') {
                            body.pop();
                        }
                    }
                    self.i += 2;
                    if quote == '\'' { Ok(Expr::Value(Value::Str(body))) } else { Ok(Expr::Template(self.interpolate(&body)?)) }
                }
                _ => err("Unrecognized token in source text."),
            },
            '(' => {
                self.i += 1;
                self.skip(true);
                let p = self.pipeline()?;
                self.skip(true);
                if self.peek() != Some(')') {
                    return if self.peek().is_none() { incomplete("Missing closing ')' in expression.") } else { err("Missing closing ')' in expression.") };
                }
                self.i += 1;
                Ok(Expr::Paren(Box::new(p)))
            }
            '{' => {
                self.i += 1;
                let block = self.block_body(Some('}'))?;
                self.i += 1;
                Ok(Expr::Block(Rc::new(block)))
            }
            '[' => {
                self.i += 1;
                let mut name = String::new();
                let mut depth = 0;
                while let Some(c) = self.peek() {
                    self.i += 1;
                    if c == '[' {
                        depth += 1;
                    } else if c == ']' {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    }
                    name.push(c);
                }
                let name = name.trim().to_string();
                if self.peek() == Some(':') && self.at(1) == Some(':') {
                    return Ok(Expr::Type(name));
                }
                // A cast, if something follows to cast.
                let save = self.i;
                self.skip(false);
                if matches!(self.peek(), Some('$' | '\'' | '"' | '(' | '@' | '[' | '-')) || self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    let e = self.unary()?;
                    return Ok(Expr::Cast(name, Box::new(e)));
                }
                self.i = save;
                Ok(Expr::Type(name))
            }
            c if c.is_ascii_digit() || c == '.' => {
                let mut n = String::new();
                while let Some(c) = self.peek() {
                    let prev_e = n.ends_with(['e', 'E']) && !n.to_ascii_lowercase().starts_with("0x");
                    if c.is_ascii_alphanumeric() || (c == '.' && self.at(1) != Some('.')) || (prev_e && (c == '-' || c == '+')) {
                        n.push(c);
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                match parse_number(&n) {
                    Some(v) => Ok(Expr::Value(v)),
                    None => err(&alloc::format!("Unexpected token '{n}' in expression or statement.")),
                }
            }
            _ => {
                // A bare word in an expression: a command's output, as PowerShell would refuse; taken as a string.
                let w = self.bareword();
                if w.is_empty() {
                    return err(&alloc::format!("Unexpected token '{c}' in expression or statement."));
                }
                Ok(Expr::Value(Value::str(w)))
            }
        }
    }

    /// A double-quoted string's parts: text, `$var` and `${var}` and `$env:x`, `$(...)`, and backtick escapes.
    pub fn interpolate(&mut self, raw: &str) -> R<Vec<Part>> {
        let c: Vec<char> = raw.chars().collect();
        let mut parts = Vec::new();
        let mut lit = String::new();
        let mut i = 0;
        while i < c.len() {
            match c[i] {
                '`' if i + 1 < c.len() => {
                    lit.push(match c[i + 1] {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        'a' => '\x07',
                        'b' => '\x08',
                        'e' => '\x1b',
                        'f' => '\x0c',
                        'v' => '\x0b',
                        other => other,
                    });
                    i += 2;
                }
                '$' if i + 1 < c.len() && c[i + 1] == '(' => {
                    let mut depth = 0;
                    let start = i + 2;
                    let mut j = i + 1;
                    let mut quote: Option<char> = None;
                    while j < c.len() {
                        match quote {
                            Some(q) if c[j] == q => quote = None,
                            Some(_) => {}
                            None => match c[j] {
                                '\'' | '"' => quote = Some(c[j]),
                                '(' => depth += 1,
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            },
                        }
                        j += 1;
                    }
                    let inner: String = c[start..j.min(c.len())].iter().collect();
                    if !lit.is_empty() {
                        parts.push(Part::Lit(core::mem::take(&mut lit)));
                    }
                    let block = Parser::script(&inner)?;
                    parts.push(Part::Expr(Expr::Sub(block.stmts)));
                    i = j + 1;
                }
                '$' if i + 1 < c.len() && (word_char(c[i + 1]) || matches!(c[i + 1], '{' | '_' | '?')) => {
                    let mut p = Parser { c: c[i + 1..].to_vec(), i: 0 };
                    let name = p.var_name();
                    if !lit.is_empty() {
                        parts.push(Part::Lit(core::mem::take(&mut lit)));
                    }
                    parts.push(Part::Expr(match name.to_ascii_lowercase().as_str() {
                        "true" => Expr::Value(Value::Bool(true)),
                        "false" => Expr::Value(Value::Bool(false)),
                        "null" => Expr::Value(Value::Null),
                        _ => Expr::Var(name),
                    }));
                    i += 1 + p.i;
                }
                ch => {
                    lit.push(ch);
                    i += 1;
                }
            }
        }
        if !lit.is_empty() || parts.is_empty() {
            parts.push(Part::Lit(lit));
        }
        Ok(parts)
    }
}

impl Expr {
    /// A literal's text (a member named by a string).
    fn literal_text(&self) -> String {
        match self {
            Expr::Value(v) => v.to_string(),
            Expr::Template(parts) => parts.iter().map(|p| if let Part::Lit(s) = p { s.as_str() } else { "" }).collect(),
            _ => String::new(),
        }
    }
}
