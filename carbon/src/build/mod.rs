//! Building images from a Carbonfile: the Dockerfile's grammar, exactly ([`Carbonfile`]), and a build made of the
//! library's own calls: each `RUN` a launched machine ([`crate::MachineSpec::launch`]) frozen after
//! ([`crate::Machine::freeze_with`]), each `COPY` a copy into a machine that's made but never run
//! ([`crate::MachineSpec::create`]), every other instruction a change to the image's config.
//!
//! ```no_run
//! # fn main() -> carbon::Result<()> {
//! let carbon = carbon::Carbon::open()?;
//! let built = carbon.build("./app").tag("app:2").arg("VERSION", "2").run_with(|step| eprintln!("{step}"))?;
//! println!("{}", built.image.short_name());
//! # Ok(()) }
//! ```

mod archive;
mod ignore;
mod lex;
mod parse;
mod run;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hooks::{Connect, Decision, Exec, Filter, Hooks};

pub use ignore::Patterns;
pub use lex::{Lexer, Lookup};
pub use parse::{Args, Carbonfile, Directives, Heredoc, Instruction, ParseError, Stage};

use crate::{Carbon, Image, Isolation, LogLine, Network, Platform, Result, Rules};

type ExecHook = (Filter, Arc<dyn Fn(&Exec) -> Decision<Exec> + Send + Sync>);
type ConnectHook = (Filter, Arc<dyn Fn(&Connect) -> Decision<Connect> + Send + Sync>);

/// A build, as [`Carbon::build`] starts one: what to build from, and how. Then [`Build::run`] or [`Build::run_with`].
pub struct Build {
    pub(crate) carbon: Carbon,
    pub(crate) context: PathBuf,
    pub(crate) file: Option<PathBuf>,
    pub(crate) tags: Vec<String>,
    pub(crate) args: BTreeMap<String, String>,
    pub(crate) target: Option<String>,
    pub(crate) platform: Option<Platform>,
    pub(crate) isolation: Isolation,
    pub(crate) network: Option<Network>,
    pub(crate) memory: Option<String>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) rules: Rules,
    pub(crate) hooks: Vec<Arc<dyn Hooks>>,
    pub(crate) exec_hooks: Vec<ExecHook>,
    pub(crate) connect_hooks: Vec<ConnectHook>,
    pub(crate) no_cache: bool,
    pub(crate) output: Option<PathBuf>,
    pub(crate) secrets: BTreeMap<String, PathBuf>,
}

impl Carbon {
    /// A build from the folder `context`: its `Carbonfile`, else its `Dockerfile`, unless [`Build::file`] names one.
    pub fn build(&self, context: impl Into<PathBuf>) -> Build {
        Build {
            carbon: self.clone(),
            context: context.into(),
            file: None,
            tags: Vec::new(),
            args: BTreeMap::new(),
            target: None,
            platform: None,
            isolation: Isolation::Auto,
            network: None,
            memory: None,
            timeout: None,
            rules: Rules::new(),
            hooks: Vec::new(),
            exec_hooks: Vec::new(),
            connect_hooks: Vec::new(),
            no_cache: false,
            output: None,
            secrets: BTreeMap::new(),
        }
    }
}

impl Build {
    /// The file to build: a path as this process sees it (`-f`), not under the context.
    pub fn file(mut self, file: impl Into<PathBuf>) -> Build {
        self.file = Some(file.into());
        self
    }

    /// A name for what's built (`-t`), as many as wanted.
    pub fn tag(mut self, name: impl Into<String>) -> Build {
        self.tags.push(name.into());
        self
    }

    /// A build argument (`--build-arg NAME=VALUE`): the value of an `ARG` the file declares.
    pub fn arg(mut self, name: impl Into<String>, value: impl Into<String>) -> Build {
        self.args.insert(name.into(), value.into());
        self
    }

    /// The stage to build, by its name or number (`--target`): the last by default. Only the stages it needs are built.
    pub fn target(mut self, stage: impl Into<String>) -> Build {
        self.target = Some(stage.into());
        self
    }

    /// The platform to build for: Linux for this host's CPU by default. x86-64 on Apple silicon runs translated.
    pub fn platform(mut self, platform: Platform) -> Build {
        self.platform = Some(platform);
        self
    }

