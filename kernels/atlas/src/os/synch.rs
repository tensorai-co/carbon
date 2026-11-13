//! kernel32's threads and synchronization objects, in host Rust: what NT does in its kernel. The fast paths
//! (critical sections, SRW locks, condition variables, TLS) are guest code in ntdll.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::handles::{CURRENT_PROCESS, CURRENT_THREAD, Handle};
use crate::objects::{Kind, Object, State, WAIT_FAILED};
use crate::process::Task;

const TRUE: u64 = 1;
const FALSE: u64 = 0;
const STILL_ACTIVE: u32 = 259;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_NOT_OWNER: u32 = 288;
const ERROR_TOO_MANY_POSTS: u32 = 298;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_FILE_NOT_FOUND: u32 = 2;

impl Task {
    /// The object a handle stands for, including the current process's and thread's pseudo-handles.
    pub fn object(&self, handle: u64) -> Option<Arc<Object>> {
        match handle {
            CURRENT_PROCESS => Some(self.process.object.clone()),
            CURRENT_THREAD => Some(self.control.object.clone()),
            _ => match lock(&self.process.state).handles.get(handle)? {
                Handle::Object(object) => Some(object),
                _ => None,
            },
        }
    }

    fn new_handle(&self, object: Arc<Object>) -> u64 {
        self.new_handle_with(object, false)
    }

    /// A handle to an object, inheritable or not.
    fn new_handle_with(&self, object: Arc<Object>, inheritable: bool) -> u64 {
        lock(&self.process.state).handles.add_with(Handle::Object(object), inheritable)
    }

    /// The program's last error, in its TEB.
    pub fn set_last_error(&mut self, cx: &mut Cx<'_>, error: u32) {
        let at = self.control.teb + self.abi().teb.last_error;
        let _ = self.mem(cx, true, |t| user::write_u32(&mut t.space(), at, error));
    }

    fn name(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> Option<String> {
        if at == 0 {
            return None;
        }
        if wide {
            self.mem(cx, false, |t| user::read_wstr(&t.space(), at, 260)).ok()
        } else {
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 260)).ok().map(|b| crate::ansi::text(&b))
        }
    }

    /// Makes a named or unnamed object and a handle to it, setting ERROR_ALREADY_EXISTS if the name was taken.
    /// `attributes` is the SECURITY_ATTRIBUTES the program gave, which says whether the handle is inheritable.
    fn create_object(&mut self, cx: &mut Cx<'_>, attributes: u64, name: Option<String>, kind: Kind, state: State) -> Reply {
        let inheritable = self.inheritable_by(cx, attributes);
        match self.k.objects.create(name.as_deref(), kind, state) {
            Ok((object, existed)) => {
                if !existed && let Some(d) = self.attributes_descriptor(cx, attributes) {
                    let d = d.filled(&lock(&self.effective_token().state)).mapped(&super::secapi::kind_mapping(&object.kind));
                    *lock(&object.security) = Some(d);
                }
                self.set_last_error(cx, if existed { ERROR_ALREADY_EXISTS } else { 0 });
                Reply::Value(self.new_handle_with(object, inheritable))
            }
            Err(()) => {
                self.set_last_error(cx, ERROR_INVALID_HANDLE);
                Reply::Value(0)
            }
        }
    }

    /// Opens a named object, if its descriptor allows `access`.
    fn open_object(&mut self, cx: &mut Cx<'_>, access: u64, name: u64, kind: fn(&Kind) -> bool, mapping: &crate::security::Mapping) -> Reply {
        self.open_named(cx, access, name, true, kind, mapping)
    }

    /// `Open*`, by a UTF-16 name or an ANSI one.
    fn open_named(&mut self, cx: &mut Cx<'_>, access: u64, name: u64, wide: bool, kind: fn(&Kind) -> bool, mapping: &crate::security::Mapping) -> Reply {
        let found = self.name(cx, name, wide).and_then(|n| self.k.objects.named(&n)).filter(|o| kind(&o.kind));
        match found {
            Some(object) if !self.allows(&object.security, access as u32, mapping) => {
                self.set_last_error(cx, 5); // ERROR_ACCESS_DENIED
                Reply::Value(0)
            }
            Some(object) => Reply::Value(self.new_handle(object)),
            None => {
                self.set_last_error(cx, ERROR_FILE_NOT_FOUND);
                Reply::Value(0)
            }
        }
    }

    fn wait_reply(&mut self, cx: &mut Cx<'_>, handles: &[u64], all: bool, timeout: u32, alertable: bool) -> Reply {
        let objects: Option<Vec<_>> = handles.iter().map(|&h| self.object(h)).collect();
        let Some(objects) = objects.filter(|o| !o.is_empty() && o.len() <= 64) else {
            self.set_last_error(cx, ERROR_INVALID_HANDLE);
            return Reply::Value(u64::from(WAIT_FAILED));
        };
        match self.wait_for(cx, &objects, all, timeout, alertable) {
            Ok(code) => Reply::Value(u64::from(code)),
            Err(reply) => reply,
        }
    }

    pub(crate) fn thread_of(&self, handle: u64) -> Option<Arc<crate::threads::ThreadControl>> {
        let object = self.object(handle)?;
        let Kind::Thread(tid) = object.kind else { return None };
        let own = lock(&self.process.state).threads.get(&tid).cloned();
        own.or_else(|| self.k.thread(tid))
    }
}

