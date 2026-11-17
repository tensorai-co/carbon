//! Substitution and quoting in a Carbonfile's words, as BuildKit's `shell/lex.go` does them: `$VAR`, `${VAR}` and its
//! modifiers (`:-` `-` `:+` `+` `:?` `?` `#` `##` `%` `%%` `/` `//`), single and double quotes, and the escape character.
//! [`Lexer::word`] gives one word (ENV's and LABEL's values, WORKDIR, USER), [`Lexer::words`] splits on whitespace that
//! isn't quoted (EXPOSE, COPY's sources and so on).

/// What a variable is, when it's looked up: `None` when it isn't set.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The lexer, with the file's escape character (`\`, or `` ` `` with `# escape=`).
#[derive(Debug, Clone, Copy)]
pub struct Lexer {
    pub escape: char,
    /// Keep quotes in the result (used to find here-documents' markers).
    pub raw_quotes: bool,
    /// Keep escape characters in the result.
    pub raw_escapes: bool,
    /// Quotes are ordinary characters: substitution only, as in a here-document's lines.
    pub skip_quotes: bool,
}

impl Default for Lexer {
    fn default() -> Lexer {
        Lexer::new('\\')
    }
}

/// Words as they're made: whitespace from a variable's value splits, quoted whitespace doesn't.
#[derive(Default)]
struct Words {
    words: Vec<String>,
    word: String,
    in_word: bool,
}

impl Words {
    fn add_char(&mut self, c: char) {
        if c.is_whitespace() {
            if self.in_word && !self.word.is_empty() {
                self.words.push(std::mem::take(&mut self.word));
                self.in_word = false;
            }
        } else {
            self.add_raw_char(c);
        }
    }
    fn add_raw_char(&mut self, c: char) {
        self.word.push(c);
        self.in_word = true;
    }
    fn add_string(&mut self, s: &str) {
        s.chars().for_each(|c| self.add_char(c));
    }
    fn add_raw_string(&mut self, s: &str) {
        self.word.push_str(s);
        self.in_word = true;
    }
    fn finish(mut self) -> Vec<String> {
        if !self.word.is_empty() {
            self.words.push(self.word);
        }
        self.words
    }
}

struct Scan<'a> {
    chars: Vec<char>,
    at: usize,
    lex: Lexer,
    env: Lookup<'a>,
}

