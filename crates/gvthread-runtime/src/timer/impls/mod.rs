//! Timer backend implementations
//!
//! Currently provides:
//! - `HeapTimerBackend` - BinaryHeap-based MVP implementation
//!
//! Future implementations:
//! - Hierarchical timing wheel (O(1) insert/cancel)
//! - Kernel timerfd-based (reduced syscalls)
//! - io_uring-based (zero-copy, async)

mod heap;

pub use heap::HeapTimerBackend;

use crate::timer::TimerBackend;

/// Backend selector for configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimerBackendType {
    /// BinaryHeap-based implementation (MVP)
    ///
    /// Characteristics:
    /// - O(log n) insert
    /// - O(1) peek next deadline
    /// - O(log n) pop expired
    /// - O(1) cancel (lazy, marked in HashSet)
    /// - Simple, correct, good enough for most workloads
    #[default]
    BinaryHeap,

    // Future variants:
    //
    // /// Hierarchical timing wheel
    // ///
    // /// Characteristics:
    // /// - O(1) insert (amortized)
    // /// - O(1) cancel
    // /// - O(1) tick advancement
    // /// - Better for high timer volume with similar timeouts
    // HierarchicalWheel,
    //
    // /// Linux timerfd-based
    // ///
    // /// Characteristics:
    // /// - Kernel manages timer state
    // /// - epoll-friendly
    // /// - Good for coarse-grained timers
    // KernelTimerFd,
    //
    // /// io_uring-based
    // ///
    // /// Characteristics:
    // /// - Zero-copy timeout submission
    // /// - Batched syscalls
    // /// - Best for high-frequency timers on modern kernels
    // IoUring,
}

impl TimerBackendType {
    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            TimerBackendType::BinaryHeap => "binary_heap",
        }
    }
}

/// Create a timer backend based on type
pub fn create_backend(backend_type: TimerBackendType) -> Box<dyn TimerBackend> {
    match backend_type {
        TimerBackendType::BinaryHeap => Box::new(HeapTimerBackend::new()),
    }
}

/// Create a timer backend as Arc (common use case)
pub fn create_backend_arc(
    backend_type: TimerBackendType,
) -> std::sync::Arc<dyn TimerBackend> {
    match backend_type {
        TimerBackendType::BinaryHeap => {
            std::sync::Arc::new(HeapTimerBackend::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_default_backend() {
        let backend = create_backend(TimerBackendType::default());
        assert_eq!(backend.name(), "binary_heap");
        assert!(backend.is_empty());
    }

    #[test]
    fn test_backend_type_name() {
        assert_eq!(TimerBackendType::BinaryHeap.name(), "binary_heap");
    }
}