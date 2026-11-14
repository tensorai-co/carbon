//! kernel32.dll: processes, the console, files, memory and time. Heaps, locks and the last error are guest code, as
//! on Windows; the rest is host Rust here.


use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::shell32::command_line_to_argv_w;
use super::advapi32::{reg_close_key, reg_create_key_ex_w, reg_delete_key_ex_w, reg_delete_value_w, reg_open_key_ex_a, reg_open_key_ex_w, reg_query_value_ex_a, reg_query_value_ex_w, reg_set_value_ex_w};
use super::fileapi::*;
use super::localeapi::*;
use super::volumes::*;
use super::ioapi::*;
use super::memapi::*;
use super::nlsapi::*;
use super::pipeapi::*;
use super::procapi::*;
use super::resapi::*;
use super::synch::*;
use super::sysapi::*;
use super::{Reply, dll};
use crate::handles::Handle;
use crate::process::Task;

dll! {
    "KERNEL32.dll";
    host GetStdHandle(1) => get_std_handle;
    host WriteFile(5) => write_file;
    host WriteConsoleA(5) => write_console_a;
    host WriteConsoleW(5) => write_console_w;
    host MultiByteToWideChar(6) => multi_byte_to_wide_char;
    host WideCharToMultiByte(8) => wide_char_to_multi_byte;
    host GetACP(0) => get_acp;
    host GetOEMCP(0) => get_oem_cp;
    host IsValidCodePage(1) => is_valid_code_page;
    host GetCPInfo(2) => get_cp_info;
    host GetCPInfoExW(3) => get_cp_info_ex_w;
    host IsDBCSLeadByte(1) => is_dbcs_lead_byte;
    host IsDBCSLeadByteEx(2) => is_dbcs_lead_byte;
    host AreFileApisANSI(0) => are_file_apis_ansi;
    host SetFileApisToANSI(0) => set_file_apis;
    host SetFileApisToOEM(0) => set_file_apis;
    host GetConsoleCP(0) => get_console_cp;
    host GetConsoleOutputCP(0) => get_console_cp;
    host SetConsoleCP(1) => set_console_cp;
    host SetConsoleOutputCP(1) => set_console_cp;
    host ReadFile(5) => read_file;
    host ReadFileEx(5) => read_file_ex;
    host WriteFileEx(5) => write_file_ex;
    host CreateIoCompletionPort(4) => create_io_completion_port;
    host PostQueuedCompletionStatus(4) => post_queued_completion_status;
    host GetQueuedCompletionStatus(5) => get_queued_completion_status;
    host GetQueuedCompletionStatusEx(6) => get_queued_completion_status_ex;
    host GetOverlappedResult(4) => get_overlapped_result;
    host GetOverlappedResultEx(5) => get_overlapped_result_ex;
    host CancelIo(1) => cancel_io;
    host CancelIoEx(2) => cancel_io_ex;
    host CancelSynchronousIo(1) => cancel_synchronous_io;
    host CreateNamedPipeW(8) => create_named_pipe_w;
    host CreateNamedPipeA(8) => create_named_pipe_a;
    host ConnectNamedPipe(2) => connect_named_pipe;
    host DisconnectNamedPipe(1) => disconnect_named_pipe;
    host WaitNamedPipeW(2) => wait_named_pipe_w;
    host WaitNamedPipeA(2) => wait_named_pipe_a;
    host SetNamedPipeHandleState(4) => set_named_pipe_handle_state;
    host GetNamedPipeInfo(5) => get_named_pipe_info;
    host GetNamedPipeClientProcessId(2) => get_named_pipe_client_process_id;
    host GetNamedPipeServerProcessId(2) => get_named_pipe_server_process_id;
    host TransactNamedPipe(7) => transact_named_pipe;
    host CallNamedPipeW(7) => call_named_pipe_w;
    host CallNamedPipeA(7) => call_named_pipe_a;
    host SetFileCompletionNotificationModes(2) => set_file_completion_notification_modes;
    host CreateFileW(7) => create_file_w;
    host CreateFileA(7) => create_file_a;
    host SetFilePointer(4) => set_file_pointer;
    host SetFilePointerEx(4) => set_file_pointer_ex;
    host GetFileSize(2) => get_file_size;
    host GetFileSizeEx(2) => get_file_size_ex;
    host GetFileType(1) => get_file_type;
    host SetEndOfFile(1) => set_end_of_file;
    host FlushFileBuffers(1) => flush_file_buffers;
    host GetFileAttributesW(1) => get_file_attributes_w;
    host GetFileAttributesA(1) => get_file_attributes_a;
    host GetFileAttributesExW(3) => get_file_attributes_ex_w;
    host GetFileAttributesExA(3) => get_file_attributes_ex_a;
    host SetFileAttributesW(2) => set_file_attributes_w;
    host SetFileAttributesA(2) => set_file_attributes_a;
    host GetFileInformationByHandle(2) => get_file_information_by_handle;
    host DeleteFileW(1) => delete_file_w;
    host DeleteFileA(1) => delete_file_a;
    host CreateDirectoryW(2) => create_directory_w;
    host CreateDirectoryA(2) => create_directory_a;
    host RemoveDirectoryW(1) => remove_directory_w;
    host RemoveDirectoryA(1) => remove_directory_a;
    host MoveFileExW(3) => move_file_ex_w;
    host MoveFileExA(3) => move_file_ex_a;
    host MoveFileW(2) => move_file_w;
    host MoveFileA(2) => move_file_a;
    host CopyFileW(3) => copy_file_w;
    host CopyFileA(3) => copy_file_a;
    host CopyFileExW(6) => copy_file_ex_w;
    host DeleteFileTransactedW(2) => delete_file_transacted_w;
    host MoveFileTransactedW(6) => move_file_transacted_w;
    host CopyFileExA(6) => copy_file_ex_a;
    host ReplaceFileW(6) => replace_file_w;
    host ReplaceFileA(6) => replace_file_a;
    host CreateHardLinkW(3) => create_hard_link_w;
    host CreateHardLinkA(3) => create_hard_link_a;
    host CreateSymbolicLinkW(3) => create_symbolic_link_w;
    host CreateSymbolicLinkA(3) => create_symbolic_link_a;
    host GetFileTime(4) => get_file_time;
    host SetFileTime(4) => set_file_time;
    host FindFirstFileW(2) => find_first_file_w;
    host FindFirstFileA(2) => find_first_file_a;
    host FindNextFileW(2) => find_next_file_w;
    host FindNextFileA(2) => find_next_file_a;
    host FindClose(1) => find_close;
    host GetCurrentDirectoryW(2) => get_current_directory_w;
    host GetCurrentDirectoryA(2) => get_current_directory_a;
    host SetCurrentDirectoryW(1) => set_current_directory_w;
    host SetCurrentDirectoryA(1) => set_current_directory_a;
    host GetFullPathNameW(4) => get_full_path_name_w;
    host GetFullPathNameA(4) => get_full_path_name_a;
    host GetLongPathNameW(3) => get_long_path_name_w;
    host GetLongPathNameA(3) => get_long_path_name_a;
    host GetTempPathW(2) => get_temp_path_w;
    host GetTempPathA(2) => get_temp_path_a;
    host GetFinalPathNameByHandleW(4) => get_final_path_name_by_handle_w;
    host GetFinalPathNameByHandleA(4) => get_final_path_name_by_handle_a;
    host GetWindowsDirectoryW(2) => get_windows_directory_w;
    host GetWindowsDirectoryA(2) => get_windows_directory_a;
    host GetSystemDirectoryW(2) => get_system_directory_w;
    host GetSystemDirectoryA(2) => get_system_directory_a;
    host GetSystemWow64DirectoryW(2) => get_system_wow64_directory_w;
    host GetSystemWow64DirectoryA(2) => get_system_wow64_directory_a;
    host Wow64DisableWow64FsRedirection(1) => wow64_disable_wow64_fs_redirection;
    host Wow64RevertWow64FsRedirection(1) => wow64_revert_wow64_fs_redirection;
    host Wow64EnableWow64FsRedirection(1) => wow64_enable_wow64_fs_redirection;
    host CloseHandle(1) => close_handle;
    guest ExitProcess => RtlExitUserProcess;
    host GetCommandLineA(0) => get_command_line_a;
    host GetCommandLineW(0) => get_command_line_w;
    guest GetModuleHandleW => AtlasGetModuleHandleW;
    guest GetModuleHandleA => AtlasGetModuleHandleA;
    guest LoadLibraryA => AtlasLoadLibraryA;
    guest LoadLibraryW => AtlasLoadLibraryW;
    guest LoadLibraryExA => AtlasLoadLibraryExA;
    guest LoadLibraryExW => AtlasLoadLibraryExW;
    guest GetProcAddress => AtlasGetProcAddress;
    guest FreeLibrary => AtlasFreeLibrary;
    guest FreeLibraryAndExitThread => AtlasFreeLibraryAndExitThread;
    guest DisableThreadLibraryCalls => AtlasDisableThreadLibraryCalls;
    guest GetModuleHandleExA => AtlasGetModuleHandleExA;
    guest GetModuleHandleExW => AtlasGetModuleHandleExW;
    guest ResolveDelayLoadedAPI => LdrResolveDelayLoadedAPI;
    guest GetModuleFileNameW => AtlasGetModuleFileNameW;
    guest GetModuleFileNameA => AtlasGetModuleFileNameA;
    guest RaiseException => AtlasRaiseException;
    guest SetUnhandledExceptionFilter => AtlasSetUnhandledExceptionFilter;
    guest AddVectoredExceptionHandler => RtlAddVectoredExceptionHandler;
    guest RemoveVectoredExceptionHandler => RtlRemoveVectoredExceptionHandler;
    guest AddVectoredContinueHandler => RtlAddVectoredContinueHandler;
    guest RemoveVectoredContinueHandler => RtlRemoveVectoredContinueHandler;
    guest RtlCaptureContext;
    guest RtlPcToFileHeader;
    guest RtlCaptureStackBackTrace;
    guest RtlLookupFunctionEntry;
    guest RtlVirtualUnwind;
    guest RtlUnwind;
    guest RtlUnwindEx;
    host VirtualProtect(4) => virtual_protect;
    guest GetCurrentProcessId => AtlasGetCurrentProcessId;
    guest GetCurrentThreadId => AtlasGetCurrentThreadId;
    guest GetCurrentProcess => AtlasGetCurrentProcess;
    guest GetCurrentThread => AtlasGetCurrentThread;
    host Sleep(1) => sleep;
    host SleepEx(2) => sleep_ex;
    host CreateThread(6) => create_thread;
    guest ExitThread => AtlasExitThread;
    host GetExitCodeThread(2) => get_exit_code_thread;
    host GetExitCodeProcess(2) => get_exit_code_process;
    host GetThreadId(1) => get_thread_id;
    host GetProcessId(1) => get_process_id;
    host SuspendThread(1) => suspend_thread;
    host ResumeThread(1) => resume_thread;
    host TerminateThread(2) => terminate_thread;
    host TerminateProcess(2) => terminate_process;
    host CreateProcessW(10) => create_process_w;
    host CreateProcessA(10) => create_process_a;
    host OpenProcess(3) => open_process;
    host GetStartupInfoW(1) => get_startup_info;
    host GetStartupInfoA(1) => get_startup_info;
    host CreatePipe(4) => create_pipe;
    host PeekNamedPipe(6) => peek_named_pipe;
    host GetHandleInformation(2) => get_handle_information;
    host SetHandleInformation(3) => set_handle_information;
    host SwitchToThread(0) => switch_to_thread;
    host CreateEventW(4) => create_event_w;
    host CreateEventA(4) => create_event_a;
    host OpenEventW(3) => open_event_w;
    host OpenEventA(3) => open_event_a;
    host SetEvent(1) => set_event;
    host ResetEvent(1) => reset_event;
    host CreateMutexW(3) => create_mutex_w;
    host CreateMutexA(3) => create_mutex_a;
    host OpenMutexW(3) => open_mutex_w;
    host OpenMutexA(3) => open_mutex_a;
    host ReleaseMutex(1) => release_mutex;
    host CreateSemaphoreW(4) => create_semaphore_w;
    host CreateSemaphoreA(4) => create_semaphore_a;
    host ReleaseSemaphore(3) => release_semaphore;
    host WaitForSingleObject(2) => wait_for_single_object;
    host WaitForSingleObjectEx(3) => wait_for_single_object_ex;
    host WaitForMultipleObjects(4) => wait_for_multiple_objects;
    host WaitForMultipleObjectsEx(5) => wait_for_multiple_objects_ex;
    host QueueUserAPC(3) => queue_user_apc;
    host DuplicateHandle(7) => duplicate_handle;
    guest GetEnvironmentVariableW => AtlasGetEnvironmentVariableW;
    guest GetEnvironmentVariableA => AtlasGetEnvironmentVariableA;
    guest SetEnvironmentVariableW => AtlasSetEnvironmentVariableW;
    guest SetEnvironmentVariableA => AtlasSetEnvironmentVariableA;
    guest ExpandEnvironmentStringsW => AtlasExpandEnvironmentStringsW;
    guest ExpandEnvironmentStringsA => AtlasExpandEnvironmentStringsA;
    guest GetEnvironmentStringsW => AtlasGetEnvironmentStringsW;
    guest GetEnvironmentStrings => AtlasGetEnvironmentStringsA;
    guest GetEnvironmentStringsA => AtlasGetEnvironmentStringsA;
    guest FreeEnvironmentStringsW => AtlasFreeEnvironmentStrings;
    guest FreeEnvironmentStringsA => AtlasFreeEnvironmentStrings;
    guest TlsAlloc => AtlasTlsAlloc;
    guest TlsFree => AtlasTlsFree;
    guest TlsGetValue => AtlasTlsGetValue;
    guest TlsSetValue => AtlasTlsSetValue;
    guest FlsAlloc => AtlasFlsAlloc;
    guest FlsFree => AtlasFlsFree;
    guest FlsGetValue => AtlasFlsGetValue;
    guest FlsSetValue => AtlasFlsSetValue;
    guest InitializeSRWLock => AtlasInitializeSRWLock;
    guest AcquireSRWLockExclusive => AtlasAcquireSRWLockExclusive;
    guest AcquireSRWLockShared => AtlasAcquireSRWLockShared;
    guest ReleaseSRWLockExclusive => AtlasReleaseSRWLockExclusive;
    guest ReleaseSRWLockShared => AtlasReleaseSRWLockShared;
    guest TryAcquireSRWLockExclusive => AtlasTryAcquireSRWLockExclusive;
    guest TryAcquireSRWLockShared => AtlasTryAcquireSRWLockShared;
    guest InitializeConditionVariable => AtlasInitializeConditionVariable;
    guest SleepConditionVariableSRW => AtlasSleepConditionVariableSRW;
    guest SleepConditionVariableCS => AtlasSleepConditionVariableCS;
    guest WakeConditionVariable => AtlasWakeConditionVariable;
    guest WakeAllConditionVariable => AtlasWakeAllConditionVariable;
    guest WaitOnAddress => AtlasWaitOnAddress;
    guest WakeByAddressSingle => AtlasWakeByAddressSingle;
    guest WakeByAddressAll => AtlasWakeByAddressAll;
    guest InitOnceExecuteOnce => AtlasInitOnceExecuteOnce;
    guest TryEnterCriticalSection => RtlTryEnterCriticalSection;
    guest DeleteCriticalSection => RtlDeleteCriticalSection;
    guest InitializeCriticalSectionAndSpinCount => AtlasInitializeCriticalSectionAndSpinCount;
    guest InitializeCriticalSectionEx => AtlasInitializeCriticalSectionEx;
    host GetTickCount64(0) => get_tick_count64;
    host GetTickCount(0) => get_tick_count64;
    host QueryPerformanceCounter(1) => query_performance_counter;
    host QueryPerformanceFrequency(1) => query_performance_frequency;
    host VirtualAlloc(4) => virtual_alloc;
    host VirtualQuery(3) => virtual_query;
    host CreateFileMappingW(6) => create_file_mapping_w;
    host CreateFileMappingA(6) => create_file_mapping_a;
    host OpenFileMappingW(3) => open_file_mapping_w;
    host OpenFileMappingA(3) => open_file_mapping_a;
    host MapViewOfFile(5) => map_view_of_file;
    host MapViewOfFileEx(6) => map_view_of_file_ex;
    host UnmapViewOfFile(1) => unmap_view_of_file;
    host FlushViewOfFile(2) => flush_view_of_file;
    host AtlasQuerySection(2) => atlas_query_section;
    host AtlasQueryProcess(2) => atlas_query_process;
    host AtlasQueryObject(5) => atlas_query_object;
    host AtlasProcessHandles(4) => atlas_process_handles;
    host GetSystemTimeAsFileTime(1) => get_system_time_as_file_time;
    host GetSystemTimePreciseAsFileTime(1) => get_system_time_as_file_time;
    host GetVersion(0) => get_version;
    host GetVersionExW(1) => get_version_ex_w;
    host GetVersionExA(1) => get_version_ex_a;
    host VerifyVersionInfoW(3) => verify_version_info_w;
    host VerifyVersionInfoA(3) => verify_version_info_a;
    host VerSetConditionMask(3) => ver_set_condition_mask;
    host GetProductInfo(5) => get_product_info;
    host GetSystemInfo(1) => get_system_info;
    host GetNativeSystemInfo(1) => get_native_system_info;
    host GlobalMemoryStatusEx(1) => global_memory_status_ex;
    host GlobalMemoryStatus(1) => global_memory_status;
    host IsProcessorFeaturePresent(1) => is_processor_feature_present;
    host IsWow64Process(2) => is_wow64_process;
    host IsWow64Process2(3) => is_wow64_process2;
    host GetSystemTimes(3) => get_system_times;
    host FindResourceW(3) => find_resource_w;
    host FindResourceA(3) => find_resource_a;
    host FindResourceExW(4) => find_resource_ex_w;
    host FindResourceExA(4) => find_resource_ex_a;
    host LoadResource(2) => load_resource;
    host LockResource(1) => lock_resource;
    host FreeResource(1) => free_resource;
    host SizeofResource(2) => sizeof_resource;
    host EnumResourceTypesW(3) => enum_resource_types_w;
    host EnumResourceTypesA(3) => enum_resource_types_a;
    host EnumResourceNamesW(4) => enum_resource_names_w;
    host EnumResourceNamesA(4) => enum_resource_names_a;
    host EnumResourceLanguagesW(5) => enum_resource_languages_w;
    host EnumResourceLanguagesA(5) => enum_resource_languages_a;
    host GetActiveProcessorCount(1) => get_active_processor_count;
    host GetMaximumProcessorCount(1) => get_active_processor_count;
    host GetActiveProcessorGroupCount(0) => get_active_processor_group_count;
    host GetMaximumProcessorGroupCount(0) => get_active_processor_group_count;
    host GetLogicalProcessorInformation(2) => get_logical_processor_information;
    host GetLogicalProcessorInformationEx(3) => get_logical_processor_information_ex;
    host GetComputerNameW(2) => get_computer_name_w;
    host GetComputerNameA(2) => get_computer_name_a;
    host GetComputerNameExW(3) => get_computer_name_ex_w;
    host GetComputerNameExA(3) => get_computer_name_ex_a;
    host GetSystemTime(1) => get_system_time;
    host GetLocalTime(1) => get_system_time;
    host FileTimeToSystemTime(2) => file_time_to_system_time;
    host SystemTimeToFileTime(2) => system_time_to_file_time;
    host FileTimeToLocalFileTime(2) => file_time_to_local_file_time;
    host LocalFileTimeToFileTime(2) => file_time_to_local_file_time;
    host GetTimeZoneInformation(1) => get_time_zone_information;
    host GetDynamicTimeZoneInformation(1) => get_dynamic_time_zone_information;
    host SystemTimeToTzSpecificLocalTime(3) => system_time_to_tz_specific_local_time;
    host TzSpecificLocalTimeToSystemTime(3) => tz_specific_local_time_to_system_time;
    host CreateWaitableTimerW(3) => create_waitable_timer_w;
    host CreateWaitableTimerA(3) => create_waitable_timer_a;
    host CreateWaitableTimerExW(4) => create_waitable_timer_ex_w;
    host CreateWaitableTimerExA(4) => create_waitable_timer_ex_a;
    host OpenWaitableTimerW(3) => open_waitable_timer_w;
    host OpenWaitableTimerA(3) => open_waitable_timer_a;
    host SetWaitableTimer(6) => set_waitable_timer;
    host SetWaitableTimerEx(7) => set_waitable_timer_ex;
    host CancelWaitableTimer(1) => cancel_waitable_timer;
    host VirtualFree(3) => virtual_free;
    host GlobalAlloc(2) => global_alloc;
    host GlobalLock(1) => global_lock;
    host GlobalUnlock(1) => global_unlock;
    host GlobalSize(1) => global_size;
    host GlobalFree(1) => global_free;
    host GlobalHandle(1) => global_lock;
    host LocalAlloc(2) => global_alloc;
    host LocalLock(1) => global_lock;
    host LocalUnlock(1) => global_unlock;
    host LocalSize(1) => global_size;
    host LocalFree(1) => global_free;
    host LocalHandle(1) => global_lock;
    host EnumSystemLocalesW(2) => enum_system_locales_w;
    host EnumSystemLocalesA(2) => enum_system_locales_a;
    guest GetLastError => RtlGetLastWin32Error;
    guest SetLastError => RtlSetLastWin32Error;
    guest GetProcessHeap => RtlGetProcessHeap;
    guest HeapAlloc => RtlAllocateHeap;
    guest HeapFree => RtlFreeHeap;
    guest EnterCriticalSection => RtlEnterCriticalSection;
    guest LeaveCriticalSection => RtlLeaveCriticalSection;
    guest InitializeCriticalSection => RtlInitializeCriticalSection;
    guest CreateThreadpool => AtlasCreateThreadpool;
    guest CloseThreadpool => AtlasCloseThreadpool;
    guest SetThreadpoolThreadMaximum => AtlasSetThreadpoolThreadMaximum;
    guest SetThreadpoolThreadMinimum => AtlasSetThreadpoolThreadMinimum;
    guest CreateThreadpoolWork => AtlasCreateThreadpoolWork;
    guest SubmitThreadpoolWork => AtlasSubmitThreadpoolWork;
    guest WaitForThreadpoolWorkCallbacks => AtlasWaitForThreadpoolWorkCallbacks;
    guest CloseThreadpoolWork => AtlasCloseThreadpoolWork;
    guest TrySubmitThreadpoolCallback => AtlasTrySubmitThreadpoolCallback;
    guest QueueUserWorkItem => AtlasQueueUserWorkItem;
    guest CreateThreadpoolTimer => AtlasCreateThreadpoolTimer;
    guest SetThreadpoolTimer => AtlasSetThreadpoolTimer;
    guest IsThreadpoolTimerSet => AtlasIsThreadpoolTimerSet;
    guest WaitForThreadpoolTimerCallbacks => AtlasWaitForThreadpoolTimerCallbacks;
    guest CloseThreadpoolTimer => AtlasCloseThreadpoolTimer;
    guest CreateThreadpoolWait => AtlasCreateThreadpoolWait;
    guest SetThreadpoolWait => AtlasSetThreadpoolWait;
    guest WaitForThreadpoolWaitCallbacks => AtlasWaitForThreadpoolWaitCallbacks;
    guest CloseThreadpoolWait => AtlasCloseThreadpoolWait;
    guest RegisterWaitForSingleObject => AtlasRegisterWaitForSingleObject;
    guest UnregisterWait => AtlasUnregisterWait;
    guest UnregisterWaitEx => AtlasUnregisterWaitEx;
    guest CreateThreadpoolCleanupGroup => AtlasCreateThreadpoolCleanupGroup;
    guest CloseThreadpoolCleanupGroupMembers => AtlasCloseThreadpoolCleanupGroupMembers;
    guest CloseThreadpoolCleanupGroup => AtlasCloseThreadpoolCleanupGroup;
    guest SetEventWhenCallbackReturns => AtlasSetEventWhenCallbackReturns;
    guest ReleaseSemaphoreWhenCallbackReturns => AtlasReleaseSemaphoreWhenCallbackReturns;
    guest ReleaseMutexWhenCallbackReturns => AtlasReleaseMutexWhenCallbackReturns;
    guest LeaveCriticalSectionWhenCallbackReturns => AtlasLeaveCriticalSectionWhenCallbackReturns;
    guest FreeLibraryWhenCallbackReturns => AtlasFreeLibraryWhenCallbackReturns;
    guest CallbackMayRunLong => AtlasCallbackMayRunLong;
    guest DisassociateCurrentThreadFromCallback => AtlasDisassociateCurrentThreadFromCallback;
    guest CreateThreadpoolIo => AtlasCreateThreadpoolIo;
    guest StartThreadpoolIo => AtlasStartThreadpoolIo;
    guest CancelThreadpoolIo => AtlasCancelThreadpoolIo;
    guest WaitForThreadpoolIoCallbacks => AtlasWaitForThreadpoolIoCallbacks;
    guest CloseThreadpoolIo => AtlasCloseThreadpoolIo;
    guest BindIoCompletionCallback => AtlasBindIoCompletionCallback;
    guest CreateTimerQueue => AtlasCreateTimerQueue;
    guest CreateTimerQueueTimer => AtlasCreateTimerQueueTimer;
    guest ChangeTimerQueueTimer => AtlasChangeTimerQueueTimer;
    guest DeleteTimerQueueTimer => AtlasDeleteTimerQueueTimer;
    guest DeleteTimerQueueEx => AtlasDeleteTimerQueueEx;
    guest DeleteTimerQueue => AtlasDeleteTimerQueue;
    guest CreateFiber => AtlasCreateFiber;
    guest CreateFiberEx => AtlasCreateFiberEx;
    guest DeleteFiber => AtlasDeleteFiber;
    guest SwitchToFiber => AtlasSwitchToFiber;
    guest ConvertThreadToFiber => AtlasConvertThreadToFiber;
    guest ConvertThreadToFiberEx => AtlasConvertThreadToFiberEx;
    guest ConvertFiberToThread => AtlasConvertFiberToThread;
    guest IsThreadAFiber => AtlasIsThreadAFiber;
    // What installers call: string and number helpers, temporary names, .ini files and the error mode.
    guest CompareFileTime => AtlasCompareFileTime;
    host GetDiskFreeSpaceW(5) => get_disk_free_space_w;
    host GetPrivateProfileStringW(6) => get_private_profile_string_w;
    host GetShortPathNameW(3) => get_short_path_name_w;
    host GetTempFileNameW(4) => get_temp_file_name_w;
    guest MulDiv => AtlasMulDiv;
    host SearchPathW(6) => search_path_w;
    host SetErrorMode(1) => set_error_mode;
    host WritePrivateProfileStringW(4) => write_private_profile_string_w;
    guest lstrcatW => AtlasLstrcatW;
    guest lstrcmpW => AtlasLstrcmpW;
    guest lstrcmpiA => AtlasLstrcmpiA;
    guest lstrcmpiW => AtlasLstrcmpiW;
    guest lstrcpyA => AtlasLstrcpyA;
    guest lstrcpyW => AtlasLstrcpyW;
    guest lstrcpynW => AtlasLstrcpynW;
    guest lstrlenA => AtlasLstrlenA;
    guest lstrlenW => AtlasLstrlenW;
    guest lstrcatA => AtlasLstrcatA;
    guest lstrcmpA => AtlasLstrcmpA;
    guest lstrcpynA => AtlasLstrcpynA;
    guest lstrcpy => AtlasLstrcpyA;
    guest lstrcat => AtlasLstrcatA;
    guest lstrlen => AtlasLstrlenA;
    guest lstrcmp => AtlasLstrcmpA;
    guest lstrcmpi => AtlasLstrcmpiA;
    guest lstrcpyn => AtlasLstrcpynA;
    // What larger programs call as they start: heaps, encoded pointers, one-time initialization, lists, the
    // debugger, locale-aware text, disks, the console, threads, processors and psapi's K32 functions.
    guest HeapReAlloc => RtlReAllocateHeap;
    guest HeapSize => RtlSizeHeap;
    guest HeapValidate => RtlValidateHeap;
    guest HeapLock => RtlLockHeap;
    guest HeapUnlock => RtlUnlockHeap;
    guest HeapWalk => AtlasHeapWalk;
    guest GetProcessHeaps => AtlasGetProcessHeaps;
    guest HeapQueryInformation => AtlasHeapQueryInformation;
    guest HeapSetInformation => AtlasHeapSetInformation;
    guest EncodePointer => RtlEncodePointer;
    guest DecodePointer => RtlDecodePointer;
    guest InitOnceBeginInitialize => AtlasInitOnceBeginInitialize;
    guest InitOnceComplete => AtlasInitOnceComplete;
    guest InitializeSListHead => AtlasInitializeSListHead;
    guest InterlockedPushEntrySList => AtlasInterlockedPushEntrySList;
    guest InterlockedPopEntrySList => AtlasInterlockedPopEntrySList;
    guest InterlockedFlushSList => AtlasInterlockedFlushSList;
    guest QueryDepthSList => AtlasQueryDepthSList;
    guest IsDebuggerPresent => AtlasIsDebuggerPresent;
    guest DebugBreak => AtlasDebugBreak;
    guest UnhandledExceptionFilter => AtlasUnhandledExceptionFilter;
    host IsBadWritePtr(2) => is_bad_write_ptr;
    host IsBadReadPtr(2) => is_bad_read_ptr;
    host OutputDebugStringA(1) => output_debug_string_a;
    host OutputDebugStringW(1) => output_debug_string_w;
    host CompareStringW(6) => compare_string_w;
    host CompareStringA(6) => compare_string_a;
    host CompareStringEx(9) => compare_string_ex;
    host LCMapStringW(6) => lc_map_string_w;
    host LCMapStringEx(9) => lc_map_string_ex;
    host GetStringTypeW(4) => get_string_type_w;
    host GetStringTypeExW(5) => get_string_type_ex_w;
    host GetDateFormatW(6) => get_date_format_w;
    host GetDateFormatA(6) => get_date_format_a;
    host GetDateFormatEx(7) => get_date_format_ex;
    host GetTimeFormatW(6) => get_time_format_w;
    host GetTimeFormatA(6) => get_time_format_a;
    host GetTimeFormatEx(6) => get_time_format_ex;
    host GetDiskFreeSpaceA(5) => get_disk_free_space_w;
    host GetDiskFreeSpaceExW(4) => get_disk_free_space_ex_w;
    host GetDiskFreeSpaceExA(4) => get_disk_free_space_ex_w;
    host GetDriveTypeW(1) => get_drive_type_w;
    host GetDriveTypeA(1) => get_drive_type_a;
    host FindFirstFileExW(6) => find_first_file_ex_w;
    host FindFirstFileExA(6) => find_first_file_ex_a;
    host DeviceIoControl(8) => device_io_control;
    host GetConsoleMode(2) => get_console_mode;
    host SetConsoleMode(2) => set_console_mode;
    host ReadConsoleA(5) => read_console_a;
    host ReadConsoleW(5) => read_console_w;
    host SetConsoleCtrlHandler(2) => set_console_ctrl_handler;
    host SetStdHandle(2) => set_std_handle;
    host OpenThread(3) => open_thread;
    host SetThreadPriority(2) => set_thread_priority;
    host GetThreadPriority(1) => get_thread_priority;
    host GetProcessAffinityMask(3) => get_process_affinity_mask;
    host SetProcessAffinityMask(2) => set_process_affinity_mask;
    host SetThreadAffinityMask(2) => set_thread_affinity_mask;
    host ProcessIdToSessionId(2) => process_id_to_session_id;
    host K32GetModuleFileNameExW(4) => get_module_file_name_ex_w;
    host K32GetModuleFileNameExA(4) => get_module_file_name_ex_a;
    host K32GetModuleInformation(4) => get_module_information;
    host K32GetProcessMemoryInfo(3) => get_process_memory_info;
    // Not written yet: what larger programs import.
    host AllocConsole(0) => accept;
    host AssignProcessToJobObject(2) => accept;
    host AttachConsole(1) => attach_console;
    host CreateEventExW(4) => create_event_ex_w;
    host CreateJobObjectA(2) => create_job_object;
    host CreateSemaphoreExW(6) => create_semaphore_ex_w;
    host CreateToolhelp32Snapshot(2) => create_toolhelp32_snapshot;
    missing DebugActiveProcess;
    missing DebugActiveProcessStop;
    host FlushInstructionCache(3) => succeed;
    host FlushProcessWriteBuffers(0) => zero;
    host FoldStringW(5) => fold_string_w;
    host FormatMessageA(7) => format_message_a;
    host FormatMessageW(7) => format_message_w;
    missing GenerateConsoleCtrlEvent;
    missing GetBinaryTypeW;
    host GetConsoleScreenBufferInfo(2) => get_console_screen_buffer_info;
    host GetCurrentProcessorNumber(0) => zero;
    missing GetFileInformationByHandleEx;
    host GetLocaleInfoA(4) => get_locale_info_a;
    host GetLocaleInfoEx(4) => get_locale_info_ex;
    host GetLocaleInfoW(4) => get_locale_info_w;
    host GetLogicalDriveStringsA(2) => get_logical_drive_strings_a;
    host GetLogicalDrives(0) => get_logical_drives;
    host GetNumaHighestNodeNumber(1) => get_numa_highest_node_number;
    missing GetPrivateProfileStringA;
    host GetProcessIdOfThread(1) => get_process_id_of_thread;
    host GetProcessTimes(5) => get_process_times;
    missing GetProfileStringA;
    host GetStringTypeA(5) => get_string_type_a;
    host GetSystemPowerStatus(1) => get_system_power_status;
    host GetSystemWindowsDirectoryW(2) => get_windows_directory_w;
    missing GetTempFileNameA;
    host GetThreadContext(2) => get_thread_context;
    host GetThreadGroupAffinity(2) => get_thread_group_affinity;
    host GetThreadTimes(5) => get_thread_times;
    host GetUserDefaultLCID(0) => get_user_default_lcid;
    host GetUserDefaultLangID(0) => get_user_default_lcid;
    host GetVolumeInformationA(8) => get_volume_information_a;
    host GetVolumeInformationW(8) => get_volume_information_w;
    host GetVolumeNameForVolumeMountPointW(3) => get_volume_name_for_volume_mount_point_w;
    host GetVolumePathNameW(3) => get_volume_path_name_w;
    host GetVolumePathNamesForVolumeNameW(4) => get_volume_path_names_for_volume_name_w;
    guest HeapCreate => AtlasHeapCreate;
    guest HeapDestroy => AtlasHeapDestroy;
    host IsValidLocale(2) => is_valid_locale;
    host K32EnumProcessModules(4) => enum_process_modules;
    host K32GetModuleBaseNameA(4) => get_module_base_name_a;
    host LCMapStringA(6) => lc_map_string_a;
    host Module32First(2) => module32_first;
    host Module32Next(2) => module32_next;
    host Process32First(2) => process32_first;
    host Process32Next(2) => process32_next;
    host QueryUnbiasedInterruptTime(1) => query_unbiased_interrupt_time;
    host ReadDirectoryChangesW(8) => read_directory_changes_w;
    host ReadProcessMemory(5) => read_process_memory;
    host SetConsoleTextAttribute(2) => accept;
    host SetDllDirectoryA(1) => set_dll_directory_a;
    host SetDllDirectoryW(1) => set_dll_directory_w;
    missing SetFileInformationByHandle;
    host SetHandleCount(1) => handle_count;
    host SetInformationJobObject(4) => accept;
    host SetPriorityClass(2) => accept;
    host SetProcessShutdownParameters(2) => succeed;
    missing SetSystemPowerState;
    missing SetThreadContext;
    host SetThreadErrorMode(2) => set_thread_error_mode;
    host SetThreadExecutionState(1) => set_thread_execution_state;
    missing SignalObjectAndWait;
    host VirtualAllocEx(5) => virtual_alloc_ex;
    host VirtualProtectEx(5) => virtual_protect_ex;
    host VirtualQueryEx(4) => virtual_query_ex;
    host WTSGetActiveConsoleSessionId(0) => wts_get_active_console_session_id;
    host WriteProcessMemory(5) => write_process_memory;
    host K32GetModuleBaseNameW(4) => get_module_base_name_w;
    host GetLogicalDriveStringsW(2) => get_logical_drive_strings_w;
    host GetSystemDefaultLCID(0) => get_user_default_lcid;
    host GetSystemDefaultLangID(0) => get_user_default_lcid;
    host Process32FirstW(2) => process32_first_w;
    host Process32NextW(2) => process32_next_w;
    host Module32FirstW(2) => module32_first_w;
    host Module32NextW(2) => module32_next_w;
    host Thread32First(2) => thread32_first;
    host Thread32Next(2) => thread32_next;
    host CreateRemoteThread(7) => create_remote_thread;
    host CreateRemoteThreadEx(8) => create_remote_thread_ex;
    host VirtualFreeEx(4) => virtual_free_ex;
    host CheckRemoteDebuggerPresent(2) => check_remote_debugger_present;
    missing ClearCommError;
    host CompareStringOrdinal(5) => compare_string_ordinal;
    host CreateJobObjectW(2) => create_job_object;
    missing CreateMemoryResourceNotification;
    host DeleteProcThreadAttributeList(1) => accept;
    // Pages whose contents may go: kept as they are, which Windows allows.
    host DiscardVirtualMemory(2) => zero;
    missing EnumSystemLocalesEx;
    missing EscapeCommFunction;
    missing FindFirstVolumeW;
    missing FindNextVolumeW;
    missing FindVolumeClose;
    host FreeConsole(0) => accept;
    missing GetCommModemStatus;
    missing GetCommState;
    missing GetConsoleDisplayMode;
    missing GetCurrencyFormatEx;
    host GetFirmwareType(1) => get_firmware_type;
    host GetGeoInfoW(5) => get_geo_info_w;
    missing GetNumberFormatEx;
    host GetPriorityClass(1) => get_priority_class;
    host GetProcessHandleCount(2) => get_process_handle_count;
    host GetProcessMitigationPolicy(4) => get_process_mitigation_policy;
    host GetThreadLocale(0) => get_user_default_lcid;
    host GetThreadPreferredUILanguages(4) => get_preferred_ui_languages;
    host GetThreadPriorityBoost(2) => get_thread_priority_boost;
    host GetUserDefaultLocaleName(2) => get_user_default_locale_name;
    host GetUserDefaultUILanguage(0) => get_user_default_lcid;
    host GetUserGeoID(1) => get_user_geo_id;
    missing HeapCompact;
    host InitializeProcThreadAttributeList(4) => initialize_proc_thread_attribute_list;
    missing K32GetMappedFileNameW;
    host K32GetPerformanceInfo(2) => get_performance_info;
    missing K32QueryWorkingSetEx;
    host LCIDToLocaleName(4) => lcid_to_locale_name;
    host LocaleNameToLCID(2) => locale_name_to_lcid;
    host LockFile(5) => accept;
    host LockFileEx(6) => accept;
    host PowerClearRequest(2) => accept;
    host PowerCreateRequest(1) => power_create_request;
    host PowerSetRequest(2) => accept;
    host PrefetchVirtualMemory(4) => prefetch_virtual_memory;
    missing PurgeComm;
    host QueryDosDeviceW(3) => query_dos_device_w;
    host QueryFullProcessImageNameA(4) => query_full_process_image_name_a;
    host QueryFullProcessImageNameW(4) => query_full_process_image_name_w;
    host QueryInformationJobObject(5) => query_information_job_object;
    host QueryProcessCycleTime(2) => query_cycle_time;
    host QueryThreadCycleTime(2) => query_cycle_time;
    host ResolveLocaleName(3) => resolve_locale_name;
    host RtlAddFunctionTable(3) => accept;
    host RtlDeleteFunctionTable(1) => accept;
    missing SetCommState;
    missing SetCommTimeouts;
    host SetDefaultDllDirectories(1) => set_default_dll_directories;
    host SetProcessInformation(4) => accept;
    host SetProcessMitigationPolicy(3) => accept;
    host SetThreadInformation(4) => accept;
    host SetThreadPriorityBoost(2) => accept;
    host TerminateJobObject(2) => accept;
    host UnlockFile(5) => accept;
    host UnlockFileEx(5) => accept;
    host UpdateProcThreadAttribute(7) => accept;
    host WerRegisterRuntimeExceptionModule(2) => zero;
    missing Wow64GetThreadContext;
    missing terminate;
    missing CompareObjectHandles;
    host QueryUnbiasedInterruptTimePrecise(1) => query_unbiased_interrupt_time;
    // KernelBase's since Windows 8, which programs look for here when there's no KernelBase to find it in.
    host CommandLineToArgvW(2) => command_line_to_argv_w;
    host RegCloseKey(1) => reg_close_key;
    host RegOpenKeyExA(5) => reg_open_key_ex_a;
    host RegOpenKeyExW(5) => reg_open_key_ex_w;
    host RegQueryValueExA(6) => reg_query_value_ex_a;
    host RegQueryValueExW(6) => reg_query_value_ex_w;
    // The registry functions kernel32 exports too, as Windows' does: advapi32's.
    host RegCreateKeyExW(9) => reg_create_key_ex_w;
    host RegSetValueExW(6) => reg_set_value_ex_w;
    host RegDeleteKeyExW(4) => reg_delete_key_ex_w;
    host RegDeleteValueW(2) => reg_delete_value_w;
    host ReclaimVirtualMemory(2) => zero;
    host OfferVirtualMemory(3) => zero;
}