impl Scan<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }
    fn next(&mut self) -> Option<char> {
        let c = self.peek();
        self.at += usize::from(c.is_some());
        c
    }

    /// Up to `stop` (taken, not kept), or the end if `stop` is empty: the text, and its words.
    fn until(&mut self, stop: &[char], raw_escapes: bool) -> Result<(String, Vec<String>), String> {
        let mut out = String::new();
        let mut words = Words::default();
        while let Some(c) = self.peek() {
            if stop.contains(&c) {
                self.next();
                return Ok((out, words.finish()));
            }
            match c {
                '$' => {
                    let s = self.dollar()?;
                    out.push_str(&s);
                    words.add_string(&s);
                }
                '\'' if !self.lex.skip_quotes => {
                    let s = self.single_quote()?;
                    out.push_str(&s);
                    words.add_raw_string(&s);
                }
                '"' if !self.lex.skip_quotes => {
                    let s = self.double_quote()?;
                    out.push_str(&s);
                    words.add_raw_string(&s);
                }
                _ => {
                    self.next();
                    let mut c = c;
                    if c == self.lex.escape {
                        if raw_escapes || self.lex.raw_escapes {
                            out.push(c);
                            words.add_raw_char(c);
                        }
                        match self.next() {
                            Some(n) => c = n,
                            None => break,
                        }
                        words.add_raw_char(c);
                    } else {
                        words.add_char(c);
                    }
                    out.push(c);
                }
            }
        }
        if let Some(stop) = stop.first() {
            return Err(format!("unexpected end of statement while looking for matching {stop}"));
        }
        Ok((out, words.finish()))
    }

    fn single_quote(&mut self) -> Result<String, String> {
        // Everything to the next ', as it is.
        let mut out = String::new();
        self.next();
        if self.lex.raw_quotes {
            out.push('\'');
        }
        loop {
            match self.next() {
                None => return Err("unexpected end of statement while looking for matching single-quote".into()),
                Some('\'') => break,
                Some(c) => out.push(c),
            }
        }
        if self.lex.raw_quotes {
            out.push('\'');
        }
        Ok(out)
    }

    fn double_quote(&mut self) -> Result<String, String> {
        // Everything to the next ", but `$` expands and the escape character escapes ", $ and itself.
        let mut out = String::new();
        self.next();
        if self.lex.raw_quotes {
            out.push('"');
        }
        loop {
            let Some(c) = self.peek() else {
                return Err("unexpected end of statement while looking for matching double-quote".into());
            };
            if c == '"' {
                self.next();
                break;
            }
            if c == '$' {
                out.push_str(&self.dollar()?);
                continue;
            }
            self.next();
            let mut c = c;
            if c == self.lex.escape {
                if self.lex.raw_escapes {
                    out.push(c);
                }
                match self.peek() {
                    None => continue,
                    Some(n) if n == '"' || n == '$' || n == self.lex.escape => {
                        self.next();
                        c = n;
                    }
                    Some(_) => {}
                }
            }
            out.push(c);
        }
        if self.lex.raw_quotes {
            out.push('"');
        }
        Ok(out)
    }

    fn name(&mut self) -> String {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if name.is_empty() && c.is_ascii_digit() {
                while let Some(d) = self.peek().filter(char::is_ascii_digit) {
                    self.next();
                    name.push(d);
                }
                return name;
            }
            if name.is_empty() && matches!(c, '@' | '*' | '#' | '?' | '-' | '$' | '!' | '0') {
                self.next();
                return c.to_string();
            }
            if !c.is_alphanumeric() && c != '_' {
                break;
            }
            self.next();
            name.push(c);
        }
        name
    }

    fn dollar(&mut self) -> Result<String, String> {
        self.next();
        if self.peek() != Some('{') {
            let name = self.name();
            if name.is_empty() {
                return Ok("$".into());
            }
            return Ok((self.env)(&name).unwrap_or_default());
        }
        self.next();
        match self.peek() {
            None => return Err("syntax error: missing '}'".into()),
            Some('{' | '}' | ':') => return Err("syntax error: bad substitution".into()),
            _ => {}
        }
        let name = self.name();
        let Some(mut c) = self.next() else { return Err("syntax error: missing '}'".into()) };
        let mut modifier = c.to_string();
        let mut null_is_unset = false;
        match c {
            '}' => Ok((self.env)(&name).unwrap_or_default()),
            ':' | '+' | '-' | '?' | '#' | '%' => {
                if c == ':' {
                    null_is_unset = true;
                    c = self.next().ok_or("syntax error: missing '}'")?;
                    modifier.push(c);
                    if !matches!(c, '+' | '-' | '?') {
                        return Err(format!("unsupported modifier ({modifier}) in substitution"));
                    }
                }
                let raw = c == '#' || c == '%';
                let word = match self.until(&['}'], raw) {
                    Ok((word, _)) => word,
                    Err(_) if self.peek().is_none() => return Err("syntax error: missing '}'".into()),
                    Err(e) => return Err(e),
                };
                let value = (self.env)(&name);
                let empty = value.as_deref().is_none_or(str::is_empty);
                match c {
                    '-' => Ok(if value.is_none() || (null_is_unset && empty) { word } else { value.unwrap_or_default() }),
                    '+' => Ok(if value.is_none() || (null_is_unset && empty) { String::new() } else { word }),
                    '?' => {
                        if value.is_none() {
                            return Err(format!("{name}: {}", if word.is_empty() { "is not allowed to be unset" } else { &word }));
                        }
                        if null_is_unset && empty {
                            return Err(format!("{name}: {}", if word.is_empty() { "is not allowed to be empty" } else { &word }));
                        }
                        Ok(value.unwrap_or_default())
                    }
                    _ => {
                        let value = value.unwrap_or_default();
                        let (greedy, pattern) = match word.strip_prefix(c) {
                            Some(rest) => (true, rest.to_string()),
                            None => (false, word),
                        };
                        Ok(if c == '%' { trim_suffix(&value, &pattern, greedy, self.lex.escape) } else { trim_prefix(&value, &pattern, greedy, self.lex.escape) })
                    }
                }
            }
            '/' => {
                let all = self.peek() == Some('/');
                if all {
                    self.next();
                }
                let pattern = match self.until(&['/'], true) {
                    Ok((p, _)) => p,
                    Err(_) if self.peek().is_none() => return Err("syntax error: missing '/' in ${}".into()),
                    Err(e) => return Err(e),
                };
                let replacement = match self.until(&['}'], true) {
                    Ok((r, _)) => r,
                    Err(_) if self.peek().is_none() => return Err("syntax error: missing '}'".into()),
                    Err(e) => return Err(e),
                };
                let value = (self.env)(&name).unwrap_or_default();
                Ok(replace(&value, &pattern, &replacement, all, self.lex.escape))
            }
            _ => Err(format!("unsupported modifier ({modifier}) in substitution")),
        }
    }
}

