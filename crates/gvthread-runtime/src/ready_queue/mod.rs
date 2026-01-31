//! Ready Queue abstraction for GVThread scheduling
//!
//! Provides a trait-based abstraction allowing different scheduling strategies.
//!
//! # Implementations
//! - `SimpleQueue` - Go-like per-worker + global queue (MVP)

mod simple;

pub use simple::SimpleQueue;

use gvthread_core::id::GVThreadId;
use gvthread_core::state::Priority;

/// Trait for ready queue implementations
///
/// All implementations must be thread-safe (Send + Sync).
pub trait ReadyQueue: Send + Sync {
    /// Make a GVThread ready to run
    ///
    /// # Arguments
    /// * `id` - GVThread ID
    /// * `priority` - Priority level (ignored in MVP)
    /// * `hint_worker` - Preferred worker's local queue (None = global)
    fn push(&self, id: GVThreadId, priority: Priority, hint_worker: Option<usize>);
    
    /// Get next GVThread for this worker
    ///
    /// Order: local queue → global queue → steal from others
    ///
    /// # Returns
    /// * `Some((id, priority))` - Work found
    /// * `None` - No work (worker should park)
    fn pop(&self, worker_id: usize) -> Option<(GVThreadId, Priority)>;
    
    /// Park worker until work available or timeout
    fn park(&self, worker_id: usize, timeout_ms: u64);
    
    /// Wake one parked worker
    fn wake_one(&self);
    
    /// Wake all parked workers (shutdown)
    fn wake_all(&self);
    
    /// Approximate ready count (for diagnostics)
    fn len(&self) -> usize;
    
    /// Check if empty
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}