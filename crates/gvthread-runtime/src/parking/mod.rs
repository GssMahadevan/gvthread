//! Worker parking mechanism
//!
//! Provides efficient sleep/wake for idle workers.
//! Platform-specific implementations use the most efficient primitive available.

use std::time::Duration;

/// Platform-specific worker parking mechanism
/// 
/// Workers call `park()` when no work is available.
/// Wake sources (spawn, wake_gvthread, timer) call `wake_one()` or `wake_all()`.
pub trait WorkerParking: Send + Sync {
    /// Park the current worker until signaled or timeout
    /// 
    /// Returns:
    /// - `true` if woken by signal
    /// - `false` if timeout or spurious wakeup
    /// 
    /// Workers should re-check for work after returning regardless of return value.
    fn park(&self, timeout: Option<Duration>) -> bool;
    
    /// Wake one parked worker
    /// 
    /// If no workers are parked, the signal may be lost (not queued).
    /// This is fine - it means workers are busy.
    fn wake_one(&self);
    
    /// Wake all parked workers
    /// 
    /// Used for shutdown or when many GVThreads become ready at once.
    fn wake_all(&self);
    
    /// Number of currently parked workers (hint, may be stale)
    /// 
    /// Can be used to skip wake calls when no workers are parked.
    fn parked_count(&self) -> usize;
}

// Platform-specific implementations
cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        mod futex_linux;
        pub use futex_linux::FutexParking as PlatformParking;
    } else {
        // Fallback for other platforms - to be implemented
        mod fallback;
        pub use fallback::FallbackParking as PlatformParking;
    }
}

/// Create a new platform-appropriate parking instance
pub fn new_parking() -> Box<dyn WorkerParking> {
    Box::new(PlatformParking::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    
    #[test]
    fn test_park_timeout() {
        let parking = new_parking();
        let start = std::time::Instant::now();
        let result = parking.park(Some(Duration::from_millis(50)));
        let elapsed = start.elapsed();
        
        // Should timeout (false) and take ~50ms
        assert!(!result || elapsed < Duration::from_millis(100));
        assert!(elapsed >= Duration::from_millis(40)); // Allow some slack
    }
    
    #[test]
    fn test_wake_one() {
        let parking = Arc::new(PlatformParking::new());
        let parking2 = Arc::clone(&parking);
        
        let handle = thread::spawn(move || {
            // Park with long timeout
            parking2.park(Some(Duration::from_secs(10)))
        });
        
        // Give thread time to park
        thread::sleep(Duration::from_millis(50));
        
        // Wake it
        parking.wake_one();
        
        // Should complete quickly
        let result = handle.join().unwrap();
        assert!(result); // Woken by signal
    }
}