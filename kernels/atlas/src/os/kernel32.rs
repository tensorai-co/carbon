//! kernel32.dll: processes, the console, files, memory and time. Heaps, locks and the last error are guest code, as
//! on Windows; the rest is host Rust here.


use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::fileapi::*;
use super::ioapi::*;
use super::memapi::*;
use super::pipeapi::*;
use super::procapi::*;
use super::synch::*;
use super::{Reply, dll};
use crate::handles::Handle;
use crate::process::Task;

dll! {
    "KERNEL32.dll";
    host GetStdHandle(1) => get_std_handle;
    host WriteFile(5) => write_file;
    host WriteConsoleA(5) => write_console_a;
    host WriteConsoleW(5) => write_console_w;
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
    host SetFileAttributesW(2) => set_file_attributes_w;
    host GetFileInformationByHandle(2) => get_file_information_by_handle;
    host DeleteFileW(1) => delete_file_w;
    host DeleteFileA(1) => delete_file_a;
    host CreateDirectoryW(2) => create_directory_w;
    host CreateDirectoryA(2) => create_directory_a;
    host RemoveDirectoryW(1) => remove_directory_w;
    host MoveFileExW(3) => move_file_ex_w;
    host MoveFileW(2) => move_file_w;
    host CopyFileW(3) => copy_file_w;
    host FindFirstFileW(2) => find_first_file_w;
    host FindFirstFileA(2) => find_first_file_a;
    host FindNextFileW(2) => find_next_file_w;
    host FindNextFileA(2) => find_next_file_a;
    host FindClose(1) => find_close;
    host GetCurrentDirectoryW(2) => get_current_directory_w;
    host GetCurrentDirectoryA(2) => get_current_directory_a;
    host SetCurrentDirectoryW(1) => set_current_directory_w;
    host GetFullPathNameW(4) => get_full_path_name_w;
    host GetLongPathNameW(3) => get_long_path_name_w;
    host GetTempPathW(2) => get_temp_path_w;
    host GetFinalPathNameByHandleW(4) => get_final_path_name_by_handle_w;
    host GetWindowsDirectoryW(2) => get_windows_directory_w;
    host GetSystemDirectoryW(2) => get_system_directory_w;
    host CloseHandle(1) => close_handle;
    host ExitProcess(1) => exit_process;
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
    guest GetModuleFileNameW => AtlasGetModuleFileNameW;
    guest RaiseException => AtlasRaiseException;
    guest SetUnhandledExceptionFilter => AtlasSetUnhandledExceptionFilter;
    guest RtlCaptureContext;
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
    host SetEvent(1) => set_event;
    host ResetEvent(1) => reset_event;
    host CreateMutexW(3) => create_mutex_w;
    host CreateMutexA(3) => create_mutex_a;
    host OpenMutexW(3) => open_mutex_w;
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
    host GetSystemTimeAsFileTime(1) => get_system_time_as_file_time;
    host GetSystemTimePreciseAsFileTime(1) => get_system_time_as_file_time;
    host CreateWaitableTimerW(3) => create_waitable_timer_w;
    host CreateWaitableTimerA(3) => create_waitable_timer_a;
    host CreateWaitableTimerExW(4) => create_waitable_timer_ex_w;
    host OpenWaitableTimerW(3) => open_waitable_timer_w;
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
    host EnumSystemLocalesW(2) => enum_system_locales_w;
    guest GetLastError => RtlGetLastWin32Error;
    guest SetLastError => RtlSetLastWin32Error;
    guest GetProcessHeap => RtlGetProcessHeap;
    guest HeapAlloc => RtlAllocateHeap;
    guest HeapFree => RtlFreeHeap;
    guest EnterCriticalSection => RtlEnterCriticalSection;
    guest LeaveCriticalSection => RtlLeaveCriticalSection;
    guest InitializeCriticalSection => RtlInitializeCriticalSection;
    missing CreateFiber;
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

fn write_console_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    write_file(task, cx, &[a[0], a[1], a[2], a[3], 0])
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

fn exit_process(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::ExitProcess(a[0] as u32)
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
    Reply::Value(task.virtual_alloc(cx, a[0], a[1], a[2] as u32, a[3] as u32).unwrap_or(0))
}

fn virtual_protect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if task.virtual_protect(cx, a[0], a[1], a[2] as u32, a[3]) == 0 { TRUE } else { FALSE })
}

fn virtual_free(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
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
    let callback = a[0];
    for locale in ["00000409", "00000809", "0000040C"] {
        let text: Vec<u8> = locale.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        match task.upcall_with_data(cx, callback, &text, &[]) {
            Ok(0) => break,
            Ok(_) => {}
            Err(ended) => return ended,
        }
    }
    Reply::Value(TRUE)
}
