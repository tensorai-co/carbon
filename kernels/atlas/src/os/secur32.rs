//! secur32.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "SECUR32.dll";
    missing AcceptSecurityContext;
    missing AcquireCredentialsHandleA;
    missing ApplyControlToken;
    missing CompleteAuthToken;
    missing DecryptMessage;
    missing DeleteSecurityContext;
    missing EncryptMessage;
    missing FreeContextBuffer;
    missing FreeCredentialsHandle;
    missing InitializeSecurityContextA;
    missing QueryContextAttributesA;
    missing SetContextAttributesA;
    // Not written yet: what larger programs import.
    missing AcquireCredentialsHandleW;
    missing GetUserNameExW;
    missing InitializeSecurityContextW;
    missing LsaConnectUntrusted;
    missing LsaDeregisterLogonProcess;
    missing LsaFreeReturnBuffer;
    missing LsaLogonUser;
    missing QueryContextAttributesW;
    missing QuerySecurityPackageInfoW;
}
