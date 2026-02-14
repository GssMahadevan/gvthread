//! `FixedPool` — default `WorkerPool` implementation.
//!
//! Spawns N OS threads at creation. Workers dequeue from a lock-free
//! MPMC queue, execute the syscall via libc, and push results to a
//! lock-free result queue. The dispatcher polls the result queue.
//!
//! No dynamic scaling. Simple, predictable, safe.

use ksvc_core::entry::SubmitEntry;
use ksvc_core::error::{KsvcError, Result};
use ksvc_core::worker::{WorkerCompletion, WorkerPool};

use crossbeam_queue::ArrayQueue;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

/// A work item for the pool.
#[derive(Clone, Copy)]
struct WorkItem {
    entry: SubmitEntry,
}

/// Shared state between dispatcher and workers.
struct PoolInner {
    /// Work queue: dispatcher → workers.
    work_queue: ArrayQueue<WorkItem>,
    /// Result queue: workers → dispatcher.
    result_queue: ArrayQueue<WorkerCompletion>,
    /// Number of workers currently executing a syscall.
    active: AtomicUsize,
    /// Shutdown flag.
    shutdown: AtomicBool,
    /// Total worker count.
    total: usize,
}

pub struct FixedPool {
    inner: Arc<PoolInner>,
    handles: Vec<thread::JoinHandle<()>>,
}

impl FixedPool {
    /// Create a pool with `n` workers.
    ///
    /// `queue_depth`: max pending work items before enqueue fails.
    pub fn new(n: usize, queue_depth: usize) -> Self {
        let n = n.max(1).min(32);
        let inner = Arc::new(PoolInner {
            work_queue: ArrayQueue::new(queue_depth),
            result_queue: ArrayQueue::new(queue_depth),
            active: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            total: n,
        });

        let mut handles = Vec::with_capacity(n);
        for worker_id in 0..n {
            let inner = Arc::clone(&inner);
            let handle = thread::Builder::new()
                .name(format!("ksvc-worker-{}", worker_id))
                .spawn(move || worker_loop(inner, worker_id))
                .expect("failed to spawn worker thread");
            handles.push(handle);
        }

        FixedPool { inner, handles }
    }

    /// Default pool sizing: min(8, nproc/2), at least 2.
    pub fn auto_sized(queue_depth: usize) -> Self {
        let cpus = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let n = (cpus / 2).max(2).min(8);
        Self::new(n, queue_depth)
    }
}

impl WorkerPool for FixedPool {
    fn enqueue(&self, entry: &SubmitEntry) -> Result<()> {
        if self.inner.shutdown.load(Ordering::Relaxed) {
            return Err(KsvcError::WorkerUnavailable);
        }
        let item = WorkItem { entry: *entry };
        self.inner
            .work_queue
            .push(item)
            .map_err(|_| KsvcError::WorkerUnavailable)
    }

    fn poll_completions(&self, buf: &mut [WorkerCompletion], max: usize) -> usize {
        let mut count = 0;
        while count < max && count < buf.len() {
            match self.inner.result_queue.pop() {
                Some(comp) => {
                    buf[count] = comp;
                    count += 1;
                }
                None => break,
            }
        }
        count
    }

    fn active_workers(&self) -> usize {
        self.inner.active.load(Ordering::Relaxed)
    }

    fn total_workers(&self) -> usize {
        self.inner.total
    }

    fn max_workers(&self) -> usize {
        self.inner.total
    }

    fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        // Workers will see the flag and exit after current work
    }
}

impl Drop for FixedPool {
    fn drop(&mut self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        // Workers will exit after draining or on next poll.
        // We don't join here to avoid blocking the dispatcher.
        // In production, call shutdown() + join explicitly.
    }
}

/// Worker thread main loop.
fn worker_loop(inner: Arc<PoolInner>, _worker_id: usize) {
    loop {
        if inner.shutdown.load(Ordering::Relaxed) {
            break;
        }

        match inner.work_queue.pop() {
            Some(item) => {
                inner.active.fetch_add(1, Ordering::Relaxed);
                let result = execute_syscall(&item.entry);
                inner.active.fetch_sub(1, Ordering::Relaxed);

                let completion = WorkerCompletion {
                    corr_id: item.entry.corr_id,
                    result,
                };
                // If result queue is full, we spin-retry briefly.
                // This should be rare — the dispatcher drains it each loop.
                let mut retries = 0;
                while inner.result_queue.push(completion).is_err() {
                    retries += 1;
                    if retries > 1000 || inner.shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
            None => {
                // No work available — brief sleep to avoid busy-wait.
                // In production, use a condvar or futex for wake-on-push.
                thread::park_timeout(std::time::Duration::from_millis(1));
            }
        }
    }
}

/// Execute a Tier 2 syscall via libc::syscall.
///
/// This runs on a worker thread — it MAY block. That's the point.
fn execute_syscall(entry: &SubmitEntry) -> i64 {
    let a = &entry.args;
    // Safety: we're making a raw syscall with the provided arguments.
    // The caller (GVThread) is responsible for argument validity.
    let ret = unsafe {
        libc::syscall(
            entry.syscall_nr as libc::c_long,
            a[0] as libc::c_long,
            a[1] as libc::c_long,
            a[2] as libc::c_long,
            a[3] as libc::c_long,
            a[4] as libc::c_long,
            a[5] as libc::c_long,
        )
    };
    if ret < 0 {
        // libc::syscall returns -1 and sets errno.
        // Convert to negative errno for KSVC convention.
        let errno = unsafe { *libc::__errno_location() };
        -(errno as i64)
    } else {
        ret as i64
    }
}
