//! A small backtracking regular expression engine, enough of .NET's for `-match`, `-replace`, `-split` and
//! `Select-String`: literals, `.`, classes (`[a-z]`, `[^...]`, `\d \w \s` and their negations), anchors (`^ $ \b`),
//! groups with alternation and captures (`(a|b)`, `(?:...)`, `(?<name>...)`), and greedy or lazy `* + ? {n,m}`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
enum Node {
    Char(char),
    Any,
    Class(Vec<(char, char)>, bool),
    Start,
    End,
    WordBoundary,
    Group(Box<Alt>, Option<usize>),
    Repeat(Box<Node>, usize, usize, bool),
}

#[derive(Debug, Clone)]
struct Alt(Vec<Vec<Node>>);

pub struct Regex {
    alt: Alt,
    groups: usize,
    pub names: Vec<(String, usize)>,
    fold: bool,
}

/// A match: where each group (0 for the whole) started and ended, in characters.
pub type Captures = Vec<Option<(usize, usize)>>;

struct Parser<'a> {
    p: &'a [char],
    i: usize,
    groups: usize,
    names: Vec<(String, usize)>,
}

fn class_escape(c: char) -> Option<(Vec<(char, char)>, bool)> {
    let digits = alloc::vec![('0', '9')];
    let word = alloc::vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')];
    let space = alloc::vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r'), ('\x0b', '\x0c')];
    Some(match c {
        'd' => (digits, false),
        'D' => (digits, true),
        'w' => (word, false),
        'W' => (word, true),
        's' => (space, false),
        'S' => (space, true),
        _ => return None,
    })
}

fn escaped(c: char) -> char {
    match c {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        'f' => '\x0c',
        'v' => '\x0b',
        '0' => '\0',
        c => c,
    }
}

impl Parser<'_> {
    fn alt(&mut self) -> Result<Alt, String> {
        let mut branches = alloc::vec![Vec::new()];
        while self.i < self.p.len() {
            let c = self.p[self.i];
            match c {
                '|' => {
                    self.i += 1;
                    branches.push(Vec::new());
                }
                ')' => break,
                _ => {
                    let atom = self.atom()?;
                    let atom = self.quantifier(atom)?;
                    if let Some(b) = branches.last_mut() {
                        b.push(atom);
                    }
                }
            }
        }
        Ok(Alt(branches))
    }

    fn quantifier(&mut self, atom: Node) -> Result<Node, String> {
        let Some(&c) = self.p.get(self.i) else { return Ok(atom) };
        let (min, max) = match c {
            '*' => (0, usize::MAX),
            '+' => (1, usize::MAX),
            '?' => (0, 1),
            '{' => {
                let close = self.p[self.i..].iter().position(|&c| c == '}').map(|n| n + self.i);
                let Some(close) = close else { return Ok(atom) };
                let inner: String = self.p[self.i + 1..close].iter().collect();
                let (a, b) = match inner.split_once(',') {
                    Some((a, b)) => (a.trim().parse().ok(), if b.trim().is_empty() { Some(usize::MAX) } else { b.trim().parse().ok() }),
                    None => (inner.trim().parse().ok(), inner.trim().parse().ok()),
                };
                let (Some(a), Some(b)) = (a, b) else { return Ok(atom) };
                self.i = close;
                (a, b)
            }
            _ => return Ok(atom),
        };
        self.i += 1;
        let lazy = self.p.get(self.i) == Some(&'?');
        if lazy {
            self.i += 1;
        }
        Ok(Node::Repeat(Box::new(atom), min, max, !lazy))
    }

    fn atom(&mut self) -> Result<Node, String> {
        let c = self.p[self.i];
        self.i += 1;
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '(' => {
                let mut capture = true;
                let mut name = None;
                if self.p.get(self.i) == Some(&'?') {
                    match self.p.get(self.i + 1) {
                        Some(':') => {
                            capture = false;
                            self.i += 2;
                        }
                        Some('<') | Some('\'') => {
                            let close = if self.p[self.i + 1] == '<' { '>' } else { '\'' };
                            let end = self.p[self.i + 2..].iter().position(|&c| c == close).ok_or("an unclosed group name")? + self.i + 2;
                            name = Some(self.p[self.i + 2..end].iter().collect::<String>());
                            self.i = end + 1;
                        }
                        _ => return Err(String::from("unsupported group")),
                    }
                }
                let index = if capture {
                    self.groups += 1;
                    if let Some(n) = name {
                        self.names.push((n, self.groups));
                    }
                    Some(self.groups)
                } else {
                    None
                };
                let alt = self.alt()?;
                if self.p.get(self.i) != Some(&')') {
                    return Err(String::from("Not enough )'s."));
                }
                self.i += 1;
                Node::Group(Box::new(alt), index)
            }
            '[' => {
                let negate = self.p.get(self.i) == Some(&'^');
                if negate {
                    self.i += 1;
                }
                let mut ranges = Vec::new();
                let mut first = true;
                while self.i < self.p.len() && (self.p[self.i] != ']' || first) {
                    first = false;
                    let mut c = self.p[self.i];
                    self.i += 1;
                    if c == '\\' && self.i < self.p.len() {
                        let e = self.p[self.i];
                        self.i += 1;
                        if let Some((set, false)) = class_escape(e) {
                            ranges.extend(set);
                            continue;
                        }
                        c = escaped(e);
                    }
                    if self.p.get(self.i) == Some(&'-') && self.p.get(self.i + 1).is_some_and(|&n| n != ']') {
                        let mut hi = self.p[self.i + 1];
                        self.i += 2;
                        if hi == '\\' && self.i < self.p.len() {
                            hi = escaped(self.p[self.i]);
                            self.i += 1;
                        }
                        ranges.push((c, hi));
                    } else {
                        ranges.push((c, c));
                    }
                }
                self.i += 1;
                Node::Class(ranges, negate)
            }
            '\\' => {
                let e = *self.p.get(self.i).ok_or("a pattern can't end in \\")?;
                self.i += 1;
                if e == 'b' {
                    Node::WordBoundary
                } else if let Some((set, neg)) = class_escape(e) {
                    Node::Class(set, neg)
                } else {
                    Node::Char(escaped(e))
                }
            }
            c => Node::Char(c),
        })
    }
}

