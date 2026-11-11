//! What kernels need from the host they run on: the standard streams (stdin read by a thread of its own, so it can be
//! waited on), clocks, randomness, the memory the host could give, and whether the host trusts a certificate chain. A
//! kernel never calls the host's OS itself; it asks a `Host`, so the same kernel runs on any host, and tests can give
//! it a fake one.
//!
//! The shared part is here; what only one host knows how to do is in its folder: `macos/`, `linux/` and `windows/`
//! (with `unix.rs` for what macOS and Linux do alike). [`Std`] is this host's `Host`. Sockets and DNS are `net`'s.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
mod unix;

#[cfg(target_os = "macos")]
#[path = "macos/mod.rs"]
mod sys;
#[cfg(target_os = "linux")]
#[path = "linux/mod.rs"]
mod sys;
#[cfg(windows)]
#[path = "windows/mod.rs"]
mod sys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

/// What a certificate chain must be good for, which decides the rules it's checked by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateUsage {
    /// Ends at a root the host trusts, with each certificate in date and signed by the next.
    Any,
    /// A TLS server's, checked as a client would: everything in `Any`, plus the host's own rules for servers.
    Server,
    /// A TLS client's, checked as a server would.
    Client,
}

/// What's wrong with a certificate chain, as a set. Empty means the host trusts it.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct CertificateTrust(pub u32);

impl CertificateTrust {
    pub const TRUSTED: CertificateTrust = CertificateTrust(0);
    /// Ends at a root the host doesn't trust.
    pub const UNTRUSTED_ROOT: CertificateTrust = CertificateTrust(1);
    /// Doesn't reach a root from what it was given.
    pub const PARTIAL_CHAIN: CertificateTrust = CertificateTrust(2);
    /// A certificate is expired, or not yet valid.
    pub const NOT_TIME_VALID: CertificateTrust = CertificateTrust(4);
    /// A certificate isn't allowed for what was asked.
    pub const NOT_VALID_FOR_USAGE: CertificateTrust = CertificateTrust(8);
    pub const REVOKED: CertificateTrust = CertificateTrust(16);
    /// A signature is wrong, or uses a method no longer accepted.
    pub const BAD_SIGNATURE: CertificateTrust = CertificateTrust(32);
    /// A CA's constraints on what's under it aren't kept.
    pub const BAD_CONSTRAINTS: CertificateTrust = CertificateTrust(64);
    /// What it was given isn't a certificate.
    pub const MALFORMED: CertificateTrust = CertificateTrust(128);
    /// The host can't check certificates, so it trusts none.
    pub const CANT_CHECK: CertificateTrust = CertificateTrust(256);
    /// The server's certificate isn't for the name it was asked about.
    pub const WRONG_NAME: CertificateTrust = CertificateTrust(512);

    const NAMES: [(CertificateTrust, &'static str); 10] = [
        (Self::UNTRUSTED_ROOT, "untrusted root"),
        (Self::PARTIAL_CHAIN, "partial chain"),
        (Self::NOT_TIME_VALID, "not valid at this time"),
        (Self::NOT_VALID_FOR_USAGE, "not valid for this use"),
        (Self::REVOKED, "revoked"),
        (Self::BAD_SIGNATURE, "bad signature"),
        (Self::BAD_CONSTRAINTS, "bad constraints"),
        (Self::MALFORMED, "malformed"),
        (Self::CANT_CHECK, "can't check"),
        (Self::WRONG_NAME, "wrong name"),
    ];

    pub fn has(self, other: CertificateTrust) -> bool {
        self.0 & other.0 != 0
    }

    pub fn is_trusted(self) -> bool {
        self.0 == 0
    }

    /// This set without `other`.
    pub fn without(self, other: CertificateTrust) -> CertificateTrust {
        CertificateTrust(self.0 & !other.0)
    }
}

impl core::ops::BitOr for CertificateTrust {
    type Output = CertificateTrust;
    fn bitor(self, other: CertificateTrust) -> CertificateTrust {
        CertificateTrust(self.0 | other.0)
    }
}

impl core::ops::BitOrAssign for CertificateTrust {
    fn bitor_assign(&mut self, other: CertificateTrust) {
        self.0 |= other.0;
    }
}

impl std::fmt::Debug for CertificateTrust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_trusted() {
            return f.write_str("trusted");
        }
        let names: Vec<&str> = Self::NAMES.iter().filter(|(flag, _)| self.has(*flag)).map(|(_, name)| *name).collect();
        f.write_str(&names.join(", "))
    }
}

