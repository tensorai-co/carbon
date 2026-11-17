//! A Carbonfile's grammar, which is the Dockerfile's, as BuildKit's `frontend/dockerfile/parser` reads it: parser
//! directives, comments, the escape character and line continuations, instructions in any case, builder flags, each
//! instruction's argument forms (JSON or shell, words, `name=value` pairs, HEALTHCHECK's, ONBUILD's), and
//! here-documents. Then the instructions are grouped into stages, as `instructions.Parse` groups them.

use super::lex::Lexer;

/// Where and why a file couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// From 1; 0 for the file as a whole.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 { write!(f, "{}", self.message) } else { write!(f, "line {}: {}", self.line, self.message) }
    }
}

fn error<T>(line: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError { line, message: message.into() })
}

/// The parser directives at the top of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directives {
    /// `# syntax=`: the frontend asked for.
    pub syntax: Option<String>,
    /// `# escape=`: `\` (the default) or `` ` ``.
    pub escape: char,
    /// `# check=`: which build checks to run, and whether they fail the build.
    pub check: Option<String>,
}

impl Default for Directives {
    fn default() -> Directives {
        Directives { syntax: None, escape: '\\', check: None }
    }
}

/// An instruction's arguments, in the form its instruction reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Args {
    /// Nothing after the instruction.
    None,
    /// The exec form: a JSON array of strings (RUN, CMD, ENTRYPOINT, SHELL, COPY, ADD, VOLUME).
    Json(Vec<String>),
    /// The rest of the line, as written (RUN's, CMD's and ENTRYPOINT's shell form, WORKDIR, USER, STOPSIGNAL,
    /// MAINTAINER).
    Text(String),
    /// Words split on whitespace (FROM, EXPOSE, and COPY's, ADD's and VOLUME's shell form), or on whitespace that isn't
    /// quoted (ARG). Quotes are still in them: substitution takes them out.
    Words(Vec<String>),
    /// `name=value` pairs (ENV, LABEL), quotes still in, or the old form's one `name value`.
    Pairs(Vec<(String, String)>),
    /// HEALTHCHECK: `NONE`, or `CMD` and its command.
    Health(Option<Box<Args>>),
    /// ONBUILD: the instruction it holds.
    OnBuild(Box<Instruction>),
}

/// A here-document: `<<EOF` (or `<<-EOF`, `<<"EOF"`, `3<<EOF`) and its lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heredoc {
    pub name: String,
    /// Its lines, each with its line end.
    pub content: String,
    /// Substitution in its lines: off when its marker is quoted.
    pub expand: bool,
    /// `<<-`: leading tabs taken off each line.
    pub chomp: bool,
    /// The descriptor before `<<`, if one was written.
    pub fd: Option<u32>,
}

/// One instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// Its first and last line, from 1.
    pub line: usize,
    pub end_line: usize,
    /// Lowercase: `run`, `copy`, `from`.
    pub keyword: String,
    /// Its line as written, continuations joined (without here-documents' lines).
    pub original: String,
    /// `--name=value` flags, as written, without their `--`.
    pub flags: Vec<String>,
    pub args: Args,
    pub heredocs: Vec<Heredoc>,
    /// The comment lines just before it.
    pub comments: Vec<String>,
}

impl Instruction {
    /// Its name as the reference writes it: `RUN`, `COPY`.
    pub fn name(&self) -> String {
        self.keyword.to_ascii_uppercase()
    }

    /// The value of a flag (`--from=build` is `flag("from")`), the last if it's given more than once.
    pub fn flag<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        self.flags(name).last()
    }

    /// Every value of a flag given more than once (`--mount`, `--exclude`).
    pub fn flags<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.flags.iter().filter_map(move |f| {
            let (k, v) = f.split_once('=').unwrap_or((f.as_str(), "true"));
            (k == name).then_some(v)
        })
    }

    /// Whether a boolean flag is on (`--link`, `--link=true`).
    pub fn flag_on(&self, name: &str) -> bool {
        self.flag(name).is_some_and(|v| v != "false")
    }
}

/// A stage: a `FROM` and what follows it, to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    /// What it starts from, as written (substitution happens when it's built): `debian:13`, `${BASE}`, `scratch`.
    pub base: String,
    /// `AS name`, lowercased as Docker keeps it.
    pub name: Option<String>,
    /// `FROM --platform=`.
    pub platform: Option<String>,
    pub from: Instruction,
    pub instructions: Vec<Instruction>,
}

