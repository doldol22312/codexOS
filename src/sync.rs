#![allow(dead_code)]

use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::task;

pub struct Mutex<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            task::yield_now();
            spin_loop();
        }
    }

    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(MutexGuard { mutex: self })
        } else {
            None
        }
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}

pub struct Semaphore {
    permits: AtomicU32,
}

impl Semaphore {
    pub const fn new(initial_permits: u32) -> Self {
        Self {
            permits: AtomicU32::new(initial_permits),
        }
    }

    pub fn available_permits(&self) -> u32 {
        self.permits.load(Ordering::Acquire)
    }

    pub fn try_acquire(&self) -> bool {
        loop {
            let current = self.permits.load(Ordering::Acquire);
            if current == 0 {
                return false;
            }

            if self
                .permits
                .compare_exchange_weak(
                    current,
                    current - 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return true;
            }
        }
    }

    pub fn acquire(&self) {
        while !self.try_acquire() {
            task::yield_now();
            spin_loop();
        }
    }

    pub fn release(&self) {
        self.permits.fetch_add(1, Ordering::Release);
    }

    pub fn add_permits(&self, count: u32) {
        if count == 0 {
            return;
        }
        self.permits.fetch_add(count, Ordering::Release);
    }
}
