//! ncrypt.dll: CNG Key Storage Provider API. No hardware/TPM key storage provider is present, so
//! `NCryptOpenStorageProvider` returns `NTE_NOT_SUPPORTED` and `NCryptFreeObject` succeeds.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "NCRYPT.dll";
    missing NCryptCreatePersistedKey;
    missing NCryptDeleteKey;
    missing NCryptExportKey;
    missing NCryptFinalizeKey;
    host NCryptFreeObject(1) => ncrypt_free_object;
    missing NCryptImportKey;
    host NCryptOpenStorageProvider(3) => ncrypt_open_storage_provider;
    missing NCryptSetProperty;
    // Not written yet: what larger programs import.
    missing NCryptGetProperty;
    missing NCryptIsAlgSupported;
    missing NCryptOpenKey;
    missing NCryptSignHash;
}

const NTE_NOT_SUPPORTED: u64 = 0x8009_0029;

fn ncrypt_open_storage_provider(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] != 0 {
        let _ = task.write_ptr(cx, a[0], 0);
    }
    Reply::Value(NTE_NOT_SUPPORTED)
}

fn ncrypt_free_object(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}
