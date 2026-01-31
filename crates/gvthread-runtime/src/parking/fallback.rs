//! Fallback parking using std::sync::Condvar
//!
//! Used on platforms without futex support.
//! Less efficient but portable.

use super::WorkerParking;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Condvar-based parking (fallback)
pub struct FallbackParking {
    /// Mutex for condvar
    mutex: Mutex<bool>,  // bool = wake_pending
    
    /// Condition variable
    condvar: Condvar,
    
    /// Count of parked workers
    parked: AtomicUsize,
}

impl FallbackParking {
    /// Create a new fallback parking instance
    pub fn new() -> Self {
        Self {
            mutex: Mutex::new(false),
            condvar: Condvar::new(),
            parked: AtomicUsize::new(0),
        }
    }
}

impl Default for FallbackParking {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerParking for FallbackParking {
    fn park(&self, timeout: Option<Duration>) -> bool {
        self.parked.fetch_add(1, Ordering::SeqCst);
        
        let mut guard = self.mutex.lock().unwrap();
        
        // Check if wake is already pending
        if *guard {
            *guard = false; // Consume the wake
            self.parked.fetch_sub(1, Ordering::SeqCst);
            return true;
        }
        
        // Wait on condvar
        let result = match timeout {
            Some(t) => {
                let (g, timeout_result) = self.condvar.wait_timeout(guard, t).unwrap();
                guard = g;
                !timeout_result.timed_out()
            }
            None => {
                guard = self.condvar.wait(guard).unwrap();
                true
            }
        };
        
        // Consume wake flag if set
        if *guard {
            *guard = false;
        }
        
        self.parked.fetch_sub(1, Ordering::SeqCst);
        result
    }
    
    fn wake_one(&self) {
        if self.parked.load(Ordering::Acquire) == 0 {
            return;
        }
        
        {
            let mut guard = self.mutex.lock().unwrap();
            *guard = true;
        }
        self.condvar.notify_one();
    }
    
    fn wake_all(&self) {
        if self.parked.load(Ordering::Acquire) == 0 {
            return;
        }
        
        {
            let mut guard = self.mutex.lock().unwrap();
            *guard = true;
        }
        self.condvar.notify_all();
    }
    
    fn parked_count(&self) -> usize {
        self.parked.load(Ordering::Relaxed)
    }
}