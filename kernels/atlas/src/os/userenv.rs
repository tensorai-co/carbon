//! userenv.dll: Group Policy and AppContainer helpers.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "USERENV.dll";
    host CreateAppContainerProfile(6) => e_notimpl;
    host CreateEnvironmentBlock(3) => zero;
    host DeriveAppContainerSidFromAppContainerName(2) => e_notimpl;
    host DestroyEnvironmentBlock(1) => one;
    host EnterCriticalPolicySection(1) => one;
    host LeaveCriticalPolicySection(1) => one;
    host RegisterGPNotification(2) => one;
    host UnregisterGPNotification(1) => one;
}

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn one(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

fn e_notimpl(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0x8000_4001)
}
