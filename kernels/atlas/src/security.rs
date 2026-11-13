//! Security as Windows describes it: SIDs, ACEs and ACLs, security descriptors in both their forms, SDDL, privileges,
//! the machine's accounts, and tokens.
//!
//! Everyone is one user, `carbon` (`registry::USER`), an administrator, elevated, at high integrity, as on a machine
//! with UAC turned off. Each process has a primary token, a copy of its parent's; a program can change its own copy
//! (enable privileges, lower its integrity) without changing anyone else's.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

pub const ERROR_INVALID_SID: u32 = 1337;
pub const ERROR_INVALID_ACL: u32 = 1336;
pub const ERROR_INVALID_PARAMETER: u32 = 87;

// --- SIDs ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sid {
    /// The identifier authority: 48 bits.
    pub authority: u64,
    pub subs: Vec<u32>,
}

impl Sid {
    pub fn new(authority: u64, subs: &[u32]) -> Sid {
        Sid { authority, subs: subs.to_vec() }
    }

    /// `S-1-5-32-544`'s form, or None for one that isn't a SID.
    pub fn parse(s: &str) -> Option<Sid> {
        let rest = s.strip_prefix("S-").or_else(|| s.strip_prefix("s-"))?;
        let mut parts = rest.split('-');
        if parts.next()? != "1" {
            return None;
        }
        let authority = parts.next()?;
        let authority = match authority.strip_prefix("0x").or_else(|| authority.strip_prefix("0X")) {
            Some(hex) => u64::from_str_radix(hex, 16).ok()?,
            None => authority.parse().ok()?,
        };
        let subs: Vec<u32> = parts.map(|p| p.parse().ok()).collect::<Option<_>>()?;
        (authority < 1 << 48 && subs.len() <= 15).then_some(Sid { authority, subs })
    }

    /// A SID in its binary form, if `bytes` starts with one.
    pub fn from_bytes(bytes: &[u8]) -> Option<Sid> {
        let (&revision, &count) = (bytes.first()?, bytes.get(1)?);
        if revision != 1 || count > 15 || bytes.len() < 8 + 4 * usize::from(count) {
            return None;
        }
        let authority = bytes[2..8].iter().fold(0u64, |a, &b| a << 8 | u64::from(b));
        let subs = (0..usize::from(count)).map(|i| u32::from_le_bytes(bytes[8 + 4 * i..12 + 4 * i].try_into().unwrap())).collect();
        Some(Sid { authority, subs })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![1, self.subs.len() as u8];
        b.extend(&self.authority.to_be_bytes()[2..]);
        for s in &self.subs {
            b.extend(s.to_le_bytes());
        }
        b
    }

    /// The machine's own accounts and groups: `S-1-5-21-…` with this one's RID.
    pub fn local(rid: u32) -> Sid {
        let mut sid = machine();
        sid.subs.push(rid);
        sid
    }
}

impl std::fmt::Display for Sid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.authority >> 32 != 0 {
            write!(f, "S-1-0x{:012X}", self.authority)?;
        } else {
            write!(f, "S-1-{}", self.authority)?;
        }
        for s in &self.subs {
            write!(f, "-{s}")?;
        }
        Ok(())
    }
}

/// The user's SID, and the machine's domain SID it's in.
pub fn user() -> Sid {
    Sid::parse(crate::registry::USER).expect("the user's SID")
}

pub fn machine() -> Sid {
    let mut sid = user();
    sid.subs.pop();
    sid
}

/// The logon session everything runs in, and its SID.
pub const LOGON: u64 = 0x6_c4f8;

pub fn logon_sid() -> Sid {
    Sid::new(5, &[5, (LOGON >> 32) as u32, LOGON as u32])
}

/// SDDL's two-letter names for SIDs that are the same on every machine.
const ALIASES: &[(&str, u64, &[u32])] = &[
    ("WD", 1, &[0]),
    ("CO", 3, &[0]),
    ("CG", 3, &[1]),
    ("OW", 3, &[4]),
    ("NU", 5, &[2]),
    ("IU", 5, &[4]),
    ("SU", 5, &[6]),
    ("AN", 5, &[7]),
    ("PS", 5, &[10]),
    ("AU", 5, &[11]),
    ("RC", 5, &[12]),
    ("SY", 5, &[18]),
    ("LS", 5, &[19]),
    ("NS", 5, &[20]),
    ("WR", 5, &[33]),
    ("BA", 5, &[32, 544]),
    ("BU", 5, &[32, 545]),
    ("BG", 5, &[32, 546]),
    ("PU", 5, &[32, 547]),
    ("AO", 5, &[32, 548]),
    ("SO", 5, &[32, 549]),
    ("PO", 5, &[32, 550]),
    ("BO", 5, &[32, 551]),
    ("RE", 5, &[32, 552]),
    ("RU", 5, &[32, 554]),
    ("RD", 5, &[32, 555]),
    ("NO", 5, &[32, 556]),
    ("MU", 5, &[32, 558]),
    ("LU", 5, &[32, 559]),
    ("IS", 5, &[32, 568]),
    ("CY", 5, &[32, 569]),
    ("ER", 5, &[32, 573]),
    ("AC", 15, &[2, 1]),
    ("LW", 16, &[4096]),
    ("ME", 16, &[8192]),
    ("MP", 16, &[8448]),
    ("HI", 16, &[12288]),
    ("SI", 16, &[16384]),
];

/// SDDL's names for the machine's own accounts, by RID.
const LOCAL_ALIASES: &[(&str, u32)] = &[("LA", 500), ("LG", 501)];

/// A SID as SDDL writes it: an alias, or `S-1-…`.
pub fn sid_to_sddl(sid: &Sid) -> String {
    ALIASES.iter().find(|(_, a, s)| *a == sid.authority && *s == sid.subs.as_slice()).map(|(n, _, _)| n.to_string()).unwrap_or_else(|| sid.to_string())
}

/// A SID from SDDL or `ConvertStringSidToSid`: `S-1-…` or an alias.
pub fn sid_from_sddl(s: &str) -> Option<Sid> {
    if let Some((_, a, subs)) = ALIASES.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(s)) {
        return Some(Sid::new(*a, subs));
    }
    if let Some((_, rid)) = LOCAL_ALIASES.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)) {
        return Some(Sid::local(*rid));
    }
    Sid::parse(s)
}

