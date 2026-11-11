//! TLS for pulling images: rustls, with the host deciding which servers to trust. The server's chain and name go to
//! `host::Host::check_certificates`, so the host's own store and rules decide (Security.framework on macOS, the system
//! bundle on Linux, CryptoAPI on Windows), as they would for any other program there; rustls checks the handshake's
//! signatures with ring.

use std::sync::Arc;

use host::{CertificateTrust, CertificateUsage, Host};
use pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, OtherError, SignatureScheme};

/// Why the host wouldn't trust a server, as rustls passes it on.
#[derive(Debug)]
struct Untrusted(String, CertificateTrust);

impl std::fmt::Display for Untrusted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "this host doesn't trust {}'s certificate: {:?}", self.0, self.1)
    }
}

impl std::error::Error for Untrusted {}

#[derive(Debug)]
struct HostVerifier {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for HostVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let chain: Vec<&[u8]> = std::iter::once(end_entity.as_ref()).chain(intermediates.iter().map(|c| c.as_ref())).collect();
        let name = server_name.to_str().into_owned();
        let trust = host::Std::new().check_certificates(&chain, CertificateUsage::Server, Some(&name), Some(now.as_secs() as i64));
        if trust.is_trusted() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(CertificateError::Other(OtherError(Arc::new(Untrusted(name, trust))))))
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// A client configuration that trusts what the host trusts.
pub fn client_config() -> Result<Arc<ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("setting up TLS: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(HostVerifier { provider }))
        .with_no_client_auth();
    Ok(Arc::new(config))
}
