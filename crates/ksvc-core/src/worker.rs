//! Worker pool abstraction (Tier 2 execution).
//!
//! A `WorkerPool` executes syscalls that io_uring cannot handle.
//! Workers can block independently — no head-of-line blocking.
//!
//! # Implementors
//!
//! - `FixedPool` (default): spawns N OS threads at creation time.
//!   N = min(8, nproc/2). Threads share the process context.
//!   Simple, predictable, safe.
//!
//! - `LazyPool` (future): starts with 1 thread, scales up on demand
//!   when all workers are busy, scales down on idle.
//!   Same strategy as io-wq in the kernel.
//!
//! - `InlineWorker` (testing): executes synchronously in the caller.
//!   Only for unit tests — blocks the dispatcher!

use crate::entry::SubmitEntry;
use crate::error::Result;

/// A completed worker operation.
#[derive(Debug, Clone, Copy)]
pub struct WorkerCompletion {
    pub corr_id: crate::entry::CorrId,
    pub result: i64,
}

/// Executes Tier 2 syscalls on a pool of threads/kthreads.
///
/// **Contract:**
/// - `enqueue()` must NEVER block the caller. If the pool is full,
///   it returns `Err(WorkerUnavailable)`.
/// - Workers execute in the owning process's context (mm/cred/files).
/// - Workers may block (that's the whole point).
/// - Completed results are collected via `poll_completions()`.
pub trait WorkerPool: Send + Sync {
    /// Enqueue a syscall entry for execution on a worker thread.
    ///
    /// Returns immediately. The worker will execute the syscall
    /// and make the result available via `poll_completions()`.
    fn enqueue(&self, entry: &SubmitEntry) -> Result<()>;

    /// Poll for completed worker operations (non-blocking).
    ///
    /// Returns the number of completions written into `buf`.
    fn poll_completions(&self, buf: &mut [WorkerCompletion], max: usize) -> usize;

    /// Number of workers currently executing (busy count).
    fn active_workers(&self) -> usize;

    /// Total number of workers (busy + idle).
    fn total_workers(&self) -> usize;

    /// Maximum number of workers this pool can have.
    fn max_workers(&self) -> usize;

    /// Gracefully shut down all workers. Blocks until drained.
    fn shutdown(&self);
}