/// `CreateWellKnownSid`'s types (`WELL_KNOWN_SID_TYPE`): the SID, given the domain for the account ones.
pub fn well_known(kind: u32, domain: Option<&Sid>) -> Option<Sid> {
    let s = |a: u64, subs: &[u32]| Some(Sid::new(a, subs));
    let builtin = |rid: u32| Sid::new(5, &[32, rid]);
    match kind {
        0 => s(0, &[0]),
        1 => s(1, &[0]),
        2 => s(2, &[0]),
        3 => s(3, &[0]),
        4 => s(3, &[1]),
        5 => s(3, &[2]),
        6 => s(3, &[3]),
        7 => s(5, &[]),
        8..=11 => s(5, &[kind - 7]),
        12..=14 => s(5, &[kind - 6]),
        15 => s(5, &[9]),
        16 => s(5, &[10]),
        17 => s(5, &[11]),
        18 => s(5, &[12]),
        19 => s(5, &[13]),
        20 => s(5, &[14]),
        21 => s(5, &[5]),
        22 => s(5, &[18]),
        23 => s(5, &[19]),
        24 => s(5, &[20]),
        25 => s(5, &[32]),
        26..=34 => Some(builtin(544 + kind - 26)),
        35..=37 => Some(builtin(554 + kind - 35)),
        38 | 39 => {
            let mut sid = domain.cloned().unwrap_or_else(machine);
            sid.subs.push(if kind == 38 { 500 } else { 501 });
            Some(sid)
        }
        65 => s(16, &[0]),
        66 => s(16, &[4096]),
        67 => s(16, &[8192]),
        68 => s(16, &[12288]),
        69 => s(16, &[16384]),
        70 => s(5, &[33]),
        71 => s(3, &[4]),
        _ => None,
    }
}

/// `SID_NAME_USE`s.
pub const SID_TYPE_USER: u32 = 1;
pub const SID_TYPE_GROUP: u32 = 2;
pub const SID_TYPE_ALIAS: u32 = 4;
pub const SID_TYPE_WELL_KNOWN: u32 = 5;
pub const SID_TYPE_LABEL: u32 = 10;

/// The names `LookupAccountSid` gives, and `LookupAccountName` takes: (SID, domain, name, use).
pub fn accounts() -> Vec<(Sid, &'static str, &'static str, u32)> {
    let builtin = |rid: u32| Sid::new(5, &[32, rid]);
    let nt = |rid: u32| Sid::new(5, &[rid]);
    vec![
        (user(), "CARBON", "carbon", SID_TYPE_USER),
        (Sid::local(500), "CARBON", "Administrator", SID_TYPE_USER),
        (Sid::local(501), "CARBON", "Guest", SID_TYPE_USER),
        (Sid::local(513), "CARBON", "None", SID_TYPE_GROUP),
        (Sid::new(1, &[0]), "", "Everyone", SID_TYPE_WELL_KNOWN),
        (Sid::new(2, &[0]), "", "LOCAL", SID_TYPE_WELL_KNOWN),
        (Sid::new(2, &[1]), "", "CONSOLE LOGON", SID_TYPE_WELL_KNOWN),
        (Sid::new(3, &[0]), "", "CREATOR OWNER", SID_TYPE_WELL_KNOWN),
        (Sid::new(3, &[1]), "", "CREATOR GROUP", SID_TYPE_WELL_KNOWN),
        (Sid::new(3, &[4]), "", "OWNER RIGHTS", SID_TYPE_WELL_KNOWN),
        (nt(2), "NT AUTHORITY", "NETWORK", SID_TYPE_WELL_KNOWN),
        (nt(4), "NT AUTHORITY", "INTERACTIVE", SID_TYPE_WELL_KNOWN),
        (nt(6), "NT AUTHORITY", "SERVICE", SID_TYPE_WELL_KNOWN),
        (nt(7), "NT AUTHORITY", "ANONYMOUS LOGON", SID_TYPE_WELL_KNOWN),
        (nt(10), "NT AUTHORITY", "SELF", SID_TYPE_WELL_KNOWN),
        (nt(11), "NT AUTHORITY", "Authenticated Users", SID_TYPE_WELL_KNOWN),
        (nt(12), "NT AUTHORITY", "RESTRICTED", SID_TYPE_WELL_KNOWN),
        (nt(15), "NT AUTHORITY", "This Organization", SID_TYPE_WELL_KNOWN),
        (nt(18), "NT AUTHORITY", "SYSTEM", SID_TYPE_WELL_KNOWN),
        (nt(19), "NT AUTHORITY", "LOCAL SERVICE", SID_TYPE_WELL_KNOWN),
        (nt(20), "NT AUTHORITY", "NETWORK SERVICE", SID_TYPE_WELL_KNOWN),
        (nt(113), "NT AUTHORITY", "Local account", SID_TYPE_WELL_KNOWN),
        (nt(114), "NT AUTHORITY", "Local account and member of Administrators group", SID_TYPE_WELL_KNOWN),
        (Sid::new(5, &[64, 10]), "NT AUTHORITY", "NTLM Authentication", SID_TYPE_WELL_KNOWN),
        (builtin(544), "BUILTIN", "Administrators", SID_TYPE_ALIAS),
        (builtin(545), "BUILTIN", "Users", SID_TYPE_ALIAS),
        (builtin(546), "BUILTIN", "Guests", SID_TYPE_ALIAS),
        (builtin(547), "BUILTIN", "Power Users", SID_TYPE_ALIAS),
        (builtin(551), "BUILTIN", "Backup Operators", SID_TYPE_ALIAS),
        (builtin(555), "BUILTIN", "Remote Desktop Users", SID_TYPE_ALIAS),
        (Sid::new(15, &[2, 1]), "APPLICATION PACKAGE AUTHORITY", "ALL APPLICATION PACKAGES", SID_TYPE_WELL_KNOWN),
        (Sid::new(16, &[0]), "Mandatory Label", "Untrusted Mandatory Level", SID_TYPE_LABEL),
        (Sid::new(16, &[4096]), "Mandatory Label", "Low Mandatory Level", SID_TYPE_LABEL),
        (Sid::new(16, &[8192]), "Mandatory Label", "Medium Mandatory Level", SID_TYPE_LABEL),
        (Sid::new(16, &[8448]), "Mandatory Label", "Medium Plus Mandatory Level", SID_TYPE_LABEL),
        (Sid::new(16, &[12288]), "Mandatory Label", "High Mandatory Level", SID_TYPE_LABEL),
        (Sid::new(16, &[16384]), "Mandatory Label", "System Mandatory Level", SID_TYPE_LABEL),
    ]
}

// --- ACEs and ACLs ---

pub const ACCESS_ALLOWED: u8 = 0;
pub const ACCESS_DENIED: u8 = 1;
pub const SYSTEM_AUDIT: u8 = 2;
pub const SYSTEM_MANDATORY_LABEL: u8 = 0x11;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    pub kind: u8,
    pub flags: u8,
    pub mask: u32,
    pub sid: Sid,
    /// An ACE of a kind atlas doesn't take apart (object ACEs, callback ACEs): its bytes, kept as they are.
    pub raw: Option<Vec<u8>>,
}

impl Ace {
    pub fn new(kind: u8, flags: u8, mask: u32, sid: Sid) -> Ace {
        Ace { kind, flags, mask, sid, raw: None }
    }

