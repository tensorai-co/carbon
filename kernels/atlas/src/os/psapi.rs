//! psapi.dll: a process's modules and memory. On Windows these forward to kernel32's `K32` functions; here they're
//! the same host Rust (`procapi`).

use super::dll;
use super::procapi::*;

dll! {
    "PSAPI.DLL";
    host GetModuleFileNameExW(4) => get_module_file_name_ex_w;
    host GetModuleFileNameExA(4) => get_module_file_name_ex_a;
    host GetModuleInformation(4) => get_module_information;
    host GetProcessMemoryInfo(3) => get_process_memory_info;
}
