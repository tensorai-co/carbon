//! crypt32.dll: certificates. Not written yet: the store and chain checks HTTPS needs come with Winsock.

use super::dll;

dll! {
    "CRYPT32.dll";
    missing CertOpenStore;
    missing CertCloseStore;
    missing CertCreateCertificateContext;
    missing CertFreeCertificateContext;
    missing CertAddCertificateContextToStore;
    missing CertGetCertificateChain;
    missing CertFreeCertificateChain;
}
