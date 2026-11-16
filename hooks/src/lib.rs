//! `hooks`: what a machine does, as it does it, and what can be decided about it.
//!
//! Carbon owns the whole kernel, so every program a machine starts, everything it writes to its terminal, every
//! connection it makes and every name it looks up passes through Carbon's own code. Each of those places is a **hook
//! point**: it makes an [`Event`] (who's asking, and for what) and asks the machine's [`Hub`] **on the thread that
//! asked**, which waits for the answer. So a decision really decides: nothing has happened yet.
//!
//! A hook can, depending on the event:
//!
//! | | |
//! | --- | --- |
//! | **observe** | see it; nothing changes |
//! | **allow** or **deny** | let it happen, or give the program the error its own system would give |
//! | **rewrite** | let it happen differently: other arguments, another address, other bytes |
//! | **answer** | do it here instead, and give the program the result: an exec's output and exit code, a lookup's addresses |
//!
//! Hooks are a chain: the first that decides (anything but [`Decision::Pass`]) decides, and a machine with nothing
//! hooked costs one atomic load per hook point. This crate has no dependencies, so the kernels and `net` can depend
//! on it; the `carbon` crate re-exports it as `carbon::hooks`.
//!
//! ```
//! use hooks::{Decision, Exec, Hooks, Hub, Reply};
//!
//! struct Agent;
//!
//! impl Hooks for Agent {
//!     fn exec(&self, e: &Exec) -> Decision<Exec> {
//!         match e.program() {
//!             "rm" if e.argv().iter().any(|a| a == "-rf") => Decision::deny(),
//!             "git" if e.args().first().map(String::as_str) == Some("push") => {
//!                 Decision::answer(Reply::new().stdout("Everything up-to-date\n").exit(0))
//!             }
//!             _ => Decision::pass(),
//!         }
//!     }
//! }
//!
//! let hub = Hub::new();
//! hub.add(Agent);
//! let decided = hub.exec(Exec::new("/usr/bin/rm", ["rm", "-rf", "/"]));
//! assert!(matches!(decided, Decision::Deny));
//! ```

use std::borrow::Cow;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, SystemTime};

// ---------------------------------------------------------------------------------------------------------------
// Kinds and interest

/// What kind of thing a hook point reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A program about to start.
    Exec,
    /// Bytes a process writes to the machine's stdout or stderr, or to an `exec`'s.
    Output,
    /// A connection out (TCP), or a datagram to a new destination (UDP).
    Connect,
    /// A name the machine looks up.
    Lookup,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Exec, Kind::Output, Kind::Connect, Kind::Lookup];

    fn bit(self) -> u32 {
        match self {
            Kind::Exec => 1,
            Kind::Output => 2,
            Kind::Connect => 4,
            Kind::Lookup => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Exec => "exec",
            Kind::Output => "output",
            Kind::Connect => "connect",
            Kind::Lookup => "lookup",
        }
    }

    pub fn parse(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of kinds: what a hook, an observer or a subscription wants events for. Events no one wants aren't made.
///
/// ```
/// use hooks::{Interest, Kind};
/// let network = Interest::connect() | Interest::lookup();
/// assert!(network.has(Kind::Connect) && !network.has(Kind::Exec));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Interest(u32);

impl Interest {
    pub fn none() -> Interest {
        Interest(0)
    }
    pub fn all() -> Interest {
        Interest(15)
    }
    pub fn exec() -> Interest {
        Interest(Kind::Exec.bit())
    }
    pub fn output() -> Interest {
        Interest(Kind::Output.bit())
    }
    pub fn connect() -> Interest {
        Interest(Kind::Connect.bit())
    }
    pub fn lookup() -> Interest {
        Interest(Kind::Lookup.bit())
    }
    /// Connections and lookups.
    pub fn network() -> Interest {
        Interest::connect() | Interest::lookup()
    }
    pub fn of(kind: Kind) -> Interest {
        Interest(kind.bit())
    }
    pub fn has(self, kind: Kind) -> bool {
        self.0 & kind.bit() != 0
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    /// Its kinds.
    pub fn kinds(self) -> impl Iterator<Item = Kind> {
        Kind::ALL.into_iter().filter(move |k| self.has(*k))
    }
}

impl std::ops::BitOr for Interest {
    type Output = Interest;
    fn bitor(self, other: Interest) -> Interest {
        Interest(self.0 | other.0)
    }
}

impl std::ops::BitOrAssign for Interest {
    fn bitor_assign(&mut self, other: Interest) {
        self.0 |= other.0;
    }
}

impl fmt::Debug for Interest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.kinds()).finish()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Events

/// Which kernel's system an exec is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
}