const TRUE: u64 = 1;
const FALSE: u64 = 0;
/// `INVALID_HANDLE_VALUE`.
const INVALID: u64 = u64::MAX;

fn get_std_handle(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    // STD_INPUT_HANDLE is -10, STD_OUTPUT_HANDLE -11, STD_ERROR_HANDLE -12.
    let which = a[0] as u32 as i32;
    let handles = lock(&task.process.state).std_handles;
    Reply::Value(match which {
        -10 => handles[0],
        -11 => handles[1],
        -12 => handles[2],
        _ => INVALID,
    })
}

/// Writes to a console handle (or any handle that takes writes), giving how much was written.
fn write_to(task: &mut Task, handle: u64, data: &[u8]) -> Option<usize> {
    let h = lock(&task.process.state).handles.get(handle)?;
    match h {
        Handle::Console(stream) => {
            task.k.host.write(stream, data);
            Some(data.len())
        }
        Handle::File(file) => task.write_to_file(&file, data, None).ok(),
        _ => None,
    }
}

/// `WriteConsoleA`: its bytes are in the console's output code page (437, unless it's been set), shown as text.
fn write_console_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let cp = task.k.console_cp.load(std::sync::atomic::Ordering::Relaxed);
    if cp == crate::ansi::UTF8 {
        return write_file(task, cx, &[a[0], a[1], a[2], a[3], 0]);
    }
    let count = a[2] as u32 as usize;
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], count)) else { return Reply::Value(FALSE) };
    let text = String::from_utf16_lossy(&crate::ansi::decode(cp, &raw, false).unwrap_or_default());
    let Some(_) = write_to(task, a[0], text.as_bytes()) else { return Reply::Value(FALSE) };
    if a[3] != 0 && task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[3], count as u32)).is_err() {
        return Reply::Value(FALSE);
    }
    Reply::Value(TRUE)
}

