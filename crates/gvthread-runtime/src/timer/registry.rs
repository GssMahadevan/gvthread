//! TimerRegistry - High-level timer API for GVThread runtime
//!
//! Provides ergonomic methods for common timer operations while
//! abstracting over the underlying backend implementation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::timer::{TimerBackend, TimerEntry, TimerHandle, TimerType};

/// High-level timer API used by GVThread runtime
///
/// Wraps a `TimerBackend` and provides convenient methods for common
/// timer operations. Thread-safe via Arc.
///
/// # Example
///
/// ```ignore
/// use gvthread::timer::{TimerRegistry, impls::HeapTimerBackend};
/// use std::sync::Arc;
/// use std::time::Duration;
///
/// let backend = Arc::new(HeapTimerBackend::new());
/// let registry = TimerRegistry::new(backend);
///
/// // Schedule preemption (with worker affinity)
/// let handle = registry.schedule_preempt(gvt_id, worker_id, Duration::from_millis(10));
///
/// // Cancel if GVT yields early
/// registry.cancel(handle);
/// ```
pub struct TimerRegistry {
    backend: Arc<dyn TimerBackend>,
}

impl TimerRegistry {
    /// Create a new timer registry with the given backend
    pub fn new(backend: Arc<dyn TimerBackend>) -> Self {
        Self { backend }
    }

    /// Get a reference to the underlying backend
    ///
    /// Useful for passing to the timer thread.
    pub fn backend(&self) -> Arc<dyn TimerBackend> {
        self.backend.clone()
    }

    /// Get backend name (for logging/metrics)
    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    // ========================================================================
    // Preemption timers
    // ========================================================================

    /// Schedule preemption for a GVT
    ///
    /// The GVT will be marked for preemption after `time_slice` duration.
    /// Worker affinity ensures the GVT returns to the same worker for
    /// cache locality.
    ///
    /// # Arguments
    ///
    /// * `gvt_id` - The GVT to preempt
    /// * `worker_id` - The worker running this GVT (for affinity)
    /// * `time_slice` - Duration before preemption
    ///
    /// # Returns
    ///
    /// Timer handle for cancellation (call when GVT yields voluntarily)
    #[inline]
    pub fn schedule_preempt(
        &self,
        gvt_id: u32,
        worker_id: u8,
        time_slice: Duration,
    ) -> TimerHandle {
        let entry = TimerEntry::preempt(gvt_id, worker_id, time_slice);
        self.backend.insert(entry)
    }

    // ========================================================================
    // Sleep timers
    // ========================================================================

    /// Schedule a GVT sleep
    ///
    /// The GVT will be woken after `duration` and placed back on the
    /// ready queue.
    ///
    /// # Arguments
    ///
    /// * `gvt_id` - The GVT to wake
    /// * `duration` - How long to sleep
    /// * `affinity` - Optional worker affinity (None = any worker)
    #[inline]
    pub fn schedule_sleep(
        &self,
        gvt_id: u32,
        duration: Duration,
        affinity: Option<u8>,
    ) -> TimerHandle {
        let entry = TimerEntry::sleep(gvt_id, duration, affinity);
        self.backend.insert(entry)
    }

    /// Schedule a GVT sleep until absolute deadline
    #[inline]
    pub fn schedule_sleep_until(
        &self,
        gvt_id: u32,
        deadline: Instant,
        affinity: Option<u8>,
    ) -> TimerHandle {
        let entry = TimerEntry::at(gvt_id, deadline, affinity, TimerType::Sleep);
        self.backend.insert(entry)
    }

    // ========================================================================
    // Timeout timers
    // ========================================================================

    /// Schedule a timeout for an async operation
    ///
    /// Used for I/O timeouts, channel receives with timeout, etc.
    /// The timeout fires if the operation doesn't complete in time.
    ///
    /// # Arguments
    ///
    /// * `gvt_id` - The GVT waiting on the operation
    /// * `duration` - Timeout duration
    /// * `affinity` - Optional worker affinity
    #[inline]
    pub fn schedule_timeout(
        &self,
        gvt_id: u32,
        duration: Duration,
        affinity: Option<u8>,
    ) -> TimerHandle {
        let entry = TimerEntry::timeout(gvt_id, duration, affinity);
        self.backend.insert(entry)
    }

    /// Schedule a timeout with absolute deadline
    #[inline]
    pub fn schedule_timeout_at(
        &self,
        gvt_id: u32,
        deadline: Instant,
        affinity: Option<u8>,
    ) -> TimerHandle {
        let entry = TimerEntry::at(gvt_id, deadline, affinity, TimerType::Timeout);
        self.backend.insert(entry)
    }

