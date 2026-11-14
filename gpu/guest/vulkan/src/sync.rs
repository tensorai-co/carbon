//! A lock with no C library under it: the driver's few shared things (a device's records not yet sent) are held for
//! moments, so a spin is enough.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct Spin<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Spin<T> {}

impl<T> Spin<T> {
    pub(crate) const fn new(value: T) -> Spin<T> {
        Spin { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    pub(crate) fn lock(&self) -> Guard<'_, T> {
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        Guard { spin: self }
    }
}

pub(crate) struct Guard<'a, T> {
    spin: &'a Spin<T>,
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.spin.value.get() }
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.spin.value.get() }
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        self.spin.locked.store(false, Ordering::Release);
    }
}
