//! # Reactor — the io_uring completion poller
//!
//! The reactor runs on a dedicated OS thread. It:
//! 1. Dequeues `IoRequest`s from a lock-free MPSC queue
//! 2. Submits them to io_uring via `BasicIoUring`
//! 3. Polls completions via `flush_and_wait()`
//! 4. Writes results to a results slab
//! 5. Wakes the corresponding GVThread via `scheduler::wake_gvthread()`
//!
//! This is the GVThread equivalent of Go's netpoller.

use ksvc_core::entry::{CorrId, SubmitEntry};
use ksvc_core::io_backend::{IoBackend, IoCompletion};
use ksvc_core::router::SyscallRouter;

use ksvc_module::basic_iouring::{BasicIoUring, BasicIoUringConfig};
use ksvc_module::probe_router::ProbeRouter;

use gvthread_core::id::GVThreadId;
use gvthread_core::state::Priority;
use gvthread_runtime::scheduler;

use crossbeam_queue::ArrayQueue;

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::thread;

/// An I/O request from a GVThread to the reactor.
#[derive(Debug)]
pub struct IoRequest {
    /// Correlation ID = GVThread's slot index.
    pub corr_id: CorrId,
    /// The syscall number.
    pub syscall_nr: u32,
    /// Syscall arguments.
    pub args: [u64; 6],
    /// Priority of the requesting GVThread (for wake).
    pub priority: Priority,
}

/// Reactor configuration.
pub struct ReactorConfig {
    /// io_uring SQ size (power of 2).
    pub sq_entries: u32,
    /// Max GVThreads (determines results slab size).
    pub max_slots: usize,
    /// MPSC queue capacity for incoming requests.
    pub queue_capacity: usize,
}

impl Default for ReactorConfig {
    fn default() -> Self {
        Self {
            sq_entries: 1024,
            max_slots: 65536,
            queue_capacity: 16384,
        }
    }
}

/// Shared state between reactor thread and GVThreads.
pub struct ReactorShared {
    /// MPSC queue: GVThreads push, reactor pops.
    pub(crate) request_queue: ArrayQueue<IoRequest>,
    /// Results slab indexed by GVThread slot index.
    /// Reactor writes, GVThread reads after wake.
    pub(crate) results: Box<[AtomicI64]>,
    /// Shutdown signal.
    pub(crate) shutdown: AtomicBool,
    /// How many slots are available.
    pub(crate) max_slots: usize,
}

impl ReactorShared {
    fn new(config: &ReactorConfig) -> Self {
        let mut results = Vec::with_capacity(config.max_slots);
        for _ in 0..config.max_slots {
            results.push(AtomicI64::new(0));
        }
        Self {
            request_queue: ArrayQueue::new(config.queue_capacity),
            results: results.into_boxed_slice(),
            shutdown: AtomicBool::new(false),
            max_slots: config.max_slots,
        }
    }

    /// Read the I/O result for a given slot. Called by the GVThread after wake.
    #[inline]
    pub fn read_result(&self, slot: u32) -> i64 {
        self.results[slot as usize].load(Ordering::Acquire)
    }

    /// Write the I/O result for a given slot. Called by the reactor.
    #[inline]
    fn write_result(&self, slot: u32, result: i64) {
        self.results[slot as usize].store(result, Ordering::Release);
    }
}

