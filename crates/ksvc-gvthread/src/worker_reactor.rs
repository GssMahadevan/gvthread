//! # Per-worker io_uring reactor pool
//!
//! Each worker OS thread owns its own io_uring instance.  GVThreads submit
//! I/O directly to their worker's ring (inline, no MPSC queue, no cross-thread
//! hop).  The worker polls its own ring for completions and wakes GVThreads
//! on the same core — zero lock contention, zero cache-line bouncing.
//!
//! ## Comparison with shared `Reactor`
//!
//! ```text
//!  SHARED REACTOR (old)           WORKER REACTOR (new)
//!  ───────────────────            ────────────────────
//!  GVThread → MPSC → Reactor     GVThread → inline SQE push
//!  Reactor → CQE → Mutex → RQ   Worker poll → CQE → local queue
//!  3 cross-thread hops / I/O     0 cross-thread hops / I/O
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! let pool = WorkerReactorPool::init_global(num_workers, 1024, 100_000);
//! // ... hooks are auto-installed, workers will poll their own ring
//! ```

use ksvc_core::entry::{CorrId, SubmitEntry};
use ksvc_core::io_backend::{IoBackend, IoCompletion};
use ksvc_core::router::SyscallRouter;

use ksvc_module::basic_iouring::{BasicIoUring, BasicIoUringConfig};
use ksvc_module::probe_router::ProbeRouter;

use gvthread_core::id::GVThreadId;
use gvthread_core::state::Priority;
use gvthread_runtime::scheduler;

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

// ── Per-worker io_uring instance ─────────────────────────────────────

/// One io_uring + router per worker.  NOT thread-safe — only accessed
/// by its owning worker thread.
struct WorkerRing {
    io: BasicIoUring,
    router: ProbeRouter,
    comp_buf: Vec<IoCompletion>,
}

// ── Worker Reactor Pool ──────────────────────────────────────────────

/// Pool of per-worker io_uring instances.
///
/// # Safety
///
/// Each `UnsafeCell<WorkerRing>` is only accessed by its owning worker
/// thread (worker N only touches `rings[N]`).  The `results` slab uses
/// `AtomicI64` for safe cross-thread access (worker writes, GVThread reads
/// after wake).
pub struct WorkerReactorPool {
    /// Per-worker rings.  Index = worker_id.
    rings: Vec<UnsafeCell<WorkerRing>>,
    /// Results slab indexed by GVThread slot ID.
    results: Box<[AtomicI64]>,
    /// Number of workers.
    num_workers: usize,
    /// Shutdown flag.
    shutdown: AtomicBool,
}

// Safety: see struct doc.  Each UnsafeCell is worker-pinned.
unsafe impl Sync for WorkerReactorPool {}
unsafe impl Send for WorkerReactorPool {}

impl WorkerReactorPool {
    /// Create the pool (does NOT install hooks — call `install_hooks()` next).
    pub fn new(num_workers: usize, sq_entries: u32, max_slots: usize) -> Self {
        let mut rings = Vec::with_capacity(num_workers);

        for i in 0..num_workers {
            let io = BasicIoUring::new(BasicIoUringConfig {
                sq_entries,
                ..Default::default()
            })
            .unwrap_or_else(|e| panic!("worker-reactor[{}]: io_uring setup failed: {:?}", i, e));

            let supported = io.probe_opcodes_static();
            let router = ProbeRouter::new(&supported);

            if i == 0 {
                let counts = router.tier_counts();
                eprintln!(
                    "worker-reactor: {} workers × io_uring sq={} — T0:{} T1:{} T2:{} T3:{}",
                    num_workers, sq_entries,
                    counts.tier0, counts.tier1, counts.tier2, counts.tier3,
                );
            }

            rings.push(UnsafeCell::new(WorkerRing {
                io,
                router,
                comp_buf: vec![
                    IoCompletion { corr_id: CorrId(0), result: 0, flags: 0 };
                    256
                ],
            }));
        }

        let mut results = Vec::with_capacity(max_slots);
        for _ in 0..max_slots {
            results.push(AtomicI64::new(0));
        }

        Self {
            rings,
            results: results.into_boxed_slice(),
            num_workers,
            shutdown: AtomicBool::new(false),
        }
    }

    /// Create the pool, store it globally, and install worker hooks.
    ///
    /// Returns an `Arc` for passing to `GvtListener` / `GvtStream`.
    pub fn init_global(
        num_workers: usize,
        sq_entries: u32,
        max_slots: usize,
    ) -> Arc<Self> {
        let pool = Arc::new(Self::new(num_workers, sq_entries, max_slots));

        // Store in global so hooks + submit_and_park_worker can access it
        unsafe {
            GLOBAL_POOL = Some(pool.clone());
        }

        // Install hooks into the scheduler's worker loop
        scheduler::set_worker_io_hooks(
            hook_poll,
            hook_has_io,
            hook_wait_io,
        );

        pool
    }

    // ── Submit (called from GVThread context on worker thread) ───────

