//! Certificate chains, checked by macOS's own trust settings: the roots it trusts and its rules for TLS servers,
//! including a server's name when one is given.
//! Failures are read from the per-certificate details rather than the single error returned, which names only one
//! problem, and not always the one that matters: a self-signed certificate is reported as valid for too long before
//! it's reported as untrusted.

use std::ffi::{c_char, c_void};
use std::ptr;

use crate::{CertificateTrust, CertificateUsage};

type CFTypeRef = *const c_void;
type CFIndex = isize;
type Boolean = u8;

#[repr(C)]
struct CFArrayCallBacks {
    version: CFIndex,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeArrayCallBacks: CFArrayCallBacks;
    fn CFRelease(object: CFTypeRef);
    fn CFDataCreate(allocator: CFTypeRef, bytes: *const u8, length: CFIndex) -> CFTypeRef;
    fn CFArrayCreate(allocator: CFTypeRef, values: *const CFTypeRef, count: CFIndex, callbacks: *const CFArrayCallBacks) -> CFTypeRef;
    fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: CFIndex) -> CFTypeRef;
    fn CFDictionaryGetCount(dictionary: CFTypeRef) -> CFIndex;
    fn CFDictionaryGetKeysAndValues(dictionary: CFTypeRef, keys: *mut CFTypeRef, values: *mut CFTypeRef);
    fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFStringCreateWithBytes(allocator: CFTypeRef, bytes: *const u8, length: CFIndex, encoding: u32, external: Boolean) -> CFTypeRef;
    fn CFStringGetCString(string: CFTypeRef, buffer: *mut c_char, size: CFIndex, encoding: u32) -> Boolean;
    fn CFGetTypeID(object: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFDateCreate(allocator: CFTypeRef, at: f64) -> CFTypeRef;
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecCertificateCreateWithData(allocator: CFTypeRef, data: CFTypeRef) -> CFTypeRef;
    fn SecPolicyCreateBasicX509() -> CFTypeRef;
    fn SecPolicyCreateSSL(server: Boolean, hostname: CFTypeRef) -> CFTypeRef;
    fn SecTrustCreateWithCertificates(certificates: CFTypeRef, policies: CFTypeRef, trust: *mut CFTypeRef) -> i32;
    fn SecTrustSetNetworkFetchAllowed(trust: CFTypeRef, allowed: Boolean) -> i32;
    fn SecTrustSetVerifyDate(trust: CFTypeRef, date: CFTypeRef) -> i32;
    fn SecTrustEvaluateWithError(trust: CFTypeRef, error: *mut CFTypeRef) -> bool;
    fn SecTrustCopyResult(trust: CFTypeRef) -> CFTypeRef;
}

const UTF8: u32 = 0x0800_0100;
/// Seconds from 1970 to 2001, which is when Core Foundation's time starts.
const CF_EPOCH: i64 = 978_307_200;

/// Something of Core Foundation's that's ours to release.
struct Owned(CFTypeRef);

impl Owned {
    fn new(object: CFTypeRef) -> Option<Owned> {
        (!object.is_null()).then(|| Owned(object))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: it's ours, and not null.
        unsafe { CFRelease(self.0) };
    }
}

fn is(object: CFTypeRef, type_id: usize) -> bool {
    // SAFETY: a live object, as each caller has it from a container that's still kept.
    !object.is_null() && unsafe { CFGetTypeID(object) } == type_id
}