/// Where a program's start came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The machine's first program, before the machine boots.
    First,
    /// A program in the machine starting another: lambda's `execve`, atlas's `CreateProcess`.
    Program,
    /// From outside the machine: `carbon exec`, `Machine::exec`.
    Outside,
}

/// A program about to start. `path` is the program's file as the machine names it (`/usr/bin/git`,
/// `C:\Windows\System32\cmd.exe`) where the kernel has found it, else as it was asked for; `argv` its arguments, the
/// first being its own name (on Windows, its command line split as `CommandLineToArgvW` splits it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exec {
    pub os: Os,
    pub origin: Origin,
    /// The process it starts in (lambda's `execve` keeps the pid), and its parent, where they're known.
    pub pid: Option<u32>,
    pub parent: Option<u32>,
    pub path: String,
    pub argv: Vec<String>,
    /// `NAME=value`.
    pub env: Vec<String>,
    /// Its working folder, where it's known.
    pub cwd: Option<String>,
    /// Whether the program's file is there. A shell looking along its `PATH` tries each folder in turn, and each try
    /// is asked about, so a program that isn't there at all can still be answered; one not found and allowed just
    /// fails, as it would have.
    pub found: bool,
}

impl Exec {
    /// A Linux program's start from inside the machine, with no environment: for tests and rewrites.
    pub fn new<I, S>(path: &str, argv: I) -> Exec
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Exec { os: Os::Linux, origin: Origin::Program, pid: None, parent: None, path: path.to_string(), argv: argv.into_iter().map(Into::into).collect(), env: Vec::new(), cwd: None, found: true }
    }

    /// The program's name: its file's, without a folder or (on Windows) `.exe`, as `git` or `cmd`.
    pub fn program(&self) -> &str {
        let path = if self.path.is_empty() { self.argv.first().map_or("", String::as_str) } else { &self.path };
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        if self.os == Os::Windows && name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".exe") { &name[..name.len() - 4] } else { name }
    }

    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// Its arguments after its own name.
    pub fn args(&self) -> &[String] {
        self.argv.get(1..).unwrap_or_default()
    }

    /// An environment variable's value.
    pub fn var(&self, name: &str) -> Option<&str> {
        self.env.iter().find_map(|e| e.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')))
    }

    /// This, with another argument at the end: for a rewrite.
    pub fn arg(mut self, arg: impl Into<String>) -> Exec {
        self.argv.push(arg.into());
        self
    }

    /// This, with these arguments instead (the first being the program's name).
    pub fn with_argv<I, S>(mut self, argv: I) -> Exec
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.argv = argv.into_iter().map(Into::into).collect();
        self
    }

    /// This, running another program's file.
    pub fn with_path(mut self, path: impl Into<String>) -> Exec {
        self.path = path.into();
        self
    }

    /// This, with an environment variable set.
    pub fn with_var(mut self, name: &str, value: &str) -> Exec {
        self.env.retain(|e| e.split('=').next() != Some(name));
        self.env.push(format!("{name}={value}"));
        self
    }

    /// Its arguments as one line, for showing.
    pub fn command_line(&self) -> String {
        self.argv.iter().map(|a| if a.is_empty() || a.contains([' ', '"', '\'']) { format!("{a:?}") } else { a.clone() }).collect::<Vec<_>>().join(" ")
    }
}

/// A standard stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Whose output: the machine's own stdout and stderr (its first program's, and everything that shares them), or an
/// `exec`'s, by the number Carbon gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Machine,
    Exec(u64),
}

/// What a program writes to the machine's stdout or stderr, or to an `exec`'s, as it writes it: before a terminal's
/// line discipline (so before `\n` becomes `\r\n`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub stream: Stream,
    pub source: Source,
    pub bytes: Vec<u8>,
}

impl Output {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Its bytes as text, any that aren't UTF-8 replaced.
    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }

    /// Its bytes replaced by `bytes`, as a decision: what the program wrote is written as this instead.
    pub fn replace(&self, bytes: impl Into<Vec<u8>>) -> Decision<Output> {
        Decision::Rewrite(Output { bytes: bytes.into(), ..self.clone() })
    }
}

/// A transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Tcp,
    Udp,
}

/// A connection out of the machine, before it's made: a TCP connect, or the first datagram to a UDP destination.
/// `name` is the name the machine looked up that gave this address, if it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connect {
    pub proto: Proto,
    pub to: SocketAddr,
    pub name: Option<String>,
}

impl Connect {
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    pub fn ip(&self) -> IpAddr {
        self.to.ip()
    }
    pub fn port(&self) -> u16 {
        self.to.port()
    }
    /// Made to `to` instead, as a decision: the program thinks it reached what it asked for.
    pub fn redirect(&self, to: SocketAddr) -> Decision<Connect> {
        Decision::Rewrite(Connect { to, ..self.clone() })
    }
}

