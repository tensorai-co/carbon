//! ncrypt.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "NCRYPT.dll";
    missing NCryptCreatePersistedKey;
    missing NCryptDeleteKey;
    missing NCryptExportKey;
    missing NCryptFinalizeKey;
    missing NCryptFreeObject;
    missing NCryptImportKey;
    missing NCryptOpenStorageProvider;
    missing NCryptSetProperty;
    // Not written yet: what larger programs import.
    missing NCryptGetProperty;
    missing NCryptIsAlgSupported;
    missing NCryptOpenKey;
    missing NCryptSignHash;
}