    /// Where each `RUN` runs: a hardware VM, the sandbox, or the best there is (the default, which says so when it falls
    /// back).
    pub fn isolation(mut self, isolation: Isolation) -> Build {
        self.isolation = isolation;
        self
    }

    /// Every `RUN`'s network, unless its `--network` says otherwise.
    pub fn network(mut self, network: Network) -> Build {
        self.network = Some(network);
        self
    }

    /// How much memory each `RUN` may use: `2g`.
    pub fn memory(mut self, size: impl Into<String>) -> Build {
        self.memory = Some(size.into());
        self
    }

    /// The longest a `RUN` may take: it's stopped after, and the build fails.
    pub fn timeout(mut self, timeout: Duration) -> Build {
        self.timeout = Some(timeout);
        self
    }

    /// Rules for every `RUN`'s machine: programs that may not start, where the network may go.
    pub fn rules(mut self, rules: Rules) -> Build {
        self.rules = rules;
        self
    }

    /// Hooks for every `RUN`'s machine, asked in this process.
    pub fn hooks(mut self, hooks: impl Hooks) -> Build {
        self.hooks.push(Arc::new(hooks));
        self
    }

    /// Decides the programs every `RUN` starts.
    pub fn on_exec(mut self, filter: Filter, f: impl Fn(&Exec) -> Decision<Exec> + Send + Sync + 'static) -> Build {
        self.exec_hooks.push((filter, Arc::new(f)));
        self
    }

    /// Decides the connections every `RUN` makes.
    pub fn on_connect(mut self, filter: Filter, f: impl Fn(&Connect) -> Decision<Connect> + Send + Sync + 'static) -> Build {
        self.connect_hooks.push((filter, Arc::new(f)));
        self
    }

    /// Runs every step, finding none in the cache (`--no-cache`).
    pub fn no_cache(mut self, no_cache: bool) -> Build {
        self.no_cache = no_cache;
        self
    }

    /// Also copies the built stage's files to this host folder (`--output DIR`).
    pub fn output(mut self, dir: impl Into<PathBuf>) -> Build {
        self.output = Some(dir.into());
        self
    }

    /// A secret a `RUN --mount=type=secret,id=ID` reads (`--secret id=ID,src=FILE`): at `/run/secrets/ID` for that
    /// step only, never in the image or its cache key.
    pub fn secret(mut self, id: impl Into<String>, file: impl Into<PathBuf>) -> Build {
        self.secrets.insert(id.into(), file.into());
        self
    }

    /// Builds, quietly.
    pub fn run(self) -> Result<Built> {
        self.run_with(|_| {})
    }

    /// Builds, telling `progress` each step as it starts, is found in the cache, writes, and ends.
    pub fn run_with(self, mut progress: impl FnMut(&Step)) -> Result<Built> {
        run::build(self, &mut progress)
    }
}

/// A build's progress.
#[derive(Debug, Clone)]
pub enum Step {
    /// A stage starts: its number, its name, and what it starts from.
    Stage { stage: usize, name: Option<String>, base: String },
    /// An instruction starts: its stage, its number in the stage (from 1), and the instruction as written.
    Started { stage: usize, n: usize, instruction: String },
    /// Found in the cache: nothing ran.
    Cached { stage: usize, n: usize },
    /// What a `RUN` wrote.
    Output { stage: usize, n: usize, line: LogLine },
    /// Something the build says along the way: an image being pulled, a fallback.
    Note(String),
    /// Works, but not as asked: a build argument nothing uses, a `USER` not honored yet.
    Warning(String),
    /// An instruction ended.
    Done { stage: usize, n: usize, took: Duration },
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Stage { stage, name: Some(name), base } => write!(f, "[stage {stage}: {name}] FROM {base}"),
            Step::Stage { stage, name: None, base } => write!(f, "[stage {stage}] FROM {base}"),
            Step::Started { stage, n, instruction } => write!(f, "[{stage}:{n}] {instruction}"),
            Step::Cached { stage, n } => write!(f, "[{stage}:{n}] cached"),
            Step::Output { stage, n, line } => write!(f, "[{stage}:{n}] {}", line.text().trim_end_matches(['\n', '\r'])),
            Step::Note(text) => write!(f, "{text}"),
            Step::Warning(text) => write!(f, "warning: {text}"),
            Step::Done { stage, n, took } => write!(f, "[{stage}:{n}] done in {:.1}s", took.as_secs_f64()),
        }
    }
}

