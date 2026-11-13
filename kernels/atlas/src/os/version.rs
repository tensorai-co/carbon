//! version.dll: files' version information, read from their resources (`os/resapi.rs`).

use super::dll;
use super::resapi::*;

dll! {
    "VERSION.dll";
    host GetFileVersionInfoSizeW(2) => get_file_version_info_size_w;
    host GetFileVersionInfoSizeExW(3) => get_file_version_info_size_ex_w;
    host GetFileVersionInfoW(4) => get_file_version_info_w;
    host GetFileVersionInfoExW(5) => get_file_version_info_ex_w;
    host VerQueryValueW(4) => ver_query_value_w;
    host GetFileVersionInfoSizeA(2) => get_file_version_info_size_a;
    host GetFileVersionInfoSizeExA(3) => get_file_version_info_size_ex_a;
    host GetFileVersionInfoA(4) => get_file_version_info_a;
    host GetFileVersionInfoExA(5) => get_file_version_info_ex_a;
    host VerQueryValueA(4) => ver_query_value_a;
}
