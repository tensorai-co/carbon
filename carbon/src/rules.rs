//! Rules: decisions that need no code, checked on the asking thread with no call out. They're what `carbon run`'s
//! `--deny-exec`, `--allow` and `--deny` make, and a [`Hooks`] like any other, so they go in a machine's chain where
//! they're added.

use std::net::IpAddr;

use hooks::{Connect, Decision, Exec, Hooks, Interest};

use crate::error::{Error, Result};

/// Programs that may not start, and where the network may and may not go.
///
/// ```
/// let rules = carbon::Rules::new()
///     .deny_exec("rm -rf *")
///     .deny("0.0.0.0/0").unwrap()
///     .allow("pypi.org:443").unwrap();
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    deny_exec: Vec<Vec<String>>,
    allow: Vec<Target>,
    deny: Vec<Target>,
}

/// Where a network rule applies: a name (and the names under it) or a network of addresses, on a port or any.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Name(String, Option<u16>),
    Net(IpAddr, u8, Option<u16>),
}

impl Target {
    fn parse(text: &str) -> Result<Target> {
        let bad = || Error::invalid(format!("{text} isn't a host, an address or a network: pypi.org, pypi.org:443, 10.0.0.0/8, [::1]:53"));
        // A port at the end, after a name, an IPv4 address or network, or a bracketed IPv6 one.
        let (what, port) = if let Some(rest) = text.strip_prefix('[') {
            let (inside, after) = rest.split_once(']').ok_or_else(bad)?;
            let port = match after.strip_prefix(':') {
                Some(p) => Some(p.parse().map_err(|_| bad())?),
                None if after.is_empty() => None,
                None => return Err(bad()),
            };
            (inside.to_string(), port)
        } else if text.matches(':').count() == 1 {
            let (what, p) = text.split_once(':').ok_or_else(bad)?;
            (what.to_string(), Some(p.parse().map_err(|_| bad())?))
        } else {
            (text.to_string(), None)
        };
        let (addr, prefix) = match what.split_once('/') {
            Some((a, p)) => (a, Some(p.parse::<u8>().map_err(|_| bad())?)),
            None => (what.as_str(), None),
        };
        if let Ok(ip) = addr.parse::<IpAddr>() {
            let most = if ip.is_ipv4() { 32 } else { 128 };
            let prefix = prefix.unwrap_or(most);
            if prefix > most {
                return Err(bad());
            }
            return Ok(Target::Net(ip, prefix, port));
        }
        if prefix.is_some() || addr.is_empty() || !addr.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '*') {
            return Err(bad());
        }
        Ok(Target::Name(addr.trim_start_matches("*.").to_ascii_lowercase(), port))
    }

    fn matches(&self, c: &Connect) -> bool {
        let port_ok = |port: &Option<u16>| port.is_none_or(|p| p == c.port());
        match self {
            Target::Name(name, port) => port_ok(port) && c.name().is_some_and(|n| hooks::Filter::name_matches(name, n)),
            Target::Net(net, prefix, port) => port_ok(port) && in_network(c.ip(), *net, *prefix),
        }
    }
}

fn in_network(ip: IpAddr, net: IpAddr, prefix: u8) -> bool {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    };
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(b)) => prefix == 0 || (u32::from(a) ^ u32::from(b)) >> (32 - u32::from(prefix)) == 0,
        (IpAddr::V6(a), IpAddr::V6(b)) => prefix == 0 || (u128::from(a) ^ u128::from(b)) >> (128 - u32::from(prefix)) == 0,
        _ => false,
    }
}

/// Whether `text` matches `pattern`, where `*` is any run of characters and `?` any one.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

impl Rules {
    pub fn new() -> Rules {
        Rules::default()
    }

    /// Refuses to start a program whose arguments match `pattern`, words split by spaces: the first is the program's
    /// name (`rm`, not `/bin/rm`), each after it one argument, with `*` and `?` as a shell has them; a last word that's
    /// only `*` takes any arguments left, or none. `rm -rf *` refuses `rm -rf /` and `rm -rf a b`, not `rm -r -f /`.
    pub fn deny_exec(mut self, pattern: &str) -> Rules {
        let words: Vec<String> = pattern.split_whitespace().map(String::from).collect();
        if !words.is_empty() {
            self.deny_exec.push(words);
        }
        self
    }