/// What a build made.
#[derive(Debug, Clone)]
pub struct Built {
    /// The image: named by the first tag, or unnamed (`sha256:…`) without one.
    pub image: Image,
    /// How many instructions ran, and how many were found in the cache.
    pub ran: usize,
    pub cached: usize,
    pub took: Duration,
}

#[cfg(test)]
mod tests {
    use crate::files::tests::rootfs_tar;

    /// A build with nothing to run: copies, an unpacked archive, a here-document, the config; then again, all cached.
    #[test]
    fn builds_without_running_anything() {
        let home = std::env::temp_dir().join(format!("carbon-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let carbon = crate::Carbon::open_at(&home).unwrap();
        let base = rootfs_tar(&[("etc/passwd", b"root:x:0:0::/root:/bin/sh\napp:x:1000:1000::/home/app:/bin/sh\n", 0o644), ("etc/group", b"root:x:0:\napp:x:1000:\n", 0o644)]);
        carbon.import("base:1", &base, Some(images::Platform::linux_here()), &[]).unwrap();

        let ctx = home.join("ctx");
        std::fs::create_dir_all(ctx.join("src/cache")).unwrap();
        std::fs::write(ctx.join("src/main.py"), "print(1)\n").unwrap();
        std::fs::write(ctx.join("src/cache/x"), "junk").unwrap();
        std::fs::write(ctx.join(".dockerignore"), "src/cache\n").unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &rootfs_tar(&[("tool/run.sh", b"#!/bin/sh\necho hi\n", 0o755)])).unwrap();
        std::fs::write(ctx.join("tool.tar.gz"), gz.finish().unwrap()).unwrap();
        std::fs::write(
            ctx.join("Dockerfile"),
            "FROM base:1\nARG WHO=world\nENV GREETING=\"hello ${WHO}\"\nWORKDIR /app\nCOPY --chown=app src/ ./\nADD tool.tar.gz /opt/\nCOPY <<EOF /etc/greeting\n$GREETING\nEOF\nEXPOSE 8000\nCMD [\"python\", \"main.py\"]\n",
        )
        .unwrap();

        let built = carbon.build(&ctx).tag("app:1").arg("WHO", "carbon").run().unwrap();
        assert_eq!(built.image.short_name(), "app:1");
        assert_eq!((built.ran, built.cached), (3, 0));
        let c = &built.image.config;
        assert_eq!((c.workdir.as_str(), c.cmd.as_slice(), c.exposed_ports.as_slice()), ("/app", &["python".to_string(), "main.py".to_string()][..], &["8000/tcp".to_string()][..]));
        assert!(c.env.contains(&"GREETING=hello carbon".to_string()));

        let m = carbon.machine("app:1").name("check").create().unwrap();
        let files = m.files();
        assert_eq!(files.read_to_string("/app/main.py").unwrap(), "print(1)\n");
        assert_eq!(files.stat("/app/main.py").unwrap().uid, 1000);
        assert!(files.stat("/app/cache").is_err(), "the ignore file leaves it out");
        assert_eq!(files.read_to_string("/opt/tool/run.sh").unwrap(), "#!/bin/sh\necho hi\n");
        assert_eq!(files.stat("/opt/tool/run.sh").unwrap().mode & 0o777, 0o755);
        assert_eq!(files.read_to_string("/etc/greeting").unwrap(), "hello carbon\n");
        m.remove().unwrap();

        let again = carbon.build(&ctx).tag("app:1").arg("WHO", "carbon").run().unwrap();
        assert_eq!((again.ran, again.cached), (0, 3));
        // A changed file runs its COPY again, and what follows it.
        std::fs::write(ctx.join("src/main.py"), "print(2)\n").unwrap();
        let changed = carbon.build(&ctx).arg("WHO", "carbon").run().unwrap();
        assert_eq!((changed.ran, changed.cached), (3, 0));
        // Neither did any build leave a machine behind.
        assert!(carbon.machines().is_empty());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