pub trait Host: Send + Sync + 'static {
    /// Writes to the host's stdout or stderr.
    fn write(&self, stream: Stream, data: &[u8]);
    /// Reads from the host's stdin, waiting for it: 0 is its end.
    fn read(&self, buffer: &mut [u8]) -> usize;
    /// Waits up to `timeout` (forever if None) for stdin to have data or end, or for `wake`. Whether stdin has.
    fn wait_input(&self, timeout: Option<Duration>) -> bool;
    /// Makes a `wait_input` that's waiting now, or the next one, come back at once, from any thread.
    fn wake(&self) {}
    /// Calls `notify` whenever stdin gets data or ends, from then on, so a kernel can wake what waits for it.
    fn watch_input(&self, _notify: Arc<dyn Fn() + Send + Sync>) {}
    /// Wall-clock time, in nanoseconds since 1970.
    fn wall(&self) -> u64;
    /// A clock that only goes forward, in nanoseconds since the host started it.
    fn monotonic(&self) -> u64;
    /// Fills `buffer` with random bytes.
    fn random(&self, buffer: &mut [u8]);
    /// How many CPUs the host has.
    fn cpus(&self) -> usize;
    /// How much memory the host could give now without paging, in bytes.
    fn memory_available(&self) -> u64 {
        u64::MAX
    }
    /// Whether stdin and stdout are a terminal, which can be the machine's console.
    fn is_terminal(&self) -> bool {
        false
    }
    /// Puts the terminal in raw mode, for the machine to do its own line editing and echo, or puts it back.
    fn set_raw(&self, _raw: bool) {}
    /// The terminal's rows and columns.
    fn terminal_size(&self) -> Option<(u16, u16)> {
        None
    }
    /// Checks a certificate chain against the host's own trust settings, which alone decide which roots are trusted.
    /// `chain` starts with the certificate being checked, in DER, followed by any that might lead to a root, in any
    /// order. `name` is the server's name, for `CertificateUsage::Server`, or None to leave names to the caller. `at`
    /// is when, in seconds since 1970, or now. Nothing is fetched from the network, to complete a chain or to check
    /// revocation, so a chain missing a certificate isn't trusted. Every failure the host finds is reported, not just
    /// the first. A host that can't check trusts nothing.
    fn check_certificates(&self, chain: &[&[u8]], usage: CertificateUsage, name: Option<&str>, at: Option<i64>) -> CertificateTrust {
        let _ = (chain, usage, name, at);
        CertificateTrust::CANT_CHECK
    }
}

/// What's been read from the host's stdin and not yet given out, filled by a thread of its own, so a kernel can wait
/// for it with a timeout (`poll`, `select`) as well as read it.
#[derive(Default)]
struct Input {
    /// What's been read, whether stdin has ended, and whether a waiter has been woken.
    state: Mutex<(VecDeque<u8>, bool, bool)>,
    ready: Condvar,
    /// Called whenever data comes or stdin ends.
    watchers: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
}