/// A name the machine looks up (its DNS, and atlas's `getaddrinfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup {
    pub name: String,
}

impl Lookup {
    pub fn new(name: impl Into<String>) -> Lookup {
        Lookup { name: name.into() }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Any event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Exec(Exec),
    Output(Output),
    Connect(Connect),
    Lookup(Lookup),
}

impl Event {
    pub fn kind(&self) -> Kind {
        match self {
            Event::Exec(_) => Kind::Exec,
            Event::Output(_) => Kind::Output,
            Event::Connect(_) => Kind::Connect,
            Event::Lookup(_) => Kind::Lookup,
        }
    }

    /// One line saying what it is, for logs: `exec git push`, `connect tcp pypi.org (151.101.0.223:443)`.
    pub fn describe(&self) -> String {
        match self {
            Event::Exec(e) => format!("exec {}", e.command_line()),
            Event::Output(o) => {
                let stream = if o.stream == Stream::Stdout { "stdout" } else { "stderr" };
                format!("output {stream} {:?}", o.text())
            }
            Event::Connect(c) => {
                let proto = if c.proto == Proto::Tcp { "tcp" } else { "udp" };
                match &c.name {
                    Some(name) => format!("connect {proto} {name} ({})", c.to),
                    None => format!("connect {proto} {}", c.to),
                }
            }
            Event::Lookup(l) => format!("lookup {}", l.name),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Decisions

/// What an answered exec gives the program that started it: output, written as if the program had, and an exit
/// code. No program runs: the process exists, writes this, and ends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reply {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: i32,
}

impl Reply {
    pub fn new() -> Reply {
        Reply::default()
    }
    pub fn stdout(mut self, bytes: impl AsRef<[u8]>) -> Reply {
        self.stdout.extend_from_slice(bytes.as_ref());
        self
    }
    pub fn stderr(mut self, bytes: impl AsRef<[u8]>) -> Reply {
        self.stderr.extend_from_slice(bytes.as_ref());
        self
    }
    pub fn exit(mut self, code: i32) -> Reply {
        self.code = code;
        self
    }
}

/// An answer that can't be given: for events that have none (yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Never {}

/// An answer, whichever event's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Exec(Reply),
    Lookup(Vec<IpAddr>),
}

mod sealed {
    pub trait Sealed {}
}

/// One of the events a hook point makes: [`Exec`], [`Output`], [`Connect`] or [`Lookup`].
pub trait Hooked: sealed::Sealed + Clone + fmt::Debug + Send + Sync + 'static {
    /// What answering it gives: [`Reply`] for an exec, addresses for a lookup, [`Never`] where there's no answer.
    type Answer: Clone + fmt::Debug + Send + Sync + 'static;
    const KIND: Kind;
    fn into_event(self) -> Event;
    fn from_event(event: Event) -> Option<Self>;
    fn answer_of(answer: Self::Answer) -> Option<Answer>;
    fn answer_from(answer: Answer) -> Option<Self::Answer>;
}

macro_rules! hooked {
    ($t:ident, $answer:ty, $variant:ident, |$a:ident| $of:expr, |$b:ident| $from:expr) => {
        impl sealed::Sealed for $t {}
        impl Hooked for $t {
            type Answer = $answer;
            const KIND: Kind = Kind::$variant;
            fn into_event(self) -> Event {
                Event::$variant(self)
            }
            fn from_event(event: Event) -> Option<Self> {
                match event {
                    Event::$variant(e) => Some(e),
                    _ => None,
                }
            }
            #[allow(unreachable_code, unused_variables)]
            fn answer_of($a: $answer) -> Option<Answer> {
                $of
            }
            #[allow(unreachable_patterns)]
            fn answer_from($b: Answer) -> Option<$answer> {
                $from
            }
        }
    };
}

hooked!(Exec, Reply, Exec, |a| Some(Answer::Exec(a)), |b| match b {
    Answer::Exec(r) => Some(r),
    _ => None,
});
hooked!(Output, Never, Output, |a| match a {}, |_b| None);
hooked!(Connect, Never, Connect, |a| match a {}, |_b| None);
hooked!(Lookup, Vec<IpAddr>, Lookup, |a| Some(Answer::Lookup(a)), |b| match b {
    Answer::Lookup(ips) => Some(ips),
    _ => None,
});

/// What a hook decides about an event.
#[derive(Debug, Clone)]
pub enum Decision<E: Hooked> {
    /// No opinion: the next hook decides (and if none does, it's allowed).
    Pass,
    /// It happens, as asked.
    Allow,
    /// It doesn't: the program gets the error its system gives (EACCES and ERROR_ACCESS_DENIED for an exec, a
    /// refused connection, a name that isn't there); output is dropped.
    Deny,
    /// It happens, as this instead.
    Rewrite(E),
    /// It doesn't, and the program gets this as its result.
    Answer(E::Answer),
}

impl<E: Hooked> Decision<E> {
    pub fn pass() -> Self {
        Decision::Pass
    }
    pub fn allow() -> Self {
        Decision::Allow
    }
    pub fn deny() -> Self {
        Decision::Deny
    }
    pub fn rewrite(event: E) -> Self {
        Decision::Rewrite(event)
    }
    pub fn answer(answer: E::Answer) -> Self {
        Decision::Answer(answer)
    }
    pub fn is_pass(&self) -> bool {
        matches!(self, Decision::Pass)
    }

