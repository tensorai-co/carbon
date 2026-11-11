//! Certificate chains, checked by Windows' own chain engine against the user's and the machine's trusted roots. Windows
//! keeps few roots on disk and adds the rest from Microsoft's root program as a chain first needs one: that's its trust
//! store, so that's allowed; nothing is fetched from a certificate's own URLs (intermediates, revocation lists). The chain's
//! error status is a set, so every failure comes back at once; a server's name is checked by the SSL policy, told to
//! ignore everything else.

use std::ffi::c_void;
use std::ptr;

use crate::{CertificateTrust, CertificateUsage};

type Handle = *mut c_void;

#[repr(C)]
struct CertTrustStatus {
    error_status: u32,
    info_status: u32,
}

#[repr(C)]
struct CertChainContext {
    size: u32,
    trust_status: CertTrustStatus,
    // The rest isn't read.
}

#[repr(C)]
struct CertEnhKeyUsage {
    count: u32,
    identifiers: *const *const u8,
}

#[repr(C)]
struct CertUsageMatch {
    kind: u32,
    usage: CertEnhKeyUsage,
}

#[repr(C)]
struct CertChainPara {
    size: u32,
    requested_usage: CertUsageMatch,
}

#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
struct SslExtraCertChainPolicyPara {
    size: u32,
    auth_type: u32,
    checks: u32,
    server_name: *const u16,
}

#[repr(C)]
struct CertChainPolicyPara {
    size: u32,
    flags: u32,
    extra: *mut c_void,
}

#[repr(C)]
struct CertChainPolicyStatus {
    size: u32,
    error: u32,
    chain_index: i32,
    element_index: i32,
    extra: *mut c_void,
}

#[link(name = "crypt32")]
unsafe extern "system" {
    fn CertCreateCertificateContext(encoding: u32, encoded: *const u8, length: u32) -> *const c_void;
    fn CertFreeCertificateContext(context: *const c_void) -> i32;
    fn CertOpenStore(provider: usize, encoding: u32, crypt_prov: usize, flags: u32, para: *const c_void) -> Handle;
    fn CertCloseStore(store: Handle, flags: u32) -> i32;
    fn CertAddEncodedCertificateToStore(store: Handle, encoding: u32, encoded: *const u8, length: u32, disposition: u32, context: *mut *const c_void) -> i32;
    fn CertGetCertificateChain(
        engine: Handle,
        context: *const c_void,
        time: *const FileTime,
        additional: Handle,
        para: *const CertChainPara,
        flags: u32,
        reserved: *mut c_void,
        chain: *mut *const CertChainContext,
    ) -> i32;
    fn CertVerifyCertificateChainPolicy(policy: usize, chain: *const CertChainContext, para: *const CertChainPolicyPara, status: *mut CertChainPolicyStatus) -> i32;
    fn CertFreeCertificateChain(chain: *const CertChainContext);
}

const ENCODING: u32 = 0x1 | 0x10000; // X509_ASN_ENCODING | PKCS_7_ASN_ENCODING
const CERT_STORE_PROV_MEMORY: usize = 2;
const CERT_STORE_ADD_ALWAYS: u32 = 4;
const USAGE_MATCH_TYPE_AND: u32 = 0;
const CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL: u32 = 0x8000_0000;
const CERT_CHAIN_POLICY_SSL: usize = 4;
const AUTHTYPE_SERVER: u32 = 2;
const CERT_E_CN_NO_MATCH: u32 = 0x800B_010F;
// Everything the SSL policy could complain of but the name.
const IGNORE_ALL_BUT_NAME: u32 = 0x80 | 0x100 | 0x200 | 0x2000; // revocation, unknown CA, wrong usage, date
/// From 1601 to 1970, in seconds.
const FILETIME_EPOCH: i64 = 11_644_473_600;

const SERVER_AUTH: &[u8] = b"1.3.6.1.5.5.7.3.1\0";
const CLIENT_AUTH: &[u8] = b"1.3.6.1.5.5.7.3.2\0";

/// What the chain's error bits mean in `host`'s terms. Revocation that couldn't be checked offline isn't a failure.
fn meaning(errors: u32) -> CertificateTrust {
    const BITS: [(u32, CertificateTrust); 13] = [
        (0x1, CertificateTrust::NOT_TIME_VALID),
        (0x4, CertificateTrust::REVOKED),
        (0x8, CertificateTrust::BAD_SIGNATURE),
        (0x10, CertificateTrust::NOT_VALID_FOR_USAGE),
        (0x20, CertificateTrust::UNTRUSTED_ROOT),
        (0x80, CertificateTrust::PARTIAL_CHAIN),         // cyclic
        (0x100, CertificateTrust::MALFORMED),            // an extension it can't read
        (0x200, CertificateTrust::BAD_CONSTRAINTS),      // policy constraints
        (0x400, CertificateTrust::BAD_CONSTRAINTS),      // basic constraints
        (0xF800, CertificateTrust::BAD_CONSTRAINTS),     // name constraints, every kind
        (0x10000, CertificateTrust::PARTIAL_CHAIN),
        (0x10_0000, CertificateTrust::BAD_SIGNATURE),    // a weak signature
        (0x400_0000, CertificateTrust::REVOKED),         // explicitly distrusted
    ];
    let mut trust = CertificateTrust::TRUSTED;
    for (bits, meaning) in BITS {
        if errors & bits != 0 {
            trust |= meaning;
        }
    }
    trust
}

