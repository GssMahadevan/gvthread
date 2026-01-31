//! Internal spinlock for scheduler synchronization
//!
//! This is a simple spinlock used internally by the scheduler.
//! NOT intended for use by GVThreads (use SchedMutex instead).

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A simple spinlock
///
/// This spinlock is designed for short critical sections in the scheduler.
/// It spins in a loop waiting for the lock, with a pause instruction hint.
///
/// # Warning
///
/// Do not use this from GVThread code! Use `SchedMutex` instead, which
/// properly yields to the scheduler when contended.
pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// Safety: SpinLock provides exclusive access to T
unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Create a new spinlock containing the given value
    #[inline]
    pub const fn new(value: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(value),
        }
    }
    
    /// Acquire the lock, spinning until it's available
    #[inline]
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        loop {
            // Try to acquire with weak CAS (can spuriously fail, but faster)
            if self.locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return SpinLockGuard { lock: self };
            }
            
            // Spin with backoff
            let mut spin_count = 0u32;
            while self.locked.load(Ordering::Relaxed) {
                spin_count = spin_count.wrapping_add(1);
                
                // Exponential backoff with pause hints
                for _ in 0..spin_count.min(64) {
                    core::hint::spin_loop();
                }
            }
        }
    }
    
    /// Try to acquire the lock without spinning
    #[inline]
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        if self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinLockGuard { lock: self })
        } else {
            None
        }
    }
    
    /// Get mutable access without locking (unsafe)
    ///
    /// # Safety
    ///
    /// Caller must ensure exclusive access to the lock.
    #[inline]
    pub unsafe fn get_mut_unchecked(&self) -> &mut T {
        &mut *self.data.get()
    }
    
    /// Check if the lock is currently held
    #[inline]
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

impl<T: Default> Default for SpinLock<T> {
    fn default() -> Self {
        SpinLock::new(T::default())
    }
}

/// Guard that releases the spinlock when dropped
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<'a, T> Deref for SpinLockGuard<'a, T> {
    type Target = T;
    
    #[inline]
    fn deref(&self) -> &T {
        // Safety: We hold the lock
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for SpinLockGuard<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // Safety: We hold the lock
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    
    #[test]
    fn test_spinlock_basic() {
        let lock = SpinLock::new(0u32);
        {
            let mut guard = lock.lock();
            *guard = 42;
        }
        {
            let guard = lock.lock();
            assert_eq!(*guard, 42);
        }
    }
    
    #[test]
    fn test_spinlock_try_lock() {
        let lock = SpinLock::new(0u32);
        
        let guard = lock.try_lock();
        assert!(guard.is_some());
        
        // While held, try_lock should fail
        let guard2 = lock.try_lock();
        assert!(guard2.is_none());
        
        drop(guard);
        
        // Now should succeed
        let guard3 = lock.try_lock();
        assert!(guard3.is_some());
    }
    
    #[test]
    fn test_spinlock_concurrent() {
        let lock = Arc::new(SpinLock::new(0u32));
        let mut handles = vec![];
        
        for _ in 0..4 {
            let lock = Arc::clone(&lock);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    let mut guard = lock.lock();
                    *guard += 1;
                }
            }));
        }
        
        for h in handles {
            h.join().unwrap();
        }
        
        let guard = lock.lock();
        assert_eq!(*guard, 4000);
    }
}