    /// The ACE at the start of `bytes`, and its size.
    pub fn from_bytes(bytes: &[u8]) -> Option<(Ace, usize)> {
        let size = usize::from(u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?));
        let body = bytes.get(..size).filter(|_| size >= 8)?;
        let (kind, flags) = (body[0], body[1]);
        let ace = match kind {
            ACCESS_ALLOWED | ACCESS_DENIED | SYSTEM_AUDIT | 3 | SYSTEM_MANDATORY_LABEL => {
                let mask = u32::from_le_bytes(body[4..8].try_into().unwrap());
                Ace { kind, flags, mask, sid: Sid::from_bytes(&body[8..])?, raw: None }
            }
            _ => Ace { kind, flags, mask: 0, sid: Sid::new(0, &[]), raw: Some(body.to_vec()) },
        };
        Some((ace, size))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        let sid = self.sid.to_bytes();
        let size = 8 + sid.len();
        let mut b = vec![self.kind, self.flags];
        b.extend((size as u16).to_le_bytes());
        b.extend(self.mask.to_le_bytes());
        b.extend(sid);
        b
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acl {
    pub revision: u8,
    pub aces: Vec<Ace>,
    /// How big it was made (`InitializeAcl`'s length): room for ACEs to come. At least what its ACEs take.
    pub size: usize,
}

impl Acl {
    pub fn new(aces: Vec<Ace>) -> Acl {
        let mut acl = Acl { revision: 2, aces, size: 0 };
        acl.size = acl.used();
        acl
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Acl> {
        let revision = *bytes.first()?;
        if !(2..=4).contains(&revision) {
            return None;
        }
        let size = usize::from(u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?));
        let count = usize::from(u16::from_le_bytes(bytes.get(4..6)?.try_into().ok()?));
        let body = bytes.get(..size).filter(|_| size >= 8)?;
        let mut aces = Vec::with_capacity(count);
        let mut at = 8;
        for _ in 0..count {
            let (ace, len) = Ace::from_bytes(body.get(at..)?)?;
            aces.push(ace);
            at += len;
        }
        Some(Acl { revision, aces, size })
    }

    /// The bytes its header and ACEs take (`AclBytesInUse`).
    pub fn used(&self) -> usize {
        8 + self.aces.iter().map(|a| a.to_bytes().len()).sum::<usize>()
    }

    /// Its bytes, `size` of them.
    pub fn to_bytes(&self) -> Vec<u8> {
        let size = self.size.max(self.used());
        let mut b = vec![self.revision, 0];
        b.extend((size as u16).to_le_bytes());
        b.extend((self.aces.len() as u16).to_le_bytes());
        b.extend([0, 0]);
        for ace in &self.aces {
            b.extend(ace.to_bytes());
        }
        b.resize(size, 0);
        b
    }
}

// --- Security descriptors ---

pub const SE_OWNER_DEFAULTED: u16 = 0x1;
pub const SE_GROUP_DEFAULTED: u16 = 0x2;
pub const SE_DACL_PRESENT: u16 = 0x4;
pub const SE_DACL_DEFAULTED: u16 = 0x8;
pub const SE_SACL_PRESENT: u16 = 0x10;
pub const SE_SACL_DEFAULTED: u16 = 0x20;
pub const SE_DACL_AUTO_INHERIT_REQ: u16 = 0x100;
pub const SE_SACL_AUTO_INHERIT_REQ: u16 = 0x200;
pub const SE_DACL_AUTO_INHERITED: u16 = 0x400;
pub const SE_SACL_AUTO_INHERITED: u16 = 0x800;
pub const SE_DACL_PROTECTED: u16 = 0x1000;
pub const SE_SACL_PROTECTED: u16 = 0x2000;
pub const SE_SELF_RELATIVE: u16 = 0x8000;

/// A security descriptor, taken apart. A DACL that's present but None is a NULL DACL: everyone may do anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Descriptor {
    pub control: u16,
    pub owner: Option<Sid>,
    pub group: Option<Sid>,
    pub sacl: Option<Acl>,
    pub dacl: Option<Acl>,
}

impl Descriptor {
    /// From its self-relative form (what objects will keep).
    #[allow(dead_code)]
    pub fn from_relative(bytes: &[u8]) -> Option<Descriptor> {
        if bytes.len() < 20 || bytes[0] != 1 {
            return None;
        }
        let control = u16::from_le_bytes([bytes[2], bytes[3]]);
        let offset = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let part = |at: usize| (offset(at) != 0).then(|| bytes.get(offset(at)..)).flatten();
        let sid = |at: usize| match part(at) {
            Some(b) => Sid::from_bytes(b).map(Some),
            None => Some(None),
        };
        let acl = |at: usize, present: u16| match part(at) {
            Some(b) if control & present != 0 => Acl::from_bytes(b).map(Some),
            _ => Some(None),
        };
        Some(Descriptor { control: control & !SE_SELF_RELATIVE, owner: sid(4)?, group: sid(8)?, sacl: acl(12, SE_SACL_PRESENT)?, dacl: acl(16, SE_DACL_PRESENT)? })
    }

    /// Its self-relative form: the header, then the SACL, the DACL, the owner and the group.
    pub fn to_relative(&self) -> Vec<u8> {
        let mut b = vec![0u8; 20];
        b[0] = 1;
        b[2..4].copy_from_slice(&(self.control | SE_SELF_RELATIVE).to_le_bytes());
        let parts = [(12, self.sacl.as_ref().map(Acl::to_bytes)), (16, self.dacl.as_ref().map(Acl::to_bytes)), (4, self.owner.as_ref().map(Sid::to_bytes)), (8, self.group.as_ref().map(Sid::to_bytes))];
        for (at, part) in parts {
            if let Some(part) = part {
                let offset = b.len() as u32;
                b[at..at + 4].copy_from_slice(&offset.to_le_bytes());
                b.extend(part);
            }
        }
        b
    }

    /// SDDL: the parts `info` asks for (OWNER 1, GROUP 2, DACL 4, SACL 8, LABEL 0x10).
    pub fn to_sddl(&self, info: u32) -> String {
        let mut s = String::new();
        if info & 1 != 0 && let Some(owner) = &self.owner {
            s += &format!("O:{}", sid_to_sddl(owner));
        }
        if info & 2 != 0 && let Some(group) = &self.group {
            s += &format!("G:{}", sid_to_sddl(group));
        }
        if info & 4 != 0 && self.control & SE_DACL_PRESENT != 0 {
            s += "D:";
            s += &acl_flags(self.control, SE_DACL_PROTECTED, SE_DACL_AUTO_INHERIT_REQ, SE_DACL_AUTO_INHERITED);
            match &self.dacl {
                Some(acl) => s += &aces_to_sddl(&acl.aces),
                None => s += "NO_ACCESS_CONTROL",
            }
        }
        if info & 0x18 != 0 && self.control & SE_SACL_PRESENT != 0 && let Some(sacl) = &self.sacl {
            let aces: Vec<Ace> = sacl
                .aces
                .iter()
                .filter(|a| if a.kind == SYSTEM_MANDATORY_LABEL { info & 0x10 != 0 } else { info & 8 != 0 })
                .cloned()
                .collect();
            if !aces.is_empty() || info & 8 != 0 {
                s += "S:";
                s += &acl_flags(self.control, SE_SACL_PROTECTED, SE_SACL_AUTO_INHERIT_REQ, SE_SACL_AUTO_INHERITED);
                s += &aces_to_sddl(&aces);
            }
        }
        s
    }

