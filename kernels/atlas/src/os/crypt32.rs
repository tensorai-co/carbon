//! crypt32.dll: certificates, as a program checks a TLS server's. Contexts are in the program's memory, as CryptoAPI
//! gives them: CERT_CONTEXT with its DER and a CERT_INFO read from it (version, serial, algorithms, names, validity, the
//! public key; not the extensions yet). Stores are atlas's own, in memory: a system store (ROOT, CA, MY) opens empty,
//! since the host's roots aren't listed. `CertGetCertificateChain` asks the host (`Host::check_certificates`) whether
//! it trusts the certificate with what the stores hold, and gives a chain context of the certificates it could link,
//! leaf first, with the host's verdict as CERT_TRUST_STATUS bits.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "CRYPT32.dll";
    host CertOpenStore(5) => cert_open_store;
    host CertCloseStore(2) => cert_close_store;
    host CertCreateCertificateContext(3) => cert_create_certificate_context;
    host CertFreeCertificateContext(1) => cert_free_certificate_context;
    host CertDuplicateCertificateContext(1) => cert_duplicate_certificate_context;
    host CertAddCertificateContextToStore(4) => cert_add_certificate_context_to_store;
    host CertAddEncodedCertificateToStore(6) => cert_add_encoded_certificate_to_store;
    host CertGetCertificateChain(8) => cert_get_certificate_chain;
    host CertFreeCertificateChain(1) => cert_free_certificate_chain;
}

const X509_ASN_ENCODING: u32 = 1;
const CRYPT_E_ASN1_BADTAG: u32 = 0x8009_310b;
const E_INVALIDARG: u32 = 0x8007_0057;

// ---- DER, as much of it as a certificate's fields need ----

/// One DER element: its tag, its contents, and all of it (tag and length too).
#[derive(Clone, Copy)]
struct Tlv<'a> {
    tag: u8,
    body: &'a [u8],
    whole: &'a [u8],
}

/// The element at the start of `b`, and what's after it.
fn tlv(b: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    let tag = *b.first()?;
    let first = *b.get(1)?;
    let (len, head) = if first < 0x80 {
        (first as usize, 2)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = len << 8 | *b.get(2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let end = head.checked_add(len)?;
    let body = b.get(head..end)?;
    Some((Tlv { tag, body, whole: &b[..end] }, &b[end..]))
}

/// A sequence's elements.
fn items(b: &[u8]) -> Vec<Tlv<'_>> {
    let mut out = Vec::new();
    let mut rest = b;
    while let Some((t, r)) = tlv(rest) {
        out.push(t);
        rest = r;
    }
    out
}

/// An OBJECT IDENTIFIER's contents as dotted text.
fn oid(b: &[u8]) -> String {
    let mut parts: Vec<u64> = Vec::new();
    let mut v = 0u64;
    for (i, &x) in b.iter().enumerate() {
        v = v << 7 | u64::from(x & 0x7f);
        if x & 0x80 == 0 {
            if parts.is_empty() && i < b.len() {
                let first = v.min(80) / 40;
                parts.push(first);
                parts.push(v - first * 40);
            } else {
                parts.push(v);
            }
            v = 0;
        }
    }
    parts.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

/// A UTCTime or GeneralizedTime as a FILETIME.
fn time(t: &Tlv<'_>) -> u64 {
    let s = std::str::from_utf8(t.body).unwrap_or("");
    let num = |r: std::ops::Range<usize>| s.get(r).and_then(|x| x.parse::<u16>().ok()).unwrap_or(0);
    let (year, rest) = if t.tag == 0x17 {
        let yy = num(0..2);
        (if yy >= 50 { 1900 + yy } else { 2000 + yy }, 2)
    } else {
        (num(0..4), 4)
    };
    let field = |i: usize| num(rest + 2 * i..rest + 2 * i + 2);
    crate::system::file_time([year, field(0), 0, field(1), field(2), field(3), field(4), 0]).unwrap_or(0)
}

/// What CERT_INFO holds, read from a certificate.
struct Info<'a> {
    version: u32,
    serial: &'a [u8],
    signature: String,
    signature_params: &'a [u8],
    issuer: &'a [u8],
    not_before: u64,
    not_after: u64,
    subject: &'a [u8],
    key_algorithm: String,
    key_params: &'a [u8],
    key: &'a [u8],
    unused_bits: u8,
}

/// An AlgorithmIdentifier's OID and parameters.
fn algorithm<'a>(t: &Tlv<'a>) -> (String, &'a [u8]) {
    let parts = items(t.body);
    let id = parts.first().map(|o| oid(o.body)).unwrap_or_default();
    (id, parts.get(1).map_or(&[][..], |p| p.whole))
}

