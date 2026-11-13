//! winhttp.dll: what programs ask it about proxies: there's no proxy, and none to discover (so they connect directly,
//! as they do when it isn't there at all). Its requests aren't written yet.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "WINHTTP.dll";
    host WinHttpCloseHandle(1) => win_http_close_handle;
    host WinHttpGetProxyForUrl(4) => win_http_get_proxy_for_url;
    host WinHttpOpen(5) => win_http_open;
    host WinHttpGetIEProxyConfigForCurrentUser(1) => win_http_get_ie_proxy_config_for_current_user;
    // Not written yet: what larger programs import.
    missing WinHttpAddRequestHeaders;
    missing WinHttpConnect;
    missing WinHttpCrackUrl;
    missing WinHttpCreateProxyResolver;
    missing WinHttpFreeProxyResult;
    missing WinHttpGetProxyForUrlEx;
    missing WinHttpGetProxyResult;
    missing WinHttpOpenRequest;
    missing WinHttpQueryHeaders;
    missing WinHttpReadData;
    missing WinHttpReceiveResponse;
    missing WinHttpSendRequest;
    missing WinHttpSetStatusCallback;
    missing WinHttpSetTimeouts;
    missing WinHttpWriteData;
}

const SESSION: u64 = 0x7d00;
const ERROR_WINHTTP_AUTODETECTION_FAILED: u32 = 12180;

/// `WinHttpOpen(agent, access type, proxy, bypass, flags)`: a session's handle.
fn win_http_open(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(SESSION)
}

fn win_http_close_handle(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[0] != 0))
}

/// `WinHttpGetProxyForUrl(session, url, &options, &info)`: nothing found by auto-detection or a script.
fn win_http_get_proxy_for_url(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    task.set_last_error(cx, ERROR_WINHTTP_AUTODETECTION_FAILED);
    Reply::Value(0)
}

/// `WinHttpGetIEProxyConfigForCurrentUser(&config)`: no auto-detection, script, proxy or bypass list.
fn win_http_get_ie_proxy_config_for_current_user(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.abi().width as usize;
    let zero = vec![0u8; w * 4];
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &zero)).is_ok()))
}