fn write_console_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let count = a[2] as u32 as usize;
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], count * 2)) else { return Reply::Value(FALSE) };
    let units: Vec<u16> = raw.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16_lossy(&units);
    let Some(_) = write_to(task, a[0], text.as_bytes()) else { return Reply::Value(FALSE) };
    if a[3] != 0 && task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[3], count as u32)).is_err() {
        return Reply::Value(FALSE);
    }
    Reply::Value(TRUE)
}

fn get_command_line_a(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(lock(&task.process.state).command_line.0)
}

fn get_command_line_w(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(lock(&task.process.state).command_line.1)
}

fn get_tick_count64(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(task.k.host.monotonic() / 1_000_000)
}

/// On Windows these read a counter in the program's memory, with no system call. Host exports for now.
fn query_performance_counter(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let now = task.k.host.monotonic() / 100;
    Reply::Value(if task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[0], now)).is_ok() { TRUE } else { FALSE })
}

fn query_performance_frequency(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[0], 10_000_000)).is_ok() { TRUE } else { FALSE })
}

fn virtual_alloc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    virtual_alloc_from(task, cx, a)
}

pub(crate) fn virtual_alloc_from(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.virtual_alloc(cx, a[0], a[1], a[2] as u32, a[3] as u32).unwrap_or(0))
}

