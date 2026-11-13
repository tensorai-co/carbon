//! netapi32.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "NETAPI32.dll";
    missing DsRoleFreeMemory;
    missing DsRoleGetPrimaryDomainInformation;
    missing NetApiBufferFree;
    missing NetFreeAadJoinInformation;
    missing NetGetAadJoinInformation;
    missing NetGetJoinInformation;
    missing NetUserGetInfo;
}