/// A parsed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carbonfile {
    pub directives: Directives,
    /// The ARGs before the first FROM, which FROMs (and stages that declare them again) can use.
    pub meta_args: Vec<Instruction>,
    pub stages: Vec<Stage>,
    /// Every instruction, in order, as the grammar reads them before stages.
    pub instructions: Vec<Instruction>,
    /// What the file does that works but shouldn't: an empty continuation line, for one.
    pub warnings: Vec<ParseError>,
}

const KNOWN: &[&str] = &[
    "add", "arg", "cmd", "copy", "entrypoint", "env", "expose", "from", "healthcheck", "label", "maintainer", "onbuild", "run", "shell", "stopsignal", "user", "volume",
    "workdir",
];

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0b' | '\x0c')
}

fn trim_left(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

fn is_comment(line: &str) -> bool {
    trim_left(line).starts_with('#')
}

/// `^#\s*([a-zA-Z][a-zA-Z0-9]*)\s*=\s*(.+?)\s*$`
fn directive(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix('#')?.trim_start_matches(is_space);
    let key_len = rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len());
    let key = &rest[..key_len];
    if !key.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let value = rest[key_len..].trim_start_matches(is_space).strip_prefix('=')?.trim_matches(is_space);
    (!value.is_empty()).then(|| (key.to_ascii_lowercase(), value.to_string()))
}

/// A line without the escape character (and trailing blanks) that continues it, and whether it ended there.
fn continuation(line: &str, escape: char) -> (String, bool) {
    let trimmed = line.trim_end_matches([' ', '\t']);
    match trimmed.strip_suffix(escape) {
        Some(rest) => (rest.to_string(), false),
        None => (line.to_string(), true),
    }
}

/// Builder flags at the start of an instruction's arguments (`--from=build --chown="a b"`), and what's after them.
fn builder_flags(line: &str) -> (Vec<String>, String) {
    #[derive(PartialEq)]
    enum Phase {
        Spaces,
        Word,
        Quote,
    }
    let chars: Vec<char> = line.chars().collect();
    let byte_at = |pos: usize| chars[..pos].iter().map(|c| c.len_utf8()).sum::<usize>();
    let (mut words, mut word, mut phase, mut quote, mut blank_ok) = (Vec::new(), String::new(), Phase::Spaces, '\0', false);
    let mut pos = 0;
    while pos <= chars.len() {
        let end = pos == chars.len();
        let c = chars.get(pos).copied().unwrap_or('\0');
        if phase == Phase::Spaces {
            if end {
                break;
            }
            if c.is_whitespace() {
                pos += 1;
                continue;
            }
            if c != '-' || chars.get(pos + 1) != Some(&'-') {
                return (strip_dashes(words), line[byte_at(pos)..].to_string());
            }
            phase = Phase::Word;
        }
        if end {
            if word != "--" && (blank_ok || !word.is_empty()) {
                words.push(word);
            }
            break;
        }
        if phase == Phase::Word {
            if c.is_whitespace() {
                phase = Phase::Spaces;
                if word == "--" {
                    return (strip_dashes(words), line[byte_at(pos)..].to_string());
                }
                if blank_ok || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                blank_ok = false;
                pos += 1;
                continue;
            }
            if c == '\'' || c == '"' {
                quote = c;
                blank_ok = true;
                phase = Phase::Quote;
                pos += 1;
                continue;
            }
            let mut c = c;
            if c == '\\' {
                if pos + 1 == chars.len() {
                    pos += 1;
                    continue;
                }
                pos += 1;
                c = chars[pos];
            }
            word.push(c);
            pos += 1;
            continue;
        }
        // In quotes.
        if c == quote {
            phase = Phase::Word;
            pos += 1;
            continue;
        }
        let mut c = c;
        if c == '\\' {
            if pos + 1 == chars.len() {
                phase = Phase::Word;
                pos += 1;
                continue;
            }
            pos += 1;
            c = chars[pos];
        }
        word.push(c);
        pos += 1;
    }
    (strip_dashes(words), String::new())
}

fn strip_dashes(words: Vec<String>) -> Vec<String> {
    words.into_iter().map(|w| w.strip_prefix("--").map(String::from).unwrap_or(w)).collect()
}

