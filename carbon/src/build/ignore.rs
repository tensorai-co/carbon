//! Ignore files' patterns, as Docker's (`moby/patternmatcher`) read and match them: one a line, `#` comments, `!` for
//! an exception, `*` and `?` within a path's part, `**` across parts, `[...]` classes; each pattern cleaned
//! (`./a//b/` is `a/b`) and matched from the context's root. A path is excluded by the last pattern that matches it or
//! a folder it's in. [`Patterns`] is also what [`crate::Copy::exclude`] and `COPY --exclude` use.

use super::lex::glob;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    parts: Vec<String>,
    exception: bool,
}

/// A list of patterns, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Patterns {
    patterns: Vec<Pattern>,
}

/// A path as the patterns see it: `/` between parts, no `.` parts, no leading or trailing `/`.
fn clean(path: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p.to_string()),
        }
    }
    parts
}

impl Patterns {
    /// From an ignore file's text.
    pub fn parse(text: &str) -> Patterns {
        Patterns::new(text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')))
    }

    /// From patterns, each as an ignore file's line.
    pub fn new<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> Patterns {
        let mut patterns = Vec::new();
        for line in lines {
            let line = line.as_ref().trim();
            let (exception, pattern) = match line.strip_prefix('!') {
                Some(rest) => (true, rest.trim()),
                None => (false, line),
            };
            let parts = clean(pattern);
            if parts.is_empty() {
                continue;
            }
            patterns.push(Pattern { parts, exception });
        }
        Patterns { patterns }
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Whether `path` (from the root: `src/main.rs`) is excluded: by the last pattern that matches it, or matches a
    /// folder it's in.
    pub fn excludes(&self, path: &str) -> bool {
        let path = clean(path);
        let mut excluded = false;
        for pattern in &self.patterns {
            // An exception only matters if something before excluded it, and a pattern only if nothing has.
            if pattern.exception != excluded {
                continue;
            }
            let hit = (1..=path.len()).any(|n| matches(&pattern.parts, &path[..n]));
            if hit {
                excluded = !pattern.exception;
            }
        }
        excluded
    }

    /// Whether a folder might hold something an exception brings back, so a walk can't skip it whole even though it's
    /// excluded.
    pub fn may_include_inside(&self, dir: &str) -> bool {
        let dir = clean(dir);
        self.patterns.iter().any(|p| p.exception && (p.parts.iter().any(|x| x.contains("**")) || p.parts.len() > dir.len() && (0..dir.len()).all(|i| part_matches(&p.parts[i], &dir[i]))))
    }
}

fn part_matches(pattern: &str, part: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = part.chars().collect();
    glob(&p, &t, '\\', true)
}

/// Whether a pattern's parts match a path's, all of them: `**` takes any number of parts, none included.
fn matches(pattern: &[String], path: &[String]) -> bool {
    match pattern.first() {
        None => path.is_empty(),
        Some(p) if p == "**" => (0..=path.len()).any(|n| matches(&pattern[1..], &path[n..])),
        Some(p) if p.contains("**") => {
            // `a**b` within one part, as Docker reads it: `*` that crosses `/`.
            let rest: Vec<char> = pattern.join("/").chars().collect();
            let joined: Vec<char> = path.join("/").chars().collect();
            glob(&rest, &joined, '\\', false)
        }
        Some(p) => !path.is_empty() && part_matches(p, &path[0]) && matches(&pattern[1..], &path[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_docker_matches() {
        let p = Patterns::parse("# comment\nnode_modules\n*.log\n!keep.log\n/build/\n**/*.tmp\ndocs/**/draft*\n./secret//key\n");
        assert!(p.excludes("node_modules"));
        assert!(p.excludes("node_modules/a/b.js"));
        assert!(!p.excludes("src/node_modules"), "patterns are from the root");
        assert!(p.excludes("app.log"));
        assert!(!p.excludes("keep.log"));
        assert!(!p.excludes("logs/app.log"), "* doesn't cross /");
        assert!(p.excludes("build/out.o"));
        assert!(p.excludes("a.tmp") && p.excludes("x/y/z.tmp"));
        assert!(p.excludes("docs/draft1.md") && p.excludes("docs/a/b/draft.md"));
        assert!(p.excludes("secret/key"));
        assert!(!p.excludes("src/main.rs"));
    }

    #[test]
    fn exceptions_inside_excluded_folders() {
        let p = Patterns::parse("vendor\n!vendor/keep\n");
        assert!(p.excludes("vendor/other"));
        assert!(!p.excludes("vendor/keep/file"));
        assert!(p.may_include_inside("vendor"));
        assert!(!Patterns::parse("vendor").may_include_inside("vendor"));
    }

    #[test]
    fn classes_and_question_marks() {
        let p = Patterns::new(["file[0-9].txt", "?.md"]);
        assert!(p.excludes("file3.txt") && !p.excludes("filex.txt"));
        assert!(p.excludes("a.md") && !p.excludes("ab.md"));
    }
}
