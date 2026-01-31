//! GVThread-aware mutex
//!
//! Unlike std::sync::Mutex, this mutex yields to the scheduler
//! when contended instead of blocking the OS thread.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::VecDeque;
use crate::id::GVThreadId;
use crate::spinlock::SpinLock;
use crate::error::SchedResult;

/// A mutex that yields to the scheduler when contended
///
/// This mutex is designed for use within GVThreads. When a GVThread
/// tries to acquire a locked mutex, it yields to the scheduler instead
/// of blocking the OS thread.
///
/// # Example
///
/// ```ignore
/// let mutex = SchedMutex::new(0);
///
/// // In a GVThread:
/// {
///     let mut guard = mutex.lock(&token)?;
///     *guard += 1;
/// } // Guard dropped, mutex unlocked
/// ```
pub struct SchedMutex<T> {
    /// Lock state
    locked: AtomicBool,
    
    /// Protected data
    data: UnsafeCell<T>,
    
    /// Queue of waiting GVThreads (FIFO for fairness)
    waiters: SpinLock<VecDeque<GVThreadId>>,
}

// Safety: SchedMutex provides exclusive access to T
unsafe impl<T: Send> Send for SchedMutex<T> {}
unsafe impl<T: Send> Sync for SchedMutex<T> {}

impl<T> SchedMutex<T> {
    /// Create a new mutex containing the given value
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(value),
            waiters: SpinLock::new(VecDeque::new()),
        }
    }
    
    /// Acquire the lock, yielding to scheduler if contended
    ///
    /// Returns a guard that releases the lock when dropped.
    pub fn lock(&self) -> SchedResult<SchedMutexGuard<'_, T>> {
        // Fast path: try to acquire immediately
        if self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return Ok(SchedMutexGuard { mutex: self });
        }
        
        // Slow path: need to wait
        self.lock_slow()
    }
    
    fn lock_slow(&self) -> SchedResult<SchedMutexGuard<'_, T>> {
        // In real implementation:
        // 1. Add current GVThread to waiters
        // 2. Yield to scheduler
        // 3. When woken, we own the lock
        
        // For now, spin with yield (placeholder until scheduler integration)
        loop {
            if self.locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(SchedMutexGuard { mutex: self });
            }
            
            // Yield to scheduler (placeholder: just yield OS thread)
            std::thread::yield_now();
            std::hint::spin_loop();
        }
    }
    
    /// Try to acquire the lock without blocking
    pub fn try_lock(&self) -> Option<SchedMutexGuard<'_, T>> {
        if self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SchedMutexGuard { mutex: self })
        } else {
            None
        }
    }
    
    /// Check if the mutex is currently locked
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
    
    /// Get mutable access to the underlying data
    ///
    /// This requires mutable access to the mutex, guaranteeing no other
    /// references exist.
    pub fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }
    
    /// Consume the mutex and return the inner value
    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
    
    fn unlock(&self) {
        // Check if there are waiters
        let waiter = {
            let mut waiters = self.waiters.lock();
            waiters.pop_front()
        };
        
        if let Some(_waiter) = waiter {
            // Transfer ownership to waiter
            // TODO: Wake waiter via scheduler
            // scheduler.mark_ready(waiter);
            // Note: We don't clear locked because waiter now owns it
        } else {
            // No waiters, just unlock
            self.locked.store(false, Ordering::Release);
        }
    }
}

impl<T: Default> Default for SchedMutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for SchedMutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.try_lock() {
            Some(guard) => f.debug_struct("SchedMutex")
                .field("data", &*guard)
                .finish(),
            None => f.debug_struct("SchedMutex")
                .field("data", &"<locked>")
                .finish(),
        }
    }
}

/// Guard that releases the mutex when dropped
pub struct SchedMutexGuard<'a, T> {
    mutex: &'a SchedMutex<T>,
}

impl<'a, T> Deref for SchedMutexGuard<'a, T> {
    type Target = T;
    
    fn deref(&self) -> &T {
        // Safety: We hold the lock
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> DerefMut for SchedMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        // Safety: We hold the lock
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T> Drop for SchedMutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    
    #[test]
    fn test_basic_lock() {
        let mutex = SchedMutex::new(0);
        
        {
            let mut guard = mutex.lock().unwrap();
            *guard = 42;
        }
        
        {
            let guard = mutex.lock().unwrap();
            assert_eq!(*guard, 42);
        }
    }
    
    #[test]
    fn test_try_lock() {
        let mutex = SchedMutex::new(0);
        
        let guard = mutex.try_lock();
        assert!(guard.is_some());
        
        // While held, try_lock should fail
        let guard2 = mutex.try_lock();
        assert!(guard2.is_none());
        
        drop(guard);
        
        // Now should succeed
        let guard3 = mutex.try_lock();
        assert!(guard3.is_some());
    }
    
    #[test]
    fn test_concurrent() {
        let mutex = Arc::new(SchedMutex::new(0));
        let mut handles = vec![];
        
        for _ in 0..4 {
            let mutex = Arc::clone(&mutex);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    let mut guard = mutex.lock().unwrap();
                    *guard += 1;
                }
            }));
        }
        
        for h in handles {
            h.join().unwrap();
        }
        
        let guard = mutex.lock().unwrap();
        assert_eq!(*guard, 4000);
    }
    
    #[test]
    fn test_into_inner() {
        let mutex = SchedMutex::new(42);
        let value = mutex.into_inner();
        assert_eq!(value, 42);
    }
}