impl Input {
    fn get() -> &'static Arc<Input> {
        static INPUT: OnceLock<Arc<Input>> = OnceLock::new();
        INPUT.get_or_init(|| {
            let input = Arc::new(Input::default());
            let filler = input.clone();
            let _ = std::thread::Builder::new().name("carbon stdin".into()).spawn(move || {
                let mut buffer = [0u8; 4096];
                loop {
                    let n = std::io::stdin().lock().read(&mut buffer).unwrap_or(0);
                    let mut state = filler.state.lock().unwrap_or_else(|p| p.into_inner());
                    if n == 0 {
                        state.1 = true;
                    } else {
                        state.0.extend(&buffer[..n]);
                    }
                    filler.ready.notify_all();
                    drop(state);
                    let watchers = filler.watchers.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    for notify in watchers {
                        notify();
                    }
                    if n == 0 {
                        return;
                    }
                }
            });
            input
        })
    }

    /// Waits for data, the end or a wake, up to `deadline`. Whether data or the end came.
    fn wait(&self, deadline: Option<Instant>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if !state.0.is_empty() || state.1 {
                return true;
            }
            if state.2 {
                state.2 = false;
                return false;
            }
            match deadline {
                None => state = self.ready.wait(state).unwrap_or_else(|p| p.into_inner()),
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return false;
                    }
                    state = self.ready.wait_timeout(state, deadline - now).unwrap_or_else(|p| p.into_inner()).0;
                }
            }
        }
    }

    fn wake(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).2 = true;
        self.ready.notify_all();
    }

    fn read(&self, buffer: &mut [u8]) -> usize {
        while !self.wait(None) {}
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let n = buffer.len().min(state.0.len());
        for (b, byte) in buffer.iter_mut().zip(state.0.drain(..n)) {
            *b = byte;
        }
        n
    }
}

/// This host, through Rust's `std` and, for the rest, its own folder.
pub struct Std {
    start: Instant,
    /// Captured output, for tests, instead of the real stdout and stderr.
    capture: Option<Mutex<Vec<u8>>>,
}

impl Std {
    pub fn new() -> Std {
        Std { start: Instant::now(), capture: None }
    }

    /// One that keeps what's written instead of printing it: see `captured`.
    pub fn capturing() -> Std {
        Std { start: Instant::now(), capture: Some(Mutex::new(Vec::new())) }
    }

    /// Everything written so far, to stdout and stderr alike, if capturing.
    pub fn captured(&self) -> Vec<u8> {
        self.capture.as_ref().map(|c| c.lock().map(|c| c.clone()).unwrap_or_default()).unwrap_or_default()
    }
}

impl Default for Std {
    fn default() -> Std {
        Std::new()
    }
}

impl Host for Std {
    fn write(&self, stream: Stream, data: &[u8]) {
        if let Some(capture) = &self.capture {
            if let Ok(mut capture) = capture.lock() {
                capture.extend_from_slice(data);
            }
            return;
        }
        let _ = match stream {
            Stream::Out => std::io::stdout().lock().write_all(data).and_then(|()| std::io::stdout().flush()),
            Stream::Err => std::io::stderr().lock().write_all(data),
        };
    }

    fn read(&self, buffer: &mut [u8]) -> usize {
        Input::get().read(buffer)
    }

    fn wait_input(&self, timeout: Option<Duration>) -> bool {
        Input::get().wait(timeout.map(|t| Instant::now() + t))
    }

    fn wake(&self) {
        Input::get().wake();
    }

    fn watch_input(&self, notify: Arc<dyn Fn() + Send + Sync>) {
        Input::get().watchers.lock().unwrap_or_else(|p| p.into_inner()).push(notify);
    }

    fn wall(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
    }

    fn monotonic(&self) -> u64 {
        self.start.elapsed().as_nanos() as u64
    }

    fn random(&self, buffer: &mut [u8]) {
        sys::random(buffer);
    }

    fn cpus(&self) -> usize {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
    }

    fn memory_available(&self) -> u64 {
        sys::memory_available()
    }

    fn is_terminal(&self) -> bool {
        use std::io::IsTerminal;
        self.capture.is_none() && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }

