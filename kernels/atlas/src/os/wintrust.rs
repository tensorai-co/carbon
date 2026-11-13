//! wintrust.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "WINTRUST.dll";
    missing CryptCATAdminAcquireContext;
    missing CryptCATAdminCalcHashFromFileHandle;
    missing CryptCATAdminEnumCatalogFromHash;
    missing CryptCATAdminReleaseCatalogContext;
    missing CryptCATAdminReleaseContext;
    missing CryptCATCatalogInfoFromContext;
    missing WTHelperGetProvCertFromChain;
    missing WTHelperGetProvSignerFromChain;
    missing WTHelperProvDataFromStateData;
    missing WinVerifyTrust;
}