/// Words split on whitespace that isn't quoted, with quotes and escapes kept (ENV's, LABEL's and ARG's).
fn parse_words(rest: &str, escape: char) -> Vec<String> {
    #[derive(PartialEq)]
    enum Phase {
        Spaces,
        Word,
        Quote,
    }
    let chars: Vec<char> = rest.chars().collect();
    let (mut words, mut word, mut phase, mut quote, mut blank_ok) = (Vec::new(), String::new(), Phase::Spaces, '\0', false);
    let mut pos = 0;
    while pos <= chars.len() {
        let end = pos == chars.len();
        let c = chars.get(pos).copied().unwrap_or('\0');
        if phase == Phase::Spaces {
            if end {
                break;
            }
            if c.is_whitespace() {
                pos += 1;
                continue;
            }
            phase = Phase::Word;
        }
        if end {
            if blank_ok || !word.is_empty() {
                words.push(word);
            }
            break;
        }
        if phase == Phase::Word {
            if c.is_whitespace() {
                phase = Phase::Spaces;
                if blank_ok || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                blank_ok = false;
                pos += 1;
                continue;
            }
            if c == '\'' || c == '"' {
                quote = c;
                blank_ok = true;
                phase = Phase::Quote;
            }
            let mut c = c;
            if c == escape {
                if pos + 1 == chars.len() {
                    pos += 1;
                    continue;
                }
                word.push(c);
                pos += 1;
                c = chars[pos];
            }
            word.push(c);
            pos += 1;
            continue;
        }
        if c == quote {
            phase = Phase::Word;
        }
        let mut c = c;
        if c == escape && quote != '\'' {
            if pos + 1 == chars.len() {
                phase = Phase::Word;
                pos += 1;
                continue;
            }
            pos += 1;
            word.push(c);
            c = chars[pos];
        }
        word.push(c);
        pos += 1;
    }
    words
}

fn split_whitespace(rest: &str) -> Vec<String> {
    rest.split(is_space).filter(|w| !w.is_empty()).map(String::from).collect()
}

/// A JSON array of strings, if `rest` is one; an error if it's an array with something else in it.
fn json_array(rest: &str, line: usize) -> Result<Option<Vec<String>>, ParseError> {
    let rest = trim_left(rest);
    if !rest.starts_with('[') {
        return Ok(None);
    }
    let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<serde_json::Value>();
    let value = match stream.next() {
        Some(Ok(value)) => value,
        _ => return Ok(None),
    };
    let Some(items) = value.as_array() else { return Ok(None) };
    let mut out = Vec::new();
    for item in items {
        match item.as_str() {
            Some(s) => out.push(s.to_string()),
            None => return error(line, "only strings are allowed in the exec form (a JSON array)"),
        }
    }
    Ok(Some(out))
}

fn maybe_json(rest: &str, line: usize) -> Result<Args, ParseError> {
    if rest.is_empty() {
        return Ok(Args::None);
    }
    Ok(match json_array(rest, line)? {
        Some(list) => Args::Json(list),
        None => Args::Text(rest.to_string()),
    })
}

fn maybe_json_list(rest: &str, line: usize) -> Result<Args, ParseError> {
    if rest.is_empty() {
        return Ok(Args::None);
    }
    Ok(match json_array(rest, line)? {
        Some(list) => Args::Json(list),
        None => Args::Words(split_whitespace(rest)),
    })
}

fn name_values(rest: &str, keyword: &str, escape: char, line: usize) -> Result<Args, ParseError> {
    let words = parse_words(rest, escape);
    if words.is_empty() {
        return error(line, format!("{} requires at least one argument", keyword.to_ascii_uppercase()));
    }
    if !words[0].contains('=') {
        // The old form: `ENV name value with spaces`.
        let mut parts = rest.trim_start_matches(is_space).splitn(2, is_space);
        let (name, value) = (parts.next().unwrap_or(""), parts.next().map(|v| v.trim_start_matches(is_space)).unwrap_or(""));
        if value.is_empty() {
            return error(line, format!("{} must have two arguments", keyword.to_ascii_uppercase()));
        }
        return Ok(Args::Pairs(vec![(name.to_string(), value.to_string())]));
    }
    let mut pairs = Vec::new();
    for word in words {
        let Some((name, value)) = word.split_once('=') else {
            return error(line, format!("Syntax error - can't find = in {word:?}. Must be of the form: name=value"));
        };
        if name.is_empty() {
            return error(line, format!("{} names can not be blank", keyword.to_ascii_uppercase()));
        }
        pairs.push((name.to_string(), value.to_string()));
    }
    Ok(Args::Pairs(pairs))
}