    /// From SDDL: `O:owner G:group D:flags(ace)… S:flags(ace)…`, each part optional.
    pub fn from_sddl(s: &str) -> Result<Descriptor, u32> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = s.as_bytes();
        // Where each part starts: a letter and a colon, outside an ACE's parentheses.
        let mut starts = Vec::new();
        let mut depth = 0;
        for (i, &c) in bytes.iter().enumerate() {
            match c {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'O' | b'G' | b'D' | b'S' if depth == 0 && bytes.get(i + 1) == Some(&b':') => starts.push(i),
                _ => {}
            }
        }
        if starts.first().is_some_and(|&f| f != 0) || (starts.is_empty() && !s.is_empty()) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let mut d = Descriptor::default();
        for (n, &start) in starts.iter().enumerate() {
            let end = starts.get(n + 1).copied().unwrap_or(s.len());
            let body = &s[start + 2..end];
            match bytes[start] {
                b'O' => d.owner = Some(sid_from_sddl(body).ok_or(ERROR_INVALID_SID)?),
                b'G' => d.group = Some(sid_from_sddl(body).ok_or(ERROR_INVALID_SID)?),
                b'D' => {
                    let (flags, acl) = acl_from_sddl(body)?;
                    d.control |= SE_DACL_PRESENT | flags_to_control(&flags, SE_DACL_PROTECTED, SE_DACL_AUTO_INHERIT_REQ, SE_DACL_AUTO_INHERITED);
                    d.dacl = acl;
                }
                _ => {
                    let (flags, acl) = acl_from_sddl(body)?;
                    d.control |= SE_SACL_PRESENT | flags_to_control(&flags, SE_SACL_PROTECTED, SE_SACL_AUTO_INHERIT_REQ, SE_SACL_AUTO_INHERITED);
                    d.sacl = Some(acl.unwrap_or_else(|| Acl::new(Vec::new())));
                }
            }
        }
        Ok(d)
    }
}

fn acl_flags(control: u16, protected: u16, required: u16, inherited: u16) -> String {
    let mut s = String::new();
    for (bit, name) in [(protected, "P"), (required, "AR"), (inherited, "AI")] {
        if control & bit != 0 {
            s += name;
        }
    }
    s
}

fn flags_to_control(flags: &[&str], protected: u16, required: u16, inherited: u16) -> u16 {
    flags.iter().map(|f| match *f {
        "P" => protected,
        "AR" => required,
        "AI" => inherited,
        _ => 0,
    }).fold(0, |a, b| a | b)
}

/// An ACL's SDDL: its flags (`P`, `AI`, `AR`, `NO_ACCESS_CONTROL`), then ACEs in parentheses. None for a NULL DACL.
fn acl_from_sddl(s: &str) -> Result<(Vec<&str>, Option<Acl>), u32> {
    let head = &s[..s.find('(').unwrap_or(s.len())];
    let mut flags = Vec::new();
    let mut rest = head;
    let mut null = false;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("NO_ACCESS_CONTROL") {
            null = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("AI").or_else(|| rest.strip_prefix("AR")) {
            flags.push(&rest[..2]);
            rest = r;
        } else if let Some(r) = rest.strip_prefix('P') {
            flags.push("P");
            rest = r;
        } else {
            return Err(ERROR_INVALID_PARAMETER);
        }
    }
    let mut aces = Vec::new();
    let mut body = &s[head.len()..];
    while !body.is_empty() {
        let inner = body.strip_prefix('(').ok_or(ERROR_INVALID_PARAMETER)?;
        let close = inner.find(')').ok_or(ERROR_INVALID_PARAMETER)?;
        aces.push(ace_from_sddl(&inner[..close])?);
        body = &inner[close + 1..];
    }
    if null && !aces.is_empty() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok((flags, (!null).then(|| Acl::new(aces))))
}

const ACE_TYPES: &[(&str, u8)] = &[("A", ACCESS_ALLOWED), ("D", ACCESS_DENIED), ("AU", SYSTEM_AUDIT), ("AL", 3), ("ML", SYSTEM_MANDATORY_LABEL)];
const ACE_FLAGS: &[(&str, u8)] = &[("OI", 0x1), ("CI", 0x2), ("NP", 0x4), ("IO", 0x8), ("ID", 0x10), ("SA", 0x40), ("FA", 0x80)];
/// Rights SDDL names: the files' and keys' whole sets first, then each right on its own.
const RIGHTS: &[(&str, u32)] = &[
    ("FA", 0x1f_01ff),
    ("FR", 0x12_0089),
    ("FW", 0x12_0116),
    ("FX", 0x12_00a0),
    ("KA", 0xf_003f),
    ("KR", 0x2_0019),
    ("KW", 0x2_0006),
    ("KX", 0x2_0019),
    ("GA", 0x1000_0000),
    ("GX", 0x2000_0000),
    ("GW", 0x4000_0000),
    ("GR", 0x8000_0000),
    ("SD", 0x1_0000),
    ("RC", 0x2_0000),
    ("WD", 0x4_0000),
    ("WO", 0x8_0000),
    ("CC", 0x1),
    ("DC", 0x2),
    ("LC", 0x4),
    ("SW", 0x8),
    ("RP", 0x10),
    ("WP", 0x20),
    ("DT", 0x40),
    ("LO", 0x80),
    ("CR", 0x100),
];
/// A mandatory label's: no write up, no read up, no execute up.
const LABEL_RIGHTS: &[(&str, u32)] = &[("NW", 0x1), ("NR", 0x2), ("NX", 0x4)];

/// One ACE's SDDL: `type;flags;rights;object;inherited object;account`.
fn ace_from_sddl(s: &str) -> Result<Ace, u32> {
    let f: Vec<&str> = s.split(';').collect();
    if f.len() < 6 {
        return Err(ERROR_INVALID_ACL);
    }
    let kind = ACE_TYPES.iter().find(|(n, _)| *n == f[0]).ok_or(ERROR_INVALID_ACL)?.1;
    if !f[3].is_empty() || !f[4].is_empty() {
        return Err(ERROR_INVALID_ACL);
    }
    let mut flags = 0;
    for pair in chunks(f[1])? {
        flags |= ACE_FLAGS.iter().find(|(n, _)| *n == pair).ok_or(ERROR_INVALID_ACL)?.1;
    }
    let mask = if let Some(hex) = f[2].strip_prefix("0x").or_else(|| f[2].strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).map_err(|_| ERROR_INVALID_ACL)?
    } else if !f[2].is_empty() && f[2].bytes().all(|b| b.is_ascii_digit()) {
        f[2].parse().map_err(|_| ERROR_INVALID_ACL)?
    } else {
        let mut mask = 0;
        for pair in chunks(f[2])? {
            mask |= RIGHTS.iter().chain(LABEL_RIGHTS).find(|(n, _)| *n == pair).ok_or(ERROR_INVALID_ACL)?.1;
        }
        mask
    };
    Ok(Ace::new(kind, flags, mask, sid_from_sddl(f[5]).ok_or(ERROR_INVALID_SID)?))
}

fn chunks(s: &str) -> Result<Vec<&str>, u32> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return Err(ERROR_INVALID_ACL);
    }
    Ok((0..s.len()).step_by(2).map(|i| &s[i..i + 2]).collect())
}

fn aces_to_sddl(aces: &[Ace]) -> String {
    aces.iter().map(|a| format!("({})", ace_to_sddl(a))).collect()
}

