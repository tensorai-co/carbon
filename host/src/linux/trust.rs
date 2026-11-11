//! Certificate chains, checked against the roots in the system's CA bundle, the one OpenSSL and every distribution's
//! tools use (`SSL_CERT_FILE` if it's set, else the distribution's own file), with webpki doing the checking.
//!
//! webpki stops at the first thing wrong, so a chain is checked more than once to report more than that: against the
//! system's roots; if that fails on something else first (its use), against them again for any use, to learn whether it
//! reaches a trusted root at all; if it doesn't, against the chain's own self-signed certificates as roots, which tells
//! an untrusted root from a chain that stops short and finds what else is wrong (its dates, its use, its signatures).
//! A certificate out of date is noted and looked at again at a time it's in date. A server's name is checked on its own.

use std::sync::OnceLock;
use std::time::Duration;

use pki_types::pem::PemObject;
use pki_types::{CertificateDer, ServerName, TrustAnchor, UnixTime};
use webpki::{EndEntityCert, Error, ExtendedKeyUsageValidator, KeyPurposeIdIter, KeyUsage};

use crate::{CertificateTrust, CertificateUsage};

/// Where distributions keep their bundle of trusted roots, most common first.
const BUNDLES: [&str; 5] = [
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
    "/etc/ssl/cert.pem",
];

/// The system's trusted roots, read once.
fn system_roots() -> &'static [TrustAnchor<'static>] {
    static ROOTS: OnceLock<Vec<TrustAnchor<'static>>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let file = std::env::var("SSL_CERT_FILE").ok().into_iter().chain(BUNDLES.iter().map(|s| s.to_string()));
        for path in file {
            let Ok(certificates) = CertificateDer::pem_file_iter(&path) else { continue };
            let roots: Vec<TrustAnchor<'static>> = certificates
                .filter_map(Result::ok)
                .filter_map(|der| webpki::anchor_from_trusted_cert(&der).ok().map(|anchor| anchor.to_owned()))
                .collect();
            if !roots.is_empty() {
                return roots;
            }
        }
        Vec::new()
    })
}

pub fn check(chain: &[&[u8]], usage: CertificateUsage, name: Option<&str>, at: Option<i64>) -> CertificateTrust {
    let roots = system_roots();
    if roots.is_empty() {
        return CertificateTrust::CANT_CHECK;
    }
    check_against(chain, usage, name, at, roots)
}

/// Accepts any extended key usage, or none.
struct AnyUsage;

impl ExtendedKeyUsageValidator for AnyUsage {
    fn validate(&self, _: KeyPurposeIdIter<'_, '_>) -> Result<(), Error> {
        Ok(())
    }
}

/// What webpki's error means in `host`'s terms. Older webpki errors without their details are still mapped, since
/// webpki may still give them.
#[allow(deprecated)]
fn meaning(error: &Error) -> CertificateTrust {
    match error {
        Error::UnknownIssuer => CertificateTrust::UNTRUSTED_ROOT,
        Error::CertExpired { .. } | Error::CertNotValidYet { .. } | Error::InvalidCertValidity => CertificateTrust::NOT_TIME_VALID,
        Error::RequiredEkuNotFound | Error::RequiredEkuNotFoundContext(_) | Error::EmptyEkuExtension => CertificateTrust::NOT_VALID_FOR_USAGE,
        Error::CertRevoked | Error::CertDistrusted => CertificateTrust::REVOKED,
        Error::InvalidSignatureForPublicKey
        | Error::UnsupportedSignatureAlgorithm
        | Error::UnsupportedSignatureAlgorithmForPublicKey
        | Error::UnsupportedSignatureAlgorithmContext(_)
        | Error::UnsupportedSignatureAlgorithmForPublicKeyContext(_)
        | Error::SignatureAlgorithmMismatch => CertificateTrust::BAD_SIGNATURE,
        Error::CaUsedAsEndEntity | Error::EndEntityUsedAsCa | Error::PathLenConstraintViolated | Error::NameConstraintViolation => {
            CertificateTrust::BAD_CONSTRAINTS
        }
        Error::BadDer | Error::BadDerTime | Error::TrailingData(_) | Error::UnsupportedCertVersion => CertificateTrust::MALFORMED,
        Error::CertNotValidForName(_) => CertificateTrust::WRONG_NAME,
        _ => CertificateTrust::NOT_VALID_FOR_USAGE,
    }
}