/// One instruction from its joined line.
fn instruction(line_text: &str, at: usize, escape: char, comments: Vec<String>) -> Result<Instruction, ParseError> {
    let trimmed = line_text.trim_matches(is_space);
    let (word, rest) = match trimmed.find(is_space) {
        Some(i) => (&trimmed[..i], trimmed[i..].trim_start_matches(is_space)),
        None => (trimmed, ""),
    };
    let keyword = word.to_ascii_lowercase();
    let (flags, rest) = if rest.is_empty() { (Vec::new(), String::new()) } else { builder_flags(rest) };
    let rest = rest.trim_start_matches(is_space);
    let args = match keyword.as_str() {
        "run" | "cmd" | "entrypoint" | "shell" => maybe_json(rest, at)?,
        "copy" | "add" | "volume" => maybe_json_list(rest, at)?,
        "from" | "expose" => Args::Words(split_whitespace(rest)),
        "env" | "label" => name_values(rest, &keyword, escape, at)?,
        "arg" => {
            let words = parse_words(rest, escape);
            if words.is_empty() {
                return error(at, "ARG requires at least one argument");
            }
            Args::Words(words)
        }
        "workdir" | "user" | "stopsignal" | "maintainer" => {
            if rest.is_empty() { Args::None } else { Args::Text(rest.to_string()) }
        }
        "healthcheck" => {
            let mut parts = rest.splitn(2, is_space);
            let kind = parts.next().unwrap_or("").to_ascii_uppercase();
            let command = parts.next().unwrap_or("").trim_start_matches(is_space);
            match kind.as_str() {
                "NONE" if command.is_empty() => Args::Health(None),
                "NONE" => return error(at, "HEALTHCHECK NONE takes no arguments"),
                "CMD" if command.is_empty() => return error(at, "missing command after HEALTHCHECK CMD"),
                "CMD" => Args::Health(Some(Box::new(maybe_json(command, at)?))),
                "" => return error(at, "HEALTHCHECK requires an argument"),
                other => return error(at, format!("Unknown type {other:?} in HEALTHCHECK (try CMD)")),
            }
        }
        "onbuild" => {
            if rest.is_empty() {
                return error(at, "ONBUILD requires at least one argument");
            }
            let inner = instruction(rest, at, escape, Vec::new())?;
            match inner.keyword.as_str() {
                "onbuild" => return error(at, "Chaining ONBUILD via `ONBUILD ONBUILD` isn't allowed"),
                "from" | "maintainer" => return error(at, format!("{} isn't allowed as an ONBUILD trigger", inner.name())),
                _ => {}
            }
            Args::OnBuild(Box::new(inner))
        }
        _ => {
            if rest.is_empty() { Args::None } else { Args::Text(rest.to_string()) }
        }
    };
    Ok(Instruction { line: at, end_line: at, keyword, original: trimmed.to_string(), flags, args, heredocs: Vec::new(), comments })
}

/// The here-document markers in an instruction's line, in order.
fn heredoc_markers(line: &str) -> Vec<Heredoc> {
    let lex = Lexer { raw_quotes: true, raw_escapes: true, ..Lexer::default() };
    let words = lex.words(line, &|_| None).unwrap_or_default();
    let mut out = Vec::new();
    for word in words {
        let digits = word.find(|c: char| !c.is_ascii_digit()).unwrap_or(word.len());
        let Some(rest) = word[digits..].strip_prefix("<<") else { continue };
        let (chomp, rest) = match rest.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, rest),
        };
        if rest.is_empty() || rest.contains('<') {
            continue;
        }
        let quoted = rest.contains(['"', '\'']);
        let name = Lexer::default().word(rest, &|_| None).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let fd = (digits > 0).then(|| word[..digits].parse().ok()).flatten();
        out.push(Heredoc { name, content: String::new(), expand: !quoted, chomp, fd });
    }
    out
}