    /// Lets connections to `target` through, whatever [`Rules::deny`] says: a host (`pypi.org`, which takes its
    /// subdomains too), an address, or a network (`10.0.0.0/8`), each with a port or not (`pypi.org:443`).
    pub fn allow(mut self, target: &str) -> Result<Rules> {
        self.allow.push(Target::parse(target)?);
        Ok(self)
    }

    /// Refuses connections to `target`, unless an [`Rules::allow`] lets them through: `0.0.0.0/0` and `::/0` for
    /// everything.
    pub fn deny(mut self, target: &str) -> Result<Rules> {
        self.deny.push(Target::parse(target)?);
        Ok(self)
    }

    pub fn is_empty(&self) -> bool {
        self.deny_exec.is_empty() && self.allow.is_empty() && self.deny.is_empty()
    }

    fn exec_denied(&self, e: &Exec) -> bool {
        self.deny_exec.iter().any(|pattern| {
            let mut words = std::iter::once(e.program()).chain(e.args().iter().map(String::as_str));
            for (i, p) in pattern.iter().enumerate() {
                if p == "*" && i == pattern.len() - 1 && i > 0 {
                    return true;
                }
                match words.next() {
                    Some(word) if glob(p, word) => {}
                    _ => return false,
                }
            }
            words.next().is_none()
        })
    }
}

impl Hooks for Rules {
    fn exec(&self, e: &Exec) -> Decision<Exec> {
        if self.exec_denied(e) { Decision::deny() } else { Decision::pass() }
    }

    fn connect(&self, c: &Connect) -> Decision<Connect> {
        if self.allow.iter().any(|t| t.matches(c)) {
            Decision::allow()
        } else if self.deny.iter().any(|t| t.matches(c)) {
            Decision::refuse()
        } else {
            Decision::pass()
        }
    }

    fn interest(&self) -> Interest {
        let mut interest = Interest::none();
        if !self.deny_exec.is_empty() {
            interest |= Interest::exec();
        }
        if !self.allow.is_empty() || !self.deny.is_empty() {
            interest |= Interest::connect();
        }
        interest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn to(name: Option<&str>, addr: &str) -> Connect {
        Connect { proto: hooks::Proto::Tcp, to: addr.parse::<SocketAddr>().unwrap(), name: name.map(String::from) }
    }

    #[test]
    fn exec_patterns() {
        let rules = Rules::new().deny_exec("rm -rf *").deny_exec("shutdown *").deny_exec("curl ?ttp*");
        let denied = |argv: &[&str]| rules.exec_denied(&Exec::new(&format!("/bin/{}", argv[0]), argv.iter().copied()));
        assert!(denied(&["rm", "-rf", "/"]));
        assert!(denied(&["rm", "-rf", "a", "b"]));
        assert!(denied(&["rm", "-rf"]));
        assert!(!denied(&["rm", "-r", "-f", "/"]));
        assert!(!denied(&["ls", "-rf"]));
        assert!(denied(&["shutdown"]));
        assert!(denied(&["curl", "http://x"]));
        assert!(!denied(&["curl", "-s", "http://x"]));
        assert_eq!(rules.interest(), Interest::exec());
    }

    #[test]
    fn allow_beats_deny() {
        let rules = Rules::new().deny("0.0.0.0/0").unwrap().deny("::/0").unwrap().allow("pypi.org:443").unwrap().allow("10.89.0.0/16").unwrap();
        assert!(matches!(rules.connect(&to(Some("pypi.org"), "151.101.0.223:443")), Decision::Allow));
        assert!(matches!(rules.connect(&to(Some("files.pypi.org"), "151.101.0.223:443")), Decision::Allow));
        assert!(matches!(rules.connect(&to(Some("pypi.org"), "151.101.0.223:80")), Decision::Deny));
        assert!(matches!(rules.connect(&to(None, "10.89.3.4:22")), Decision::Allow));
        assert!(matches!(rules.connect(&to(None, "[2001:db8::1]:443")), Decision::Deny));
        assert!(Rules::new().allow("10.0.0.0/33").is_err());
        assert!(Rules::new().allow("not a host").is_err());
        assert_eq!(Target::parse("[::1]:53").unwrap(), Target::Net("::1".parse().unwrap(), 128, Some(53)));
    }
}