/// What one of macOS's named checks means in `host`'s terms. An unnamed check still failed, so it counts as not allowed
/// for this use.
fn meaning(check: &str) -> CertificateTrust {
    match check {
        "AnchorTrusted" | "AnchorSHA1" | "AnchorSHA256" | "AnchorApple" => CertificateTrust::UNTRUSTED_ROOT,
        "MissingIntermediate" => CertificateTrust::PARTIAL_CHAIN,
        "TemporalValidity" | "ValidLeaf" | "ValidIntermediates" | "ValidRoot" => CertificateTrust::NOT_TIME_VALID,
        "ExtendedKeyUsage" | "KeyUsage" | "ServerAuthEKU" | "UsageConstraints" => CertificateTrust::NOT_VALID_FOR_USAGE,
        "Revocation" | "RevocationResponseRequired" | "BlackListedLeaf" | "BlackListedKey" => CertificateTrust::REVOKED,
        "NameConstraints" | "PolicyConstraints" => CertificateTrust::BAD_CONSTRAINTS,
        "SSLHostname" => CertificateTrust::WRONG_NAME,
        _ if check.starts_with("BasicConstraints") => CertificateTrust::BAD_CONSTRAINTS,
        _ if check.starts_with("Signature") || check.starts_with("Weak") || check.starts_with("SystemTrustedWeak") || check.starts_with("KeySize") => {
            CertificateTrust::BAD_SIGNATURE
        }
        _ => CertificateTrust::NOT_VALID_FOR_USAGE,
    }
}

/// What the failed checks in a trust result add up to.
///
/// # Safety
///
/// `result` is a live dictionary from `SecTrustCopyResult`.
unsafe fn failed_checks(result: CFTypeRef) -> CertificateTrust {
    let mut trust = CertificateTrust::TRUSTED;
    let name = b"TrustResultDetails";
    // SAFETY: the bytes are as long as that.
    let Some(key) = Owned::new(unsafe { CFStringCreateWithBytes(ptr::null(), name.as_ptr(), name.len() as CFIndex, UTF8, 0) }) else {
        return trust;
    };
    // SAFETY: as the caller says; what comes back isn't ours, and lives as long as `result`.
    let details = unsafe { CFDictionaryGetValue(result, key.0) };
    // SAFETY: no arguments.
    if !is(details, unsafe { CFArrayGetTypeID() }) {
        return trust;
    }
    // SAFETY: an array, as just seen.
    for index in 0..unsafe { CFArrayGetCount(details) } {
        // SAFETY: in range.
        let certificate = unsafe { CFArrayGetValueAtIndex(details, index) };
        // SAFETY: no arguments.
        if !is(certificate, unsafe { CFDictionaryGetTypeID() }) {
            continue;
        }
        // SAFETY: a dictionary, as just seen.
        let count = unsafe { CFDictionaryGetCount(certificate) } as usize;
        let mut keys = vec![ptr::null(); count];
        // SAFETY: room for each key, and the values aren't asked for.
        unsafe { CFDictionaryGetKeysAndValues(certificate, keys.as_mut_ptr(), ptr::null_mut()) };
        for check in keys {
            // SAFETY: no arguments.
            if !is(check, unsafe { CFStringGetTypeID() }) {
                continue;
            }
            let mut buffer = [0 as c_char; 128];
            // SAFETY: a string, into a buffer of this size, which it ends with a nul.
            if unsafe { CFStringGetCString(check, buffer.as_mut_ptr(), buffer.len() as CFIndex, UTF8) } == 0 {
                trust |= CertificateTrust::NOT_VALID_FOR_USAGE;
                continue;
            }
            // SAFETY: nul-terminated, as it said.
            let check = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
            if check != "StatusCodes" {
                trust |= meaning(&check);
            }
        }
    }
    trust
}