fn ace_to_sddl(ace: &Ace) -> String {
    let kind = ACE_TYPES.iter().find(|(_, k)| *k == ace.kind).map_or("A", |(n, _)| n);
    let flags: String = ACE_FLAGS.iter().filter(|(_, f)| ace.flags & f != 0).map(|(n, _)| *n).collect();
    let table = if ace.kind == SYSTEM_MANDATORY_LABEL { LABEL_RIGHTS } else { &RIGHTS[8..] };
    let rights = if let Some((n, _)) = RIGHTS[..8].iter().find(|(_, m)| *m == ace.mask).filter(|_| ace.kind != SYSTEM_MANDATORY_LABEL) {
        n.to_string()
    } else if ace.mask != 0 && table.iter().fold(ace.mask, |m, (_, b)| m & !b) == 0 {
        table.iter().filter(|(_, b)| ace.mask & b != 0).map(|(n, _)| *n).collect()
    } else {
        format!("0x{:x}", ace.mask)
    };
    format!("{kind};{flags};{rights};;;{}", sid_to_sddl(&ace.sid))
}

// --- Access ---

/// How a type of object's generic rights map to its own (`GENERIC_MAPPING`).
#[derive(Debug, Clone, Copy)]
pub struct Mapping {
    pub read: u32,
    pub write: u32,
    pub execute: u32,
    pub all: u32,
}

pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const GENERIC_EXECUTE: u32 = 0x2000_0000;
pub const GENERIC_ALL: u32 = 0x1000_0000;
pub const READ_CONTROL: u32 = 0x2_0000;
pub const WRITE_DAC: u32 = 0x4_0000;
pub const WRITE_OWNER: u32 = 0x8_0000;
pub const ACCESS_SYSTEM_SECURITY: u32 = 0x100_0000;
pub const MAXIMUM_ALLOWED: u32 = 0x200_0000;

pub const EVENT: Mapping = Mapping { read: 0x2_0001, write: 0x2_0002, execute: 0x10_0000, all: 0x1f_0003 };
pub const MUTANT: Mapping = Mapping { read: 0x2_0001, write: 0x2_0000, execute: 0x10_0000, all: 0x1f_0001 };
pub const SEMAPHORE: Mapping = Mapping { read: 0x2_0001, write: 0x2_0002, execute: 0x10_0000, all: 0x1f_0003 };
pub const TIMER: Mapping = Mapping { read: 0x2_0001, write: 0x2_0002, execute: 0x10_0000, all: 0x1f_0003 };
pub const PROCESS: Mapping = Mapping { read: 0x2_0410, write: 0x2_0bea, execute: 0x12_1001, all: 0x1f_ffff };
pub const THREAD: Mapping = Mapping { read: 0x2_0048, write: 0x2_0037, execute: 0x12_1800, all: 0x1f_ffff };
pub const SECTION: Mapping = Mapping { read: 0x2_0005, write: 0x2_0002, execute: 0x2_0008, all: 0xf_001f };
pub const FILE: Mapping = Mapping { read: 0x12_0089, write: 0x12_0116, execute: 0x12_00a0, all: 0x1f_01ff };
pub const KEY: Mapping = Mapping { read: 0x2_0019, write: 0x2_0006, execute: 0x2_0019, all: 0xf_003f };
pub const TOKEN: Mapping = Mapping { read: 0x2_0008, write: 0x2_00e0, execute: 0x2_0000, all: 0xf_01ff };
pub const WINDOW_STATION: Mapping = Mapping { read: 0x2_0303, write: 0x2_001c, execute: 0x2_0060, all: 0xf_037f };
pub const DESKTOP: Mapping = Mapping { read: 0x2_0041, write: 0x2_00be, execute: 0x2_0100, all: 0xf_01ff };

/// A mask's generic rights as the type's own (`MapGenericMask`).
pub fn map_generic(mask: u32, m: &Mapping) -> u32 {
    let mut out = mask & !(GENERIC_READ | GENERIC_WRITE | GENERIC_EXECUTE | GENERIC_ALL);
    for (bit, to) in [(GENERIC_READ, m.read), (GENERIC_WRITE, m.write), (GENERIC_EXECUTE, m.execute), (GENERIC_ALL, m.all)] {
        if mask & bit != 0 {
            out |= to;
        }
    }
    out
}

/// What files and keys are given when nothing's said: what Windows gives a folder's files and keys under it.
pub const FILE_DEFAULT: &str = "O:BAG:SYD:(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;ID;0x1301bf;;;AU)(A;ID;0x1200a9;;;BU)";
pub const KEY_DEFAULT: &str = "O:BAG:SYD:(A;CIID;KA;;;SY)(A;CIID;KA;;;BA)(A;CIID;KA;;;S-1-5-21-0-0-0-1000)(A;CIID;KR;;;BU)";

/// `*_SECURITY_INFORMATION`: which parts a call reads or writes.
pub const OWNER_INFO: u32 = 1;
pub const GROUP_INFO: u32 = 2;
pub const DACL_INFO: u32 = 4;
pub const SACL_INFO: u32 = 8;
pub const LABEL_INFO: u32 = 0x10;
pub const PROTECTED_DACL: u32 = 0x8000_0000;
pub const PROTECTED_SACL: u32 = 0x4000_0000;
pub const UNPROTECTED_DACL: u32 = 0x2000_0000;
pub const UNPROTECTED_SACL: u32 = 0x1000_0000;

impl Descriptor {
    /// What an object made with no descriptor gets from its maker's token: its owner, primary group and default DACL.
    pub fn defaults(t: &TokenState) -> Descriptor {
        Descriptor { control: SE_DACL_PRESENT, owner: Some(t.owner.clone()), group: Some(t.primary_group.clone()), sacl: None, dacl: t.default_dacl.clone() }
    }

    /// The parts a descriptor the program gave leaves out, filled from the token's defaults.
    pub fn filled(mut self, t: &TokenState) -> Descriptor {
        self.owner.get_or_insert_with(|| t.owner.clone());
        self.group.get_or_insert_with(|| t.primary_group.clone());
        if self.control & SE_DACL_PRESENT == 0 {
            self.control |= SE_DACL_PRESENT;
            self.dacl = t.default_dacl.clone();
        }
        self
    }

    /// As it's kept on an object of a type: its ACEs' generic rights as the type's own, as Windows maps them when
    /// it gives an object a descriptor (all but inherit-only ACEs, which are for objects to come, and labels).
    pub fn mapped(mut self, m: &Mapping) -> Descriptor {
        for acl in [&mut self.dacl, &mut self.sacl].into_iter().flatten() {
            for ace in acl.aces.iter_mut().filter(|a| a.flags & 0x8 == 0 && a.kind != SYSTEM_MANDATORY_LABEL && a.raw.is_none()) {
                ace.mask = map_generic(ace.mask, m);
            }
        }
        self
    }