/// The check itself, against `roots`.
pub(crate) fn check_against(chain: &[&[u8]], usage: CertificateUsage, name: Option<&str>, at: Option<i64>, roots: &[TrustAnchor<'_>]) -> CertificateTrust {
    let ders: Vec<CertificateDer<'_>> = chain.iter().map(|der| CertificateDer::from(*der)).collect();
    let Ok(leaf) = EndEntityCert::try_from(&ders[0]) else { return CertificateTrust::MALFORMED };
    let time = match at {
        Some(at) => UnixTime::since_unix_epoch(Duration::from_secs(at.max(0) as u64)),
        None => UnixTime::now(),
    };
    let algorithms = webpki::ALL_VERIFICATION_ALGS;
    let verify = |anchors: &[TrustAnchor<'_>], time: UnixTime, usage: CertificateUsage| -> Result<(), Error> {
        let intermediates = &ders[1..];
        let result = match usage {
            CertificateUsage::Any => leaf.verify_for_usage(algorithms, anchors, intermediates, time, AnyUsage, None, None),
            CertificateUsage::Server => leaf.verify_for_usage(algorithms, anchors, intermediates, time, KeyUsage::server_auth(), None, None),
            CertificateUsage::Client => leaf.verify_for_usage(algorithms, anchors, intermediates, time, KeyUsage::client_auth(), None, None),
        };
        result.map(|_| ())
    };
    // A certificate out of date stops webpki before anything else: note it, and look again at a time it's in date,
    // which webpki's error says, to find what else is wrong.
    let mut trust = CertificateTrust::TRUSTED;
    let mut time = time;
    let checked = |anchors: &[TrustAnchor<'_>], trust: &mut CertificateTrust, time: &mut UnixTime, usage: CertificateUsage| {
        let mut result = verify(anchors, *time, usage);
        for _ in 0..ders.len() + 1 {
            *time = match result {
                Err(Error::CertExpired { not_after, .. }) => not_after,
                Err(Error::CertNotValidYet { not_before, .. }) => not_before,
                _ => break,
            };
            *trust |= CertificateTrust::NOT_TIME_VALID;
            result = verify(anchors, *time, usage);
        }
        result
    };

    // The chain's own self-signed certificates, as roots: what it would reach if the host trusted them.
    let self_signed = |der: &&CertificateDer<'_>| EndEntityCert::try_from(*der).is_ok_and(|cert| cert.issuer() == cert.subject());
    let own: Vec<TrustAnchor<'_>> = ders.iter().filter(self_signed).filter_map(|der| webpki::anchor_from_trusted_cert(der).ok()).collect();
    // No trusted root: if the chain reaches one of its own, that's a root this host doesn't trust; if not, it doesn't
    // reach a root at all. Either way, checking against its own shows what else is wrong.
    let untrusted = |trust: &mut CertificateTrust, time: &mut UnixTime, usage| match checked(&own, trust, time, usage) {
        Ok(()) => *trust |= CertificateTrust::UNTRUSTED_ROOT,
        Err(Error::UnknownIssuer) => *trust |= CertificateTrust::PARTIAL_CHAIN,
        Err(other) => *trust |= CertificateTrust::UNTRUSTED_ROOT | meaning(&other),
    };
    match checked(roots, &mut trust, &mut time, usage) {
        Ok(()) => {}
        Err(Error::UnknownIssuer) => untrusted(&mut trust, &mut time, usage),
        Err(error) => {
            trust |= meaning(&error);
            // webpki may stop at a certificate's use before looking for its issuer: whether it reaches a trusted root
            // at all is checked for any use.
            match checked(roots, &mut trust, &mut time, CertificateUsage::Any) {
                Err(Error::UnknownIssuer) => untrusted(&mut trust, &mut time, CertificateUsage::Any),
                Err(other) => trust |= meaning(&other),
                Ok(()) => {}
            }
        }
    }
    if let (CertificateUsage::Server, Some(name)) = (usage, name) {
        match ServerName::try_from(name) {
            Ok(server) => {
                if leaf.verify_is_valid_for_subject_name(&server).is_err() {
                    trust |= CertificateTrust::WRONG_NAME;
                }
            }
            Err(_) => trust |= CertificateTrust::WRONG_NAME,
        }
    }
    trust
}
