//! hid.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "HID.dll";
    missing HidD_FreePreparsedData;
    missing HidD_GetAttributes;
    missing HidD_GetHidGuid;
    missing HidD_GetPreparsedData;
    missing HidP_GetCaps;
    // Not written yet: what larger programs import.
    missing HidD_GetProductString;
    missing HidD_GetSerialNumberString;
    missing HidP_GetButtonCaps;
    missing HidP_GetScaledUsageValue;
    missing HidP_GetUsageValue;
    missing HidP_GetUsagesEx;
    missing HidP_GetValueCaps;
}