pub fn create_thread(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.create_thread(cx, a[1], a[2], a[3], a[4] as u32) {
        Ok((object, tid)) => {
            if a[5] != 0 {
                let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[5], tid as u32));
            }
            Reply::Value(task.new_handle(object))
        }
        Err(_) => {
            task.set_last_error(cx, 8); // ERROR_NOT_ENOUGH_MEMORY
            Reply::Value(0)
        }
    }
}

fn exit_code(task: &mut Task, cx: &mut Cx<'_>, handle: u64, out: u64) -> Reply {
    let Some(object) = task.object(handle) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    let code = lock(&object.state).exit.unwrap_or(STILL_ACTIVE);
    Reply::Value(if task.mem(cx, true, |t| user::write_u32(&mut t.space(), out, code)).is_ok() { TRUE } else { FALSE })
}

pub fn get_exit_code_thread(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    exit_code(task, cx, a[0], a[1])
}

pub fn get_exit_code_process(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    exit_code(task, cx, a[0], a[1])
}

pub fn get_thread_id(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(match task.object(a[0]).map(|o| o.kind) {
        Some(Kind::Thread(tid)) => tid,
        _ => 0,
    })
}

pub fn get_process_id(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(match task.object(a[0]).map(|o| o.kind) {
        Some(Kind::Process(pid)) => pid,
        _ => 0,
    })
}

pub fn suspend_thread(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(thread) = task.thread_of(a[0]) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(u64::from(u32::MAX));
    };
    let previous = thread.suspend.fetch_add(1, Ordering::AcqRel);
    // It stops at its next chance: a running CPU is preempted, and parks on its way back to the program.
    thread.cpu.preempt();
    Reply::Value(u64::from(previous))
}

pub fn resume_thread(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(thread) = task.thread_of(a[0]) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(u64::from(u32::MAX));
    };
    // A compare-and-swap loop: `fetch_update` is renamed in newer Rust, and this reads the same in both.
    let mut previous = thread.suspend.load(Ordering::Acquire);
    while let Err(now) = thread.suspend.compare_exchange_weak(previous, previous.saturating_sub(1), Ordering::AcqRel, Ordering::Acquire) {
        previous = now;
    }
    if previous == 1 {
        thread.waiter.wake();
    }
    Reply::Value(u64::from(previous))
}

pub fn terminate_thread(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(thread) = task.thread_of(a[0]) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    if thread.tid == task.tid {
        return Reply::ExitThread(a[1] as u32);
    }
    lock(&thread.killed).get_or_insert(a[1] as u32);
    thread.waiter.interrupt();
    thread.cpu.preempt();
    Reply::Value(TRUE)
}

