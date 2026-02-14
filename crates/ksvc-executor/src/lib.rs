//! # ksvc-executor — The Dispatcher Loop
//!
//! The dispatcher is the beating heart of KSVC. It runs on a dedicated
//! thread and executes this loop:
//!
//! ```text
//! loop {
//!     1. Drain io_uring completions → write to KSVC completion ring
//!     2. Drain worker pool completions → write to KSVC completion ring
//!     3. If any completions written → notify userspace (once)
//!     4. Dequeue batch from KSVC submit ring
//!     5. For each entry:
//!          route_table[syscall_nr] → Tier 1? submit to io_uring
//!                                  → Tier 2? enqueue to worker pool
//!                                  → else? write -ENOSYS completion
//!     6. Flush io_uring SQEs
//!     7. If no work → brief sleep
//! }
//! ```
//!
//! The dispatcher is fully generic over all trait implementations.
//! Swap any component and the dispatcher doesn't change.

use ksvc_core::entry::{CorrId, SubmitEntry, CompletionEntry};
use ksvc_core::error::KsvcError;
use ksvc_core::io_backend::{IoBackend, IoCompletion};
use ksvc_core::notifier::Notifier;
use ksvc_core::router::SyscallRouter;
use ksvc_core::tier::Tier;
use ksvc_core::worker::{WorkerCompletion, WorkerPool};

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Configuration for the dispatcher loop.
pub struct DispatcherConfig {
    /// Maximum entries to dequeue per batch from the submit ring.
    pub max_batch: usize,
    /// Maximum io_uring completions to drain per iteration.
    pub max_io_completions: usize,
    /// Maximum worker completions to drain per iteration.
    pub max_worker_completions: usize,
    /// Sleep duration (microseconds) when idle.
    pub idle_sleep_us: u64,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            max_batch: 64,
            max_io_completions: 128,
            max_worker_completions: 64,
            idle_sleep_us: 100, // 100μs
        }
    }
}

/// The submit ring reader — reads entries from mmap'd submit ring.
///
/// This is the counterpart to RingCompletionSink: it reads from the
/// submit ring that userspace writes to.
pub struct SubmitRing {
    base: *const u8,
    entries: *const SubmitEntry,
    size: u32,
    mask: u32,
    local_head: u64,
}

unsafe impl Send for SubmitRing {}

impl SubmitRing {
    /// # Safety
    /// `base` must point to a valid KSVC submit ring.
    pub unsafe fn new(base: *const u8, size: u32) -> Self {
        assert!(size.is_power_of_two());
        let entries = base.add(64) as *const SubmitEntry;
        let head_ptr = base.add(16) as *const AtomicU64;
        let current_head = (*head_ptr).load(Ordering::Acquire);
        Self {
            base,
            entries,
            size,
            mask: size - 1,
            local_head: current_head,
        }
    }

    /// Read the producer's tail (how far userspace has written).
    fn read_tail(&self) -> u64 {
        unsafe {
            let tail_ptr = self.base.add(24) as *const AtomicU64;
            (*tail_ptr).load(Ordering::Acquire)
        }
    }

    /// Publish head to shared memory (tells userspace we've consumed up to here).
    fn publish_head(&self) {
        unsafe {
            let head_ptr = self.base.add(16) as *const AtomicU64;
            (*head_ptr).store(self.local_head, Ordering::Release);
        }
    }

    /// Dequeue up to `max` entries.
    pub fn dequeue_batch(&mut self, buf: &mut [SubmitEntry], max: usize) -> usize {
        let tail = self.read_tail();
        let available = (tail - self.local_head) as usize;
        let count = available.min(max).min(buf.len());

        for i in 0..count {
            let idx = (self.local_head & self.mask as u64) as usize;
            buf[i] = unsafe { std::ptr::read_volatile(self.entries.add(idx)) };
            self.local_head += 1;
        }

        if count > 0 {
            self.publish_head();
        }
        count
    }
}

/// The completion ring writer — writes completions that userspace reads.
///
/// Simplified version that takes &mut self (the dispatcher is single-threaded).
pub struct CompletionRing {
    base: *mut u8,
    entries: *mut CompletionEntry,
    size: u32,
    mask: u32,
    local_tail: u64,
    completions_written: u32,
}

unsafe impl Send for CompletionRing {}

impl CompletionRing {
    /// # Safety
    /// `base` must point to a valid KSVC completion ring.
    pub unsafe fn new(base: *mut u8, size: u32) -> Self {
        assert!(size.is_power_of_two());
        let entries = base.add(64) as *mut CompletionEntry;
        let tail_ptr = base.add(24) as *const AtomicU64;
        let current_tail = (*tail_ptr).load(Ordering::Acquire);
        Self {
            base,
            entries,
            size,
            mask: size - 1,
            local_tail: current_tail,
            completions_written: 0,
        }
    }

    fn read_head(&self) -> u64 {
        unsafe {
            let head_ptr = self.base.add(16) as *const AtomicU64;
            (*head_ptr).load(Ordering::Acquire)
        }
    }

    fn publish_tail(&self) {
        unsafe {
            let tail_ptr = self.base.add(24) as *const AtomicU64;
            (*tail_ptr).store(self.local_tail, Ordering::Release);
        }
    }

    fn available(&self) -> u32 {
        let head = self.read_head();
        self.size - (self.local_tail - head) as u32
    }

    /// Write a completion entry. Returns false if ring is full.
    pub fn push(&mut self, corr_id: CorrId, result: i64, flags: u32) -> bool {
        if self.available() == 0 {
            return false;
        }
        let idx = (self.local_tail & self.mask as u64) as usize;
        let entry = CompletionEntry {
            corr_id,
            result,
            flags,
            _pad: 0,
        };
        unsafe {
            std::ptr::write_volatile(self.entries.add(idx), entry);
        }
        self.local_tail += 1;
        self.completions_written += 1;
        true
    }