    fn set_raw(&self, raw: bool) {
        sys::set_raw(raw);
    }

    fn terminal_size(&self) -> Option<(u16, u16)> {
        sys::terminal_size()
    }

    fn check_certificates(&self, chain: &[&[u8]], usage: CertificateUsage, name: Option<&str>, at: Option<i64>) -> CertificateTrust {
        if chain.is_empty() {
            return CertificateTrust::MALFORMED;
        }
        sys::check_certificates(chain, usage, name, at)
    }
}

// Linux's checks, run on every host against test roots, so they're tested where they're written.
#[cfg(all(test, not(target_os = "linux")))]
#[path = "linux/trust.rs"]
#[allow(dead_code)]
mod linux_trust;

#[cfg(test)]
mod tests {
    use super::{CertificateTrust as T, CertificateUsage as U, Host, Std};

    // Made by tests/certs/make.sh: a test root, intermediate, and server and client leaves under it for carbon.test; a
    // self-signed server certificate; and registry-1.docker.io's chain as its server sent it.
    const ROOT: &[u8] = include_bytes!("../tests/certs/root.der");
    const INTERMEDIATE: &[u8] = include_bytes!("../tests/certs/intermediate.der");
    const SERVER: &[u8] = include_bytes!("../tests/certs/server.der");
    const CLIENT: &[u8] = include_bytes!("../tests/certs/client.der");
    const SELF_SIGNED: &[u8] = include_bytes!("../tests/certs/selfsigned.der");
    const DOCKER: [&[u8]; 3] = [
        include_bytes!("../tests/certs/docker-0.der"),
        include_bytes!("../tests/certs/docker-1.der"),
        include_bytes!("../tests/certs/docker-2.der"),
    ];
    /// 2027-01-15, when every certificate here is in date: the Docker leaf runs from 2026-08-25 to 2027-03-10, the
    /// test leaves from 2026-11-11 to 2027-12-13.
    const AT: Option<i64> = Some(1_800_000_000);
    /// 2027-05-12, after the Docker leaf; 2023-11-14, before the test certificates.
    const AFTER_DOCKER: Option<i64> = Some(1_810_000_000);
    const BEFORE_TESTS: Option<i64> = Some(1_700_000_000);

    fn check(chain: &[&[u8]], usage: U, name: Option<&str>, at: Option<i64>) -> T {
        Std::new().check_certificates(chain, usage, name, at)
    }

    #[test]
    fn rubbish_isnt_a_certificate() {
        assert_eq!(check(&[b"not a certificate"], U::Any, None, AT), T::MALFORMED);
        assert_eq!(check(&[], U::Server, None, AT), T::MALFORMED);
    }

    #[test]
    fn a_real_server_chain_is_trusted_for_its_names() {
        assert_eq!(check(&DOCKER, U::Server, Some("registry-1.docker.io"), AT), T::TRUSTED);
        assert_eq!(check(&DOCKER, U::Server, Some("docker.io"), AT), T::TRUSTED);
        assert_eq!(check(&DOCKER, U::Server, None, AT), T::TRUSTED);
        assert_eq!(check(&DOCKER, U::Any, None, AT), T::TRUSTED);
    }

    #[test]
    fn a_real_chain_for_another_name_or_time_or_use() {
        assert_eq!(check(&DOCKER, U::Server, Some("carbon.test"), AT), T::WRONG_NAME);
        assert_eq!(check(&DOCKER, U::Server, Some("registry-1.docker.io"), AFTER_DOCKER), T::NOT_TIME_VALID);
        assert_eq!(check(&DOCKER, U::Client, None, AT), T::NOT_VALID_FOR_USAGE);
    }