    // ========================================================================
    // Periodic timers
    // ========================================================================

    /// Schedule a periodic timer
    ///
    /// The timer will fire every `interval` until cancelled.
    /// Useful for heartbeats, status updates, etc.
    ///
    /// # Arguments
    ///
    /// * `gvt_id` - The GVT to wake periodically
    /// * `interval` - Time between firings
    /// * `affinity` - Optional worker affinity
    #[inline]
    pub fn schedule_periodic(
        &self,
        gvt_id: u32,
        interval: Duration,
        affinity: Option<u8>,
    ) -> TimerHandle {
        let entry = TimerEntry::periodic(gvt_id, interval, affinity);
        self.backend.insert(entry)
    }

    // ========================================================================
    // Generic timer
    // ========================================================================

    /// Schedule a custom timer entry
    ///
    /// For cases not covered by the convenience methods.
    #[inline]
    pub fn schedule(&self, entry: TimerEntry) -> TimerHandle {
        self.backend.insert(entry)
    }

    // ========================================================================
    // Cancellation
    // ========================================================================

    /// Cancel a scheduled timer
    ///
    /// Best-effort cancellation - the timer may have already fired.
    /// Returns true if the timer was found and cancelled.
    ///
    /// Common use: Cancel preemption timer when GVT yields voluntarily.
    #[inline]
    pub fn cancel(&self, handle: TimerHandle) -> bool {
        self.backend.cancel(handle)
    }

    // ========================================================================
    // Queries
    // ========================================================================

    /// Number of active (non-cancelled) timers
    #[inline]
    pub fn active_timers(&self) -> usize {
        self.backend.len()
    }

    /// Check if any timers are scheduled
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.backend.is_empty()
    }

    /// Get the next timer deadline (if any)
    #[inline]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.backend.next_deadline()
    }

    /// Time until next timer fires (None if no timers)
    #[inline]
    pub fn time_until_next(&self) -> Option<Duration> {
        self.backend
            .next_deadline()
            .map(|d| d.saturating_duration_since(Instant::now()))
    }
}

impl Clone for TimerRegistry {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
        }
    }
}

impl std::fmt::Debug for TimerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimerRegistry")
            .field("backend", &self.backend.name())
            .field("active_timers", &self.backend.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::impls::HeapTimerBackend;

    fn test_registry() -> TimerRegistry {
        let backend = Arc::new(HeapTimerBackend::new());
        TimerRegistry::new(backend)
    }

    #[test]
    fn test_preempt() {
        let registry = test_registry();

        let handle = registry.schedule_preempt(42, 3, Duration::from_millis(10));
        assert_eq!(registry.active_timers(), 1);

        registry.cancel(handle);
        assert_eq!(registry.active_timers(), 0);
    }

    #[test]
    fn test_sleep() {
        let registry = test_registry();

        let _handle = registry.schedule_sleep(42, Duration::from_millis(100), Some(1));
        assert_eq!(registry.active_timers(), 1);
        assert!(registry.next_deadline().is_some());
    }

    #[test]
    fn test_timeout() {
        let registry = test_registry();

        let handle = registry.schedule_timeout(42, Duration::from_millis(50), None);
        assert_eq!(registry.active_timers(), 1);

        // Simulate operation completing before timeout
        registry.cancel(handle);
        assert_eq!(registry.active_timers(), 0);
    }

    #[test]
    fn test_periodic() {
        let registry = test_registry();

        let _handle = registry.schedule_periodic(42, Duration::from_millis(100), None);
        assert_eq!(registry.active_timers(), 1);
    }

    #[test]
    fn test_clone() {
        let registry1 = test_registry();
        let registry2 = registry1.clone();

        // Both share the same backend
        registry1.schedule_sleep(1, Duration::from_secs(1), None);
        assert_eq!(registry2.active_timers(), 1);
    }

    #[test]
    fn test_time_until_next() {
        let registry = test_registry();

        assert!(registry.time_until_next().is_none());

        registry.schedule_sleep(1, Duration::from_millis(100), None);

        let time = registry.time_until_next().unwrap();
        assert!(time <= Duration::from_millis(100));
    }

    #[test]
    fn test_debug() {
        let registry = test_registry();
        let debug = format!("{:?}", registry);
        assert!(debug.contains("TimerRegistry"));
        assert!(debug.contains("binary_heap"));
    }
}