    /// Just the parts `info` asks for; the label alone is the SACL's mandatory label ACEs.
    pub fn only(&self, info: u32) -> Descriptor {
        let mut d = Descriptor::default();
        if info & OWNER_INFO != 0 {
            d.owner = self.owner.clone();
        }
        if info & GROUP_INFO != 0 {
            d.group = self.group.clone();
        }
        if info & DACL_INFO != 0 {
            d.control |= self.control & (SE_DACL_PRESENT | SE_DACL_DEFAULTED | SE_DACL_PROTECTED | SE_DACL_AUTO_INHERITED | SE_DACL_AUTO_INHERIT_REQ);
            d.dacl = self.dacl.clone();
        }
        if info & (SACL_INFO | LABEL_INFO) != 0 && let Some(sacl) = &self.sacl {
            let aces: Vec<Ace> = sacl.aces.iter().filter(|a| if a.kind == SYSTEM_MANDATORY_LABEL { info & LABEL_INFO != 0 } else { info & SACL_INFO != 0 }).cloned().collect();
            d.control |= SE_SACL_PRESENT | self.control & (SE_SACL_PROTECTED | SE_SACL_AUTO_INHERITED);
            d.sacl = Some(Acl::new(aces));
        }
        d
    }

    /// Sets the parts `info` names from `from`: what `SetSecurityInfo` does.
    pub fn set(&mut self, info: u32, from: &Descriptor) {
        if info & OWNER_INFO != 0 && from.owner.is_some() {
            self.owner = from.owner.clone();
        }
        if info & GROUP_INFO != 0 && from.group.is_some() {
            self.group = from.group.clone();
        }
        if info & DACL_INFO != 0 {
            self.control |= SE_DACL_PRESENT;
            self.dacl = from.dacl.clone();
            if from.control & SE_DACL_PRESENT == 0 {
                self.dacl = None;
            }
        }
        let others = |sacl: &Option<Acl>, keep_label: bool| -> Vec<Ace> {
            sacl.iter().flat_map(|a| a.aces.iter()).filter(|a| (a.kind == SYSTEM_MANDATORY_LABEL) == keep_label).cloned().collect()
        };
        if info & (SACL_INFO | LABEL_INFO) != 0 {
            let mut aces = Vec::new();
            // The label's from `from` if it's being set, else as it was; the rest likewise.
            aces.extend(if info & SACL_INFO != 0 { others(&from.sacl, false) } else { others(&self.sacl, false) });
            aces.extend(if info & LABEL_INFO != 0 { others(&from.sacl, true) } else { others(&self.sacl, true) });
            self.control |= SE_SACL_PRESENT;
            self.sacl = Some(Acl::new(aces));
        }
        for (bit, on, flag) in [(PROTECTED_DACL, true, SE_DACL_PROTECTED), (UNPROTECTED_DACL, false, SE_DACL_PROTECTED), (PROTECTED_SACL, true, SE_SACL_PROTECTED), (UNPROTECTED_SACL, false, SE_SACL_PROTECTED)] {
            if info & bit != 0 {
                if on {
                    self.control |= flag;
                } else {
                    self.control &= !flag;
                }
            }
        }
    }

    /// Its mandatory label: the integrity level and policy (no write up, read up, execute up). Medium, no write up,
    /// for an object that has none, as Windows takes it.
    pub fn label(&self) -> (u32, u32) {
        self.sacl
            .iter()
            .flat_map(|a| a.aces.iter())
            .find(|a| a.kind == SYSTEM_MANDATORY_LABEL && a.flags & 0x8 == 0)
            .map_or((0x2000, 1), |a| (a.sid.subs.last().copied().unwrap_or(0), a.mask))
    }
}

/// Whether `t` may have `desired` of what `d` guards, and what it's granted: `AccessCheck`'s rules.
/// - The mandatory label: a token below the object's integrity is refused what the label's policy says.
/// - A NULL DACL allows everything; an empty one nothing.
/// - Each right is decided by the first ACE that names it, for the user or an enabled group (a deny-only group
///   only for denied ACEs); inherit-only ACEs don't count.
/// - The owner may always read and change the DACL, unless an OWNER RIGHTS ACE says otherwise. Enabled privileges
///   give `WRITE_OWNER` (SeTakeOwnershipPrivilege) and `ACCESS_SYSTEM_SECURITY` (SeSecurityPrivilege).
pub fn access_check(d: &Descriptor, t: &TokenState, desired: u32, m: &Mapping) -> Result<u32, ()> {
    let desired = map_generic(desired, m);
    let maximum = desired & MAXIMUM_ALLOWED != 0;
    let wanted = desired & !MAXIMUM_ALLOWED;
    let enabled = |luid: u32| t.privileges.iter().any(|p| p.0 == luid && p.1 & SE_PRIVILEGE_ENABLED != 0);
    let everything = m.all | 0x1f_0000;
    let mut privileged = 0;
    if enabled(9) {
        privileged |= WRITE_OWNER;
    }
    // The SACL only when it's asked for by name: MAXIMUM_ALLOWED doesn't include it.
    if enabled(8) && wanted & ACCESS_SYSTEM_SECURITY != 0 {
        privileged |= ACCESS_SYSTEM_SECURITY;
    }
    // The label.
    let (level, policy) = d.label();
    let mine = t.integrity().subs.last().copied().unwrap_or(0);
    let refused = if mine < level {
        let mut r = 0;
        for (bit, rights) in [(1, m.write), (2, m.read), (4, m.execute)] {
            if policy & bit != 0 {
                r |= rights & !(READ_CONTROL | 0x10_0000);
            }
        }
        // Writing the DACL, owner or the object itself counts as writing up.
        if policy & 1 != 0 {
            r |= WRITE_DAC | WRITE_OWNER | 0x1_0000;
        }
        r
    } else {
        0
    };
    // The DACL.
    let (mut allowed, mut denied) = (0u32, 0u32);
    match (&d.dacl, d.control & SE_DACL_PRESENT != 0) {
        (None, true) => allowed = everything,
        (dacl, _) => {
            let aces: &[Ace] = dacl.as_ref().map_or(&[], |a| a.aces.as_slice());
            let owner_rights = Sid::new(3, &[4]);
            let is_owner = d.owner.as_ref().is_some_and(|o| t.has(o));
            if is_owner && !aces.iter().any(|a| a.sid == owner_rights && a.flags & 0x8 == 0) {
                allowed |= READ_CONTROL | WRITE_DAC;
            }
            for ace in aces.iter().filter(|a| a.flags & 0x8 == 0 && a.raw.is_none()) {
                let mask = map_generic(ace.mask, m);
                let applies = |deny: bool| {
                    t.has(&ace.sid)
                        || (ace.sid == owner_rights && is_owner)
                        || (deny && t.groups.iter().any(|(s, at)| *s == ace.sid && at & SE_GROUP_USE_FOR_DENY_ONLY != 0))
                };
                match ace.kind {
                    ACCESS_ALLOWED if applies(false) => allowed |= mask & !denied,
                    ACCESS_DENIED if applies(true) => denied |= mask & !allowed,
                    _ => {}
                }
            }
        }
    }
    let granted = (allowed | privileged) & !refused;
    if wanted & !granted != 0 {
        return Err(());
    }
    if maximum {
        let all = granted & (everything | ACCESS_SYSTEM_SECURITY);
        return if all == 0 && wanted == 0 { Err(()) } else { Ok(all | wanted) };
    }
    Ok(wanted)
}

// --- Privileges ---

