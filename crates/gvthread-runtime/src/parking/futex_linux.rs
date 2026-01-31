//! Linux futex-based worker parking
//!
//! Uses the futex syscall for efficient sleep/wake with minimal overhead.
//! 
//! Futex word semantics:
//! - 0 = no wake pending
//! - 1 = wake pending (workers should check for work)
//!
//! When a worker parks:
//! 1. Increment parked count
//! 2. FUTEX_WAIT on futex word (blocks if word == 0)
//! 3. Decrement parked count on return
//!
//! When waking:
//! 1. Set futex word to 1
//! 2. FUTEX_WAKE to wake N waiters
//! 3. Reset futex word to 0

use super::WorkerParking;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

/// Linux futex-based parking
pub struct FutexParking {
    /// Futex word: 0 = sleep, 1 = wake pending
    futex: AtomicU32,
    
    /// Count of parked workers (for optimization)
    parked: AtomicUsize,
}

impl FutexParking {
    /// Create a new futex parking instance
    pub fn new() -> Self {
        Self {
            futex: AtomicU32::new(0),
            parked: AtomicUsize::new(0),
        }
    }
}

impl Default for FutexParking {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerParking for FutexParking {
    fn park(&self, timeout: Option<Duration>) -> bool {
        // Increment parked count
        self.parked.fetch_add(1, Ordering::SeqCst);
        
        // Check if wake is already pending
        if self.futex.load(Ordering::Acquire) != 0 {
            // Wake pending, don't sleep
            self.parked.fetch_sub(1, Ordering::SeqCst);
            // Reset the wake flag (we consumed it)
            self.futex.store(0, Ordering::Release);
            return true;
        }
        
        // Prepare timeout
        let timespec = timeout.map(|d| libc::timespec {
            tv_sec: d.as_secs() as i64,
            tv_nsec: d.subsec_nanos() as i64,
        });
        
        let timespec_ptr = match &timespec {
            Some(ts) => ts as *const libc::timespec,
            None => std::ptr::null(),
        };
        
        // FUTEX_WAIT: sleep if futex == 0
        let result = unsafe {
            libc::syscall(
                libc::SYS_futex,
                self.futex.as_ptr(),
                libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                0u32,           // Expected value (sleep if futex == 0)
                timespec_ptr,   // Timeout
                std::ptr::null::<u32>(), // uaddr2 (unused)
                0u32,           // val3 (unused)
            )
        };
        
        // Decrement parked count
        self.parked.fetch_sub(1, Ordering::SeqCst);
        
        // Result: 0 = woken, -1 = error (timeout or spurious)
        if result == 0 {
            true // Woken by FUTEX_WAKE
        } else {
            let errno = unsafe { *libc::__errno_location() };
            // ETIMEDOUT = timeout, EAGAIN = value changed (spurious), EINTR = signal
            // All are "not woken by wake_one/wake_all"
            errno != libc::ETIMEDOUT && errno != libc::EAGAIN && errno != libc::EINTR
        }
    }
    
    fn wake_one(&self) {
        // Check if any workers are parked
        if self.parked.load(Ordering::Acquire) == 0 {
            return; // No one to wake
        }
        
        // Set wake flag
        self.futex.store(1, Ordering::Release);
        
        // FUTEX_WAKE: wake 1 waiter
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                self.futex.as_ptr(),
                libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
                1i32,           // Wake at most 1 waiter
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0u32,
            );
        }
    }
    
    fn wake_all(&self) {
        // Check if any workers are parked
        if self.parked.load(Ordering::Acquire) == 0 {
            return; // No one to wake
        }
        
        // Set wake flag
        self.futex.store(1, Ordering::Release);
        
        // FUTEX_WAKE: wake all waiters
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                self.futex.as_ptr(),
                libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
                i32::MAX,       // Wake all waiters
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0u32,
            );
        }
    }
    
    fn parked_count(&self) -> usize {
        self.parked.load(Ordering::Relaxed)
    }
}

// Safety: FutexParking only contains atomics
unsafe impl Send for FutexParking {}
unsafe impl Sync for FutexParking {}