fn parse(der: &[u8]) -> Option<Info<'_>> {
    let (cert, _) = tlv(der)?;
    let tbs = items(cert.body).into_iter().next()?;
    let mut fields = items(tbs.body).into_iter().peekable();
    let version = match fields.peek() {
        Some(t) if t.tag == 0xa0 => {
            let v = fields.next()?;
            let (int, _) = tlv(v.body)?;
            u32::from(*int.body.last()?)
        }
        _ => 0,
    };
    let serial = fields.next()?.body;
    let (signature, signature_params) = algorithm(&fields.next()?);
    let issuer = fields.next()?.whole;
    let validity = items(fields.next()?.body);
    let subject = fields.next()?.whole;
    let spki = items(fields.next()?.body);
    let (key_algorithm, key_params) = algorithm(spki.first()?);
    let bits = spki.get(1)?.body;
    Some(Info {
        version,
        serial,
        signature,
        signature_params,
        issuer,
        not_before: validity.first().map_or(0, time),
        not_after: validity.get(1).map_or(0, time),
        subject,
        key_algorithm,
        key_params,
        key: bits.get(1..).unwrap_or(&[]),
        unused_bits: bits.first().copied().unwrap_or(0),
    })
}

// ---- Structures in the program's memory ----

/// Lays out a structure as the program's ISA does: fields at their alignment, pointers its width.
struct Layout {
    bytes: Vec<u8>,
    width: usize,
}

impl Layout {
    fn new(width: u64) -> Layout {
        Layout { bytes: Vec::new(), width: width as usize }
    }

    fn align(&mut self, n: usize) {
        while !self.bytes.len().is_multiple_of(n) {
            self.bytes.push(0);
        }
    }

    fn u32(&mut self, v: u32) {
        self.align(4);
        self.bytes.extend(v.to_le_bytes());
    }

    fn ptr(&mut self, v: u64) {
        self.align(self.width);
        self.bytes.extend(&v.to_le_bytes()[..self.width]);
    }

    fn u64(&mut self, v: u64) {
        self.u32(v as u32);
        self.u32((v >> 32) as u32);
    }

    fn blob(&mut self, len: usize, at: u64) {
        self.u32(len as u32);
        self.ptr(if len == 0 { 0 } else { at });
    }

    fn end(mut self) -> Vec<u8> {
        self.align(self.width);
        self.bytes
    }
}

/// What atlas keeps for each certificate context it gave: its DER and how many references there are.
struct Context {
    der: Vec<u8>,
    refs: u32,
}

#[derive(Default)]
struct State {
    /// Stores, by handle: the DER of each certificate in each.
    stores: HashMap<(u64, u64), Vec<Vec<u8>>>,
    /// Contexts, by their address in their process.
    contexts: HashMap<(u64, u64), Context>,
    /// Chain contexts, by address: the certificate contexts in them.
    chains: HashMap<(u64, u64), Vec<u64>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static NEXT_STORE: AtomicU64 = AtomicU64::new(0x00c5_0000);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut s = lock(&STATE);
    f(s.get_or_insert_with(State::default))
}

