//! userenv.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "USERENV.dll";
    missing CreateAppContainerProfile;
    missing CreateEnvironmentBlock;
    missing DeriveAppContainerSidFromAppContainerName;
    missing DestroyEnvironmentBlock;
    missing EnterCriticalPolicySection;
    missing LeaveCriticalPolicySection;
    missing RegisterGPNotification;
    missing UnregisterGPNotification;
}