    /// As a [`Verdict`], for an observer: a pass is an allow.
    pub fn verdict(&self) -> Verdict {
        match self {
            Decision::Pass | Decision::Allow => Verdict::Allow,
            Decision::Deny => Verdict::Deny,
            Decision::Rewrite(e) => Verdict::Rewrite(e.clone().into_event()),
            Decision::Answer(a) => E::answer_of(a.clone()).map_or(Verdict::Deny, Verdict::Answer),
        }
    }

    /// From a [`Verdict`]: one for another kind of event is a deny, so a mistake can't let something through.
    pub fn from_verdict(verdict: Verdict) -> Self {
        match verdict {
            Verdict::Allow => Decision::Allow,
            Verdict::Deny => Decision::Deny,
            Verdict::Rewrite(e) => E::from_event(e).map_or(Decision::Deny, Decision::Rewrite),
            Verdict::Answer(a) => E::answer_from(a).map_or(Decision::Deny, Decision::Answer),
        }
    }
}

impl Decision<Connect> {
    /// Refused, as if nothing listened there.
    pub fn refuse() -> Self {
        Decision::Deny
    }
}

impl Decision<Lookup> {
    /// The name isn't there.
    pub fn nxdomain() -> Self {
        Decision::Deny
    }
}

/// A decision, whichever event's: what an observer sees was decided, and what a subscriber decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny,
    Rewrite(Event),
    Answer(Answer),
}

impl Verdict {
    pub fn name(&self) -> &'static str {
        match self {
            Verdict::Allow => "allow",
            Verdict::Deny => "deny",
            Verdict::Rewrite(_) => "rewrite",
            Verdict::Answer(_) => "answer",
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Hooks and filters

/// A policy as a type: each method is asked about one kind of event, on the thread that asked, and returns
/// [`Decision::Pass`] unless it's overridden. `interest` says which kinds to ask about at all; by default, every kind,
/// so a type that only decides execs should say so, and output isn't made into events for it.
pub trait Hooks: Send + Sync + 'static {
    fn exec(&self, _event: &Exec) -> Decision<Exec> {
        Decision::Pass
    }
    fn output(&self, _event: &Output) -> Decision<Output> {
        Decision::Pass
    }
    fn connect(&self, _event: &Connect) -> Decision<Connect> {
        Decision::Pass
    }
    fn lookup(&self, _event: &Lookup) -> Decision<Lookup> {
        Decision::Pass
    }
    /// The kinds of event this wants to be asked about.
    fn interest(&self) -> Interest {
        Interest::all()
    }
}

impl<H: Hooks + ?Sized> Hooks for Arc<H> {
    fn exec(&self, e: &Exec) -> Decision<Exec> {
        (**self).exec(e)
    }
    fn output(&self, e: &Output) -> Decision<Output> {
        (**self).output(e)
    }
    fn connect(&self, e: &Connect) -> Decision<Connect> {
        (**self).connect(e)
    }
    fn lookup(&self, e: &Lookup) -> Decision<Lookup> {
        (**self).lookup(e)
    }
    fn interest(&self) -> Interest {
        (**self).interest()
    }
}

/// Which events of its kind a closure wants, checked before it's called.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    Any,
    /// An exec of this program ([`Exec::program`], case aside).
    Program(String),
    /// A connection to this port.
    Port(u16),
    /// A connection to this name or address, or a lookup of it; a name takes its subdomains too
    /// (`example.com` is `api.example.com`'s).
    Host(String),
    /// Output on this stream.
    Stream(Stream),
}

impl Filter {
    pub fn any() -> Filter {
        Filter::Any
    }
    pub fn program(name: impl Into<String>) -> Filter {
        Filter::Program(name.into())
    }
    pub fn port(port: u16) -> Filter {
        Filter::Port(port)
    }
    pub fn host(name: impl Into<String>) -> Filter {
        Filter::Host(name.into())
    }
    pub fn stream(stream: Stream) -> Filter {
        Filter::Stream(stream)
    }

