//! I/O backend abstraction (Tier 1 execution).
//!
//! An `IoBackend` handles the lifecycle of async I/O operations:
//! submit, poll completions, cancel.
//!
//! # Implementors
//!
//! - `BasicIoUring` (default): uses `io_uring_enter()` for submission,
//!   polls CQ for completions. No SQPOLL, no fixed files, no fixed buffers.
//!   Safe, correct, works everywhere io_uring exists.
//!
//! - `SqpollIoUring` (feature = "sqpoll"): enables `IORING_SETUP_SQPOLL`.
//!   A kernel thread polls the SQ — eliminates `io_uring_enter()` calls.
//!   Trades one CPU core for lower latency. For high-throughput servers.
//!
//! - `FixedFileIoUring` (feature = "fixed-files"): pre-registers hot fds
//!   via `IORING_REGISTER_FILES`. ~10-15% fd lookup speedup.
//!
//! - `FixedBufferIoUring` (feature = "fixed-buffers"): pre-pins memory
//!   via `IORING_REGISTER_BUFFERS`. Major win for O_DIRECT.
//!
//! These features compose: you can have SQPOLL + fixed files + fixed buffers
//! by implementing a composite backend, or by stacking decorators.

use crate::entry::{CorrId, SubmitEntry};
use crate::error::Result;

/// A completed I/O operation from the backend.
#[derive(Debug, Clone, Copy)]
pub struct IoCompletion {
    /// The correlation ID that was submitted.
    pub corr_id: CorrId,
    /// Result (return value or negative errno).
    pub result: i64,
    /// Backend-specific flags.
    pub flags: u32,
}

/// Async I/O submission and completion.
///
/// The dispatcher calls `submit()` for each Tier 1 entry, then
/// `flush()` once per batch to kick the backend, then `poll_completions()`
/// to drain finished operations.
///
/// **Contract:** `submit()` and `flush()` must NEVER block.
/// The backend is responsible for ensuring that blocking happens
/// on worker threads, not on the caller.
pub trait IoBackend: Send + Sync {
    /// Submit a single I/O operation. Queued but not yet kicked.
    ///
    /// The `entry` contains the syscall number and arguments.
    /// The implementation translates this to the backend's native format
    /// (e.g., io_uring SQE) and queues it.
    ///
    /// Returns `Ok(())` if queued, `Err(RingFull)` if the backend's
    /// submission queue is full.
    fn submit(&mut self, entry: &SubmitEntry) -> Result<()>;

    /// Kick all queued submissions to the kernel.
    ///
    /// For io_uring: calls `io_uring_enter(to_submit, 0, 0)`.
    /// For SQPOLL mode: may be a no-op (kernel thread polls).
    ///
    /// Returns the number of entries successfully submitted.
    fn flush(&mut self) -> Result<usize>;

    /// Poll for completed operations (non-blocking).
    ///
    /// Drains up to `max` completions into the provided buffer.
    /// Returns the number of completions written.
    ///
    /// **Must not block.** If no completions are ready, returns 0.
    fn poll_completions(&mut self, buf: &mut [IoCompletion], max: usize) -> usize;

    /// Cancel an in-flight operation by correlation ID.
    ///
    /// Best-effort — the operation may complete before cancellation takes effect.
    fn cancel(&mut self, corr_id: CorrId) -> Result<()>;

    /// How many operations are currently in-flight (submitted but not completed).
    fn inflight(&self) -> usize;

    /// Maximum number of entries the backend can queue before flush.
    fn capacity(&self) -> usize;

    /// Probe which io_uring opcodes are supported.
    ///
    /// Returns a bitmask or vec of supported IORING_OP_* values.
    /// Used by the `SyscallRouter` to build the routing table.
    fn probe_opcodes(&self) -> Vec<u8>;

    /// Gracefully shut down the I/O backend.
    ///
    /// Waits for in-flight operations to complete (up to a timeout),
    /// then releases resources. Called before Drop to ensure orderly
    /// drain of the io_uring CQ.
    fn shutdown(&mut self);
}