pub fn switch_to_thread(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    std::thread::yield_now();
    Reply::Value(TRUE)
}

fn create_event(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = task.name(cx, a[3], wide);
    task.create_object(cx, a[0], name, Kind::Event { manual: a[1] as u32 != 0 }, State { set: a[2] as u32 != 0, ..State::default() })
}

pub fn create_event_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_event(task, cx, a, true)
}

pub fn create_event_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_event(task, cx, a, false)
}

pub fn open_event_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.open_object(cx, a[0], a[2], |k| matches!(k, Kind::Event { .. }), &crate::security::EVENT)
}

pub fn open_event_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.open_named(cx, a[0], a[2], false, |k| matches!(k, Kind::Event { .. }), &crate::security::EVENT)
}

fn change_event(task: &mut Task, cx: &mut Cx<'_>, handle: u64, set: bool) -> Reply {
    match task.object(handle).filter(|o| matches!(o.kind, Kind::Event { .. })) {
        Some(event) => {
            task.k.objects.signal(&event, |s| s.set = set);
            Reply::Value(TRUE)
        }
        None => {
            task.set_last_error(cx, ERROR_INVALID_HANDLE);
            Reply::Value(FALSE)
        }
    }
}

pub fn set_event(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    change_event(task, cx, a[0], true)
}

pub fn reset_event(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    change_event(task, cx, a[0], false)
}

fn create_mutex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = task.name(cx, a[2], wide);
    let owned = a[1] as u32 != 0;
    let state = if owned { State { owner: task.tid, count: 1, ..State::default() } } else { State::default() };
    let reply = task.create_object(cx, a[0], name, Kind::Mutant, state);
    if owned && let Reply::Value(handle) = reply && let Some(object) = task.object(handle) {
        lock(&task.control.owned).push(Arc::downgrade(&object));
    }
    reply
}

pub fn create_mutex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_mutex(task, cx, a, true)
}

pub fn create_mutex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_mutex(task, cx, a, false)
}

pub fn open_mutex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.open_object(cx, a[0], a[2], |k| *k == Kind::Mutant, &crate::security::MUTANT)
}

pub fn open_mutex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.open_named(cx, a[0], a[2], false, |k| *k == Kind::Mutant, &crate::security::MUTANT)
}

pub fn release_mutex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(mutant) = task.object(a[0]).filter(|o| o.kind == Kind::Mutant) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    let tid = task.tid;
    let released = task.k.objects.signal(&mutant, |s| {
        if s.owner != tid {
            return false;
        }
        s.count -= 1;
        if s.count == 0 {
            s.owner = 0;
        }
        true
    });
    if !released {
        task.set_last_error(cx, ERROR_NOT_OWNER);
    }
    Reply::Value(if released { TRUE } else { FALSE })
}

fn create_semaphore(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let (initial, max) = (a[1] as i32 as i64, a[2] as i32 as i64);
    if max <= 0 || initial < 0 || initial > max {
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Reply::Value(0);
    }
    let name = task.name(cx, a[3], wide);
    task.create_object(cx, a[0], name, Kind::Semaphore { max }, State { count: initial, ..State::default() })
}

pub fn create_semaphore_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_semaphore(task, cx, a, true)
}

pub fn create_semaphore_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_semaphore(task, cx, a, false)
}

pub fn release_semaphore(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(semaphore) = task.object(a[0]) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    let Kind::Semaphore { max } = semaphore.kind else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    let by = a[1] as i32 as i64;
    let previous = task.k.objects.signal(&semaphore, |s| {
        if by <= 0 || s.count + by > max {
            return None;
        }
        let previous = s.count;
        s.count += by;
        Some(previous)
    });
    let Some(previous) = previous else {
        task.set_last_error(cx, ERROR_TOO_MANY_POSTS);
        return Reply::Value(FALSE);
    };
    if a[2] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[2], previous as u32));
    }
    Reply::Value(TRUE)
}

