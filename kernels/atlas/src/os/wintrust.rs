//! wintrust.dll: Authenticode trust verification.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

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
    host WinVerifyTrust(3) => win_verify_trust;
}

fn win_verify_trust(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}
