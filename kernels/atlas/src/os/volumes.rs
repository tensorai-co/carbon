//! kernel32's drives, volumes and power: the machine has one drive, C:, an NTFS volume, on mains power.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::process::Task;

const TRUE: u64 = 1;
/// C:'s volume, as `GetVolumeNameForVolumeMountPointW` names it.
const VOLUME: &str = "\\\\?\\Volume{3b4e1f2a-0c5d-4e6f-8a9b-0c1d2e3f4a5b}\\";

/// `GetLogicalDrives()`: C:.
pub fn get_logical_drives(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1 << 2)
}

/// Strings, each with its NUL, then one more NUL: what `GetLogicalDriveStrings` and the volume path lists give. Its
/// length without the last NUL, or the room it needs with it.
fn put_list(task: &mut Task, cx: &mut Cx<'_>, out: u64, room: u64, items: &[&str], wide: bool) -> u64 {
    let mut text = String::new();
    for i in items {
        text.push_str(i);
        text.push('\0');
    }
    text.push('\0');
    let units = text.chars().count() as u64;
    if room < units {
        return units;
    }
    let bytes: Vec<u8> = if wide { text.encode_utf16().flat_map(u16::to_le_bytes).collect() } else { text.into_bytes() };
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes));
    units - 1
}

/// `GetLogicalDriveStringsW(room, out)` and `A`: "C:\".
pub fn get_logical_drive_strings_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(put_list(task, cx, a[1], a[0] as u32 as u64, &["C:\\"], true))
}

pub fn get_logical_drive_strings_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(put_list(task, cx, a[1], a[0] as u32 as u64, &["C:\\"], false))
}

/// `GetVolumeInformationW(root, name, name room, &serial, &longest name, &flags, file system, its room)` and `A`:
/// C:'s, NTFS with its usual flags, unnamed.
fn get_volume_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    const NTFS_FLAGS: u32 = 0x03e7_00ff;
    if a[0] != 0 {
        let root = task.text_arg(cx, a[0], wide);
        if !root.to_ascii_uppercase().starts_with("C:") && !root.starts_with("\\\\?\\Volume") {
            return task.fail(cx, 3);
        }
    }
    let put = |task: &mut Task, cx: &mut Cx<'_>, at: u64, room: u64, s: &str| {
        if at != 0 && room > s.len() as u64 {
            let bytes: Vec<u8> = if wide { s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() } else { s.bytes().chain([0]).collect() };
            let _ = task.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes));
        }
    };
    put(task, cx, a[1], a[2] as u32 as u64, "");
    for (at, v) in [(a[3], 0x5a3c_17e2u32), (a[4], 255), (a[5], NTFS_FLAGS)] {
        if at != 0 {
            let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), at, v));
        }
    }
    put(task, cx, a[6], a[7] as u32 as u64, "NTFS");
    Reply::Value(TRUE)
}

pub fn get_volume_information_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_volume_information(task, cx, a, true)
}

pub fn get_volume_information_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_volume_information(task, cx, a, false)
}

/// A string written as the volume calls write theirs: TRUE if it fit, else FALSE with ERROR_FILENAME_EXCED_RANGE.
fn put_string(task: &mut Task, cx: &mut Cx<'_>, out: u64, room: u64, s: &str) -> Reply {
    let units: Vec<u16> = s.encode_utf16().chain([0]).collect();
    if room < units.len() as u64 {
        return task.fail(cx, 206);
    }
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    if task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(TRUE)
}

/// `GetVolumePathNameW(path, out, room)`: every path is on C:.
pub fn get_volume_path_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    put_string(task, cx, a[1], a[2] as u32 as u64, "C:\\")
}

/// `GetVolumeNameForVolumeMountPointW(mount point, out, room)`: C:'s volume.
pub fn get_volume_name_for_volume_mount_point_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if !task.text_arg(cx, a[0], true).to_ascii_uppercase().starts_with("C:") {
        return task.fail(cx, 87);
    }
    put_string(task, cx, a[1], a[2] as u32 as u64, VOLUME)
}

/// `GetVolumePathNamesForVolumeNameW(volume, out, room, &needed)`: where C:'s volume is mounted, C:\.
pub fn get_volume_path_names_for_volume_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const NEEDED: u32 = 5;
    if a[3] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[3], NEEDED));
    }
    if (a[2] as u32) < NEEDED {
        return task.fail(cx, 234);
    }
    put_list(task, cx, a[1], a[2] as u32 as u64, &["C:\\"], true);
    Reply::Value(TRUE)
}

/// `GetSystemPowerStatus(&status)`: on mains power, with no battery.
pub fn get_system_power_status(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let status = [1u8, 128, 255, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &status)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(TRUE)
}

/// `QueryDosDeviceW(name, out, room)`: what a DOS device name is: C: is the volume's device; with no name, every
/// name there is (C:). Its length with the list's last NUL.
pub fn query_dos_device_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let items = if a[0] == 0 {
        vec!["C:"]
    } else if task.text_arg(cx, a[0], true).eq_ignore_ascii_case("C:") {
        vec!["\\Device\\HarddiskVolume3"]
    } else {
        return task.fail(cx, 2);
    };
    let n = put_list(task, cx, a[1], a[2] as u32 as u64, &items, true);
    if n >= a[2] as u32 as u64 { task.fail(cx, 122) } else { Reply::Value(n + 1) }
}