    /// Whether `name` is `host` or under it.
    pub fn name_matches(host: &str, name: &str) -> bool {
        let (host, name) = (host.trim_end_matches('.'), name.trim_end_matches('.'));
        name.eq_ignore_ascii_case(host) || (name.len() > host.len() + 1 && name[name.len() - host.len()..].eq_ignore_ascii_case(host) && name.as_bytes()[name.len() - host.len() - 1] == b'.')
    }

    pub fn matches(&self, event: &Event) -> bool {
        match (self, event) {
            (Filter::Any, _) => true,
            (Filter::Program(p), Event::Exec(e)) => e.program().eq_ignore_ascii_case(p),
            (Filter::Port(p), Event::Connect(c)) => c.port() == *p,
            (Filter::Host(h), Event::Connect(c)) => c.name().is_some_and(|n| Filter::name_matches(h, n)) || h.parse::<IpAddr>().is_ok_and(|ip| ip == c.ip()),
            (Filter::Host(h), Event::Lookup(l)) => Filter::name_matches(h, &l.name),
            (Filter::Stream(s), Event::Output(o)) => o.stream == *s,
            _ => false,
        }
    }
}

type Closure<E> = Box<dyn Fn(&E) -> Decision<E> + Send + Sync>;

enum Entry {
    Hooks(Box<dyn Hooks>, Interest),
    Exec(Filter, Closure<Exec>),
    Output(Filter, Closure<Output>),
    Connect(Filter, Closure<Connect>),
    Lookup(Filter, Closure<Lookup>),
}

impl Entry {
    fn interest(&self) -> Interest {
        match self {
            Entry::Hooks(_, i) => *i,
            Entry::Exec(..) => Interest::exec(),
            Entry::Output(..) => Interest::output(),
            Entry::Connect(..) => Interest::connect(),
            Entry::Lookup(..) => Interest::lookup(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Observers and subscriptions

/// What happened: an event, the machine it was in, and what was decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub machine: String,
    pub event: Event,
    pub verdict: Verdict,
    pub at: SystemTime,
}

/// A stream of what happened in a machine, bounded: an observer that falls behind loses events, and is told how many.
pub struct Observer {
    events: Receiver<Observed>,
    lost: Arc<AtomicU64>,
}

impl Observer {
    /// The next event, waiting for it; None once the machine's hub is gone.
    pub fn next_event(&self) -> Option<Observed> {
        self.events.recv().ok()
    }
    /// The next event within `timeout`, if one comes.
    pub fn next_timeout(&self, timeout: Duration) -> Result<Observed, Ended> {
        self.events.recv_timeout(timeout).map_err(|e| if e == RecvTimeoutError::Timeout { Ended::TimedOut } else { Ended::Gone })
    }
    /// An event that's there now, if one is.
    pub fn try_next(&self) -> Option<Observed> {
        self.events.try_recv().ok()
    }
    /// How many events were lost because this fell behind, so far.
    pub fn lost(&self) -> u64 {
        self.lost.load(Ordering::Relaxed)
    }
}

impl Iterator for Observer {
    type Item = Observed;
    fn next(&mut self) -> Option<Observed> {
        self.next_event()
    }
}

/// Why a wait ended without an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    TimedOut,
    /// The machine's hub is gone.
    Gone,
}

/// How long a subscriber has to decide, and what's decided if it doesn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeout {
    pub after: Duration,
    pub default: Verdict,
}

impl Timeout {
    /// This long, then allowed (unless [`Timeout::default_deny`]).
    pub fn after(after: Duration) -> Timeout {
        Timeout { after, default: Verdict::Allow }
    }
    pub fn secs(secs: u64) -> Timeout {
        Timeout::after(Duration::from_secs(secs))
    }
    pub fn millis(millis: u64) -> Timeout {
        Timeout::after(Duration::from_millis(millis))
    }
    pub fn default_deny(mut self) -> Timeout {
        self.default = Verdict::Deny;
        self
    }
    pub fn default_allow(mut self) -> Timeout {
        self.default = Verdict::Allow;
        self
    }
}

/// An event waiting for a subscriber's decision, while the program's thread waits. Decide from any thread, before
/// the subscription's timeout; dropping it undecided passes it on to whatever's next.
pub struct Pending {
    machine: String,
    event: Event,
    reply: Sender<Verdict>,
}

impl Pending {
    pub fn event(&self) -> &Event {
        &self.event
    }
    pub fn machine(&self) -> &str {
        &self.machine
    }
    pub fn allow(self) {
        self.decide(Verdict::Allow);
    }
    pub fn deny(self) {
        self.decide(Verdict::Deny);
    }
    /// Decides: a rewrite or an answer for another kind of event than this one's is a deny.
    pub fn decide(self, verdict: Verdict) {
        let _ = self.reply.send(verdict);
    }
}

impl fmt::Debug for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pending").field("machine", &self.machine).field("event", &self.event).finish()
    }
}

