//! BinaryHeap-based timer backend (MVP implementation)
//!
//! Simple, correct, and efficient enough for most workloads.
//!
//! # Complexity
//!
//! - Insert: O(log n)
//! - Cancel: O(1) amortized (lazy cancellation)
//! - Poll expired: O(k log n) where k = number of expired timers
//! - Next deadline: O(1)
//!
//! # Cancellation Strategy
//!
//! Uses lazy cancellation: cancelled handles are stored in a HashSet,
//! and skipped when polling. This avoids O(n) removal from the heap.
//! The cancelled set is cleaned up when the heap becomes empty.

use std::collections::{BinaryHeap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

use crate::timer::{ExpiredTimer, TimerBackend, TimerEntry, TimerHandle, TimerType};

/// Wrapper for heap ordering (min-heap by deadline)
struct HeapEntry(TimerEntry);

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.0.deadline == other.0.deadline && self.0.handle == other.0.handle
    }
}

impl Eq for HeapEntry {}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse ordering for min-heap (earliest deadline first)
        // Tie-break by handle for deterministic ordering
        match other.0.deadline.cmp(&self.0.deadline) {
            std::cmp::Ordering::Equal => other.0.handle.0.cmp(&self.0.handle.0),
            ord => ord,
        }
    }
}

/// Internal state protected by mutex
struct HeapInner {
    /// Min-heap of timer entries
    heap: BinaryHeap<HeapEntry>,

    /// Cancelled timer handles (lazy cancellation)
    cancelled: HashSet<TimerHandle>,

    /// Stats: total timers inserted
    total_inserted: u64,

    /// Stats: total timers fired
    total_fired: u64,

    /// Stats: total timers cancelled
    total_cancelled: u64,
}

impl HeapInner {
    fn new(capacity: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(capacity),
            cancelled: HashSet::with_capacity(capacity / 4),
            total_inserted: 0,
            total_fired: 0,
            total_cancelled: 0,
        }
    }
}

/// BinaryHeap-based timer backend
///
/// Thread-safe via internal Mutex. The lock is held briefly during
/// insert/cancel/poll operations.
///
/// # Example
///
/// ```ignore
/// use gvthread::timer::{TimerBackend, TimerEntry};
/// use gvthread::timer::impls::HeapTimerBackend;
/// use std::time::Duration;
///
/// let backend = HeapTimerBackend::new();
///
/// // Insert a sleep timer
/// let entry = TimerEntry::sleep(42, Duration::from_millis(100), None);
/// let handle = backend.insert(entry);
///
/// // Cancel if needed
/// backend.cancel(handle);
/// ```
pub struct HeapTimerBackend {
    inner: Mutex<HeapInner>,
}

impl HeapTimerBackend {
    /// Create a new heap-based timer backend
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    /// Create with specified initial capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(HeapInner::new(capacity)),
        }
    }

    /// Get statistics snapshot
    pub fn stats(&self) -> HeapTimerStats {
        let inner = self.inner.lock().unwrap();
        HeapTimerStats {
            active: inner.heap.len(),
            pending_cancellations: inner.cancelled.len(),
            total_inserted: inner.total_inserted,
            total_fired: inner.total_fired,
            total_cancelled: inner.total_cancelled,
        }
    }
}

impl Default for HeapTimerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerBackend for HeapTimerBackend {
    fn insert(&self, entry: TimerEntry) -> TimerHandle {
        let handle = entry.handle;
        let mut inner = self.inner.lock().unwrap();
        inner.heap.push(HeapEntry(entry));
        inner.total_inserted += 1;
        handle
    }

    fn cancel(&self, handle: TimerHandle) -> bool {
        let mut inner = self.inner.lock().unwrap();
        // Lazy cancellation: just mark as cancelled
        let inserted = inner.cancelled.insert(handle);
        if inserted {
            inner.total_cancelled += 1;
        }
        inserted
    }

    fn poll_expired(&self, now: Instant) -> Vec<ExpiredTimer> {
        let mut inner = self.inner.lock().unwrap();
        let mut expired = Vec::new();
        let mut to_reschedule = Vec::new();

        while let Some(entry) = inner.heap.peek() {
            if entry.0.deadline > now {
                break; // Heap is sorted, no more expired
            }

            let entry = inner.heap.pop().unwrap().0;

            // Skip if cancelled
            if inner.cancelled.remove(&entry.handle) {
                continue;
            }

            // Handle periodic timers
            if let Some(rescheduled) = entry.reschedule() {
                to_reschedule.push(rescheduled);
            }

            inner.total_fired += 1;

            expired.push(ExpiredTimer {
                gvt_id: entry.gvt_id,
                worker_affinity: entry.worker_affinity,
                timer_type: entry.timer_type,
            });
        }

        // Reschedule periodic timers
        for entry in to_reschedule {
            inner.heap.push(HeapEntry(entry));
        }

        // Periodic cleanup: if heap is empty, clear cancelled set
        if inner.heap.is_empty() {
            inner.cancelled.clear();
        }

        expired
    }

    fn next_deadline(&self) -> Option<Instant> {
        let inner = self.inner.lock().unwrap();

        // Skip cancelled entries at the front
        // Note: This is approximate - there might be cancelled entries we don't see
        inner.heap.peek().map(|e| e.0.deadline)
    }

    fn len(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        // Subtract cancelled from heap size for accurate count
        inner.heap.len().saturating_sub(inner.cancelled.len())
    }