impl Carbonfile {
    /// Reads a file: its directives, its instructions, and its stages.
    pub fn parse(text: &str) -> Result<Carbonfile, ParseError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        // Lines with their ends, so here-documents keep them.
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let strip_end = |l: &str| l.strip_suffix('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).unwrap_or(l).to_string();
        let mut directives = Directives::default();
        let (mut directives_done, mut seen) = (false, Vec::<String>::new());
        let mut instructions = Vec::new();
        let mut warnings = Vec::new();
        let mut comments: Vec<String> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let raw = strip_end(lines[i]);
            i += 1;
            let start = i;
            if !directives_done {
                match directive(&raw) {
                    Some((key, value)) if matches!(key.as_str(), "syntax" | "escape" | "check") => {
                        if seen.contains(&key) {
                            return error(start, format!("only one {key} parser directive can be used"));
                        }
                        seen.push(key.clone());
                        match key.as_str() {
                            "escape" => {
                                directives.escape = match value.as_str() {
                                    "\\" => '\\',
                                    "`" => '`',
                                    _ => return error(start, format!("invalid escape token '{value}' does not match ` or \\")),
                                }
                            }
                            "syntax" => directives.syntax = Some(value),
                            _ => directives.check = Some(value),
                        }
                        continue;
                    }
                    _ => directives_done = true,
                }
            }
            if is_comment(&raw) {
                let comment = trim_left(&raw)[1..].trim().to_string();
                if comment.is_empty() {
                    comments.clear();
                } else {
                    comments.push(comment);
                }
                continue;
            }
            let first = trim_left(&raw);
            let (mut line, mut ended) = continuation(first, directives.escape);
            if ended && line.trim_matches(is_space).is_empty() {
                continue;
            }
            let mut empty_continuation = false;
            while !ended && i < lines.len() {
                let next = strip_end(lines[i]);
                i += 1;
                if is_comment(&next) {
                    continue;
                }
                if trim_left(&next).is_empty() {
                    empty_continuation = true;
                    continue;
                }
                let (more, end) = continuation(&next, directives.escape);
                line.push_str(&more);
                ended = end;
            }
            if empty_continuation {
                warnings.push(ParseError { line: start, message: "empty continuation lines will become errors in a future release".into() });
            }
            let mut ins = instruction(&line, start, directives.escape, std::mem::take(&mut comments))?;
            if matches!(ins.keyword.as_str(), "run" | "copy" | "add") && line.contains("<<") {
                for mut doc in heredoc_markers(&line) {
                    let mut ended = false;
                    while i < lines.len() {
                        let text = lines[i];
                        i += 1;
                        let bare = strip_end(text);
                        let candidate = if doc.chomp { bare.trim_start_matches('\t').to_string() } else { bare.clone() };
                        if candidate == doc.name {
                            ended = true;
                            break;
                        }
                        let kept = if doc.chomp { text.trim_start_matches('\t') } else { text };
                        doc.content.push_str(kept);
                        if !kept.ends_with('\n') {
                            doc.content.push('\n');
                        }
                    }
                    if !ended {
                        return error(start, format!("unterminated heredoc {}", doc.name));
                    }
                    ins.heredocs.push(doc);
                }
            }
            ins.end_line = i;
            instructions.push(ins);
        }
        if instructions.is_empty() {
            return error(0, "the file has no instructions");
        }
        let (meta_args, stages) = stages(&instructions)?;
        Ok(Carbonfile { directives, meta_args, stages, instructions, warnings })
    }

    /// A stage by its name or its number, as `--target` and `COPY --from` name them.
    pub fn stage(&self, name: &str) -> Option<usize> {
        let lower = name.to_ascii_lowercase();
        self.stages.iter().position(|s| s.name.as_deref() == Some(lower.as_str())).or_else(|| name.parse::<usize>().ok().filter(|&n| n < self.stages.len()))
    }
}