impl Lexer {
    pub fn new(escape: char) -> Lexer {
        Lexer { escape, raw_quotes: false, raw_escapes: false, skip_quotes: false }
    }

    /// One word: substitution done, quotes and escapes taken out.
    pub fn word(&self, text: &str, env: Lookup<'_>) -> Result<String, String> {
        let mut scan = Scan { chars: text.chars().collect(), at: 0, lex: *self, env };
        scan.until(&[], false).map(|(s, _)| s)
    }

    /// Words: as [`Lexer::word`], split where whitespace isn't quoted or escaped (including whitespace a variable's
    /// value brings).
    pub fn words(&self, text: &str, env: Lookup<'_>) -> Result<Vec<String>, String> {
        let mut scan = Scan { chars: text.chars().collect(), at: 0, lex: *self, env };
        scan.until(&[], false).map(|(_, w)| w)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Shell patterns, for `#`, `%` and `/`

/// Whether all of `text` matches the shell pattern `pattern`: `*`, `?`, `[...]` (with `!` or `^`), and escapes.
pub(crate) fn glob(pattern: &[char], text: &[char], escape: char, slash_special: bool) -> bool {
    let (mut p, mut t) = (0, 0);
    let (mut star_p, mut star_t) = (None, 0);
    while t < text.len() {
        if p < pattern.len() {
            match pattern[p] {
                '*' => {
                    star_p = Some(p);
                    star_t = t;
                    p += 1;
                    continue;
                }
                '?' if !(slash_special && text[t] == '/') => {
                    p += 1;
                    t += 1;
                    continue;
                }
                '[' => {
                    if let Some((matched, next)) = class(pattern, p, text[t], escape)
                        && matched
                        && !(slash_special && text[t] == '/')
                    {
                        p = next;
                        t += 1;
                        continue;
                    }
                }
                c => {
                    let (lit, next) = if c == escape && p + 1 < pattern.len() { (pattern[p + 1], p + 2) } else { (c, p + 1) };
                    if lit == text[t] && !(c == '?' || c == '[') {
                        p = next;
                        t += 1;
                        continue;
                    }
                }
            }
        }
        // Back to the last star, which takes one more character (but not a `/` where slashes are special).
        match star_p {
            Some(sp) if !(slash_special && text[star_t] == '/') => {
                star_t += 1;
                t = star_t;
                p = sp + 1;
            }
            _ => return false,
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// A `[...]` class at `pattern[at]`: whether `c` is in it, and where the pattern goes on. None if it isn't closed.
fn class(pattern: &[char], at: usize, c: char, escape: char) -> Option<(bool, usize)> {
    let mut i = at + 1;
    let negate = matches!(pattern.get(i), Some('!' | '^'));
    if negate {
        i += 1;
    }
    let mut matched = false;
    let mut first = true;
    while i < pattern.len() {
        let mut lo = pattern[i];
        if lo == ']' && !first {
            return Some((matched != negate, i + 1));
        }
        first = false;
        if lo == escape && i + 1 < pattern.len() {
            i += 1;
            lo = pattern[i];
        }
        let mut hi = lo;
        if pattern.get(i + 1) == Some(&'-') && pattern.get(i + 2).is_some_and(|&h| h != ']') {
            hi = pattern[i + 2];
            i += 2;
        }
        if lo <= c && c <= hi {
            matched = true;
        }
        i += 1;
    }
    None
}

fn trim_prefix(value: &str, pattern: &str, greedy: bool, escape: char) -> String {
    let (v, p): (Vec<char>, Vec<char>) = (value.chars().collect(), pattern.chars().collect());
    let lens: Box<dyn Iterator<Item = usize>> = if greedy { Box::new((0..=v.len()).rev()) } else { Box::new(0..=v.len()) };
    for n in lens {
        if glob(&p, &v[..n], escape, false) {
            return v[n..].iter().collect();
        }
    }
    value.to_string()
}

fn trim_suffix(value: &str, pattern: &str, greedy: bool, escape: char) -> String {
    let (v, p): (Vec<char>, Vec<char>) = (value.chars().collect(), pattern.chars().collect());
    let starts: Box<dyn Iterator<Item = usize>> = if greedy { Box::new(0..=v.len()) } else { Box::new((0..=v.len()).rev()) };
    for start in starts {
        if glob(&p, &v[start..], escape, false) {
            return v[..start].iter().collect();
        }
    }
    value.to_string()
}

/// `${VAR/pattern/replacement}`: the longest match, leftmost first; every one with `//`.
fn replace(value: &str, pattern: &str, replacement: &str, all: bool, escape: char) -> String {
    let (v, p): (Vec<char>, Vec<char>) = (value.chars().collect(), pattern.chars().collect());
    if p.is_empty() {
        return value.to_string();
    }
    let mut out = String::new();
    let mut i = 0;
    let mut replaced = false;
    while i < v.len() {
        let found = if replaced && !all { None } else { (i + 1..=v.len()).rev().find(|&end| glob(&p, &v[i..end], escape, false)) };
        match found {
            Some(end) => {
                out.push_str(replacement);
                i = end;
                replaced = true;
            }
            None => {
                out.push(v[i]);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        match name {
            "PWD" => Some("/home".into()),
            "SHELL" => Some("bash".into()),
            "EMPTY" => Some(String::new()),
            "FILE" => Some("archive.tar.gz".into()),
            "SPACED" => Some("a  b".into()),
            _ => None,
        }
    }

    fn word(text: &str) -> Result<String, String> {
        Lexer::default().word(text, &env)
    }

    /// Cases from BuildKit's `shell/envVarTest`, and the modifiers' tests in `lex_test.go`.
    #[test]
    fn words_as_buildkit_makes_them() {
        for (text, want) in [
            ("hello", "hello"),
            ("he'll'o", "hello"),
            ("he'llo", "error"),
            ("he\\'llo", "he'llo"),
            ("he\\\\'ll'o", "he\\llo"),
            ("abc\\", "abc"),
            ("abc\\\\", "abc\\"),
            ("\"abc\\\\\"", "abc\\"),
            ("'abc\\\\'", "abc\\\\"),
            ("\"abc\\\\\\\"\"", "abc\\\""),
            ("\"a\\b\"", "a\\b"),
            ("$PWD", "/home"),
            ("\\$PWD", "$PWD"),
            ("'$PWD'", "$PWD"),
            ("\"$PWD\"", "/home"),
            ("\"\\$PWD\"", "$PWD"),
            ("${PWD}", "/home"),
            ("${PWD}x", "/homex"),
            ("$PWDx", ""),
            ("he$1x", "hex"),
            ("he$.x", "he$.x"),
            ("he$pwd.", "he."),
            ("he${PWD", "error"),
            ("${}", "error"),
            ("${:}", "error"),
            ("${PWD:-x}", "/home"),
            ("${UNSET:-x}", "x"),
            ("${EMPTY:-x}", "x"),
            ("${EMPTY-x}", ""),
            ("${UNSET-x}", "x"),
            ("${PWD:+x}", "x"),
            ("${UNSET:+x}", ""),
            ("${EMPTY+x}", "x"),
            ("${EMPTY:+x}", ""),
            ("${UNSET:-${PWD}}", "/home"),
            ("${UNSET:-'quoted ${PWD}'}", "quoted ${PWD}"),
            ("${PWD:?}", "/home"),
            ("${UNSET:?}", "error"),
            ("${UNSET?oops}", "error"),
            ("${EMPTY?}", ""),
            ("${FILE#*.}", "tar.gz"),
            ("${FILE##*.}", "gz"),
            ("${FILE%.*}", "archive.tar"),
            ("${FILE%%.*}", "archive"),
            ("${FILE/tar/zip}", "archive.zip.gz"),
            ("${FILE//a/A}", "Archive.tAr.gz"),
            ("${FILE/a*./X}", "Xgz"),
            ("${FILE:#x}", "error"),
        ] {
            let got = word(text).unwrap_or_else(|_| "error".into());
            assert_eq!(got, want, "{text}");
        }
    }

    #[test]
    fn words_split_where_not_quoted() {
        let words = |t: &str| Lexer::default().words(t, &env).unwrap();
        assert_eq!(words("a b  c"), ["a", "b", "c"]);
        assert_eq!(words("'a b' \"c d\" e\\ f"), ["a b", "c d", "e f"]);
        assert_eq!(words("$SPACED x"), ["a", "b", "x"]);
        assert_eq!(words("\"$SPACED\" x"), ["a  b", "x"]);
        assert!(words("").is_empty());
    }

    #[test]
    fn heredoc_lines_keep_their_quotes() {
        let lex = Lexer { skip_quotes: true, raw_escapes: true, ..Lexer::default() };
        assert_eq!(lex.word("say \"$PWD\" 'x'\n", &env).unwrap(), "say \"/home\" 'x'\n");
    }

    #[test]
    fn backtick_escapes_in_windows_files() {
        let lex = Lexer::new('`');
        assert_eq!(lex.word("C:\\app\\bin", &env).unwrap(), "C:\\app\\bin");
        assert_eq!(lex.word("a`$PWD", &env).unwrap(), "a$PWD");
    }
}
