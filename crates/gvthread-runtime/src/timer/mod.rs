//! Timer subsystem for GVThread runtime
//!
//! Provides abstracted timer functionality with pluggable backends.
//! MVP uses BinaryHeap; future implementations can use timing wheels,
//! kernel timerfd, or io_uring.
//!
//! # Architecture
//!
//! ```text
//!                     TimerRegistry (API)
//!                           │
//!                           ▼
//!               ┌───────────────────────┐
//!               │   dyn TimerBackend    │  ◄── Trait abstraction
//!               └───────────────────────┘
//!                           │
//!          ┌────────────────┼────────────────┐
//!          ▼                ▼                ▼
//!    HeapBackend      WheelBackend     KernelBackend
//!       (MVP)           (future)         (future)
//!          │
//!          ▼
//!     TimerThread ──poll_expired()──► ReadyQueue.wake(gvt_id, affinity)
//! ```

mod entry;
pub mod impls;
mod registry;
mod worker;

pub use entry::{TimerEntry, TimerHandle, TimerType};
pub use impls::{create_backend, TimerBackendType};
pub use registry::TimerRegistry;
pub use worker::{spawn_timer_thread, TimerThreadConfig};

use std::time::Instant;

/// Expired timer info passed to ready queue
#[derive(Debug, Clone)]
pub struct ExpiredTimer {
    pub gvt_id: u32,
    pub worker_affinity: Option<u8>,
    pub timer_type: TimerType,
}

/// Core timer trait - implement this for different backends
///
/// All implementations must be thread-safe (Send + Sync) as the timer
/// thread and worker threads may interact concurrently.
pub trait TimerBackend: Send + Sync {
    /// Insert a timer entry, returns handle for cancellation
    fn insert(&self, entry: TimerEntry) -> TimerHandle;

    /// Cancel a timer by handle (best-effort, may already have fired)
    /// Returns true if timer was found and cancelled
    fn cancel(&self, handle: TimerHandle) -> bool;

    /// Poll for expired timers, returns expired entries to wake
    /// Called by timer thread - should be non-blocking or very fast
    fn poll_expired(&self, now: Instant) -> Vec<ExpiredTimer>;

    /// Hint: when is the next timer due? (for smart sleeping)
    /// Returns None if no timers are scheduled
    fn next_deadline(&self) -> Option<Instant>;

    /// Number of active timers (for metrics)
    fn len(&self) -> usize;

    /// Check if no timers are scheduled
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Optional: backend name for debugging/metrics
    fn name(&self) -> &'static str {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_timer_handle_uniqueness() {
        let h1 = TimerHandle::new();
        let h2 = TimerHandle::new();
        let h3 = TimerHandle::new();

        assert_ne!(h1, h2);
        assert_ne!(h2, h3);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_timer_entry_creation() {
        let entry = TimerEntry::preempt(42, 3, Duration::from_millis(10));
        assert_eq!(entry.gvt_id, 42);
        assert_eq!(entry.worker_affinity, Some(3));
        assert!(matches!(entry.timer_type, TimerType::Preempt));
    }

    #[test]
    fn test_heap_backend_basic() {
        use impls::HeapTimerBackend;

        let backend = HeapTimerBackend::new();
        assert!(backend.is_empty());

        let entry = TimerEntry::sleep(1, Duration::from_millis(100), None);
        let handle = backend.insert(entry);

        assert_eq!(backend.len(), 1);
        assert!(backend.next_deadline().is_some());

        // Cancel
        assert!(backend.cancel(handle));
        
        // Poll should skip cancelled
        let expired = backend.poll_expired(Instant::now() + Duration::from_secs(1));
        assert!(expired.is_empty());
    }
}