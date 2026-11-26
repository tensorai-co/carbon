//! ncrypt.dll: CNG Key Storage Provider API. No hardware/TPM key storage provider is present, so
//! `NCryptOpenStorageProvider` returns `NTE_NOT_SUPPORTED` and `NCryptFreeObject` succeeds.

use kernels_core::thread::Cx;

use super::bcrypt::{
    b_crypt_close_algorithm_provider, b_crypt_gen_random, b_crypt_open_algorithm_provider, not_supported as bcrypt_not_supported,
    ok as bcrypt_ok,
};
use super::{Reply, dll};
use crate::process::Task;

dll! {
    "NCRYPT.dll";
    host NCryptCreatePersistedKey(6) => ncrypt_not_supported;
    missing NCryptDeleteKey;
    host NCryptExportKey(8) => ncrypt_not_supported;
    host NCryptFinalizeKey(2) => ncrypt_not_supported;
    host NCryptFreeObject(1) => ncrypt_free_object;
    missing NCryptImportKey;
    host NCryptOpenStorageProvider(3) => ncrypt_open_storage_provider;
    missing NCryptSetProperty;
    // Not written yet: what larger programs import.
    missing NCryptGetProperty;
    missing NCryptIsAlgSupported;
    host NCryptOpenKey(5) => ncrypt_not_supported;
    host NCryptSignHash(8) => ncrypt_not_supported;
    // CNG BCrypt forwarders exported by ncrypt.dll on Windows.
    host BCryptCloseAlgorithmProvider(2) => b_crypt_close_algorithm_provider;
    host BCryptCreateHash(7) => bcrypt_not_supported;
    host BCryptDestroyHash(1) => bcrypt_ok;
    host BCryptFinishHash(4) => bcrypt_not_supported;
    host BCryptGenRandom(4) => b_crypt_gen_random;
    host BCryptHashData(4) => bcrypt_not_supported;
    host BCryptOpenAlgorithmProvider(4) => b_crypt_open_algorithm_provider;
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

fn ncrypt_not_supported(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(NTE_NOT_SUPPORTED)
}
