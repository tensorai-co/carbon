//! cmd's grammar: a line (or a block over several) after `%` expansion, into commands joined by `&`, `&&`, `||` and
//! `|`, with parenthesized blocks, `^` escapes, quotes, redirections, and IF, FOR and REM, whose bodies are commands.
//! FOR's variables and delayed `!` expansion are left in the text, for when it runs.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub enum Redir {
    /// `n>file`, `n>>file`.
    Out { fd: u8, path: String, append: bool },
    /// `<file`.
    In(String),
    /// `n>&m`.
    Dup { fd: u8, to: u8 },
}

#[derive(Debug, Clone)]
pub enum Cond {
    Errorlevel(String),
    Exist(String),
    Defined(String),
    CmdExtVersion(String),
    /// `a==b`, or `a OP b` with EQU, NEQ, LSS, LEQ, GTR or GEQ.
    Compare(String, String, String),
}

#[derive(Debug, Clone)]
pub enum ForKind {
    Plain,
    Dirs,
    Recursive(Option<String>),
    Count,
    Lines(String),
}

#[derive(Debug, Clone)]
pub enum Node {
    Empty,
    /// A command and its arguments as written, its redirections taken out; `quiet` if it began with `@`.
    Simple { text: String, redirs: Vec<Redir>, #[allow(dead_code)] quiet: bool },
    Block { body: Box<Node>, redirs: Vec<Redir> },
    If { not: bool, fold: bool, cond: Cond, then: Box<Node>, other: Option<Box<Node>> },
    For { kind: ForKind, var: char, set: String, body: Box<Node> },
    Seq(Box<Node>, Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    /// Each part, and its text, for running it in a cmd of its own.
    Pipe(Vec<(Node, String)>),
}

pub struct Parser {
    chars: Vec<char>,
    pos: usize,
    depth: usize,
}

impl Parser {
    pub fn parse(text: &str) -> Node {
        let mut p = Parser { chars: text.chars().collect(), pos: 0, depth: 0 };
        let mut node = p.seq(false);
        // A `)` too many at the top: what's after it is more commands, as cmd takes it.
        while p.pos < p.chars.len() {
            p.pos += 1;
            let more = p.seq(false);
            node = join(node, more, Node::Seq);
        }
        node
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.pos += 1;
        }
    }

    fn skip_blank(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn at_close(&self) -> bool {
        self.depth > 0 && self.peek() == Some(')')
    }

    /// Commands joined by `&` (and, unless `line`, newlines), up to the end, or a `)` closing the block it's in.
    fn seq(&mut self, line: bool) -> Node {
        let mut node = Node::Empty;
        loop {
            if line { self.skip_spaces() } else { self.skip_blank() }
            if self.pos >= self.chars.len() || self.at_close() || (line && self.peek() == Some('\n')) {
                return node;
            }
            let next = self.and_or(line);
            node = join(node, next, Node::Seq);
            self.skip_spaces();
            match self.peek() {
                Some('&') => self.pos += 1,
                Some('\n') if !line => self.pos += 1,
                Some('\r') => self.pos += 1,
                _ if self.at_close() => return node,
                None => return node,
                Some('\n') => return node,
                _ => {}
            }
        }
    }

    fn and_or(&mut self, line: bool) -> Node {
        let mut node = self.pipeline(line);
        loop {
            self.skip_spaces();
            match (self.peek(), self.peek_at(1)) {
                (Some('&'), Some('&')) => {
                    self.pos += 2;
                    let right = self.pipeline(line);
                    node = Node::And(Box::new(node), Box::new(right));
                }
                (Some('|'), Some('|')) => {
                    self.pos += 2;
                    let right = self.pipeline(line);
                    node = Node::Or(Box::new(node), Box::new(right));
                }
                _ => return node,
            }
        }
    }

    fn pipeline(&mut self, line: bool) -> Node {
        let mut parts = Vec::new();
        loop {
            self.skip_spaces();
            let start = self.pos;
            let node = self.command(line);
            let text: String = self.chars[start..self.pos].iter().collect();
            parts.push((node, String::from(text.trim())));
            self.skip_spaces();
            if self.peek() == Some('|') && self.peek_at(1) != Some('|') {
                self.pos += 1;
                continue;
            }
            break;
        }
        if parts.len() == 1 { parts.pop().map(|(n, _)| n).unwrap_or(Node::Empty) } else { Node::Pipe(parts) }
    }

    /// A word as IF and FOR read theirs: quoted, or up to a space.
    fn word(&mut self) -> String {
        self.skip_spaces();
        let mut w = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            if !quoted && (c == ' ' || c == '\t' || c == '\n' || (c == '=' && self.peek_at(1) == Some('=')) || (c == ')' && self.depth > 0)) {
                break;
            }
            if c == '"' {
                quoted = !quoted;
            }
            if c == '^' && !quoted {
                self.pos += 1;
                if let Some(next) = self.peek() {
                    w.push(next);
                    self.pos += 1;
                }
                continue;
            }
            w.push(c);
            self.pos += 1;
        }
        w
    }

    /// Whether the next word is `keyword` (case folded), followed by a space, `(`, or the end; taken if so.
    fn keyword(&mut self, keyword: &str) -> bool {
        let save = self.pos;
        self.skip_spaces();
        let n = keyword.chars().count();
        let found: String = self.chars.iter().skip(self.pos).take(n).collect();
        if programs::eq_ignore_case(&found, keyword) && matches!(self.peek_at(n), None | Some(' ' | '\t' | '(' | '\n' | '/' | '.' | ';' | ',' | '=')) {
            self.pos += n;
            return true;
        }
        self.pos = save;
        false
    }

    /// A body: a block, or the rest of the line.
    fn body(&mut self) -> Node {
        self.skip_spaces();
        if self.peek() == Some('(') { self.command(true) } else { self.seq(true) }
    }

    fn command(&mut self, line: bool) -> Node {
        self.skip_spaces();
        let mut quiet = false;
        while self.peek() == Some('@') {
            quiet = true;
            self.pos += 1;
            self.skip_spaces();
        }
        match self.peek() {
            None => return Node::Empty,
            Some('(') => {
                self.pos += 1;
                self.depth += 1;
                let body = self.seq(false);
                if self.peek() == Some(')') {
                    self.pos += 1;
                }
                self.depth -= 1;
                let redirs = self.redirections();
                return Node::Block { body: Box::new(body), redirs };
            }
            _ => {}
        }
        if self.keyword("if") {
            return self.parse_if();
        }
        if self.keyword("for") {
            return self.parse_for();
        }
        let save = self.pos;
        if self.keyword("rem") {
            // REM takes the rest of its line, operators and all.
            while self.peek().is_some_and(|c| c != '\n') {
                self.pos += 1;
            }
            return Node::Empty;
        }
        self.pos = save;
        let _ = line;
        self.simple(quiet)
    }

    /// Redirections after a block's `)`.
    fn redirections(&mut self) -> Vec<Redir> {
        let mut redirs = Vec::new();
        loop {
            self.skip_spaces();
            let fd = match (self.peek(), self.peek_at(1)) {
                (Some(d @ '0'..='9'), Some('>')) => {
                    self.pos += 1;
                    d as u8 - b'0'
                }
                (Some('>' | '<'), _) => 1,
                _ => return redirs,
            };
            if let Some(r) = self.redirection(fd) {
                redirs.push(r);
            }
        }
    }

    /// A redirection at `>`, `>>`, `>&` or `<`, for descriptor `fd`.
    fn redirection(&mut self, fd: u8) -> Option<Redir> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '<' {
            let path = self.target();
            return Some(Redir::In(path));
        }
        let append = self.peek() == Some('>');
        if append {
            self.pos += 1;
        }
        if self.peek() == Some('&') && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) {
            let to = self.peek_at(1)? as u8 - b'0';
            self.pos += 2;
            return Some(Redir::Dup { fd, to });
        }
        let path = self.target();
        Some(Redir::Out { fd, path, append })
    }

    /// A redirection's file.
    fn target(&mut self) -> String {
        self.skip_spaces();
        let mut t = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            if !quoted && (matches!(c, ' ' | '\t' | '\n' | '&' | '|' | '<' | '>') || (c == ')' && self.depth > 0)) {
                break;
            }
            if c == '"' {
                quoted = !quoted;
            } else if c == '^' && !quoted {
                self.pos += 1;
                if let Some(n) = self.peek() {
                    t.push(n);
                }
            } else {
                t.push(c);
            }
            self.pos += 1;
        }
        t
    }

    fn simple(&mut self, quiet: bool) -> Node {
        let mut text = String::new();
        let mut redirs = Vec::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            if quoted {
                if c == '"' {
                    quoted = false;
                }
                text.push(c);
                self.pos += 1;
                continue;
            }
            match c {
                '"' => {
                    quoted = true;
                    text.push(c);
                    self.pos += 1;
                }
                '^' => {
                    self.pos += 1;
                    if let Some(n) = self.peek() {
                        text.push(n);
                        self.pos += 1;
                    }
                }
                '&' | '|' => break,
                ')' if self.depth > 0 => break,
                '>' | '<' => {
                    // A digit just before, alone, says which descriptor.
                    let mut fd = 1;
                    if c == '>' {
                        let bytes: Vec<char> = text.chars().collect();
                        if let Some(&d) = bytes.last()
                            && d.is_ascii_digit()
                            && (bytes.len() == 1 || matches!(bytes[bytes.len() - 2], ' ' | '\t'))
                        {
                            fd = d as u8 - b'0';
                            text.pop();
                        }
                    }
                    if let Some(r) = self.redirection(fd) {
                        redirs.push(r);
                    }
                }
                _ => {
                    text.push(c);
                    self.pos += 1;
                }
            }
        }
        let text = String::from(text.trim_end_matches([' ', '\t', '\r']));
        if text.trim().is_empty() && redirs.is_empty() {
            return Node::Empty;
        }
        Node::Simple { text, redirs, quiet }
    }

    fn parse_if(&mut self) -> Node {
        let mut fold = false;
        let mut not = false;
        loop {
            if self.keyword("/i") {
                fold = true;
            } else if self.keyword("not") {
                not = true;
            } else {
                break;
            }
        }
        let cond = if self.keyword("errorlevel") {
            Cond::Errorlevel(self.word())
        } else if self.keyword("exist") {
            Cond::Exist(self.word())
        } else if self.keyword("defined") {
            Cond::Defined(self.word())
        } else if self.keyword("cmdextversion") {
            Cond::CmdExtVersion(self.word())
        } else {
            let left = self.word();
            self.skip_spaces();
            if self.peek() == Some('=') && self.peek_at(1) == Some('=') {
                self.pos += 2;
                while self.peek() == Some('=') {
                    self.pos += 1;
                }
                let right = self.word();
                Cond::Compare(left, String::from("=="), right)
            } else {
                let op = self.word();
                let right = self.word();
                Cond::Compare(left, op, right)
            }
        };
        let then = self.body();
        let then_block = matches!(then, Node::Block { .. });
        let save = self.pos;
        self.skip_spaces();
        let other = if then_block && self.keyword("else") {
            Some(Box::new(self.body()))
        } else {
            self.pos = save;
            None
        };
        Node::If { not, fold, cond, then: Box::new(then), other }
    }

    fn parse_for(&mut self) -> Node {
        let mut kind = ForKind::Plain;
        loop {
            self.skip_spaces();
            if self.keyword("/d") {
                kind = ForKind::Dirs;
            } else if self.keyword("/l") {
                kind = ForKind::Count;
            } else if self.keyword("/r") {
                self.skip_spaces();
                let root = if self.peek() == Some('%') { None } else { Some(self.word()) };
                kind = ForKind::Recursive(root);
            } else if self.keyword("/f") {
                self.skip_spaces();
                let options = if self.peek() == Some('"') {
                    let w = self.word();
                    String::from(w.trim_matches('"'))
                } else {
                    String::new()
                };
                kind = ForKind::Lines(options);
            } else {
                break;
            }
        }
        self.skip_spaces();
        // `%i` (a batch file's `%%i`, after its `%` expansion).
        let mut var = 'i';
        if self.peek() == Some('%') {
            self.pos += 1;
            if self.peek() == Some('%') {
                self.pos += 1;
            }
            if let Some(v) = self.peek() {
                var = v;
                self.pos += 1;
            }
        }
        let _ = self.keyword("in");
        self.skip_spaces();
        let mut set = String::new();
        if self.peek() == Some('(') {
            self.pos += 1;
            let mut depth = 1;
            let mut quoted = false;
            while let Some(c) = self.peek() {
                self.pos += 1;
                match c {
                    '"' | '\'' | '`' => quoted = !quoted,
                    '(' if !quoted => depth += 1,
                    ')' if !quoted => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                set.push(c);
            }
        }
        let _ = self.keyword("do");
        let body = self.body();
        Node::For { kind, var, set: String::from(set.trim()), body: Box::new(body) }
    }
}

fn join(a: Node, b: Node, f: fn(Box<Node>, Box<Node>) -> Node) -> Node {
    match (a, b) {
        (Node::Empty, b) => b,
        (a, Node::Empty) => a,
        (a, b) => f(Box::new(a), Box::new(b)),
    }
}

/// Whether a line leaves a block open: more `(` than `)` that open and close blocks, outside quotes.
pub fn unbalanced(text: &str) -> bool {
    let mut depth: i32 = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut at_command = true;
    for c in text.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '^' if !quoted => escaped = true,
            '"' => quoted = !quoted,
            '(' if !quoted && at_command => depth += 1,
            ')' if !quoted && depth > 0 => depth -= 1,
            _ => {}
        }
        // A `(` opens a block where a command starts: at the start, after an operator, or after IF's or FOR's parts.
        at_command = matches!(c, ' ' | '\t' | '&' | '|' | '(' | '\n') || (at_command && c == '@');
    }
    depth > 0
}