pub fn check(chain: &[&[u8]], usage: CertificateUsage, name: Option<&str>, at: Option<i64>) -> CertificateTrust {
    let mut certificates = Vec::with_capacity(chain.len());
    for (index, der) in chain.iter().enumerate() {
        // SAFETY: the bytes are as long as that, and copied.
        let data = Owned::new(unsafe { CFDataCreate(ptr::null(), der.as_ptr(), der.len() as CFIndex) });
        // SAFETY: data, or nothing.
        let certificate = data.and_then(|data| Owned::new(unsafe { SecCertificateCreateWithData(ptr::null(), data.0) }));
        match certificate {
            Some(certificate) => certificates.push(certificate),
            // The certificate being checked must parse. Others that don't can't be part of its chain.
            None if index == 0 => return CertificateTrust::MALFORMED,
            None => {}
        }
    }
    if certificates.is_empty() {
        return CertificateTrust::MALFORMED;
    }
    let values: Vec<CFTypeRef> = certificates.iter().map(|certificate| certificate.0).collect();
    // SAFETY: an array of live objects, which it retains.
    let Some(array) = Owned::new(unsafe { CFArrayCreate(ptr::null(), values.as_ptr(), values.len() as CFIndex, &raw const kCFTypeArrayCallBacks) }) else {
        return CertificateTrust::CANT_CHECK;
    };
    // A server's name, if one was given: without it, names are left to the caller.
    let hostname = match (usage, name) {
        // SAFETY: the bytes are as long as that.
        (CertificateUsage::Server, Some(name)) => match Owned::new(unsafe { CFStringCreateWithBytes(ptr::null(), name.as_ptr(), name.len() as CFIndex, UTF8, 0) }) {
            Some(hostname) => Some(hostname),
            None => return CertificateTrust::CANT_CHECK,
        },
        _ => None,
    };
    let hostname_ref = hostname.as_ref().map_or(ptr::null(), |h| h.0);
    // SAFETY: a live string or null, for the server's policy.
    let policy = Owned::new(unsafe {
        match usage {
            CertificateUsage::Any => SecPolicyCreateBasicX509(),
            CertificateUsage::Server => SecPolicyCreateSSL(1, hostname_ref),
            CertificateUsage::Client => SecPolicyCreateSSL(0, ptr::null()),
        }
    });
    let Some(policy) = policy else {
        return CertificateTrust::CANT_CHECK;
    };
    let mut trust = ptr::null();
    // SAFETY: live objects, and room for what comes back.
    if unsafe { SecTrustCreateWithCertificates(array.0, policy.0, &raw mut trust) } != 0 {
        return CertificateTrust::CANT_CHECK;
    }
    let Some(trust) = Owned::new(trust) else {
        return CertificateTrust::CANT_CHECK;
    };
    // A machine's certificate doesn't get to make the host fetch anything. SAFETY: a live trust object.
    if unsafe { SecTrustSetNetworkFetchAllowed(trust.0, 0) } != 0 {
        return CertificateTrust::CANT_CHECK;
    }
    if let Some(at) = at {
        // SAFETY: no pointers.
        let Some(date) = Owned::new(unsafe { CFDateCreate(ptr::null(), (at - CF_EPOCH) as f64) }) else {
            return CertificateTrust::CANT_CHECK;
        };
        // SAFETY: live objects.
        if unsafe { SecTrustSetVerifyDate(trust.0, date.0) } != 0 {
            return CertificateTrust::CANT_CHECK;
        }
    }
    let mut error = ptr::null();
    // SAFETY: a live trust object, and room for an error, which is ours if there is one.
    let trusted = unsafe { SecTrustEvaluateWithError(trust.0, &raw mut error) };
    drop(Owned::new(error));
    if trusted {
        return CertificateTrust::TRUSTED;
    }
    // SAFETY: a trust object that's been evaluated.
    let reasons = Owned::new(unsafe { SecTrustCopyResult(trust.0) })
        // SAFETY: a dictionary, from `SecTrustCopyResult`.
        .map(|result| unsafe { failed_checks(result.0) })
        .unwrap_or_default();
    // It said no, so it's no, even if why isn't something it named.
    if reasons.is_trusted() { CertificateTrust::NOT_VALID_FOR_USAGE } else { reasons }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rubbish_isnt_a_certificate() {
        assert_eq!(check(&[b"not a certificate"], CertificateUsage::Any, None, None), CertificateTrust::MALFORMED);
        assert_eq!(check(&[], CertificateUsage::Server, None, None), CertificateTrust::MALFORMED);
    }

    #[test]
    fn checks_are_named() {
        assert_eq!(meaning("AnchorTrusted"), CertificateTrust::UNTRUSTED_ROOT);
        assert_eq!(meaning("TemporalValidity"), CertificateTrust::NOT_TIME_VALID);
        assert_eq!(meaning("BasicConstraintsCA"), CertificateTrust::BAD_CONSTRAINTS);
        assert_eq!(meaning("SSLHostname"), CertificateTrust::WRONG_NAME);
        // One that's new to us still fails.
        assert_eq!(meaning("SomethingNew"), CertificateTrust::NOT_VALID_FOR_USAGE);
    }
}
