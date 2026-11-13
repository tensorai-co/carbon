//! uiautomationcore.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "UIAUTOMATIONCORE.dll";
    missing UiaGetReservedMixedAttributeValue;
    missing UiaGetReservedNotSupportedValue;
    missing UiaHostProviderFromHwnd;
    missing UiaRaiseAutomationEvent;
    missing UiaRaiseAutomationPropertyChangedEvent;
    missing UiaRaiseNotificationEvent;
    missing UiaRaiseStructureChangedEvent;
    missing UiaReturnRawElementProvider;
}