fn reply(task: &mut Task, cx: &mut Cx<'_>, r: Result<u64, u32>) -> Reply {
    match r {
        Ok(v) => Reply::Value(v),
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(0)
        }
    }
}

impl Task {
    /// A new CERT_CONTEXT for `der`, with its CERT_INFO and copies of what they point to, in one block from the heap.
    fn new_context(&mut self, cx: &mut Cx<'_>, der: &[u8], store: u64) -> Result<u64, u32> {
        let info = parse(der).ok_or(CRYPT_E_ASN1_BADTAG)?;
        let w = self.abi().width;
        // The block: CERT_CONTEXT, CERT_INFO, then the DER, the serial (little-endian, as CryptoAPI keeps it), and
        // the OIDs' text.
        let context_size = if w == 8 { 40 } else { 20 };
        let info_size = if w == 8 { 184 } else { 112 };
        let serial: Vec<u8> = info.serial.iter().rev().copied().collect();
        let mut tail: Vec<u8> = Vec::new();
        let base_tail = (context_size + info_size) as u64;
        let mut put = |bytes: &[u8]| {
            let at = base_tail + tail.len() as u64;
            tail.extend(bytes);
            tail.push(0);
            while !tail.len().is_multiple_of(8) {
                tail.push(0);
            }
            at
        };
        let der_at = put(der);
        let serial_at = put(&serial);
        let sig_oid_at = put(info.signature.as_bytes());
        let key_oid_at = put(info.key_algorithm.as_bytes());
        let total = base_tail + tail.len() as u64;
        let base = self.task_alloc(cx, &vec![0; total as usize]).map_err(|_| 8u32)?;
        if base == 0 {
            return Err(8);
        }
        // Offsets into `der` of what CERT_INFO points to.
        let inside = |part: &[u8]| base + der_at + (part.as_ptr() as u64 - der.as_ptr() as u64);
        let mut c = Layout::new(w);
        c.u32(X509_ASN_ENCODING);
        c.ptr(base + der_at);
        c.u32(der.len() as u32);
        c.ptr(base + context_size as u64);
        c.ptr(store);
        let mut i = Layout::new(w);
        i.u32(info.version);
        i.blob(serial.len(), base + serial_at);
        i.ptr(base + sig_oid_at);
        i.blob(info.signature_params.len(), inside(info.signature_params));
        i.blob(info.issuer.len(), inside(info.issuer));
        i.u64(info.not_before);
        i.u64(info.not_after);
        i.blob(info.subject.len(), inside(info.subject));
        i.ptr(base + key_oid_at);
        i.blob(info.key_params.len(), inside(info.key_params));
        i.blob(info.key.len(), inside(info.key));
        i.u32(u32::from(info.unused_bits));
        // IssuerUniqueId and SubjectUniqueId (none), then no extensions.
        for _ in 0..2 {
            i.blob(0, 0);
            i.u32(0);
        }
        i.u32(0);
        i.ptr(0);
        let mut block = c.end();
        block.resize(context_size, 0);
        let mut info_bytes = i.end();
        info_bytes.resize(info_size, 0);
        block.extend(info_bytes);
        block.extend(&tail);
        self.mem(cx, true, |t| user::write(&mut t.space(), base, &block)).map_err(|_| 998u32)?;
        let pid = self.process.pid;
        with(|s| s.contexts.insert((pid, base), Context { der: der.to_vec(), refs: 1 }));
        Ok(base)
    }

