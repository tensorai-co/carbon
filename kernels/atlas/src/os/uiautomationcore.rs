//! uiautomationcore.dll: UI Automation providers. No accessibility clients are attached, so
//! `UiaClientsAreListening` returns FALSE and provider calls succeed as no-ops.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "UIAUTOMATIONCORE.dll";
    host UiaClientsAreListening(0) => zero;
    host UiaGetReservedMixedAttributeValue(1) => e_notimpl;
    host UiaGetReservedNotSupportedValue(1) => e_notimpl;
    host UiaHostProviderFromHwnd(2) => e_notimpl;
    host UiaRaiseAutomationEvent(2) => zero;
    host UiaRaiseAutomationPropertyChangedEvent(4) => zero;
    host UiaRaiseNotificationEvent(5) => zero;
    host UiaRaiseStructureChangedEvent(4) => zero;
    host UiaReturnRawElementProvider(4) => zero;
}

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn e_notimpl(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0x8000_4001)
}