fn virtual_protect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    virtual_protect_from(task, cx, a)
}

pub(crate) fn virtual_protect_from(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if task.virtual_protect(cx, a[0], a[1], a[2] as u32, a[3]) == 0 { TRUE } else { FALSE })
}

fn virtual_free(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    virtual_free_from(task, cx, a)
}

pub(crate) fn virtual_free_from(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if task.virtual_free(cx, a[0], a[1], a[2] as u32) { TRUE } else { FALSE })
}

/// Global memory, as the clipboard and drag and drop pass data: each block committed pages of its own, its size in
/// the 16 bytes before it, and its handle the block itself (as `GMEM_FIXED`'s are; a moveable block never moves here,
/// so locking one gives its handle back). Pages come zeroed, so `GMEM_ZEROINIT` needs nothing more.
const GLOBAL_HEADER: u64 = 16;

/// A new global block holding `data`, or 0.
pub(crate) fn global_new(task: &mut Task, cx: &mut Cx<'_>, data: &[u8]) -> u64 {
    let size = data.len() as u64;
    let Some(base) = task.virtual_alloc(cx, 0, size + GLOBAL_HEADER, 0x3000, 4) else { return 0 };
    let ok = task.mem(cx, true, |t| {
        user::write_u64(&mut t.space(), base, size)?;
        user::write(&mut t.space(), base + GLOBAL_HEADER, data)
    });
    if ok.is_err() {
        task.virtual_free(cx, base, 0, 0x8000);
        return 0;
    }
    base + GLOBAL_HEADER
}

