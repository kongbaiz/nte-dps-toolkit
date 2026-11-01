//! The one place that recovers poisoned locks. A poisoned lock means a panic
//! interrupted an update; each caller supplies `repair`, which replaces the
//! possibly partial value with a consistent one. The returned flag is `true`
//! when a repair ran, so callers can surface errors, notify waiters or log.

use std::sync::{Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub fn lock_or_repair<T>(
    mutex: &Mutex<T>,
    repair: impl FnOnce(&mut T),
) -> (MutexGuard<'_, T>, bool) {
    match mutex.lock() {
        Ok(guard) => (guard, false),
        Err(poison) => {
            let mut guard = poison.into_inner();
            repair(&mut guard);
            mutex.clear_poison();
            (guard, true)
        }
    }
}

/// `Condvar::wait` with the same recovery as [`lock_or_repair`].
pub fn wait_or_repair<'a, T>(
    condvar: &Condvar,
    mutex: &Mutex<T>,
    guard: MutexGuard<'a, T>,
    repair: impl FnOnce(&mut T),
) -> (MutexGuard<'a, T>, bool) {
    match condvar.wait(guard) {
        Ok(guard) => (guard, false),
        Err(poison) => {
            let mut guard = poison.into_inner();
            repair(&mut guard);
            mutex.clear_poison();
            (guard, true)
        }
    }
}

pub fn write_or_repair<T>(
    lock: &RwLock<T>,
    repair: impl FnOnce(&mut T),
) -> (RwLockWriteGuard<'_, T>, bool) {
    match lock.write() {
        Ok(guard) => (guard, false),
        Err(poison) => {
            let mut guard = poison.into_inner();
            repair(&mut guard);
            lock.clear_poison();
            (guard, true)
        }
    }
}

/// Readers cannot repair in place, so a poisoned read repairs through a write
/// guard and then reads again.
pub fn read_or_repair<T>(lock: &RwLock<T>, repair: impl Fn(&mut T)) -> RwLockReadGuard<'_, T> {
    loop {
        match lock.read() {
            Ok(guard) => return guard,
            Err(poison) => {
                drop(poison);
                drop(write_or_repair(lock, &repair));
            }
        }
    }
}
