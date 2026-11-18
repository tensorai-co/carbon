//! `--tls`: TLS 1.3 with a certificate and key from PEM files, no client certificates.

use std::path::Path;
use std::sync::Arc;

use pki_types::pem::PemObject;
use pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;

/// The server's TLS config.
pub fn config(cert: &Path, key: &Path) -> Result<Arc<ServerConfig>, String> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert)
        .and_then(|it| it.collect::<Result<_, _>>())
        .map_err(|e| format!("--tlscert {}: {e}", cert.display()))?;
    if certs.is_empty() {
        return Err(format!("--tlscert {}: no certificates", cert.display()));
    }
    let key = PrivateKeyDer::from_pem_file(key).map_err(|e| format!("--tlskey {}: {e}", key.display()))?;
    let hint = "(keys: RSA, Ed25519, or ECDSA on a named curve, P-256 or P-384; certificates: X.509 v3)";
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("--tlscert and --tlskey: {e} {hint}"))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}