/// What a global block holds: its size from its header, its bytes after (at most 64 MiB). None for one that isn't.
pub(crate) fn global_read(task: &mut Task, cx: &mut Cx<'_>, handle: u64) -> Option<Vec<u8>> {
    let size = global_size_of(task, cx, handle)?;
    task.mem(cx, false, |t| user::read_vec(&t.space(), handle, size as usize)).ok()
}

/// Frees a global block `global_new` made.
pub(crate) fn global_release(task: &mut Task, cx: &mut Cx<'_>, handle: u64) -> bool {
    global_size_of(task, cx, handle).is_some() && task.virtual_free(cx, handle - GLOBAL_HEADER, 0, 0x8000)
}

fn global_size_of(task: &mut Task, cx: &mut Cx<'_>, handle: u64) -> Option<u64> {
    if handle < GLOBAL_HEADER {
        return None;
    }
    task.mem(cx, false, |t| user::read_u64(&t.space(), handle - GLOBAL_HEADER)).ok().filter(|&n| n <= 64 << 20)
}

fn global_alloc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] > 64 << 20 {
        return Reply::Value(0);
    }
    Reply::Value(global_new(task, cx, &vec![0; a[1] as usize]))
}

fn global_lock(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(a[0])
}

/// Still locked: FALSE, as unlocking a block that isn't moveable says.
fn global_unlock(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(FALSE)
}