/// Something of crypt32's that's ours to free.
struct Owned<T: Copy>(T, fn(T));

impl<T: Copy> Drop for Owned<T> {
    fn drop(&mut self) {
        (self.1)(self.0);
    }
}

pub fn check(chain: &[&[u8]], usage: CertificateUsage, name: Option<&str>, at: Option<i64>) -> CertificateTrust {
    let leaf = chain[0];
    // SAFETY: the bytes are as long as that, and copied.
    let context = unsafe { CertCreateCertificateContext(ENCODING, leaf.as_ptr(), leaf.len() as u32) };
    if context.is_null() {
        return CertificateTrust::MALFORMED;
    }
    let context = Owned(context, |c| {
        // SAFETY: a context from CertCreateCertificateContext, freed once.
        unsafe { CertFreeCertificateContext(c) };
    });
    // The rest of what was given, where the engine can find intermediates.
    // SAFETY: a new in-memory store.
    let store = unsafe { CertOpenStore(CERT_STORE_PROV_MEMORY, 0, 0, 0, ptr::null()) };
    if store.is_null() {
        return CertificateTrust::CANT_CHECK;
    }
    let store = Owned(store, |s| {
        // SAFETY: a store from CertOpenStore, closed once.
        unsafe { CertCloseStore(s, 0) };
    });
    for der in &chain[1..] {
        // SAFETY: the bytes are as long as that, and copied into the store. One that doesn't parse can't be in the chain.
        unsafe { CertAddEncodedCertificateToStore(store.0, ENCODING, der.as_ptr(), der.len() as u32, CERT_STORE_ADD_ALWAYS, ptr::null_mut()) };
    }
    let wanted: [*const u8; 1] = [if usage == CertificateUsage::Client { CLIENT_AUTH.as_ptr() } else { SERVER_AUTH.as_ptr() }];
    let para = CertChainPara {
        size: size_of::<CertChainPara>() as u32,
        requested_usage: CertUsageMatch {
            kind: USAGE_MATCH_TYPE_AND,
            usage: CertEnhKeyUsage { count: if usage == CertificateUsage::Any { 0 } else { 1 }, identifiers: wanted.as_ptr() },
        },
    };
    let time = at.map(|at| {
        let ticks = ((at + FILETIME_EPOCH).max(0) as u64) * 10_000_000;
        FileTime { low: ticks as u32, high: (ticks >> 32) as u32 }
    });
    let mut result = ptr::null();
    // SAFETY: live objects and structures, and room for the chain, which is ours if it's made.
    let made = unsafe {
        CertGetCertificateChain(
            ptr::null_mut(),
            context.0,
            time.as_ref().map_or(ptr::null(), |t| t as *const FileTime),
            store.0,
            &para,
            CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL,
            ptr::null_mut(),
            &mut result,
        )
    };
    if made == 0 || result.is_null() {
        return CertificateTrust::CANT_CHECK;
    }
    // SAFETY: a chain from CertGetCertificateChain, freed once.
    let result = Owned(result, |c| unsafe { CertFreeCertificateChain(c) });
    // SAFETY: a live chain context.
    let mut trust = meaning(unsafe { (*result.0).trust_status.error_status });

    if let (CertificateUsage::Server, Some(name)) = (usage, name) {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut extra = SslExtraCertChainPolicyPara {
            size: size_of::<SslExtraCertChainPolicyPara>() as u32,
            auth_type: AUTHTYPE_SERVER,
            checks: IGNORE_ALL_BUT_NAME,
            server_name: wide.as_ptr(),
        };
        let policy = CertChainPolicyPara { size: size_of::<CertChainPolicyPara>() as u32, flags: 0, extra: (&raw mut extra).cast() };
        let mut status = CertChainPolicyStatus { size: size_of::<CertChainPolicyStatus>() as u32, error: 0, chain_index: 0, element_index: 0, extra: ptr::null_mut() };
        // SAFETY: a live chain, and structures that outlive the call.
        let checked = unsafe { CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL, result.0, &policy, &mut status) };
        if checked == 0 {
            trust |= CertificateTrust::CANT_CHECK;
        } else if status.error == CERT_E_CN_NO_MATCH {
            trust |= CertificateTrust::WRONG_NAME;
        }
    }
    trust
}
