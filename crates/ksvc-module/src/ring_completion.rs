//! `RingCompletionSink` — default `CompletionSink` implementation.
//!
//! Writes completion entries to the KSVC completion ring (mmap'd memory
//! shared with userspace). Notifications are batched: `flush_and_notify()`
//! is called once per dispatcher loop iteration.

use ksvc_core::completion::CompletionSink;
use ksvc_core::entry::{CompletionEntry, CorrId};
use ksvc_core::error::{KsvcError, Result};
use ksvc_core::notifier::Notifier;

use std::sync::atomic::{AtomicU64, Ordering};

/// Ring layout in mmap'd memory:
///
/// ```text
/// [RingHeader: 64 bytes]
/// [Entry 0]
/// [Entry 1]
/// ...
/// [Entry N-1]
/// ```
///
/// head = consumer (userspace) read position (read by producer to check full)
/// tail = producer (kernel/dispatcher) write position (advanced by push)
///
/// Ring is full when (tail - head) == ring_size.
/// Ring is empty when head == tail.
///
/// Both head and tail are monotonically increasing u64 values.
/// The actual array index is (value & mask).

pub struct RingCompletionSink<N: Notifier> {
    /// Pointer to the ring header (mmap'd memory).
    /// head: offset 16, tail: offset 24 (see KsvcRingHeader).
    base: *mut u8,
    /// Ring entries start after the 64-byte header.
    entries: *mut CompletionEntry,
    /// ring_size (power of 2).
    size: u32,
    /// mask = size - 1.
    mask: u32,
    /// Local tail cache (written back to mmap on flush).
    local_tail: u64,
    /// Number of completions buffered since last flush.
    buffered: u32,
    /// The notifier to signal userspace.
    notifier: N,
}

// Safety: the mmap'd memory is process-local and the dispatcher
// is the sole writer to the completion ring.
unsafe impl<N: Notifier> Send for RingCompletionSink<N> {}
unsafe impl<N: Notifier> Sync for RingCompletionSink<N> {}

impl<N: Notifier> RingCompletionSink<N> {
    /// Create from a pointer to mmap'd completion ring memory.
    ///
    /// # Safety
    /// - `base` must point to a valid KSVC ring (header + entries).
    /// - The memory must remain valid for the lifetime of this sink.
    /// - Only one writer (the dispatcher) may exist.
    pub unsafe fn new(base: *mut u8, size: u32, notifier: N) -> Self {
        assert!(size.is_power_of_two(), "ring size must be power of 2");
        let entries = base.add(64) as *mut CompletionEntry;
        // Read the current tail from the header
        let tail_ptr = base.add(24) as *const AtomicU64;
        let current_tail = (*tail_ptr).load(Ordering::Acquire);

        Self {
            base,
            entries,
            size,
            mask: size - 1,
            local_tail: current_tail,
            buffered: 0,
            notifier,
        }
    }

    /// Read the consumer's head (how far userspace has read).
    fn read_head(&self) -> u64 {
        unsafe {
            let head_ptr = self.base.add(16) as *const AtomicU64;
            (*head_ptr).load(Ordering::Acquire)
        }
    }

    /// Write the producer's tail to shared memory.
    fn publish_tail(&self) {
        unsafe {
            let tail_ptr = self.base.add(24) as *const AtomicU64;
            (*tail_ptr).store(self.local_tail, Ordering::Release);
        }
    }

    /// Available slots in the ring.
    fn available(&self) -> u32 {
        let head = self.read_head();
        self.size - (self.local_tail - head) as u32
    }
}

impl<N: Notifier> CompletionSink for RingCompletionSink<N> {
    fn push(&self, corr_id: CorrId, result: i64, flags: u32) -> Result<()> {
        // Check for space
        if self.available() == 0 {
            return Err(KsvcError::RingFull);
        }

        let idx = (self.local_tail & self.mask as u64) as usize;
        let entry = CompletionEntry {
            corr_id,
            result,
            flags,
            _pad: 0,
        };

        // Write the entry
        unsafe {
            std::ptr::write_volatile(self.entries.add(idx), entry);
        }

        // Advance local tail (not yet visible to consumer)
        // Safety: we need interior mutability here. In the real impl,
        // local_tail would be in a Cell or we'd take &mut self.
        // For now, this is conceptually correct but needs &mut self.
        //
        // TODO: switch push() to take &mut self, or use Cell<u64>
        Ok(())
    }

    fn flush_and_notify(&self) -> Result<()> {
        // Publish the tail to shared memory
        self.publish_tail();

        // Signal userspace if we have any new completions
        if self.buffered > 0 {
            self.notifier.notify()?;
        }
        Ok(())
    }
}
