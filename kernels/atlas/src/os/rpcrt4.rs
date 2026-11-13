//! rpcrt4.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "RPCRT4.dll";
    missing UuidCreate;
    // Not written yet: what larger programs import.
    missing GetPriorityClass;
    missing RegCloseKey;
    missing RegOpenKeyExA;
    missing RegOpenKeyExW;
    missing RegQueryValueExA;
    missing RegQueryValueExW;
    missing __CxxFrameHandler3;
    missing _o___std_type_info_destroy_list;
    missing _o___stdio_common_vswprintf_s;
    missing _o__aligned_free;
    missing _o__aligned_malloc;
    missing _o__cexit;
    missing _o__configure_narrow_argv;
    missing _o__execute_onexit_table;
    missing _o__initialize_narrow_environment;
    missing _o__initialize_onexit_table;
    missing _o__purecall;
    missing _o__seh_filter_dll;
    missing _o__strlwr_s;
    missing _o__wcsicmp;
    missing _o__wcsnicmp;
    missing _o__wsplitpath_s;
    missing _o_free;
    missing _o_malloc;
    missing _o_rand;
    missing _o_srand;
    missing _o_towlower;
}