    /// A context's DER: atlas's own, or as the CERT_CONTEXT says.
    fn context_der(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<Vec<u8>, u32> {
        let pid = self.process.pid;
        if let Some(der) = with(|s| s.contexts.get(&(pid, at)).map(|c| c.der.clone())) {
            return Ok(der);
        }
        let w = self.abi().width;
        self.mem(cx, false, |t| {
            let pb = user::read_ptr(&t.space(), at + w, w)?;
            let cb = user::read_u32(&t.space(), at + 2 * w)?;
            user::read_vec(&t.space(), pb, cb as usize)
        })
        .map_err(|_| E_INVALIDARG)
    }

    fn release_context(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<(), Reply> {
        let pid = self.process.pid;
        let gone = with(|s| match s.contexts.get_mut(&(pid, at)) {
            Some(c) if c.refs > 1 => {
                c.refs -= 1;
                false
            }
            Some(_) => {
                s.contexts.remove(&(pid, at));
                true
            }
            None => false,
        });
        if gone { self.task_free(cx, at) } else { Ok(()) }
    }
}

// ---- Stores and contexts ----

/// `CertOpenStore(provider, encoding, provider handle, flags, parameter)`: a store of atlas's, empty (a system
/// store's certificates aren't listed from the host).
fn cert_open_store(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    let handle = NEXT_STORE.fetch_add(4, Ordering::Relaxed);
    let pid = task.process.pid;
    with(|s| s.stores.insert((pid, handle), Vec::new()));
    Reply::Value(handle)
}

fn cert_close_store(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pid = task.process.pid;
    with(|s| s.stores.remove(&(pid, a[0])));
    Reply::Value(1)
}

/// `CertCreateCertificateContext(encoding, der, length)`.
fn cert_create_certificate_context(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let der = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], (a[2] as u32 as usize).min(1 << 20))).map_err(|_| E_INVALIDARG)?;
        task.new_context(cx, &der, 0)
    })();
    reply(task, cx, r)
}

fn cert_free_certificate_context(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] != 0
        && let Err(reply) = task.release_context(cx, a[0])
    {
        return reply;
    }
    Reply::Value(1)
}

fn cert_duplicate_certificate_context(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pid = task.process.pid;
    with(|s| {
        if let Some(c) = s.contexts.get_mut(&(pid, a[0])) {
            c.refs += 1;
        }
    });
    Reply::Value(a[0])
}

/// Adds a certificate to a store (once: the same one again is the one there), and gives a context of the store's
/// for it if asked.
fn add_to_store(task: &mut Task, cx: &mut Cx<'_>, store: u64, der: Vec<u8>, out: u64) -> Result<u64, u32> {
    let pid = task.process.pid;
    let known = with(|s| match s.stores.get_mut(&(pid, store)) {
        Some(list) => {
            if !list.contains(&der) {
                list.push(der.clone());
            }
            true
        }
        None => false,
    });
    if !known {
        return Err(E_INVALIDARG);
    }
    if out != 0 {
        let context = task.new_context(cx, &der, store)?;
        let ptr = task.abi().ptr(context);
        task.mem(cx, true, |t| user::write(&mut t.space(), out, &ptr)).map_err(|_| 998u32)?;
    }
    Ok(1)
}

/// `CertAddCertificateContextToStore(store, context, disposition, &store's context)`.
fn cert_add_certificate_context_to_store(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let der = task.context_der(cx, a[1])?;
        add_to_store(task, cx, a[0], der, a[3])
    })();
    reply(task, cx, r)
}

/// `CertAddEncodedCertificateToStore(store, encoding, der, length, disposition, &context)`.
fn cert_add_encoded_certificate_to_store(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let der = task.mem(cx, false, |t| user::read_vec(&t.space(), a[2], (a[3] as u32 as usize).min(1 << 20))).map_err(|_| E_INVALIDARG)?;
        parse(&der).ok_or(CRYPT_E_ASN1_BADTAG)?;
        add_to_store(task, cx, a[0], der, a[5])
    })();
    reply(task, cx, r)
}

// ---- Chains ----

