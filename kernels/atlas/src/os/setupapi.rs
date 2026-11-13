//! setupapi.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "SETUPAPI.dll";
    missing CM_Get_DevNode_Status;
    missing CM_Locate_DevNodeA;
    missing SetupCopyOEMInfW;
    missing SetupDiBuildDriverInfoList;
    missing SetupDiCallClassInstaller;
    missing SetupDiClassGuidsFromNameA;
    missing SetupDiCreateDeviceInfoList;
    missing SetupDiCreateDeviceInfoW;
    missing SetupDiDestroyDeviceInfoList;
    missing SetupDiDestroyDriverInfoList;
    missing SetupDiEnumDeviceInfo;
    missing SetupDiEnumDeviceInterfaces;
    missing SetupDiEnumDriverInfoW;
    missing SetupDiGetClassDevsA;
    missing SetupDiGetClassDevsW;
    missing SetupDiGetDeviceInstanceIdA;
    missing SetupDiGetDeviceInterfaceDetailA;
    missing SetupDiGetDeviceInterfaceDetailW;
    missing SetupDiGetDevicePropertyW;
    missing SetupDiGetDeviceRegistryPropertyA;
    missing SetupDiGetDeviceRegistryPropertyW;
    missing SetupDiGetDriverInfoDetailW;
    missing SetupDiOpenClassRegKey;
    missing SetupDiOpenDevRegKey;
    missing SetupDiSetClassInstallParamsW;
    missing SetupDiSetDeviceRegistryPropertyW;
    // Not written yet: what larger programs import.
    missing SetupDiOpenDeviceInfoW;
    missing SetupDiOpenDeviceInterfaceW;
}