fn stages(instructions: &[Instruction]) -> Result<(Vec<Instruction>, Vec<Stage>), ParseError> {
    let mut meta = Vec::new();
    let mut stages: Vec<Stage> = Vec::new();
    for ins in instructions {
        if !KNOWN.contains(&ins.keyword.as_str()) {
            return error(ins.line, format!("unknown instruction: {}", ins.original.split_whitespace().next().unwrap_or("")));
        }
        if ins.keyword == "from" {
            let Args::Words(words) = &ins.args else { return error(ins.line, "FROM requires either one or three arguments") };
            let (base, name) = match words.as_slice() {
                [base] => (base.clone(), None),
                [base, as_, name] if as_.eq_ignore_ascii_case("as") => {
                    let name = name.to_ascii_lowercase();
                    let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
                    if !valid {
                        return error(ins.line, format!("invalid name for build stage: {name:?}, name can't start with a number or contain symbols"));
                    }
                    if stages.iter().any(|s| s.name.as_deref() == Some(name.as_str())) {
                        return error(ins.line, format!("duplicate stage name {name:?}"));
                    }
                    (base.clone(), Some(name))
                }
                _ => return error(ins.line, "FROM requires either one or three arguments"),
            };
            for flag in &ins.flags {
                let key = flag.split('=').next().unwrap_or("");
                if key != "platform" {
                    return error(ins.line, format!("unknown flag: {key}"));
                }
            }
            stages.push(Stage { base, name, platform: ins.flag("platform").map(String::from), from: ins.clone(), instructions: Vec::new() });
            continue;
        }
        match stages.last_mut() {
            Some(stage) => stage.instructions.push(ins.clone()),
            None if ins.keyword == "arg" => meta.push(ins.clone()),
            None => return error(ins.line, "no build stage in current context"),
        }
    }
    if stages.is_empty() {
        return error(0, "the file has no FROM");
    }
    Ok((meta, stages))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Instruction {
        Carbonfile::parse(&format!("FROM x\n{text}")).unwrap().stages[0].instructions[0].clone()
    }

    #[test]
    fn directives_and_comments() {
        let f = Carbonfile::parse("# syntax=docker/dockerfile:1\n# escape=`\n#check=skip=all\n\n# a comment\nFROM debian\nRUN echo `\n  hi\n").unwrap();
        assert_eq!(f.directives.syntax.as_deref(), Some("docker/dockerfile:1"));
        assert_eq!(f.directives.escape, '`');
        assert_eq!(f.directives.check.as_deref(), Some("skip=all"));
        assert_eq!(f.stages[0].instructions[0].args, Args::Text("echo   hi".into()));
        // A directive after the first other line is a comment.
        let f = Carbonfile::parse("FROM x\n# escape=`\nRUN a \\\n b").unwrap();
        assert_eq!(f.directives.escape, '\\');
        assert!(Carbonfile::parse("# escape=\\\n# escape=`\nFROM x").is_err());
        assert!(Carbonfile::parse("# escape=x\nFROM x").is_err());
    }

    #[test]
    fn continuations_skip_comments_and_keep_leading_space() {
        let i = one("RUN apt-get update \\\n# a comment in the middle\n    && apt-get install -y \\\n\n    curl");
        assert_eq!(i.args, Args::Text("apt-get update     && apt-get install -y     curl".into()));
        assert_eq!((i.line, i.end_line), (2, 6));
        let f = Carbonfile::parse("FROM x\nRUN a \\\n\n b").unwrap();
        assert_eq!(f.warnings.len(), 1);
    }

    #[test]
    fn exec_and_shell_forms() {
        assert_eq!(one(r#"CMD ["nginx", "-g", "daemon off;"]"#).args, Args::Json(vec!["nginx".into(), "-g".into(), "daemon off;".into()]));
        // An array with something but strings in it is refused, not taken as the shell form.
        assert!(Carbonfile::parse("FROM x\nCMD [\"a\", 1]").is_err());
        assert_eq!(one("ENTRYPOINT [not json").args, Args::Text("[not json".into()));
        assert_eq!(one("COPY a b c/").args, Args::Words(vec!["a".into(), "b".into(), "c/".into()]));
        assert_eq!(one("COPY [\"a b\", \"c/\"]").args, Args::Json(vec!["a b".into(), "c/".into()]));
        assert_eq!(one("run echo hi").keyword, "run");
    }

    #[test]
    fn flags() {
        let i = one("COPY --from=build --chown=\"app user\" --link src/ /app/");
        assert_eq!(i.flags, ["from=build", "chown=app user", "link"]);
        assert_eq!(i.flag("from"), Some("build"));
        assert!(i.flag_on("link"));
        assert_eq!(i.args, Args::Words(vec!["src/".into(), "/app/".into()]));
        let i = one("RUN --mount=type=cache,target=/root/.cache --network=none pip install x");
        assert_eq!(i.flags("mount").collect::<Vec<_>>(), ["type=cache,target=/root/.cache"]);
        assert_eq!(i.args, Args::Text("pip install x".into()));
        // `--` ends the flags.
        assert_eq!(one("RUN -- --not-a-flag").args, Args::Text("--not-a-flag".into()));
    }

    #[test]
    fn env_label_and_arg() {
        assert_eq!(one("ENV A=1 B=\"two words\" C=three\\ words").args, Args::Pairs(vec![("A".into(), "1".into()), ("B".into(), "\"two words\"".into()), ("C".into(), "three\\ words".into())]));
        assert_eq!(one("ENV PATH /usr/local/bin:$PATH").args, Args::Pairs(vec![("PATH".into(), "/usr/local/bin:$PATH".into())]));
        assert!(Carbonfile::parse("FROM x\nENV A").is_err());
        assert!(Carbonfile::parse("FROM x\nENV A=1 B").is_err());
        assert_eq!(one("LABEL \"com.example.vendor\"=\"ACME\" version=1.0").args, Args::Pairs(vec![("\"com.example.vendor\"".into(), "\"ACME\"".into()), ("version".into(), "1.0".into())]));
        assert_eq!(one("ARG VERSION=1 USER").args, Args::Words(vec!["VERSION=1".into(), "USER".into()]));
    }

    #[test]
    fn healthcheck_and_onbuild() {
        assert_eq!(one("HEALTHCHECK NONE").args, Args::Health(None));
        let i = one("HEALTHCHECK --interval=5s CMD curl -f http://localhost/");
        assert_eq!(i.flags, ["interval=5s"]);
        assert_eq!(i.args, Args::Health(Some(Box::new(Args::Text("curl -f http://localhost/".into())))));
        let Args::OnBuild(inner) = one("ONBUILD COPY . /app").args else { panic!() };
        assert_eq!((inner.keyword.as_str(), &inner.args), ("copy", &Args::Words(vec![".".into(), "/app".into()])));
        assert!(Carbonfile::parse("FROM x\nONBUILD ONBUILD RUN a").is_err());
        assert!(Carbonfile::parse("FROM x\nHEALTHCHECK WAT").is_err());
    }

    #[test]
    fn heredocs() {
        let f = Carbonfile::parse("FROM x\nRUN <<EOF\necho one\necho $HOME\nEOF\nCOPY <<-\"END\" /etc/conf\n\tkey=$v\n\tEND\nRUN python3 <<A 3<<B\nprint(1)\nA\nb\nB\n").unwrap();
        let s = &f.stages[0].instructions;
        assert_eq!(s[0].heredocs, vec![Heredoc { name: "EOF".into(), content: "echo one\necho $HOME\n".into(), expand: true, chomp: false, fd: None }]);
        assert_eq!(s[1].heredocs[0].content, "key=$v\n");
        assert!(!s[1].heredocs[0].expand && s[1].heredocs[0].chomp);
        assert_eq!(s[2].heredocs.len(), 2);
        assert_eq!(s[2].heredocs[1].fd, Some(3));
        assert_eq!(s[2].end_line, 13);
        assert!(Carbonfile::parse("FROM x\nRUN <<EOF\nno end\n").is_err());
        // A here-document's lines aren't instructions or comments.
        let f = Carbonfile::parse("FROM x\nRUN <<EOF\n# not a comment\nFROM y\nEOF\n").unwrap();
        assert_eq!(f.stages.len(), 1);
        assert_eq!(f.stages[0].instructions[0].heredocs[0].content, "# not a comment\nFROM y\n");
    }

    #[test]
    fn stages() {
        let f = Carbonfile::parse("ARG BASE=debian\nFROM ${BASE} AS Build\nRUN make\nFROM --platform=linux/amd64 scratch\nCOPY --from=build /out /\n").unwrap();
        assert_eq!(f.meta_args.len(), 1);
        assert_eq!(f.stages.len(), 2);
        assert_eq!(f.stages[0].name.as_deref(), Some("build"));
        assert_eq!(f.stages[1].platform.as_deref(), Some("linux/amd64"));
        assert_eq!(f.stage("BUILD"), Some(0));
        assert_eq!(f.stage("1"), Some(1));
        assert!(Carbonfile::parse("RUN a\nFROM x").is_err());
        assert!(Carbonfile::parse("FROM x AS a\nFROM y AS a").is_err());
        assert!(Carbonfile::parse("FROM x AS 1a").is_err());
        assert!(Carbonfile::parse("FROM x\nFOO bar").is_err());
        assert!(Carbonfile::parse("FROM x y").is_err());
    }

}