/// Every privilege, by its LUID's low part.
pub const PRIVILEGES: &[(u32, &str)] = &[
    (2, "SeCreateTokenPrivilege"),
    (3, "SeAssignPrimaryTokenPrivilege"),
    (4, "SeLockMemoryPrivilege"),
    (5, "SeIncreaseQuotaPrivilege"),
    (6, "SeMachineAccountPrivilege"),
    (7, "SeTcbPrivilege"),
    (8, "SeSecurityPrivilege"),
    (9, "SeTakeOwnershipPrivilege"),
    (10, "SeLoadDriverPrivilege"),
    (11, "SeSystemProfilePrivilege"),
    (12, "SeSystemtimePrivilege"),
    (13, "SeProfileSingleProcessPrivilege"),
    (14, "SeIncreaseBasePriorityPrivilege"),
    (15, "SeCreatePagefilePrivilege"),
    (16, "SeCreatePermanentPrivilege"),
    (17, "SeBackupPrivilege"),
    (18, "SeRestorePrivilege"),
    (19, "SeShutdownPrivilege"),
    (20, "SeDebugPrivilege"),
    (21, "SeAuditPrivilege"),
    (22, "SeSystemEnvironmentPrivilege"),
    (23, "SeChangeNotifyPrivilege"),
    (24, "SeRemoteShutdownPrivilege"),
    (25, "SeUndockPrivilege"),
    (26, "SeSyncAgentPrivilege"),
    (27, "SeEnableDelegationPrivilege"),
    (28, "SeManageVolumePrivilege"),
    (29, "SeImpersonatePrivilege"),
    (30, "SeCreateGlobalPrivilege"),
    (31, "SeTrustedCredManAccessPrivilege"),
    (32, "SeRelabelPrivilege"),
    (33, "SeIncreaseWorkingSetPrivilege"),
    (34, "SeTimeZonePrivilege"),
    (35, "SeCreateSymbolicLinkPrivilege"),
    (36, "SeDelegateSessionUserImpersonatePrivilege"),
];

pub fn privilege_named(name: &str) -> Option<u32> {
    PRIVILEGES.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(l, _)| *l)
}

pub fn privilege_name(luid: u32) -> Option<&'static str> {
    PRIVILEGES.iter().find(|(l, _)| *l == luid).map(|(_, n)| *n)
}

pub const SE_PRIVILEGE_ENABLED_BY_DEFAULT: u32 = 1;
pub const SE_PRIVILEGE_ENABLED: u32 = 2;
pub const SE_PRIVILEGE_REMOVED: u32 = 4;

// --- Tokens ---

/// `SE_GROUP_*`: mandatory, enabled by default, enabled; owner; deny only; integrity; logon ID.
pub const GROUP_DEFAULT: u32 = 0x7;
pub const SE_GROUP_OWNER: u32 = 0x8;
pub const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x10;
pub const SE_GROUP_INTEGRITY: u32 = 0x60;
pub const SE_GROUP_LOGON_ID: u32 = 0xc000_0000;

/// LUIDs for tokens, from one count for the machine.
static NEXT_LUID: AtomicU64 = AtomicU64::new(0x10_0000);