    /// Publish tail and reset counter. Returns number flushed.
    pub fn flush(&mut self) -> u32 {
        let n = self.completions_written;
        if n > 0 {
            self.publish_tail();
            self.completions_written = 0;
        }
        n
    }
}

/// The dispatcher loop — generic over all trait implementations.
///
/// This function runs on a dedicated thread. It returns when the
/// `shutdown` flag is set.
pub fn dispatcher_loop<R, B, W, N>(
    mut submit_ring: SubmitRing,
    mut completion_ring: CompletionRing,
    router: &R,
    io_backend: &mut B,
    worker_pool: &W,
    notifier: &N,
    config: &DispatcherConfig,
    shutdown: &AtomicBool,
) where
    R: SyscallRouter,
    B: IoBackend,
    W: WorkerPool,
    N: Notifier,
{
    let mut submit_buf = vec![SubmitEntry {
        corr_id: CorrId::NONE,
        syscall_nr: 0,
        flags: 0,
        args: [0; 6],
    }; config.max_batch];

    let mut io_comp_buf = vec![IoCompletion {
        corr_id: CorrId::NONE,
        result: 0,
        flags: 0,
    }; config.max_io_completions];

    let mut worker_comp_buf = vec![WorkerCompletion {
        corr_id: CorrId::NONE,
        result: 0,
    }; config.max_worker_completions];

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut did_work = false;

        // ── Step 1: Drain io_uring completions ──
        let n_io = io_backend.poll_completions(
            &mut io_comp_buf,
            config.max_io_completions,
        );
        for i in 0..n_io {
            let c = &io_comp_buf[i];
            completion_ring.push(c.corr_id, c.result, c.flags);
        }
        if n_io > 0 {
            did_work = true;
        }

        // ── Step 2: Drain worker pool completions ──
        let n_worker = worker_pool.poll_completions(
            &mut worker_comp_buf,
            config.max_worker_completions,
        );
        for i in 0..n_worker {
            let c = &worker_comp_buf[i];
            completion_ring.push(c.corr_id, c.result, 0);
        }
        if n_worker > 0 {
            did_work = true;
        }

        // ── Step 3: Flush completions + notify userspace ──
        let flushed = completion_ring.flush();
        if flushed > 0 {
            let _ = notifier.notify();
        }

        // ── Step 4: Dequeue submit ring entries ──
        let n_submit = submit_ring.dequeue_batch(
            &mut submit_buf,
            config.max_batch,
        );

        // ── Step 5: Route each entry ──
        let mut sqes_queued = 0u32;
        for i in 0..n_submit {
            let entry = &submit_buf[i];
            let route = router.route(entry.syscall_nr);

            match route.tier {
                Tier::SharedPage => {
                    // This should never happen — userspace handles Tier 0.
                    // If it does reach here, return the value from the shared
                    // page equivalent. For safety, return -ENOSYS.
                    completion_ring.push(
                        entry.corr_id,
                        -(libc::ENOSYS as i64),
                        0,
                    );
                }
                Tier::IoUring => {
                    // Translate to io_uring SQE via the backend.
                    // The backend's submit_with_opcode is specific to BasicIoUring.
                    // For the trait-generic path, we use the trait's submit().
                    match io_backend.submit(entry) {
                        Ok(()) => {
                            sqes_queued += 1;
                        }
                        Err(KsvcError::RingFull) => {
                            // io_uring SQ is full — write EAGAIN completion.
                            // The GVThread should retry.
                            completion_ring.push(
                                entry.corr_id,
                                -(libc::EAGAIN as i64),
                                0,
                            );
                        }
                        Err(_) => {
                            completion_ring.push(
                                entry.corr_id,
                                -(libc::ENOSYS as i64),
                                0,
                            );
                        }
                    }
                }
                Tier::WorkerPool => {
                    match worker_pool.enqueue(entry) {
                        Ok(()) => {}
                        Err(_) => {
                            // Worker pool full — EAGAIN
                            completion_ring.push(
                                entry.corr_id,
                                -(libc::EAGAIN as i64),
                                0,
                            );
                        }
                    }
                }
                Tier::Legacy => {
                    // Unsupported — should not reach the ring.
                    completion_ring.push(
                        entry.corr_id,
                        -(libc::ENOSYS as i64),
                        0,
                    );
                }
            }
        }

        if n_submit > 0 {
            did_work = true;
        }

        // ── Step 6: Kick io_uring (submit accumulated SQEs) ──
        if sqes_queued > 0 {
            let _ = io_backend.flush();
        }

        // ── Step 6b: Flush any completions generated during routing ──
        let flushed2 = completion_ring.flush();
        if flushed2 > 0 {
            let _ = notifier.notify();
        }

        // ── Step 7: Sleep if idle ──
        if !did_work {
            std::thread::sleep(std::time::Duration::from_micros(
                config.idle_sleep_us,
            ));
        }
    }

    // Shutdown: drain remaining completions
    let n_io = io_backend.poll_completions(
        &mut io_comp_buf,
        config.max_io_completions,
    );
    for i in 0..n_io {
        let c = &io_comp_buf[i];
        completion_ring.push(c.corr_id, c.result, c.flags);
    }
    let n_worker = worker_pool.poll_completions(
        &mut worker_comp_buf,
        config.max_worker_completions,
    );
    for i in 0..n_worker {
        let c = &worker_comp_buf[i];
        completion_ring.push(c.corr_id, c.result, 0);
    }
    let flushed = completion_ring.flush();
    if flushed > 0 {
        let _ = notifier.notify();
    }
    worker_pool.shutdown();
}