fn global_size(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(global_size_of(task, cx, a[0]).unwrap_or(0))
}

/// NULL once it's freed, else the handle back.
fn global_free(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if global_release(task, cx, a[0]) { 0 } else { a[0] })
}

/// Calls the program's callback with each locale's name, until it returns FALSE: an upcall per locale.
fn enum_system_locales_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_system_locales(task, cx, a, true)
}

fn enum_system_locales_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_system_locales(task, cx, a, false)
}

/// `EnumSystemLocales(callback, flags)`: each locale's ID, as text, to the callback until it says to stop.
fn enum_system_locales(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let callback = a[0];
    for locale in ["00000409", "00000809", "0000040C"] {
        let text: Vec<u8> = if wide { locale.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() } else { locale.bytes().chain([0]).collect() };
        match task.upcall_with_data(cx, callback, &text, &[]) {
            Ok(0) => break,
            Ok(_) => {}
            Err(ended) => return ended,
        }
    }
    Reply::Value(TRUE)
}

// --- The console's modes, reading it, and the standard handles ---

/// Whether a handle is the console's: its input, or output to it.
fn is_console(task: &Task, handle: u64) -> Option<bool> {
    match lock(&task.process.state).handles.get(handle)? {
        Handle::Input => Some(true),
        Handle::Console(_) => Some(false),
        _ => None,
    }
}

