//! Timer entry and handle types

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Globally unique timer handle for cancellation
///
/// Each timer gets a unique handle when inserted. This handle can be
/// used to cancel the timer before it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerHandle(pub u64);

impl TimerHandle {
    /// Generate a new unique timer handle
    #[inline]
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        TimerHandle(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Get the raw handle value (for debugging/logging)
    #[inline]
    pub fn raw(&self) -> u64 {
        self.0
    }
}

impl Default for TimerHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Type of timer - affects behavior on expiry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerType {
    /// Time slice preemption - GVT exceeded its quantum
    Preempt,

    /// Sleep/delay - GVT voluntarily sleeping
    Sleep,

    /// Timeout on async operation (I/O, channel, etc.)
    Timeout,

    /// Periodic timer - reschedule after fire
    Periodic {
        /// Interval between firings
        interval: Duration,
    },
}

impl TimerType {
    /// Check if this timer should be rescheduled after firing
    #[inline]
    pub fn is_periodic(&self) -> bool {
        matches!(self, TimerType::Periodic { .. })
    }

    /// Get the periodic interval, if applicable
    #[inline]
    pub fn periodic_interval(&self) -> Option<Duration> {
        match self {
            TimerType::Periodic { interval } => Some(*interval),
            _ => None,
        }
    }
}

/// Timer entry - stored in the timer backend
///
/// Contains all information needed to:
/// 1. Determine when to fire (deadline)
/// 2. Identify which GVT to wake (gvt_id)
/// 3. Route to correct worker (worker_affinity)
/// 4. Handle special cases (timer_type)
#[derive(Debug, Clone)]
pub struct TimerEntry {
    /// Unique handle for cancellation
    pub handle: TimerHandle,

    /// When this timer should fire
    pub deadline: Instant,

    /// GVT to wake when timer fires
    pub gvt_id: u32,

    /// Preferred worker queue (None = any worker)
    pub worker_affinity: Option<u8>,

    /// Type of timer (affects wake behavior)
    pub timer_type: TimerType,
}

impl TimerEntry {
    /// Create a new timer entry with explicit parameters
    pub fn new(
        gvt_id: u32,
        deadline: Instant,
        worker_affinity: Option<u8>,
        timer_type: TimerType,
    ) -> Self {
        Self {
            handle: TimerHandle::new(),
            deadline,
            gvt_id,
            worker_affinity,
            timer_type,
        }
    }

    /// Create a preemption timer
    ///
    /// Preemption timers have worker affinity to ensure the GVT
    /// returns to the same worker for cache locality.
    #[inline]
    pub fn preempt(gvt_id: u32, worker_id: u8, time_slice: Duration) -> Self {
        Self {
            handle: TimerHandle::new(),
            deadline: Instant::now() + time_slice,
            gvt_id,
            worker_affinity: Some(worker_id),
            timer_type: TimerType::Preempt,
        }
    }

    /// Create a sleep timer
    ///
    /// Sleep timers optionally have affinity - useful for maintaining
    /// cache locality when sleep is brief.
    #[inline]
    pub fn sleep(gvt_id: u32, duration: Duration, affinity: Option<u8>) -> Self {
        Self {
            handle: TimerHandle::new(),
            deadline: Instant::now() + duration,
            gvt_id,
            worker_affinity: affinity,
            timer_type: TimerType::Sleep,
        }
    }

    /// Create a timeout timer
    ///
    /// Used for async operation timeouts (I/O, channels, locks).
    #[inline]
    pub fn timeout(gvt_id: u32, duration: Duration, affinity: Option<u8>) -> Self {
        Self {
            handle: TimerHandle::new(),
            deadline: Instant::now() + duration,
            gvt_id,
            worker_affinity: affinity,
            timer_type: TimerType::Timeout,
        }
    }

    /// Create a periodic timer
    ///
    /// Periodic timers automatically reschedule after firing.
    #[inline]
    pub fn periodic(gvt_id: u32, interval: Duration, affinity: Option<u8>) -> Self {
        Self {
            handle: TimerHandle::new(),
            deadline: Instant::now() + interval,
            gvt_id,
            worker_affinity: affinity,
            timer_type: TimerType::Periodic { interval },
        }
    }

    /// Create a timer with absolute deadline
    #[inline]
    pub fn at(gvt_id: u32, deadline: Instant, affinity: Option<u8>, timer_type: TimerType) -> Self {
        Self {
            handle: TimerHandle::new(),
            deadline,
            gvt_id,
            worker_affinity: affinity,
            timer_type,
        }
    }

    /// Check if this timer has expired
    #[inline]
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Time remaining until deadline (zero if expired)
    #[inline]
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Create a rescheduled entry for periodic timers
    ///
    /// Returns None if not a periodic timer.
    pub fn reschedule(&self) -> Option<Self> {
        match self.timer_type {
            TimerType::Periodic { interval } => Some(Self {
                handle: TimerHandle::new(), // New handle for new timer
                deadline: Instant::now() + interval,
                gvt_id: self.gvt_id,
                worker_affinity: self.worker_affinity,
                timer_type: self.timer_type,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_uniqueness() {
        let handles: Vec<_> = (0..1000).map(|_| TimerHandle::new()).collect();
        let unique: std::collections::HashSet<_> = handles.iter().collect();
        assert_eq!(handles.len(), unique.len());
    }

    #[test]
    fn test_preempt_entry() {
        let entry = TimerEntry::preempt(42, 3, Duration::from_millis(10));

        assert_eq!(entry.gvt_id, 42);
        assert_eq!(entry.worker_affinity, Some(3));
        assert!(matches!(entry.timer_type, TimerType::Preempt));
        assert!(!entry.is_expired()); // Just created
    }

    #[test]
    fn test_periodic_reschedule() {
        let interval = Duration::from_millis(100);
        let entry = TimerEntry::periodic(1, interval, Some(0));

        let rescheduled = entry.reschedule().expect("should reschedule");
        assert_eq!(rescheduled.gvt_id, entry.gvt_id);
        assert_eq!(rescheduled.worker_affinity, entry.worker_affinity);
        assert_ne!(rescheduled.handle, entry.handle); // New handle
    }

    #[test]
    fn test_non_periodic_no_reschedule() {
        let entry = TimerEntry::sleep(1, Duration::from_millis(100), None);
        assert!(entry.reschedule().is_none());
    }

    #[test]
    fn test_timer_type_is_periodic() {
        assert!(!TimerType::Preempt.is_periodic());
        assert!(!TimerType::Sleep.is_periodic());
        assert!(!TimerType::Timeout.is_periodic());
        assert!(TimerType::Periodic {
            interval: Duration::from_secs(1)
        }
        .is_periodic());
    }
}