//! Completion delivery abstraction.
//!
//! A `CompletionSink` writes completion entries and notifies userspace.
//! Default impl: write to KSVC completion ring + eventfd_signal.
//! Future impl: direct futex wake on GVThread park word.

use crate::entry::{CompletionEntry, CorrId};
use crate::error::Result;

/// Writes completions and notifies the consumer.
///
/// # Implementors
///
/// - `RingCompletionSink` (default): writes to mmap'd KSVC completion ring,
///   signals via eventfd. Batches notifications (one signal per drain cycle).
///
/// - Future: `DirectWakeSink` — writes result directly into GVThread metadata
///   and unparks the specific GVThread. Skips the ring entirely for Tier 1.
pub trait CompletionSink: Send + Sync {
    /// Write a single completion. May be buffered.
    fn push(&self, corr_id: CorrId, result: i64, flags: u32) -> Result<()>;

    /// Write a batch of completions. Default: calls push() in a loop.
    fn push_batch(&self, entries: &[CompletionEntry]) -> Result<usize> {
        let mut count = 0;
        for entry in entries {
            self.push(entry.corr_id, entry.result, entry.flags)?;
            count += 1;
        }
        Ok(count)
    }

    /// Flush any buffered completions and notify the consumer.
    /// Called once per dispatcher loop iteration (not per completion).
    fn flush_and_notify(&self) -> Result<()>;
}