/// Events to decide, one at a time, as they happen: for decisions that take a while (a model, a person).
pub struct Subscription {
    events: Receiver<Pending>,
}

impl Subscription {
    /// The next event to decide, waiting for it; None once the machine's hub is gone.
    pub fn next_pending(&self) -> Option<Pending> {
        self.events.recv().ok()
    }
    pub fn next_timeout(&self, timeout: Duration) -> Result<Pending, Ended> {
        self.events.recv_timeout(timeout).map_err(|e| if e == RecvTimeoutError::Timeout { Ended::TimedOut } else { Ended::Gone })
    }
}

impl Iterator for Subscription {
    type Item = Pending;
    fn next(&mut self) -> Option<Pending> {
        self.next_pending()
    }
}

struct Watching {
    interest: Interest,
    events: SyncSender<Observed>,
    lost: Arc<AtomicU64>,
}

struct Subscribed {
    id: u64,
    interest: Interest,
    events: Sender<Pending>,
    timeout: Timeout,
}

// ---------------------------------------------------------------------------------------------------------------
// The hub

/// A machine's hooks, observers and subscriptions, shared by every hook point in it. Cloning it shares it.
#[derive(Clone, Default)]
pub struct Hub(Arc<Inner>);

#[derive(Default)]
struct Inner {
    /// The kinds anything wants: what hook points check before they make an event.
    wants: AtomicU32,
    machine: RwLock<String>,
    entries: RwLock<Vec<Entry>>,
    watching: Mutex<Vec<Watching>>,
    subscribed: Mutex<Vec<Subscribed>>,
    next_id: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl fmt::Debug for Hub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hub").field("machine", &*self.0.machine.read().unwrap_or_else(|p| p.into_inner())).field("wants", &Interest(self.0.wants.load(Ordering::Relaxed))).finish()
    }
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// Names the machine it's for, as observers see it.
    pub fn set_machine(&self, name: &str) {
        *self.0.machine.write().unwrap_or_else(|p| p.into_inner()) = name.to_string();
    }

    pub fn machine(&self) -> String {
        self.0.machine.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Whether anything wants events of `kind`: one atomic load, for hook points to check before making one.
    #[inline]
    pub fn wants(&self, kind: Kind) -> bool {
        self.0.wants.load(Ordering::Relaxed) & kind.bit() != 0
    }

    /// Whether anything is hooked or watching at all.
    pub fn is_empty(&self) -> bool {
        self.0.wants.load(Ordering::Relaxed) == 0
    }

    fn recompute(&self) {
        let mut wants = Interest::none();
        for entry in self.0.entries.read().unwrap_or_else(|p| p.into_inner()).iter() {
            wants |= entry.interest();
        }
        for w in lock(&self.0.watching).iter() {
            wants |= w.interest;
        }
        for s in lock(&self.0.subscribed).iter() {
            wants |= s.interest;
        }
        self.0.wants.store(wants.0, Ordering::Relaxed);
    }

    fn push(&self, entry: Entry) {
        self.0.entries.write().unwrap_or_else(|p| p.into_inner()).push(entry);
        self.recompute();
    }

    /// Adds a policy at the end of the chain.
    pub fn add(&self, hooks: impl Hooks) {
        let interest = hooks.interest();
        self.push(Entry::Hooks(Box::new(hooks), interest));
    }

    pub fn on_exec(&self, filter: Filter, f: impl Fn(&Exec) -> Decision<Exec> + Send + Sync + 'static) {
        self.push(Entry::Exec(filter, Box::new(f)));
    }
    pub fn on_output(&self, filter: Filter, f: impl Fn(&Output) -> Decision<Output> + Send + Sync + 'static) {
        self.push(Entry::Output(filter, Box::new(f)));
    }
    pub fn on_connect(&self, filter: Filter, f: impl Fn(&Connect) -> Decision<Connect> + Send + Sync + 'static) {
        self.push(Entry::Connect(filter, Box::new(f)));
    }
    pub fn on_lookup(&self, filter: Filter, f: impl Fn(&Lookup) -> Decision<Lookup> + Send + Sync + 'static) {
        self.push(Entry::Lookup(filter, Box::new(f)));
    }

    /// A stream of what happens, for the kinds in `interest`, holding up to `capacity` events not yet taken.
    pub fn observe(&self, interest: Interest, capacity: usize) -> Observer {
        let (events, rx) = mpsc::sync_channel(capacity.max(1));
        let lost = Arc::new(AtomicU64::new(0));
        lock(&self.0.watching).push(Watching { interest, events, lost: lost.clone() });
        self.recompute();
        Observer { events: rx, lost }
    }

    /// Events of the kinds in `interest` to decide, after the hooks have passed on them; one a subscriber doesn't
    /// decide in time gets `timeout`'s default.
    pub fn subscribe(&self, interest: Interest, timeout: Timeout) -> Subscription {
        let (events, rx) = mpsc::channel();
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        lock(&self.0.subscribed).push(Subscribed { id, interest, events, timeout });
        self.recompute();
        Subscription { events: rx }
    }

    pub fn exec(&self, event: Exec) -> Decision<Exec> {
        self.decide(event, |entry, e| match entry {
            Entry::Hooks(h, i) if i.has(Kind::Exec) => h.exec(e),
            Entry::Exec(filter, f) if filter.matches(&Event::Exec(e.clone())) => f(e),
            _ => Decision::Pass,
        })
    }

    pub fn output(&self, event: Output) -> Decision<Output> {
        self.decide(event, |entry, e| match entry {
            Entry::Hooks(h, i) if i.has(Kind::Output) => h.output(e),
            Entry::Output(filter, f) if filter.matches(&Event::Output(e.clone())) => f(e),
            _ => Decision::Pass,
        })
    }

    pub fn connect(&self, event: Connect) -> Decision<Connect> {
        self.decide(event, |entry, e| match entry {
            Entry::Hooks(h, i) if i.has(Kind::Connect) => h.connect(e),
            Entry::Connect(filter, f) if filter.matches(&Event::Connect(e.clone())) => f(e),
            _ => Decision::Pass,
        })
    }

    pub fn lookup(&self, event: Lookup) -> Decision<Lookup> {
        self.decide(event, |entry, e| match entry {
            Entry::Hooks(h, i) if i.has(Kind::Lookup) => h.lookup(e),
            Entry::Lookup(filter, f) if filter.matches(&Event::Lookup(e.clone())) => f(e),
            _ => Decision::Pass,
        })
    }

    /// The chain, then subscribers, then allowed; observers hear what was decided. Never [`Decision::Pass`].
    fn decide<E: Hooked>(&self, event: E, ask: impl Fn(&Entry, &E) -> Decision<E>) -> Decision<E> {
        let mut decided = Decision::Pass;
        for entry in self.0.entries.read().unwrap_or_else(|p| p.into_inner()).iter() {
            decided = ask(entry, &event);
            if !decided.is_pass() {
                break;
            }
        }
        if decided.is_pass() {
            decided = self.ask_subscribers(&event);
        }
        if decided.is_pass() {
            decided = Decision::Allow;
        }
        self.tell_observers(&event, &decided);
        decided
    }

    fn ask_subscribers<E: Hooked>(&self, event: &E) -> Decision<E> {
        // Each is asked without the list's lock held, since a subscriber may take its time.
        let asked: Vec<(u64, Sender<Pending>, Timeout)> = lock(&self.0.subscribed).iter().filter(|s| s.interest.has(E::KIND)).map(|s| (s.id, s.events.clone(), s.timeout.clone())).collect();
        if asked.is_empty() {
            return Decision::Pass;
        }
        let machine = self.machine();
        let mut gone = Vec::new();
        let mut decided = Decision::Pass;
        for (id, sender, timeout) in asked {
            let (reply, answer) = mpsc::channel();
            if sender.send(Pending { machine: machine.clone(), event: event.clone().into_event(), reply }).is_err() {
                gone.push(id);
                continue;
            }
            match answer.recv_timeout(timeout.after) {
                Ok(verdict) => {
                    decided = Decision::from_verdict(verdict);
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    decided = Decision::from_verdict(timeout.default);
                    break;
                }
                // Dropped undecided: the next one's to say.
                Err(RecvTimeoutError::Disconnected) => {}
            }
        }
        if !gone.is_empty() {
            lock(&self.0.subscribed).retain(|s| !gone.contains(&s.id));
            self.recompute();
        }
        decided
    }

    fn tell_observers<E: Hooked>(&self, event: &E, decided: &Decision<E>) {
        let mut watching = lock(&self.0.watching);
        if !watching.iter().any(|w| w.interest.has(E::KIND)) {
            return;
        }
        let observed = Observed { machine: self.machine(), event: event.clone().into_event(), verdict: decided.verdict(), at: SystemTime::now() };
        let before = watching.len();
        watching.retain(|w| {
            if !w.interest.has(E::KIND) {
                return true;
            }
            match w.events.try_send(observed.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    w.lost.fetch_add(1, Ordering::Relaxed);
                    true
                }
                Err(TrySendError::Disconnected(_)) => false,
            }
        });
        let changed = watching.len() != before;
        drop(watching);
        if changed {
            self.recompute();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect(name: &str, port: u16) -> Connect {
        Connect { proto: Proto::Tcp, to: SocketAddr::from(([93, 184, 216, 34], port)), name: Some(name.into()) }
    }

    #[test]
    fn nothing_hooked_wants_nothing() {
        let hub = Hub::new();
        assert!(hub.is_empty());
        assert!(matches!(hub.exec(Exec::new("/bin/ls", ["ls"])), Decision::Allow));
        hub.on_exec(Filter::any(), |_| Decision::pass());
        assert!(hub.wants(Kind::Exec) && !hub.wants(Kind::Output));
    }

    #[test]
    fn the_first_that_decides_decides() {
        let hub = Hub::new();
        hub.on_exec(Filter::program("curl"), |_| Decision::deny());
        hub.on_exec(Filter::any(), |e| Decision::rewrite(e.clone().arg("--ignore-scripts")));
        assert!(matches!(hub.exec(Exec::new("/usr/bin/curl", ["curl", "x"])), Decision::Deny));
        match hub.exec(Exec::new("/usr/bin/npm", ["npm", "install"])) {
            Decision::Rewrite(e) => assert_eq!(e.argv, ["npm", "install", "--ignore-scripts"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn programs_and_hosts_match() {
        let mut e = Exec::new("C:\\Windows\\System32\\CMD.EXE", ["cmd", "/c", "dir"]);
        e.os = Os::Windows;
        assert_eq!(e.program(), "CMD");
        assert!(Filter::program("cmd").matches(&Event::Exec(e)));
        assert!(Filter::host("example.com").matches(&Event::Connect(connect("api.example.com", 443))));
        assert!(!Filter::host("example.com").matches(&Event::Connect(connect("badexample.com", 443))));
        assert!(Filter::host("93.184.216.34").matches(&Event::Connect(connect("x", 80))));
        assert!(Filter::port(443).matches(&Event::Connect(connect("x", 443))));
    }

    #[test]
    fn answers_are_typed() {
        let hub = Hub::new();
        hub.on_lookup(Filter::host("api.internal"), |_| Decision::answer(vec![IpAddr::from([10, 89, 0, 1])]));
        hub.on_exec(Filter::program("git"), |_| Decision::answer(Reply::new().stdout("Everything up-to-date\n").exit(0)));
        assert!(matches!(hub.lookup(Lookup::new("api.internal")), Decision::Answer(ips) if ips.len() == 1));
        assert!(matches!(hub.exec(Exec::new("/usr/bin/git", ["git", "push"])), Decision::Answer(r) if r.stdout == b"Everything up-to-date\n"));
        // A verdict for the wrong kind can't let anything through.
        assert!(matches!(Decision::<Connect>::from_verdict(Verdict::Rewrite(Event::Lookup(Lookup::new("x")))), Decision::Deny));
    }

    #[test]
    fn observers_see_what_was_decided_and_lose_what_they_miss() {
        let hub = Hub::new();
        hub.set_machine("dev");
        let seen = hub.observe(Interest::exec(), 2);
        hub.on_exec(Filter::program("rm"), |_| Decision::deny());
        for _ in 0..3 {
            hub.exec(Exec::new("/bin/rm", ["rm", "-rf", "/"]));
        }
        let first = seen.try_next().unwrap();
        assert_eq!((first.machine.as_str(), &first.verdict), ("dev", &Verdict::Deny));
        assert_eq!(seen.lost(), 1);
        drop(seen);
        hub.exec(Exec::new("/bin/ls", ["ls"]));
        // Its observer gone, exec is still wanted by the hook, output by nothing.
        assert!(hub.wants(Kind::Exec) && !hub.wants(Kind::Output));
    }

    #[test]
    fn subscribers_decide_from_another_thread_in_time() {
        let hub = Hub::new();
        let events = hub.subscribe(Interest::connect(), Timeout::millis(100).default_deny());
        let decider = std::thread::spawn(move || {
            // Decided in time; then held past the timeout; then dropped undecided.
            events.next_pending().unwrap().allow();
            let held = events.next_pending().unwrap();
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
            drop(events.next_pending().unwrap());
        });
        assert!(matches!(hub.connect(connect("pypi.org", 443)), Decision::Allow));
        assert!(matches!(hub.connect(connect("x", 80)), Decision::Deny));
        std::thread::sleep(Duration::from_millis(300));
        assert!(matches!(hub.connect(connect("y", 80)), Decision::Allow));
        decider.join().unwrap();
        // The subscription gone, connections aren't wanted any more.
        assert!(matches!(hub.connect(connect("z", 80)), Decision::Allow));
        assert!(!hub.wants(Kind::Connect));
    }
}
