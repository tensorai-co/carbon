//! cfgmgr32.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "CFGMGR32.dll";
    missing CM_Get_Device_IDA;
    missing CM_Get_Parent;
    missing CM_Locate_DevNodeA;
    // Not written yet: what larger programs import.
    missing CM_Get_Child;
    missing CM_Get_DevNode_PropertyW;
    missing CM_Get_DevNode_Registry_PropertyW;
    missing CM_Get_DevNode_Status;
    missing CM_Get_Device_IDW;
    missing CM_Get_Device_ID_ListW;
    missing CM_Get_Device_ID_List_SizeW;
    missing CM_Get_Sibling;
    missing CM_Locate_DevNodeW;
    missing CM_Open_DevNode_Key;
}