/// `GetConsoleMode(handle, &mode)`: the input's processed, line and echo input with insert and quick edit; output's
/// processed output with wrapping at the line's end. A handle that isn't the console's is ERROR_INVALID_HANDLE, which
/// is how programs tell a console from a file or pipe.
fn get_console_mode(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(input) = is_console(task, a[0]) else { return task.fail(cx, 6) };
    let mode: u32 = if input { 0x1f7 } else { 0x3 };
    if task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], mode)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(TRUE)
}

/// `SetConsoleMode(handle, mode)`: taken for the console's handles; the host's terminal keeps its own modes.
fn set_console_mode(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if is_console(task, a[0]).is_none() {
        return task.fail(cx, 6);
    }
    Reply::Value(TRUE)
}

/// `ReadConsoleA(input, buffer, count, &read, control)`: as `ReadFile` reads it.
fn read_console_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if is_console(task, a[0]) != Some(true) {
        return task.fail(cx, 6);
    }
    read_file(task, cx, &[a[0], a[1], a[2], a[3], 0])
}

/// `ReadConsoleW(input, buffer, count, &read, control)`: bytes read as `ReadFile` does, then made UTF-16 from the
/// console's code page, in the same buffer (which has room: a byte becomes at most one unit).
fn read_console_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if is_console(task, a[0]) != Some(true) || a[3] == 0 {
        return task.fail(cx, if a[3] == 0 { 87 } else { 6 });
    }
    let r = read_file(task, cx, &[a[0], a[1], a[2], a[3], 0]);
    if r != Reply::Value(TRUE) {
        return r;
    }
    let Ok(n) = task.mem(cx, false, |t| user::read_u32(&t.space(), a[3])) else { return task.fail(cx, 998) };
    let Ok(bytes) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], n as usize)) else { return task.fail(cx, 998) };
    let cp = task.k.console_cp.load(std::sync::atomic::Ordering::Relaxed);
    let units = crate::ansi::decode(cp, &bytes, false).unwrap_or_default();
    let out: Vec<u8> = units.iter().take(a[2] as u32 as usize).flat_map(|u| u.to_le_bytes()).collect();
    let wrote = task.mem(cx, true, |t| {
        user::write(&mut t.space(), a[1], &out)?;
        user::write_u32(&mut t.space(), a[3], (out.len() / 2) as u32)
    });
    if wrote.is_err() { task.fail(cx, 998) } else { Reply::Value(TRUE) }
}

