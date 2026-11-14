//! setupapi.dll: the machine's devices, of which there are none that Plug and Play would list: a set of them is made
//! and enumerated empty, and opening one by name finds nothing. What reads a device's details can't be reached
//! without one, and says it isn't written yet.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "SETUPAPI.dll";
    missing CM_Get_DevNode_Status;
    missing CM_Locate_DevNodeA;
    missing SetupCopyOEMInfW;
    missing SetupDiBuildDriverInfoList;
    missing SetupDiCallClassInstaller;
    host SetupDiClassGuidsFromNameA(4) => class_guids_from_name_a;
    host SetupDiCreateDeviceInfoList(2) => new_set;
    missing SetupDiCreateDeviceInfoW;
    host SetupDiDestroyDeviceInfoList(1) => destroy_set;
    missing SetupDiDestroyDriverInfoList;
    host SetupDiEnumDeviceInfo(3) => no_more;
    host SetupDiEnumDeviceInterfaces(5) => no_more;
    missing SetupDiEnumDriverInfoW;
    host SetupDiGetClassDevsA(4) => new_set;
    host SetupDiGetClassDevsW(4) => new_set;
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
    host SetupDiOpenDeviceInfoW(5) => no_device;
    host SetupDiOpenDeviceInterfaceW(4) => no_interface;
}

/// ERROR_INVALID_HANDLE, ERROR_NO_MORE_ITEMS, ERROR_NO_SUCH_DEVINST and ERROR_NO_SUCH_DEVICE_INTERFACE.
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_NO_SUCH_DEVINST: u32 = 0xe000_020b;
const ERROR_NO_SUCH_DEVICE_INTERFACE: u32 = 0xe000_0216;

/// `SetupDiGetClassDevs(&class, enumerator, window, flags)` and `SetupDiCreateDeviceInfoList(&class, window)`: a new,
/// empty set, as setupapi's own memory (an HDEVINFO isn't a kernel handle).
fn new_set(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    let set = super::kernel32::global_new(task, cx, &[0; 16]);
    if set == 0 {
        task.set_last_error(cx, 8); // ERROR_NOT_ENOUGH_MEMORY
        return Reply::Value(u64::MAX); // INVALID_HANDLE_VALUE
    }
    Reply::Value(set)
}

/// Null, or INVALID_HANDLE_VALUE as either width passes it.
fn not_a_set(set: u64) -> bool {
    set == 0 || set as u32 == u32::MAX
}

/// `SetupDiDestroyDeviceInfoList(set)`.
fn destroy_set(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if not_a_set(a[0]) {
        return task.fail(cx, ERROR_INVALID_HANDLE);
    }
    Reply::Value(1)
}

/// `SetupDiEnumDeviceInfo(set, index, &data)` and `SetupDiEnumDeviceInterfaces`: the set has nothing in it.
fn no_more(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if not_a_set(a[0]) {
        return task.fail(cx, ERROR_INVALID_HANDLE);
    }
    task.fail(cx, ERROR_NO_MORE_ITEMS)
}

/// `SetupDiOpenDeviceInfoW(set, instance ID, window, flags, &data)`: no device has that ID.
fn no_device(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    task.fail(cx, ERROR_NO_SUCH_DEVINST)
}

/// `SetupDiOpenDeviceInterfaceW(set, path, flags, &data)`: no interface has that path.
fn no_interface(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    task.fail(cx, ERROR_NO_SUCH_DEVICE_INTERFACE)
}

/// Windows' device setup classes, by name, and their GUIDs (`devguid.h`).
const CLASSES: &[(&str, &str)] = &[
    ("Bluetooth", "e0cbf06c-cd8b-4647-bb8a-263b43f0f974"),
    ("CDROM", "4d36e965-e325-11ce-bfc1-08002be10318"),
    ("DiskDrive", "4d36e967-e325-11ce-bfc1-08002be10318"),
    ("Display", "4d36e968-e325-11ce-bfc1-08002be10318"),
    ("HIDClass", "745a17a0-74d3-11d0-b6fe-00a0c90f57da"),
    ("Image", "6bdd1fc6-810f-11d0-bec7-08002be2092f"),
    ("Keyboard", "4d36e96b-e325-11ce-bfc1-08002be10318"),
    ("Media", "4d36e96c-e325-11ce-bfc1-08002be10318"),
    ("Monitor", "4d36e96e-e325-11ce-bfc1-08002be10318"),
    ("Mouse", "4d36e96f-e325-11ce-bfc1-08002be10318"),
    ("Net", "4d36e972-e325-11ce-bfc1-08002be10318"),
    ("Ports", "4d36e978-e325-11ce-bfc1-08002be10318"),
    ("Printer", "4d36e979-e325-11ce-bfc1-08002be10318"),
    ("SoftwareDevice", "62f9c741-b25a-46ce-b54c-9bccce08b6f2"),
    ("System", "4d36e97d-e325-11ce-bfc1-08002be10318"),
    ("USB", "36fc9e60-c465-11cf-8056-444553540000"),
    ("XnaComposite", "d61ca365-5af4-4486-998b-9db4734c6ca3"),
];

/// A GUID's text (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`) as its 16 bytes in memory.
fn guid_bytes(text: &str) -> [u8; 16] {
    let hex: Vec<u8> = text.split('-').flat_map(|part| (0..part.len()).step_by(2).map(move |i| u8::from_str_radix(&part[i..i + 2], 16).unwrap_or(0))).collect();
    let mut g = [0u8; 16];
    // The first three fields are little-endian numbers; the last eight bytes are as written.
    for (i, at) in [3, 2, 1, 0, 5, 4, 7, 6].into_iter().enumerate() {
        g[i] = hex[at];
    }
    g[8..].copy_from_slice(&hex[8..16]);
    g
}

/// `SetupDiClassGuidsFromNameA(name, &guids, room, &needed)`: the class's GUID (one, for the classes Windows defines;
/// none for a name it doesn't), as many as fit, and how many there are.
fn class_guids_from_name_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    if a[0] == 0 || a[3] == 0 {
        return task.fail(cx, 87);
    }
    let name = task.text_arg(cx, a[0], false);
    let found: Vec<[u8; 16]> = CLASSES.iter().filter(|(n, _)| n.eq_ignore_ascii_case(&name)).map(|(_, g)| guid_bytes(g)).collect();
    let _ = task.mem(cx, true, |t| kernels_core::user::write_u32(&mut t.space(), a[3], found.len() as u32));
    if (a[2] as u32 as usize) < found.len() {
        return task.fail(cx, ERROR_INSUFFICIENT_BUFFER);
    }
    let bytes: Vec<u8> = found.concat();
    if !bytes.is_empty() && task.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), a[1], &bytes)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn guids_as_memory_has_them() {
        assert_eq!(super::guid_bytes("745a17a0-74d3-11d0-b6fe-00a0c90f57da"), [0xa0, 0x17, 0x5a, 0x74, 0xd3, 0x74, 0xd0, 0x11, 0xb6, 0xfe, 0x00, 0xa0, 0xc9, 0x0f, 0x57, 0xda]);
    }
}