    /// Submit an I/O request to this worker's io_uring.
    ///
    /// Called inline from `submit_and_park_worker()` — no MPSC, no
    /// cross-thread hop.  The SQE is queued in the ring's SQ and will
    /// be flushed on the next `poll()` call in the worker loop.
    #[inline]
    pub(crate) fn submit(
        &self,
        worker_id: usize,
        slot: u32,
        syscall_nr: u32,
        args: &[u64; 6],
    ) {
        let ring = unsafe { &mut *self.rings[worker_id].get() };
        let route = ring.router.route(syscall_nr);

        let entry = SubmitEntry {
            corr_id: CorrId::from_gvthread_id(slot),
            syscall_nr,
            flags: 0,
            args: *args,
        };

        match route.tier {
            ksvc_core::tier::Tier::IoUring => {
                if let Err(_e) = ring.io.submit_with_opcode(&entry, route.iouring_opcode) {
                    // Ring full — return EAGAIN, wake immediately
                    self.results[slot as usize].store(-(libc::EAGAIN as i64), Ordering::Release);
                    scheduler::wake_gvthread(GVThreadId::new(slot), Priority::Normal);
                }
            }
            _ => {
                // Not routable to io_uring
                self.results[slot as usize].store(-(libc::ENOSYS as i64), Ordering::Release);
                scheduler::wake_gvthread(GVThreadId::new(slot), Priority::Normal);
            }
        }
    }

    // ── Poll (called from worker loop) ───────────────────────────────

    /// Non-blocking poll: flush pending SQEs, drain CQEs, wake GVThreads.
    /// Returns number of GVThreads woken.
    pub(crate) fn poll(&self, worker_id: usize) -> usize {
        let ring = unsafe { &mut *self.rings[worker_id].get() };

        // Flush any pending SQEs to kernel
        let _ = ring.io.flush();

        // Drain available CQEs
        let n = ring.io.poll_completions(&mut ring.comp_buf, 256);

        for i in 0..n {
            let cqe = &ring.comp_buf[i];
            let slot = cqe.corr_id.as_gvthread_id();
            if slot == u32::MAX {
                continue; // Cancel sentinel
            }
            self.results[slot as usize].store(cqe.result, Ordering::Release);
            scheduler::wake_gvthread(GVThreadId::new(slot), Priority::Normal);
        }

        n
    }

    /// Check if this worker has any inflight I/O operations.
    #[inline]
    pub(crate) fn has_inflight(&self, worker_id: usize) -> bool {
        let ring = unsafe { &*self.rings[worker_id].get() };
        ring.io.inflight() > 0
    }

    /// Blocking wait: flush + wait for ≥1 CQE, then drain all.
    /// Returns number of GVThreads woken.
    ///
    /// Uses `io_uring_enter(min_complete=1)` — the kernel blocks this
    /// thread until a CQE is available, then returns instantly.  Zero
    /// CPU waste while waiting.
    pub(crate) fn wait_and_poll(&self, worker_id: usize) -> usize {
        let ring = unsafe { &mut *self.rings[worker_id].get() };

        // Flush + block until ≥1 CQE
        let _ = ring.io.flush_and_wait(1);

        // Drain all available CQEs
        let n = ring.io.poll_completions(&mut ring.comp_buf, 256);

        for i in 0..n {
            let cqe = &ring.comp_buf[i];
            let slot = cqe.corr_id.as_gvthread_id();
            if slot == u32::MAX {
                continue;
            }
            self.results[slot as usize].store(cqe.result, Ordering::Release);
            scheduler::wake_gvthread(GVThreadId::new(slot), Priority::Normal);
        }

        n
    }

    // ── Results slab ─────────────────────────────────────────────────

    /// Read the I/O result for a GVThread slot.
    /// Called by the GVThread after it is woken.
    #[inline]
    pub fn read_result(&self, slot: u32) -> i64 {
        self.results[slot as usize].load(Ordering::Acquire)
    }

    /// Shutdown all worker rings.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        for cell in &self.rings {
            let ring = unsafe { &mut *cell.get() };
            ring.io.shutdown();
        }
        eprintln!("worker-reactor: shutdown ({} workers)", self.num_workers);
    }
}

impl Drop for WorkerReactorPool {
    fn drop(&mut self) {
        if !self.shutdown.load(Ordering::Relaxed) {
            self.shutdown();
        }
    }
}

// ── Global pool + hook functions ─────────────────────────────────────

/// Global pool reference.  Set once by `init_global`, read by hooks and
/// `submit_and_park_worker`.
static mut GLOBAL_POOL: Option<Arc<WorkerReactorPool>> = None;

/// Get the global worker reactor pool.
///
/// # Panics
/// Panics if `init_global` was not called.
#[inline]
pub fn global_pool() -> &'static WorkerReactorPool {
    unsafe {
        GLOBAL_POOL
            .as_ref()
            .expect("WorkerReactorPool not initialized — call init_global() first")
    }
}

// Hook functions installed into the scheduler's worker loop.
// They delegate to the global pool.

fn hook_poll(worker_id: usize) -> usize {
    unsafe {
        match &GLOBAL_POOL {
            Some(pool) => pool.poll(worker_id),
            None => 0,
        }
    }
}

fn hook_has_io(worker_id: usize) -> bool {
    unsafe {
        match &GLOBAL_POOL {
            Some(pool) => pool.has_inflight(worker_id),
            None => false,
        }
    }
}

fn hook_wait_io(worker_id: usize) -> usize {
    unsafe {
        match &GLOBAL_POOL {
            Some(pool) => pool.wait_and_poll(worker_id),
            None => 0,
        }
    }
}
