//! Linux futex-based worker parking
//!
//! Uses the futex syscall for efficient sleep/wake with minimal overhead.
//! 
//! Design: Simple counter-based approach
//! - futex word represents "pending wakes" count
//! - wake_one() increments and wakes
//! - park() decrements (if > 0) or waits

use super::WorkerParking;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

/// Linux futex-based parking
pub struct FutexParking {
    /// Futex word: count of pending wakes
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
        // Try to consume a pending wake first (fast path)
        loop {
            let current = self.futex.load(Ordering::Acquire);
            if current > 0 {
                // Try to consume a wake
                if self.futex.compare_exchange_weak(
                    current,
                    current - 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ).is_ok() {
                    return true; // Consumed a pending wake
                }
                // CAS failed, retry
                continue;
            }
            break;
        }
        
        // No pending wakes, need to actually wait
        self.parked.fetch_add(1, Ordering::SeqCst);
        
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
        
        self.parked.fetch_sub(1, Ordering::SeqCst);
        
        // If futex value changed (EAGAIN), try to consume the wake
        if result != 0 {
            // Try to consume any pending wake
            loop {
                let current = self.futex.load(Ordering::Acquire);
                if current > 0 {
                    if self.futex.compare_exchange_weak(
                        current,
                        current - 1,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ).is_ok() {
                        return true;
                    }
                    continue;
                }
                break;
            }
        }
        
        result == 0
    }
    
    fn wake_one(&self) {
        // Increment pending wakes
        self.futex.fetch_add(1, Ordering::Release);
        
        // Wake one waiter if any are parked
        if self.parked.load(Ordering::Acquire) > 0 {
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
    }
    
    fn wake_all(&self) {
        // Increment pending wakes by a large amount
        let parked = self.parked.load(Ordering::Acquire);
        if parked == 0 {
            return;
        }
        
        self.futex.fetch_add(parked as u32 + 1, Ordering::Release);
        
        // Wake all waiters
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