/// Handle to the reactor (held by the GVThread runtime).
pub struct Reactor {
    shared: Arc<ReactorShared>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Reactor {
    /// Create and start the reactor.
    pub fn start(config: ReactorConfig) -> Self {
        let shared = Arc::new(ReactorShared::new(&config));
        let shared_clone = shared.clone();

        let thread = thread::Builder::new()
            .name("ksvc-reactor".into())
            .spawn(move || {
                reactor_loop(shared_clone, config.sq_entries);
            })
            .expect("failed to spawn reactor thread");

        Self {
            shared,
            thread: Some(thread),
        }
    }

    /// Get a handle to the shared state (for GVThreads to submit requests).
    pub fn shared(&self) -> Arc<ReactorShared> {
        self.shared.clone()
    }

    /// Shutdown the reactor.
    pub fn shutdown(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Reactor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The reactor loop — runs on a dedicated OS thread.
fn reactor_loop(shared: Arc<ReactorShared>, sq_entries: u32) {
    // Initialize io_uring
    let mut io = BasicIoUring::new(BasicIoUringConfig {
        sq_entries,
        ..Default::default()
    }).expect("ksvc-reactor: io_uring setup failed");

    // Probe opcodes and build routing table
    let supported = io.probe_opcodes_static();
    let router = ProbeRouter::new(&supported);
    let counts = router.tier_counts();
    eprintln!(
        "ksvc-reactor: started — T0:{} T1:{} T2:{} T3:{} sq:{}",
        counts.tier0, counts.tier1, counts.tier2, counts.tier3, sq_entries
    );

    let mut comp_buf = vec![IoCompletion {
        corr_id: CorrId(0),
        result: 0,
        flags: 0,
    }; 256];

    // Batch buffer for draining the request queue
    let mut batch: Vec<IoRequest> = Vec::with_capacity(128);

    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut did_work = false;

        // ── Step 1: Drain request queue → submit to io_uring ──
        batch.clear();
        while let Some(req) = shared.request_queue.pop() {
            batch.push(req);
            if batch.len() >= 128 {
                break;
            }
        }

        for req in &batch {
            let route = router.route(req.syscall_nr);
            let entry = SubmitEntry {
                corr_id: req.corr_id,
                syscall_nr: req.syscall_nr,
                flags: 0,
                args: req.args,
            };

            match route.tier {
                ksvc_core::tier::Tier::IoUring => {
                    if let Err(_e) = io.submit_with_opcode(&entry, route.iouring_opcode) {
                        // Ring full or unsupported — return EAGAIN
                        let slot = req.corr_id.as_gvthread_id();
                        shared.write_result(slot, -(libc::EAGAIN as i64));
                        scheduler::wake_gvthread(
                            GVThreadId::new(slot),
                            req.priority,
                        );
                    }
                }
                _ => {
                    // Not routable to io_uring — return ENOSYS
                    // (Tier 2/3 fallback could be added here)
                    let slot = req.corr_id.as_gvthread_id();
                    shared.write_result(slot, -(libc::ENOSYS as i64));
                    scheduler::wake_gvthread(
                        GVThreadId::new(slot),
                        req.priority,
                    );
                }
            }
        }

        if !batch.is_empty() {
            did_work = true;
        }

        // ── Step 2: Flush + wait for completions ──
        if io.inflight() > 0 || !batch.is_empty() {
            // Flush any pending SQEs. If inflight > 0, wait for at least 1 CQE.
            // If nothing inflight, just flush (non-blocking).
            let min_wait = if io.inflight() > 0 && batch.is_empty() { 1 } else { 0 };
            let _ = io.flush_and_wait(min_wait);
        } else if !did_work {
            // Nothing happening — flush pending then brief sleep
            let _ = io.flush();
            std::thread::sleep(std::time::Duration::from_micros(50));
            continue;
        } else {
            let _ = io.flush();
        }

        // ── Step 3: Poll completions → write results + wake GVThreads ──
        let n = io.poll_completions(&mut comp_buf, 256);
        for i in 0..n {
            let cqe = &comp_buf[i];
            let slot = cqe.corr_id.as_gvthread_id();
            if slot == u32::MAX {
                continue; // Cancel sentinel or invalid
            }

            // Write result to slab
            shared.write_result(slot, cqe.result);

            // Wake the GVThread
            // We use Normal priority as default; the original priority
            // was stashed in the request but we don't track it per-inflight
            // for simplicity. The scheduler handles priority from metadata.
            scheduler::wake_gvthread(
                GVThreadId::new(slot),
                Priority::Normal,
            );
        }

        if n > 0 {
            did_work = true;
        }

        // If no work at all, yield to avoid busy-spinning
        if !did_work {
            std::thread::yield_now();
        }
    }

    // Shutdown: drain remaining completions
    io.shutdown();
    eprintln!("ksvc-reactor: shutdown");
}