pub fn wait_for_single_object(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.wait_reply(cx, &[a[0]], false, a[1] as u32, false)
}

pub fn wait_for_single_object_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.wait_reply(cx, &[a[0]], false, a[1] as u32, a[2] as u32 != 0)
}

fn wait_multiple(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], alertable: bool) -> Reply {
    let count = (a[0] as u32).min(64) as usize;
    let width = task.convention().width();
    let Ok(handles) = task.mem(cx, false, |t| (0..count as u64).map(|i| user::read_ptr(&t.space(), a[1] + i * width, width)).collect::<Result<Vec<_>, _>>()) else {
        return Reply::Value(u64::from(WAIT_FAILED));
    };
    task.wait_reply(cx, &handles, a[2] as u32 != 0, a[3] as u32, alertable)
}

pub fn wait_for_multiple_objects(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    wait_multiple(task, cx, a, false)
}

pub fn wait_for_multiple_objects_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    wait_multiple(task, cx, a, a[4] as u32 != 0)
}

/// `Sleep` and `SleepEx`: a wait on nothing, which an APC ends if it's alertable.
pub fn sleep_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (ms, alertable) = (a[0] as u32, a[1] as u32 != 0);
    if ms == 0 && !alertable {
        std::thread::yield_now();
        return Reply::Value(0);
    }
    match task.wait_for(cx, &[], false, ms, alertable) {
        Ok(code) if code == crate::objects::WAIT_IO_COMPLETION => Reply::Value(u64::from(code)),
        Ok(_) => Reply::Value(0),
        Err(reply) => reply,
    }
}

pub fn sleep(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    sleep_ex(task, cx, &[a[0], 0])
}

pub fn queue_user_apc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(thread) = task.thread_of(a[1]) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    lock(&thread.apcs).push_back((a[0], vec![a[2]]));
    thread.waiter.wake();
    Reply::Value(TRUE)
}

/// `DuplicateHandle(source process, source, target process, &target, access, inherit, options)`, between any two
/// processes: the new handle goes in the target's table, and its value back to the caller.
pub fn duplicate_handle(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const DUPLICATE_CLOSE_SOURCE: u64 = 1;
    let [source_process, source, target_process, out, _, inherit, options] = [a[0], a[1], a[2], a[3], a[4], a[5], a[6]];
    let (Some(from), Some(to)) = (task.process_of(source_process), task.process_of(target_process)) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    // The pseudo-handles mean the source process's own self and thread; only the caller's are known here.
    let handle = match source {
        CURRENT_PROCESS if Arc::ptr_eq(&from, &task.process) => task.object(source).map(Handle::Object),
        CURRENT_THREAD if Arc::ptr_eq(&from, &task.process) => task.object(source).map(Handle::Object),
        CURRENT_PROCESS => Some(Handle::Object(from.object.clone())),
        _ => lock(&from.state).handles.get(source),
    };
    let Some(handle) = handle else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    let new = lock(&to.state).handles.add_with(handle, inherit as u32 != 0);
    if options & DUPLICATE_CLOSE_SOURCE != 0 {
        lock(&from.state).handles.close(source);
    }
    if out != 0 {
        let _ = task.write_ptr(cx, out, new);
    }
    Reply::Value(TRUE)
}

// Waitable timers.

/// `CreateWaitableTimerW(attributes, manual, name)`.
pub fn create_waitable_timer_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.name(cx, a[2], true);
    task.create_object(cx, a[0], name, Kind::Timer { manual: a[1] as u32 != 0 }, State::default())
}

/// `CreateWaitableTimerA(attributes, manual, name)`.
pub fn create_waitable_timer_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.name(cx, a[2], false);
    task.create_object(cx, a[0], name, Kind::Timer { manual: a[1] as u32 != 0 }, State::default())
}