    fn name(&self) -> &'static str {
        "binary_heap"
    }
}

/// Statistics for HeapTimerBackend
#[derive(Debug, Clone)]
pub struct HeapTimerStats {
    /// Currently active (non-cancelled) timers
    pub active: usize,
    /// Cancelled but not yet removed from heap
    pub pending_cancellations: usize,
    /// Total timers inserted (lifetime)
    pub total_inserted: u64,
    /// Total timers that fired (lifetime)
    pub total_fired: u64,
    /// Total timers cancelled (lifetime)
    pub total_cancelled: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_insert_and_poll() {
        let backend = HeapTimerBackend::new();

        // Insert timer that expires immediately
        let entry = TimerEntry::sleep(42, Duration::ZERO, Some(3));
        backend.insert(entry);

        assert_eq!(backend.len(), 1);

        // Poll should return it
        let expired = backend.poll_expired(Instant::now() + Duration::from_millis(1));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].gvt_id, 42);
        assert_eq!(expired[0].worker_affinity, Some(3));
    }

    #[test]
    fn test_ordering() {
        let backend = HeapTimerBackend::new();
        let now = Instant::now();

        // Insert in reverse order
        backend.insert(TimerEntry::at(
            3,
            now + Duration::from_millis(30),
            None,
            TimerType::Sleep,
        ));
        backend.insert(TimerEntry::at(
            1,
            now + Duration::from_millis(10),
            None,
            TimerType::Sleep,
        ));
        backend.insert(TimerEntry::at(
            2,
            now + Duration::from_millis(20),
            None,
            TimerType::Sleep,
        ));

        // Poll all at once
        let expired = backend.poll_expired(now + Duration::from_millis(50));

        // Should come out in deadline order
        assert_eq!(expired.len(), 3);
        assert_eq!(expired[0].gvt_id, 1);
        assert_eq!(expired[1].gvt_id, 2);
        assert_eq!(expired[2].gvt_id, 3);
    }

    #[test]
    fn test_cancel() {
        let backend = HeapTimerBackend::new();

        let entry = TimerEntry::sleep(42, Duration::from_secs(1), None);
        let handle = backend.insert(entry);

        assert_eq!(backend.len(), 1);

        // Cancel
        assert!(backend.cancel(handle));
        // len() subtracts cancelled
        assert_eq!(backend.len(), 0);

        // Poll shouldn't return cancelled timer
        let expired = backend.poll_expired(Instant::now() + Duration::from_secs(2));
        assert!(expired.is_empty());
    }

    #[test]
    fn test_cancel_idempotent() {
        let backend = HeapTimerBackend::new();

        let entry = TimerEntry::sleep(42, Duration::from_secs(1), None);
        let handle = backend.insert(entry);

        assert!(backend.cancel(handle));
        assert!(!backend.cancel(handle)); // Already cancelled
    }

    #[test]
    fn test_periodic_reschedule() {
        let backend = HeapTimerBackend::new();
        let interval = Duration::from_millis(10);

        let entry = TimerEntry::periodic(42, interval, None);
        backend.insert(entry);

        // First fire
        let expired = backend.poll_expired(Instant::now() + Duration::from_millis(15));
        assert_eq!(expired.len(), 1);
        assert!(matches!(
            expired[0].timer_type,
            TimerType::Periodic { .. }
        ));

        // Timer should be rescheduled
        assert_eq!(backend.len(), 1);

        // Second fire
        let expired = backend.poll_expired(Instant::now() + Duration::from_millis(30));
        assert_eq!(expired.len(), 1);

        // Still rescheduled
        assert_eq!(backend.len(), 1);
    }

    #[test]
    fn test_next_deadline() {
        let backend = HeapTimerBackend::new();
        let now = Instant::now();

        assert!(backend.next_deadline().is_none());

        backend.insert(TimerEntry::at(
            1,
            now + Duration::from_millis(100),
            None,
            TimerType::Sleep,
        ));

        let deadline = backend.next_deadline().unwrap();
        assert!(deadline > now);
        assert!(deadline <= now + Duration::from_millis(100));
    }

    #[test]
    fn test_stats() {
        let backend = HeapTimerBackend::new();

        let entry1 = TimerEntry::sleep(1, Duration::ZERO, None);
        let entry2 = TimerEntry::sleep(2, Duration::from_secs(10), None);

        backend.insert(entry1);
        let handle2 = backend.insert(entry2);

        // Fire first timer
        backend.poll_expired(Instant::now() + Duration::from_millis(1));

        // Cancel second timer
        backend.cancel(handle2);

        let stats = backend.stats();
        assert_eq!(stats.total_inserted, 2);
        assert_eq!(stats.total_fired, 1);
        assert_eq!(stats.total_cancelled, 1);
    }

    #[test]
    fn test_cleanup_on_empty() {
        let backend = HeapTimerBackend::new();

        // Insert and cancel multiple times
        for _ in 0..10 {
            let entry = TimerEntry::sleep(1, Duration::from_secs(100), None);
            let handle = backend.insert(entry);
            backend.cancel(handle);
        }

        // Poll to trigger cleanup
        backend.poll_expired(Instant::now() + Duration::from_secs(200));

        // After cleanup, cancelled set should be empty
        let stats = backend.stats();
        assert_eq!(stats.pending_cancellations, 0);
    }
}