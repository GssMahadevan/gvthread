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
pub use impls::{create_backend, HeapTimerBackend, TimerBackendType};
pub use registry::TimerRegistry;
pub use worker::{spawn_timer_thread, TimerThreadConfig, TimerThreadHandle, TimerWakeCallback};

use std::time::{Duration, Instant};

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

// ============================================================================
// TimerThread - High-level wrapper for scheduler compatibility
// ============================================================================

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// TimerThread wraps the timer subsystem for easy use by scheduler
/// 
/// Provides the old API: `new()` -> `start()` -> `shutdown()` -> `join()`
pub struct TimerThread {
    /// Timer backend
    backend: Arc<HeapTimerBackend>,
    /// Timer registry for scheduling
    registry: Option<TimerRegistry>,
    /// Thread handle (after start)
    handle: Option<TimerThreadHandle>,
    /// Shutdown signal
    shutdown: Arc<AtomicBool>,
    /// Configuration
    config: TimerThreadConfig,
}

impl TimerThread {
    /// Create a new TimerThread (does not start yet)
    pub fn new<C>(_config: &C) -> Self {
        Self {
            backend: Arc::new(HeapTimerBackend::new()),
            registry: None,
            handle: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            config: TimerThreadConfig::default(),
        }
    }
    
    /// Start the timer thread
    /// 
    /// For now, uses a no-op callback. Full integration will wire this
    /// to the ready queue.
    pub fn start(&mut self, _num_workers: usize, _max_gvthreads: usize) {
        // Create registry
        self.registry = Some(TimerRegistry::new(self.backend.clone()));
        
        // Create a no-op callback for now
        // TODO: Wire to ready queue when integrating with scheduler
        let callback = Arc::new(NoOpWakeCallback);
        
        // Spawn the timer thread
        let handle = spawn_timer_thread(
            self.backend.clone(),
            callback,
            self.shutdown.clone(),
            self.config.clone(),
        );
        
        self.handle = Some(handle);
    }
    
    /// Get the timer registry for scheduling timers
    pub fn registry(&self) -> Option<&TimerRegistry> {
        self.registry.as_ref()
    }
    
    /// Signal shutdown (non-blocking)
    pub fn shutdown(&self) {
        self.shutdown.store(true, std::sync::atomic::Ordering::Release);
        if let Some(ref handle) = self.handle {
            handle.shutdown();
        }
    }
    
    /// Wait for timer thread to exit
    pub fn join(&mut self) {
        if let Some(mut handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
    
    /// Shutdown and join in one call
    pub fn stop(&mut self) {
        self.shutdown();
        self.join();
    }
}

/// No-op wake callback (placeholder until ready queue integration)
struct NoOpWakeCallback;

impl TimerWakeCallback for NoOpWakeCallback {
    fn on_timer_expired(&self, _expired: ExpiredTimer) {
        // TODO: Wire to ready queue
        // For now, do nothing - sleep/preemption not yet integrated
    }
}

// ============================================================================
// Time utilities
// ============================================================================

/// Get current monotonic time in nanoseconds
///
/// Uses a process-wide start point for consistent measurements.
/// This is cheaper than syscalls for relative timing.
#[inline]
pub fn now_ns() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos() as u64
}

/// Get current monotonic time in microseconds
#[inline]
pub fn now_us() -> u64 {
    now_ns() / 1_000
}

/// Get current monotonic time in milliseconds
#[inline]
pub fn now_ms() -> u64 {
    now_ns() / 1_000_000
}

// ============================================================================
// Initialization
// ============================================================================

/// Initialize sleep queue with capacity
///
/// With the new timer backend design, this is a no-op.
/// The backend manages its own storage. Kept for API compatibility.
#[inline]
pub fn init_sleep_queue_with_capacity(_capacity: usize) {
    // No-op: HeapTimerBackend grows dynamically
    // Future backends may use this hint for pre-allocation
}

// ============================================================================
// Sleep API
// ============================================================================

/// Sleep the current GVThread for the specified duration
///
/// This will:
/// 1. Register a sleep timer with the timer subsystem
/// 2. Yield the current GVThread to the scheduler
/// 3. Wake when the timer expires
///
/// # Note
///
/// Currently falls back to OS thread sleep until full scheduler
/// integration is complete. The proper implementation requires:
/// - Getting current GVT ID from TLS
/// - Registering with TimerRegistry
/// - Yielding to scheduler
pub fn sleep(duration: Duration) {
    // TODO: Full integration with scheduler
    // let gvt_id = crate::tls::current_gvthread_id();
    // let worker_id = crate::tls::current_worker_id();
    // get_timer_registry().schedule_sleep(gvt_id, duration, Some(worker_id));
    // crate::scheduler::yield_current();
    
    // Temporary: OS thread sleep
    std::thread::sleep(duration);
}

/// Sleep for the specified number of milliseconds
#[inline]
pub fn sleep_ms(ms: u64) {
    sleep(Duration::from_millis(ms))
}

/// Sleep for the specified number of microseconds
#[inline]
pub fn sleep_us(us: u64) {
    sleep(Duration::from_micros(us))
}

/// Sleep for the specified number of nanoseconds
#[inline]
pub fn sleep_ns(ns: u64) {
    sleep(Duration::from_nanos(ns))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_now_ns_monotonic() {
        let t1 = now_ns();
        std::thread::sleep(Duration::from_micros(100));
        let t2 = now_ns();
        assert!(t2 > t1);
    }

    #[test]
    fn test_init_sleep_queue_noop() {
        // Should not panic
        init_sleep_queue_with_capacity(1000);
        init_sleep_queue_with_capacity(0);
    }
}