pub fn new_luid() -> u64 {
    NEXT_LUID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub struct TokenState {
    pub user: Sid,
    /// Groups and their `SE_GROUP_*` attributes; the integrity level is the one with `SE_GROUP_INTEGRITY`.
    pub groups: Vec<(Sid, u32)>,
    /// Privileges by LUID, and their `SE_PRIVILEGE_*` attributes.
    pub privileges: Vec<(u32, u32)>,
    pub owner: Sid,
    pub primary_group: Sid,
    pub default_dacl: Option<Acl>,
    /// 1 primary, 2 impersonation; and an impersonation token's level (0 anonymous … 3 delegation).
    pub kind: u32,
    pub level: u32,
    pub session: u32,
    /// Changed each time the token is.
    pub modified: u64,
}

pub struct Token {
    pub id: u64,
    pub state: Mutex<TokenState>,
    /// The token's own descriptor, if it's been given one.
    pub security: Mutex<Option<Descriptor>>,
}

impl Token {
    /// The user's token, as an elevated administrator's is: its groups, Administrators its owner, high integrity, and
    /// an administrator's privileges, three of them enabled.
    pub fn administrator() -> Token {
        let groups = vec![
            (Sid::local(513), GROUP_DEFAULT),
            (Sid::new(1, &[0]), GROUP_DEFAULT),
            (Sid::new(5, &[114]), GROUP_DEFAULT),
            (Sid::new(5, &[32, 544]), GROUP_DEFAULT | SE_GROUP_OWNER),
            (Sid::new(5, &[32, 545]), GROUP_DEFAULT),
            (Sid::new(5, &[4]), GROUP_DEFAULT),
            (Sid::new(2, &[1]), GROUP_DEFAULT),
            (Sid::new(5, &[11]), GROUP_DEFAULT),
            (Sid::new(5, &[15]), GROUP_DEFAULT),
            (Sid::new(5, &[113]), GROUP_DEFAULT),
            (logon_sid(), GROUP_DEFAULT | SE_GROUP_LOGON_ID),
            (Sid::new(2, &[0]), GROUP_DEFAULT),
            (Sid::new(5, &[64, 10]), GROUP_DEFAULT),
            (Sid::new(16, &[12288]), SE_GROUP_INTEGRITY),
        ];
        let held = [5, 8, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 22, 23, 25, 28, 29, 30, 33, 34, 35, 36];
        let on = SE_PRIVILEGE_ENABLED_BY_DEFAULT | SE_PRIVILEGE_ENABLED;
        let privileges = held.iter().map(|&l| (l, if matches!(l, 23 | 29 | 30) { on } else { 0 })).collect();
        let dacl = Acl::new(vec![Ace::new(ACCESS_ALLOWED, 0, 0x1000_0000, Sid::new(5, &[32, 544])), Ace::new(ACCESS_ALLOWED, 0, 0x1000_0000, Sid::new(5, &[18]))]);
        Token::with(TokenState {
            user: user(),
            groups,
            privileges,
            owner: Sid::new(5, &[32, 544]),
            primary_group: Sid::local(513),
            default_dacl: Some(dacl),
            kind: 1,
            level: 0,
            session: 1,
            modified: new_luid(),
        })
    }

    pub fn with(state: TokenState) -> Token {
        Token { id: new_luid(), state: Mutex::new(state), security: Mutex::new(None) }
    }

    /// A copy, with an ID of its own: `DuplicateTokenEx`'s, and what a new process gets of its parent's.
    pub fn duplicate(&self, kind: u32, level: u32) -> Token {
        let mut state = kernels_core::lock(&self.state).clone();
        state.kind = kind;
        state.level = level;
        state.modified = new_luid();
        Token::with(state)
    }
}

impl TokenState {
    /// The integrity level's SID.
    pub fn integrity(&self) -> Sid {
        self.groups.iter().find(|(_, a)| a & SE_GROUP_INTEGRITY != 0).map_or_else(|| Sid::new(16, &[8192]), |(s, _)| s.clone())
    }

    /// Whether `sid` is the user or an enabled group (not deny-only).
    pub fn has(&self, sid: &Sid) -> bool {
        *sid == self.user || self.groups.iter().any(|(s, a)| s == sid && a & 0x4 != 0 && a & SE_GROUP_USE_FOR_DENY_ONLY == 0 && a & SE_GROUP_INTEGRITY == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sids_both_ways() {
        let sid = Sid::parse("S-1-5-21-1-2-3-1001").unwrap();
        assert_eq!(sid.to_string(), "S-1-5-21-1-2-3-1001");
        assert_eq!(Sid::from_bytes(&sid.to_bytes()), Some(sid));
        assert_eq!(Sid::parse("S-1-0x1234567890AB-1").unwrap().to_string(), "S-1-0x1234567890AB-1");
        assert!(Sid::parse("S-2-5").is_none() && Sid::parse("S-1-5-x").is_none() && Sid::parse("BA").is_none());
        assert_eq!(sid_from_sddl("BA"), Some(Sid::new(5, &[32, 544])));
        assert_eq!(sid_to_sddl(&Sid::new(5, &[18])), "SY");
        assert_eq!(well_known(26, None), sid_from_sddl("BA"));
        assert_eq!(well_known(22, None), sid_from_sddl("SY"));
        assert_eq!(well_known(37, None), sid_from_sddl("NO"));
        assert_eq!(well_known(12, None), sid_from_sddl("SU"));
        assert_eq!(well_known(13, None), sid_from_sddl("AN"));
    }

    #[test]
    fn sddl_round_trips() {
        let s = "O:BAG:SYD:PAI(A;OICI;FA;;;SY)(A;OICIID;FA;;;BA)(D;;0x1200a9;;;S-1-5-21-1-2-3-1001)(A;;CCLCSWRP;;;AU)S:(ML;;NW;;;LW)";
        let d = Descriptor::from_sddl(s).unwrap();
        assert_eq!(d.to_sddl(0x1f), s);
        let back = Descriptor::from_relative(&d.to_relative()).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.to_sddl(0x1f), s);
        // Only the parts asked for; the label on its own.
        assert_eq!(d.to_sddl(4 | 1), "O:BAD:PAI(A;OICI;FA;;;SY)(A;OICIID;FA;;;BA)(D;;0x1200a9;;;S-1-5-21-1-2-3-1001)(A;;CCLCSWRP;;;AU)");
        assert_eq!(d.to_sddl(0x10), "S:(ML;;NW;;;LW)");
        // A NULL DACL, an empty one, and rubbish.
        assert_eq!(Descriptor::from_sddl("D:NO_ACCESS_CONTROL").unwrap().dacl, None);
        assert_eq!(Descriptor::from_sddl("D:").unwrap().dacl.unwrap().aces.len(), 0);
        assert!(Descriptor::from_sddl("D:(A;;FA;;;XX)").is_err());
        assert!(Descriptor::from_sddl("D:(Q;;FA;;;BA)").is_err());
        assert!(Descriptor::from_sddl("nonsense").is_err());
    }

    #[test]
    fn access_is_checked_as_windows_does() {
        let token = Token::administrator();
        let mut t = kernels_core::lock(&token.state).clone();
        let d = |sddl: &str| Descriptor::from_sddl(sddl).unwrap();
        // Allowed to Administrators, so to us; denied first to us, then allowed: denied.
        assert_eq!(access_check(&d("O:SYD:(A;;GA;;;BA)"), &t, GENERIC_READ, &EVENT), Ok(EVENT.read));
        assert!(access_check(&d("O:SYD:(D;;0x2;;;WD)(A;;GA;;;BA)"), &t, 2, &EVENT).is_err());
        // The first ACE to name a right decides it.
        assert_eq!(access_check(&d("O:SYD:(A;;0x2;;;WD)(D;;0x2;;;BA)"), &t, 2, &EVENT), Ok(2));
        // An empty DACL: nothing, but the owner may still read and write it. A NULL DACL: everything.
        assert!(access_check(&d("O:SYD:"), &t, 1, &EVENT).is_err());
        assert_eq!(access_check(&d("O:BAD:"), &t, READ_CONTROL | WRITE_DAC, &EVENT), Ok(READ_CONTROL | WRITE_DAC));
        assert!(access_check(&d("O:BAD:(A;;0x20000;;;OW)"), &t, WRITE_DAC, &EVENT).is_err());
        assert_eq!(access_check(&d("D:NO_ACCESS_CONTROL"), &t, MAXIMUM_ALLOWED, &EVENT), Ok(0x1f_0003));
        // SeSecurityPrivilege gives the SACL only when it's asked for.
        t.privileges.iter_mut().find(|p| p.0 == 8).unwrap().1 = SE_PRIVILEGE_ENABLED;
        assert_eq!(access_check(&d("O:SYD:(A;;0x1;;;WD)"), &t, MAXIMUM_ALLOWED, &EVENT), Ok(1));
        assert_eq!(access_check(&d("O:SYD:(A;;0x1;;;WD)"), &t, ACCESS_SYSTEM_SECURITY | 1, &EVENT), Ok(ACCESS_SYSTEM_SECURITY | 1));
        // Inherit-only ACEs don't count.
        assert!(access_check(&d("O:SYD:(A;IO;GA;;;BA)"), &t, 1, &EVENT).is_err());
        // Low integrity can't write to medium, but can read.
        t.groups.iter_mut().find(|g| g.1 & SE_GROUP_INTEGRITY != 0).unwrap().0 = Sid::new(16, &[4096]);
        assert!(access_check(&d("O:SYD:(A;;GA;;;WD)"), &t, EVENT.write, &EVENT).is_err());
        assert_eq!(access_check(&d("O:SYD:(A;;GA;;;WD)"), &t, EVENT.read, &EVENT), Ok(EVENT.read));
        assert_eq!(access_check(&d("O:SYD:(A;;GA;;;WD)S:(ML;;NW;;;LW)"), &t, EVENT.write, &EVENT), Ok(EVENT.write));
    }

    #[test]
    fn setting_parts() {
        let mut d = Descriptor::from_sddl("O:BAG:SYD:(A;;FA;;;BA)S:(ML;;NW;;;ME)(AU;SA;FA;;;WD)").unwrap();
        assert_eq!(d.only(LABEL_INFO).to_sddl(0x1f), "S:(ML;;NW;;;ME)");
        d.set(DACL_INFO | LABEL_INFO | PROTECTED_DACL, &Descriptor::from_sddl("D:(A;;FR;;;WD)S:(ML;;NW;;;LW)").unwrap());
        assert_eq!(d.to_sddl(0x1f), "O:BAG:SYD:P(A;;FR;;;WD)S:(AU;SA;FA;;;WD)(ML;;NW;;;LW)");
    }

    #[test]
    fn the_administrators_token() {
        let t = Token::administrator();
        let s = kernels_core::lock(&t.state);
        assert!(s.has(&Sid::new(5, &[32, 544])) && s.has(&user()) && !s.has(&Sid::new(5, &[18])));
        assert_eq!(s.integrity(), Sid::new(16, &[12288]));
        assert_eq!(privilege_named("SeDebugPrivilege"), Some(20));
        assert_eq!(privilege_name(23), Some("SeChangeNotifyPrivilege"));
        drop(s);
        let copy = t.duplicate(2, 2);
        assert_ne!(copy.id, t.id);
    }
}