/// `SetConsoleCtrlHandler(handler, add)`: kept nowhere, as the host's terminal sends atlas no Ctrl+C to pass on.
fn set_console_ctrl_handler(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(TRUE)
}

/// `SetStdHandle(which, handle)`: what `GetStdHandle` gives from now on.
fn set_std_handle(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let slot = match a[0] as u32 as i32 {
        -10 => 0,
        -11 => 1,
        -12 => 2,
        _ => return task.fail(cx, 6),
    };
    lock(&task.process.state).std_handles[slot] = a[1];
    Reply::Value(TRUE)
}

/// `DeviceIoControl(device, code, in, in size, out, out size, &returned, overlapped)`: no device atlas has takes
/// controls, so ERROR_INVALID_FUNCTION, as a file's handle gives for codes its file system doesn't know.
fn device_io_control(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if lock(&task.process.state).handles.get(a[0]).is_none() {
        return task.fail(cx, 6);
    }
    task.fail(cx, 1)
}

/// `OutputDebugStringA(text)` and `W`: with no debugger, it goes to atlas's trace when tracing is on.
fn output_debug_string(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if task.k.options.trace {
        let text = task.text_arg(cx, a[0], wide);
        task.k.host.write(host::Stream::Err, format!("[{}] debug output: {}\n", task.tid, text.trim_end()).as_bytes());
    }
    Reply::Value(0)
}

fn output_debug_string_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    output_debug_string(task, cx, a, false)
}

fn output_debug_string_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    output_debug_string(task, cx, a, true)
}

/// `PrefetchVirtualMemory(process, count, &ranges, flags)`: a hint that pages will be wanted soon. They come in as
/// they're touched here, so there's nothing to do ahead.
fn prefetch_virtual_memory(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(TRUE)
}

/// `AttachConsole(process)`: every process here shares the machine's one console (as `AllocConsole` and `FreeConsole`
/// take it), so attaching to the parent's (ATTACH_PARENT_PROCESS) or any process's succeeds; a process that isn't
/// there has no console to attach to.
fn attach_console(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pid = a[0] as u32;
    if pid != u32::MAX && task.k.process(u64::from(pid)).is_none() {
        return task.fail(cx, 87); // ERROR_INVALID_PARAMETER
    }
    Reply::Value(TRUE)
}

/// `GetConsoleScreenBufferInfo(output, &info)`: the console's buffer as the host's terminal is, its columns by its rows
/// (80 by 25 when there's no terminal to ask), its window the whole of it, the cursor at its top left (where it is in
/// the host's terminal isn't known), grey on black. An output handle that isn't the console's is ERROR_INVALID_HANDLE.
fn get_console_screen_buffer_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if is_console(task, a[0]) != Some(false) {
        return task.fail(cx, 6);
    }
    let (rows, columns) = task.k.host.terminal_size().unwrap_or((25, 80));
    let (w, h) = (columns.clamp(1, 0x7fff) as i16, rows.clamp(1, 0x7fff) as i16);
    // dwSize, dwCursorPosition, wAttributes, srWindow (left, top, right, bottom), dwMaximumWindowSize.
    let fields: [i16; 11] = [w, h, 0, 0, 0x07, 0, 0, w - 1, h - 1, w, h];
    let bytes: Vec<u8> = fields.iter().flat_map(|v| v.to_le_bytes()).collect();
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &bytes)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(TRUE)
}