/// `CreateWaitableTimerExW(attributes, name, flags, access)`: `CREATE_WAITABLE_TIMER_MANUAL_RESET` is 1.
pub fn create_waitable_timer_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.name(cx, a[1], true);
    task.create_object(cx, a[0], name, Kind::Timer { manual: a[2] as u32 & 1 != 0 }, State::default())
}

pub fn create_waitable_timer_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.name(cx, a[1], false);
    task.create_object(cx, a[0], name, Kind::Timer { manual: a[2] as u32 & 1 != 0 }, State::default())
}

pub fn open_waitable_timer_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.open_object(cx, a[0], a[2], |k| matches!(k, Kind::Timer { .. }), &crate::security::TIMER)
}

pub fn open_waitable_timer_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.open_named(cx, a[0], a[2], false, |k| matches!(k, Kind::Timer { .. }), &crate::security::TIMER)
}

/// Arms a timer's expiry at `when`, for setting number `generation`: it sets the timer, queues its APC, and arms the
/// next if it's periodic.
fn arm(k: std::sync::Weak<crate::Kernel>, timer: std::sync::Weak<Object>, generation: i64, when: std::time::Instant, period_ms: u64, apc: Option<(std::sync::Weak<crate::threads::ThreadControl>, u64, u64)>) {
    let Some(kernel) = k.upgrade() else { return };
    let timers = kernel.timers.clone();
    drop(kernel);
    timers.at(when, move || {
        let (Some(kernel), Some(object)) = (k.upgrade(), timer.upgrade()) else { return };
        let current = kernel.objects.signal(&object, |s| {
            if s.count != generation {
                return false;
            }
            s.set = true;
            true
        });
        if !current {
            return;
        }
        if let Some((thread, routine, argument)) = &apc
            && let Some(thread) = thread.upgrade()
        {
            // The APC gets its argument and the time it went off, as a FILETIME's two halves.
            let now = crate::files::filetime(kernel.host.wall());
            lock(&thread.apcs).push_back((*routine, vec![*argument, now & 0xffff_ffff, now >> 32]));
            thread.waiter.wake();
        }
        drop(kernel);
        if period_ms > 0 {
            arm(k, timer, generation, when + std::time::Duration::from_millis(period_ms), period_ms, apc);
        }
    });
}

/// `SetWaitableTimer(timer, &due, period, routine, argument, resume)`: due in 100 ns units, negative from now or
/// positive as a FILETIME; then every `period` milliseconds, if that's not 0. Setting it unsets it first.
pub fn set_waitable_timer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(timer) = task.object(a[0]).filter(|o| matches!(o.kind, Kind::Timer { .. })) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    let Ok(due) = task.mem(cx, false, |t| user::read_u64(&t.space(), a[1])) else {
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Reply::Value(FALSE);
    };
    let due = due as i64;
    let from_now = if due < 0 {
        due.unsigned_abs() * 100
    } else {
        let now = crate::files::filetime(task.k.host.wall()) as i64;
        (due - now).max(0) as u64 * 100
    };
    let when = std::time::Instant::now() + std::time::Duration::from_nanos(from_now);
    let generation = task.k.objects.signal(&timer, |s| {
        s.set = false;
        s.count += 1;
        s.count
    });
    let apc = (a[3] != 0).then(|| (Arc::downgrade(&task.control), a[3], a[4]));
    arm(Arc::downgrade(&task.k), Arc::downgrade(&timer), generation, when, u64::from(a[2] as u32), apc);
    task.set_last_error(cx, 0);
    Reply::Value(TRUE)
}

/// `SetWaitableTimerEx(timer, &due, period, routine, argument, wake context, tolerable delay)`.
pub fn set_waitable_timer_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_waitable_timer(task, cx, &a[..5])
}

/// `CancelWaitableTimer`: its expiries to come don't happen; whether it's set stays as it is.
pub fn cancel_waitable_timer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(timer) = task.object(a[0]).filter(|o| matches!(o.kind, Kind::Timer { .. })) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(FALSE);
    };
    task.k.objects.signal(&timer, |s| s.count += 1);
    Reply::Value(TRUE)
}