    #[test]
    fn a_root_the_host_doesnt_trust() {
        assert_eq!(check(&[SERVER, INTERMEDIATE, ROOT], U::Server, Some("carbon.test"), AT), T::UNTRUSTED_ROOT);
        // In any order after the first.
        assert_eq!(check(&[SERVER, ROOT, INTERMEDIATE], U::Server, Some("carbon.test"), AT), T::UNTRUSTED_ROOT);
        assert_eq!(check(&[SELF_SIGNED], U::Server, Some("carbon.test"), AT), T::UNTRUSTED_ROOT);
    }

    #[test]
    fn a_chain_that_stops_short() {
        assert!(check(&[SERVER, INTERMEDIATE], U::Server, Some("carbon.test"), AT).has(T::PARTIAL_CHAIN));
        assert!(check(&[SERVER], U::Server, Some("carbon.test"), AT).has(T::PARTIAL_CHAIN));
    }

    #[test]
    fn every_failure_at_once() {
        assert_eq!(check(&[SELF_SIGNED], U::Server, Some("carbon.test"), BEFORE_TESTS), T::UNTRUSTED_ROOT | T::NOT_TIME_VALID);
        assert_eq!(check(&[CLIENT, INTERMEDIATE, ROOT], U::Server, None, AT), T::UNTRUSTED_ROOT | T::NOT_VALID_FOR_USAGE);
        assert_eq!(check(&[SERVER, INTERMEDIATE, ROOT], U::Server, Some("other.test"), AT), T::UNTRUSTED_ROOT | T::WRONG_NAME);
    }

    #[test]
    fn names_read_well() {
        assert_eq!(format!("{:?}", T::TRUSTED), "trusted");
        assert_eq!(format!("{:?}", T::UNTRUSTED_ROOT | T::WRONG_NAME), "untrusted root, wrong name");
    }

    #[test]
    fn the_host_has_memory_and_randomness() {
        let host = Std::new();
        assert!(host.memory_available() > 16 << 20);
        let (mut a, mut b) = ([0u8; 600], [0u8; 600]);
        host.random(&mut a);
        host.random(&mut b);
        assert_ne!(a, b);
    }

    /// Linux's checks against the test root, as if it were in the system's bundle.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn linux_checks_against_its_roots() {
        use super::linux_trust::check_against;
        let root = pki_types::CertificateDer::from(ROOT);
        let roots = [webpki::anchor_from_trusted_cert(&root).expect("the test root")];
        assert_eq!(check_against(&[SERVER, INTERMEDIATE], U::Server, Some("carbon.test"), AT, &roots), T::TRUSTED);
        assert_eq!(check_against(&[SERVER, INTERMEDIATE, ROOT], U::Server, Some("other.test"), AT, &roots), T::WRONG_NAME);
        assert_eq!(check_against(&[CLIENT, INTERMEDIATE], U::Server, None, AT, &roots), T::NOT_VALID_FOR_USAGE);
        assert_eq!(check_against(&[CLIENT, INTERMEDIATE], U::Client, None, AT, &roots), T::TRUSTED);
        assert_eq!(check_against(&[SERVER, INTERMEDIATE], U::Server, None, BEFORE_TESTS, &roots), T::NOT_TIME_VALID);
        // The Docker chain doesn't reach this root, or any it was given: its top certificate is cross-signed.
        assert_eq!(check_against(&DOCKER, U::Server, Some("docker.io"), AT, &roots), T::PARTIAL_CHAIN);
        assert_eq!(check_against(&[SELF_SIGNED], U::Server, None, BEFORE_TESTS, &roots), T::UNTRUSTED_ROOT | T::NOT_TIME_VALID);
        assert!(check_against(&[SERVER], U::Server, None, AT, &[]).has(T::PARTIAL_CHAIN));
        // webpki stops at the use first; the untrusted root is still found.
        assert_eq!(check_against(&[CLIENT, INTERMEDIATE, ROOT], U::Server, None, AT, &[]), T::UNTRUSTED_ROOT | T::NOT_VALID_FOR_USAGE);
        assert_eq!(check_against(&[b"junk"], U::Any, None, AT, &roots), T::MALFORMED);
    }
}
