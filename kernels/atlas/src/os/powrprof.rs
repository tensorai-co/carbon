//! powrprof.dll: a desktop on mains power, as the machine is: no battery, the balanced scheme, and settings that
//! aren't set (so programs use their defaults).

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "POWRPROF.dll";
    host CallNtPowerInformation(5) => call_nt_power_information;
    host PowerDeterminePlatformRoleEx(1) => power_determine_platform_role;
    host PowerDeterminePlatformRole(0) => power_determine_platform_role;
    host PowerGetActiveScheme(2) => power_get_active_scheme;
    host PowerReadACValue(7) => power_read_value;
    host PowerReadDCValue(7) => power_read_value;
}

const STATUS_SUCCESS: u64 = 0;
const STATUS_BUFFER_TOO_SMALL: u64 = 0xc000_0023;
const STATUS_INVALID_PARAMETER: u64 = 0xc000_000d;
const STATUS_ACCESS_DENIED: u64 = 0xc000_0022;

/// `CallNtPowerInformation(level, in, in_size, out, out_size)`: SystemPowerCapabilities (4) and SystemBatteryState
/// (5), as a desktop on mains power has them; ProcessorInformation (11), each processor at 2 GHz. Others are refused,
/// as setting anything is.
fn call_nt_power_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let data: Vec<u8> = match a[0] as u32 {
        // SYSTEM_POWER_CAPABILITIES: 76 bytes, its first booleans (power button, sleep button, lid, S1-S5 ...) clear
        // but the power button and S5 (soft off); AcOnLine is SYSTEM_BATTERY_STATE's.
        4 => {
            let mut b = vec![0u8; 76];
            b[0] = 1;
            b[9] = 1;
            b
        }
        // SYSTEM_BATTERY_STATE: on AC, no battery.
        5 => {
            let mut b = vec![0u8; 32];
            b[0] = 1;
            b
        }
        // PROCESSOR_POWER_INFORMATION for each processor: number, max and current MHz, limit, max idle, current idle.
        11 => {
            let cpus = task.k.host.cpus().max(1) as u32;
            (0..cpus).flat_map(|n| [n, 2000, 2000, 2000, 0, 0]).flat_map(u32::to_le_bytes).collect()
        }
        _ => return Reply::Value(if a[1] != 0 { STATUS_ACCESS_DENIED } else { STATUS_INVALID_PARAMETER }),
    };
    if (a[4] as u32 as usize) < data.len() || a[3] == 0 {
        return Reply::Value(STATUS_BUFFER_TOO_SMALL);
    }
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &data)).is_err() {
        return Reply::Value(STATUS_INVALID_PARAMETER);
    }
    Reply::Value(STATUS_SUCCESS)
}

/// `PowerDeterminePlatformRoleEx(version)`: PlatformRoleDesktop.
fn power_determine_platform_role(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

/// `PowerGetActiveScheme(root, &scheme)`: the balanced scheme's GUID, from `LocalAlloc`.
fn power_get_active_scheme(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let guid = super::com::guid(0x381b_4222, 0xf694, 0x41f0, [0x96, 0x85, 0xff, 0x5b, 0xb2, 0x60, 0xdf, 0x2e]);
    let block = super::kernel32::global_new(task, cx, &guid);
    if block == 0 || a[1] == 0 {
        return Reply::Value(8);
    }
    let _ = task.write_ptr(cx, a[1], block);
    Reply::Value(0)
}

/// `PowerReadACValue(root, scheme, subgroup, setting, &type, buffer, &size)` and `DC`: ERROR_FILE_NOT_FOUND, as for a
/// setting the scheme hasn't, which leaves the program its default.
fn power_read_value(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(2)
}