/// The host's verdict as CERT_TRUST_STATUS's dwErrorStatus.
fn error_status(t: host::CertificateTrust) -> u32 {
    use host::CertificateTrust as T;
    let mut bits = 0;
    for (has, bit) in [
        (T::NOT_TIME_VALID, 0x1),
        (T::REVOKED, 0x4),
        (T::BAD_SIGNATURE, 0x8),
        (T::NOT_VALID_FOR_USAGE, 0x10),
        (T::UNTRUSTED_ROOT, 0x20),
        (T::BAD_CONSTRAINTS, 0x400),
        (T::PARTIAL_CHAIN, 0x10000),
        (T::MALFORMED, 0x8),
        (T::CANT_CHECK, 0x20),
    ] {
        if t.has(has) {
            bits |= bit;
        }
    }
    bits
}

/// The certificates from the leaf up, each issued by the next, as far as `pool` has them.
fn link(leaf: &[u8], pool: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut chain = vec![leaf.to_vec()];
    for _ in 0..8 {
        let Some(last) = chain.last().and_then(|d| parse(d).map(|i| (i.issuer.to_vec(), i.subject.to_vec()))) else { break };
        if last.0 == last.1 {
            break;
        }
        let Some(next) = pool.iter().find(|d| !chain.contains(d) && parse(d).is_some_and(|i| *i.subject == last.0[..])) else { break };
        chain.push(next.clone());
    }
    chain
}

/// What the chain's asked to be good for, from CERT_CHAIN_PARA's requested usage: server or client authentication.
fn usage(task: &mut Task, cx: &mut Cx<'_>, para: u64) -> host::CertificateUsage {
    if para == 0 {
        return host::CertificateUsage::Any;
    }
    let w = task.abi().width;
    let oids: Vec<String> = task
        .mem(cx, false, |t| {
            // cbSize, then RequestedUsage: dwType, then CERT_ENHKEY_USAGE (a count and an array of strings).
            let count = user::read_u32(&t.space(), para + 8)?.min(16);
            let list = user::read_ptr(&t.space(), para + 8 + w, w)?;
            let mut out = Vec::new();
            for i in 0..u64::from(count) {
                let s = user::read_ptr(&t.space(), list + i * w, w)?;
                out.push(String::from_utf8_lossy(&user::read_cstr(&t.space(), s, 64)?).into_owned());
            }
            Ok(out)
        })
        .unwrap_or_default();
    if oids.iter().any(|o| o == "1.3.6.1.5.5.7.3.1") {
        host::CertificateUsage::Server
    } else if oids.iter().any(|o| o == "1.3.6.1.5.5.7.3.2") {
        host::CertificateUsage::Client
    } else {
        host::CertificateUsage::Any
    }
}