impl Regex {
    pub fn new(pattern: &str, fold: bool) -> Result<Regex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { p: &chars, i: 0, groups: 0, names: Vec::new() };
        let alt = p.alt()?;
        if p.i < chars.len() {
            return Err(String::from("Too many )'s."));
        }
        Ok(Regex { alt, groups: p.groups, names: p.names, fold })
    }

    /// The first match at or after `from`.
    pub fn find_at(&self, text: &[char], from: usize) -> Option<Captures> {
        for start in from..=text.len() {
            let mut caps: Captures = alloc::vec![None; self.groups + 1];
            if let Some(end) = self.alt_at(&self.alt, text, start, &mut caps, &mut |_, _| true) {
                caps[0] = Some((start, end));
                return Some(caps);
            }
        }
        None
    }

    pub fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        self.find_at(&chars, 0).is_some()
    }

    fn same(&self, a: char, b: char) -> bool {
        a == b || (self.fold && a.to_ascii_lowercase() == b.to_ascii_lowercase())
    }

    fn in_class(&self, c: char, ranges: &[(char, char)], negate: bool) -> bool {
        let hit = ranges.iter().any(|&(lo, hi)| (lo <= c && c <= hi) || (self.fold && ((lo <= c.to_ascii_lowercase() && c.to_ascii_lowercase() <= hi) || (lo <= c.to_ascii_uppercase() && c.to_ascii_uppercase() <= hi))));
        hit != negate
    }

    /// Matches `alt` at `i`, then `k` (what comes after it); where the whole match ends.
    fn alt_at(&self, alt: &Alt, t: &[char], i: usize, caps: &mut Captures, k: &mut dyn FnMut(usize, &mut Captures) -> bool) -> Option<usize> {
        for branch in &alt.0 {
            let saved = caps.clone();
            if let Some(end) = self.seq(branch, t, i, caps, k) {
                return Some(end);
            }
            *caps = saved;
        }
        None
    }

    fn seq(&self, nodes: &[Node], t: &[char], i: usize, caps: &mut Captures, k: &mut dyn FnMut(usize, &mut Captures) -> bool) -> Option<usize> {
        let Some((first, rest)) = nodes.split_first() else {
            return if k(i, caps) { Some(i) } else { None };
        };
        let mut next = |j: usize, caps: &mut Captures| -> Option<usize> { self.seq(rest, t, j, caps, k) };
        self.node(first, t, i, caps, &mut next)
    }

    fn node(&self, node: &Node, t: &[char], i: usize, caps: &mut Captures, next: &mut dyn FnMut(usize, &mut Captures) -> Option<usize>) -> Option<usize> {
        match node {
            Node::Char(c) => (i < t.len() && self.same(t[i], *c)).then(|| next(i + 1, caps)).flatten(),
            Node::Any => (i < t.len() && t[i] != '\n').then(|| next(i + 1, caps)).flatten(),
            Node::Class(r, neg) => (i < t.len() && self.in_class(t[i], r, *neg)).then(|| next(i + 1, caps)).flatten(),
            Node::Start => (i == 0).then(|| next(i, caps)).flatten(),
            Node::End => (i == t.len() || (i + 1 == t.len() && t[i] == '\n')).then(|| next(i, caps)).flatten(),
            Node::WordBoundary => {
                let word = |c: char| c.is_alphanumeric() || c == '_';
                let before = i > 0 && word(t[i - 1]);
                let after = i < t.len() && word(t[i]);
                (before != after).then(|| next(i, caps)).flatten()
            }
            Node::Group(alt, index) => {
                let mut result = None;
                let start = i;
                let idx = *index;
                let mut cont = |j: usize, caps: &mut Captures| -> bool {
                    let old = idx.map(|n| caps[n]);
                    if let Some(n) = idx {
                        caps[n] = Some((start, j));
                    }
                    match next(j, caps) {
                        Some(end) => {
                            result = Some(end);
                            true
                        }
                        None => {
                            if let (Some(n), Some(o)) = (idx, old) {
                                caps[n] = o;
                            }
                            false
                        }
                    }
                };
                self.alt_at(alt, t, i, caps, &mut cont)?;
                result
            }
            Node::Repeat(inner, min, max, greedy) => self.repeat(inner, *min, *max, *greedy, t, i, caps, next, 0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(&self, inner: &Node, min: usize, max: usize, greedy: bool, t: &[char], i: usize, caps: &mut Captures, next: &mut dyn FnMut(usize, &mut Captures) -> Option<usize>, count: usize) -> Option<usize> {
        if count >= min && !greedy
            && let Some(end) = next(i, caps)
        {
            return Some(end);
        }
        if count < max {
            let mut more = |j: usize, caps: &mut Captures| -> Option<usize> {
                // An empty match can't repeat forever.
                if j == i && count >= min {
                    return None;
                }
                self.repeat(inner, min, max, greedy, t, j, caps, next, count + 1)
            };
            if let Some(end) = self.node(inner, t, i, caps, &mut more) {
                return Some(end);
            }
        }
        if count >= min && greedy {
            return next(i, caps);
        }
        None
    }
}

/// `-replace`: every match replaced, `$1`, `${name}` and `$0` (or `$&`) taken from it.
pub fn replace(re: &Regex, text: &str, with: &str) -> String {
    let t: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at <= t.len() {
        let Some(caps) = re.find_at(&t, at) else { break };
        let (s, e) = caps[0].unwrap_or((at, at));
        out.extend(&t[at..s]);
        let group = |n: usize| caps.get(n).copied().flatten().map(|(a, b)| t[a..b].iter().collect::<String>()).unwrap_or_default();
        let w: Vec<char> = with.chars().collect();
        let mut i = 0;
        while i < w.len() {
            if w[i] == '$' && i + 1 < w.len() {
                if w[i + 1].is_ascii_digit() {
                    let mut j = i + 1;
                    while j < w.len() && w[j].is_ascii_digit() {
                        j += 1;
                    }
                    let n: usize = w[i + 1..j].iter().collect::<String>().parse().unwrap_or(0);
                    out.push_str(&group(n));
                    i = j;
                    continue;
                }
                if w[i + 1] == '&' {
                    out.push_str(&group(0));
                    i += 2;
                    continue;
                }
                if w[i + 1] == '$' {
                    out.push('$');
                    i += 2;
                    continue;
                }
                if w[i + 1] == '{'
                    && let Some(close) = w[i..].iter().position(|&c| c == '}')
                {
                    let name: String = w[i + 2..i + close].iter().collect();
                    let n = re.names.iter().find(|(k, _)| k.eq_ignore_ascii_case(&name)).map(|(_, n)| *n).or_else(|| name.parse().ok()).unwrap_or(usize::MAX);
                    out.push_str(&group(n));
                    i += close + 1;
                    continue;
                }
            }
            out.push(w[i]);
            i += 1;
        }
        if e == s {
            if s < t.len() {
                out.push(t[s]);
            }
            at = s + 1;
        } else {
            at = e;
        }
    }
    if at < t.len() {
        out.extend(&t[at..]);
    }
    out
}

/// `-split`: the text between matches.
pub fn split(re: &Regex, text: &str) -> Vec<String> {
    let t: Vec<char> = text.chars().collect();
    let mut parts = Vec::new();
    let mut at = 0;
    let mut from = 0;
    while from <= t.len() {
        let Some(caps) = re.find_at(&t, from) else { break };
        let (s, e) = caps[0].unwrap_or((from, from));
        if e == s {
            from = s + 1;
            continue;
        }
        parts.push(t[at..s].iter().collect());
        at = e;
        from = e;
    }
    parts.push(t[at..].iter().collect());
    parts
}