/// `CertGetCertificateChain(engine, context, &time, additional store, &para, flags, reserved, &chain)`: the chain
/// the host's verdict is about, one simple chain of the certificates linked from the leaf, its trust status the
/// host's. Names are left to the caller (`CertVerifyCertificateChainPolicy`'s, to come).
fn cert_get_certificate_chain(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, u32> {
        if a[7] == 0 || a[1] == 0 {
            return Err(E_INVALIDARG);
        }
        let pid = task.process.pid;
        let leaf = task.context_der(cx, a[1])?;
        let w = task.abi().width;
        // CERT_CONTEXT's hCertStore, its last field.
        let own_store = task.mem(cx, false, |t| user::read_ptr(&t.space(), a[1] + if w == 8 { 32 } else { 16 }, w)).unwrap_or(0);
        let pool: Vec<Vec<u8>> = with(|s| [a[3], own_store].iter().flat_map(|h| s.stores.get(&(pid, *h)).cloned().unwrap_or_default()).collect());
        let at = if a[2] != 0 {
            let ft = task.mem(cx, false, |t| user::read_vec(&t.space(), a[2], 8)).map_err(|_| E_INVALIDARG)?;
            let ft = u64::from_le_bytes(ft.try_into().unwrap_or_default());
            Some((ft / 10_000_000) as i64 - 11_644_473_600)
        } else {
            None
        };
        let used = usage(task, cx, a[4]);
        let mut offered: Vec<&[u8]> = vec![&leaf];
        offered.extend(pool.iter().map(Vec::as_slice));
        let verdict = task.k.host.check_certificates(&offered, used, None, at);
        let error = error_status(verdict);
        let chain = link(&leaf, &pool);
        // A context for each certificate in it.
        let mut contexts = Vec::new();
        for der in &chain {
            contexts.push(task.new_context(cx, der, 0)?);
        }
        let n = contexts.len() as u64;
        // One block: the chain context, its simple chain, the elements, then the two pointer arrays.
        let (chain_size, simple_size, element_size) = if w == 8 { (72u64, 40u64, 56u64) } else { (56, 32, 32) };
        let simple_at = chain_size;
        let elements_at = simple_at + simple_size;
        let element_ptrs_at = elements_at + n * element_size;
        let chain_ptrs_at = element_ptrs_at + n * w;
        let total = chain_ptrs_at + w;
        let base = task.task_alloc(cx, &vec![0; total as usize]).map_err(|_| 8u32)?;
        if base == 0 {
            return Err(8);
        }
        let mut b = Vec::new();
        let mut c = Layout::new(w);
        c.u32(chain_size as u32);
        c.u32(error);
        c.u32(0);
        c.u32(1);
        c.ptr(base + chain_ptrs_at);
        c.u32(0);
        c.ptr(0);
        c.u32(0);
        c.u32(0);
        c.u32(0);
        let mut bytes = c.end();
        bytes.resize(chain_size as usize, 0);
        b.extend(bytes);
        let mut s = Layout::new(w);
        s.u32(simple_size as u32);
        s.u32(error);
        s.u32(0);
        s.u32(n as u32);
        s.ptr(base + element_ptrs_at);
        s.ptr(0);
        s.u32(0);
        s.u32(0);
        let mut bytes = s.end();
        bytes.resize(simple_size as usize, 0);
        b.extend(bytes);
        for (i, (context, der)) in contexts.iter().zip(&chain).enumerate() {
            let self_signed = parse(der).is_some_and(|x| x.issuer == x.subject);
            let mut e = Layout::new(w);
            e.u32(element_size as u32);
            e.ptr(*context);
            e.u32(if i == 0 { error } else { 0 });
            e.u32(if self_signed { 0x8 } else { 0 });
            for _ in 0..4 {
                e.ptr(0);
            }
            let mut bytes = e.end();
            bytes.resize(element_size as usize, 0);
            b.extend(bytes);
        }
        for i in 0..n {
            b.extend(task.abi().ptr(base + elements_at + i * element_size));
        }
        b.extend(task.abi().ptr(base + simple_at));
        task.mem(cx, true, |t| user::write(&mut t.space(), base, &b)).map_err(|_| 998u32)?;
        let ptr = task.abi().ptr(base);
        task.mem(cx, true, |t| user::write(&mut t.space(), a[7], &ptr)).map_err(|_| 998u32)?;
        with(|s| s.chains.insert((pid, base), contexts));
        Ok(1)
    })();
    reply(task, cx, r)
}

fn cert_free_certificate_chain(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pid = task.process.pid;
    if let Some(contexts) = with(|s| s.chains.remove(&(pid, a[0]))) {
        for c in contexts {
            if let Err(reply) = task.release_context(cx, c) {
                return reply;
            }
        }
        if let Err(reply) = task.task_free(cx, a[0]) {
            return reply;
        }
    }
    Reply::Value(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oids_and_lengths() {
        assert_eq!(oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b]), "1.2.840.113549.1.1.11");
        assert_eq!(oid(&[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01]), "1.3.6.1.5.5.7.3.1");
        let long = [&[0x30, 0x82, 0x01, 0x00][..], &[0u8; 256]].concat();
        let (t, rest) = tlv(&long).unwrap();
        assert_eq!((t.tag, t.body.len(), rest.len()), (0x30, 256, 0));
        assert!(tlv(&[0x30, 0x05, 1]).is_none());
    }
}
