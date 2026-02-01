//! Timer subsystem for GVThread runtime
//!
//! Provides:
//! - Sleep queue (BinaryHeap) for GVThread sleep/wake
//! - Preemption monitoring for stuck GVThreads
//! - Pluggable timer backends for future optimization
//!
//! # Architecture
//!
//! ```text
//!     sleep() ──► SLEEP_QUEUE (BinaryHeap)
//!                      │
//!                      ▼
//!     TimerThread ──► process_sleep_queue() ──► wake_gvthread()
//!           │
//!           └──► check_preemption() ──► set preempt flag / send signal
//! ```

mod entry;
pub mod impls;
mod registry;
mod worker;

pub use entry::{TimerEntry, TimerHandle, TimerType};
pub use impls::{create_backend, HeapTimerBackend, TimerBackendType};
pub use registry::TimerRegistry;
pub use worker::{spawn_timer_thread, TimerThreadConfig, TimerThreadHandle, TimerWakeCallback};

use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gvthread_core::id::GVThreadId;
use gvthread_core::SpinLock;

use crate::config::SchedulerConfig;
use crate::memory;
use crate::scheduler;
use crate::tls;
use crate::worker::worker_states;

/// Expired timer info passed to ready queue
#[derive(Debug, Clone)]
pub struct ExpiredTimer {
    pub gvt_id: u32,
    pub worker_affinity: Option<u8>,
    pub timer_type: TimerType,
}

/// Core timer trait - implement this for different backends
pub trait TimerBackend: Send + Sync {
    fn insert(&self, entry: TimerEntry) -> TimerHandle;
    fn cancel(&self, handle: TimerHandle) -> bool;
    fn poll_expired(&self, now: Instant) -> Vec<ExpiredTimer>;
    fn next_deadline(&self) -> Option<Instant>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }
    fn name(&self) -> &'static str { "unknown" }
}

// ============================================================================
// Time Utilities
// ============================================================================

static COARSE_TIME_NS: AtomicU64 = AtomicU64::new(0);
static START_INSTANT: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn init_time() {
    let _ = START_INSTANT.get_or_init(Instant::now);
    update_coarse_time();
}

fn update_coarse_time() {
    if let Some(start) = START_INSTANT.get() {
        let elapsed = start.elapsed().as_nanos() as u64;
        COARSE_TIME_NS.store(elapsed, Ordering::Release);
    }
}

/// Get coarse time (updated by timer thread, very cheap)
#[inline]
pub fn coarse_now_ns() -> u64 {
    COARSE_TIME_NS.load(Ordering::Acquire)
}

/// Get precise monotonic time in nanoseconds
#[inline]
pub fn now_ns() -> u64 {
    START_INSTANT.get()
        .map(|s| s.elapsed().as_nanos() as u64)
        .unwrap_or(0)
}

/// Get current monotonic time in microseconds
#[inline]
pub fn now_us() -> u64 {
    now_ns() / 1_000
}

/// Get current monotonic time in milliseconds
#[inline]
pub fn now_ms() -> u64 {
    now_ns() / 1_000_000
}

// ============================================================================
// Sleep Queue - BinaryHeap (min-heap by wake_time)
// ============================================================================

#[derive(Clone, Copy)]
struct SleepEntry {
    wake_time_ns: u64,
    gvthread_id: u32,
    generation: u32,
}

// Min-heap ordering (smallest wake_time first)
impl Ord for SleepEntry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other.wake_time_ns.cmp(&self.wake_time_ns) // Reversed for min-heap
    }
}

impl PartialOrd for SleepEntry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SleepEntry {
    fn eq(&self, other: &Self) -> bool {
        self.wake_time_ns == other.wake_time_ns
    }
}

impl Eq for SleepEntry {}

/// Sleep queue - protected by SpinLock (safe from GVThread stack)
static SLEEP_QUEUE: SpinLock<Option<BinaryHeap<SleepEntry>>> = SpinLock::new(None);

/// Initialize sleep queue with capacity
pub fn init_sleep_queue_with_capacity(capacity: usize) {
    init_time();
    let mut queue = SLEEP_QUEUE.lock();
    *queue = Some(BinaryHeap::with_capacity(capacity));
}

/// Initialize sleep queue with default capacity
pub fn init_sleep_queue() {
    init_sleep_queue_with_capacity(65536);
}

/// Add to sleep queue - called from GVThread (slot already activated)
fn add_to_sleep_queue(entry: SleepEntry) {
    let mut queue = SLEEP_QUEUE.lock();
    if let Some(ref mut q) = *queue {
        q.push(entry);
    }
}

/// Process sleep queue - wake expired GVThreads
fn process_sleep_queue() {
    let now = now_ns();
    
    loop {
        // Peek and pop under lock
        let entry = {
            let mut queue = SLEEP_QUEUE.lock();
            if let Some(ref mut q) = *queue {
                if let Some(top) = q.peek() {
                    if top.wake_time_ns <= now {
                        q.pop()
                    } else {
                        None // Not expired yet
                    }
                } else {
                    None // Empty
                }
            } else {
                None
            }
        };
        
        match entry {
            Some(entry) => {
                // Verify generation (slot not reused)
                let meta_ptr = memory::get_metadata_ptr(entry.gvthread_id);
                let meta = unsafe { &*meta_ptr };
                
                if meta.get_generation() == entry.generation {
                    let priority = meta.get_priority();
                    // wake_gvthread will set state to Ready and push to queue
                    scheduler::wake_gvthread(GVThreadId::new(entry.gvthread_id), priority);
                }
            }
            None => break, // No more expired entries
        }
    }
}

/// Get time until next wake (for timer sleep optimization)
fn time_until_next_wake() -> Option<Duration> {
    let queue = SLEEP_QUEUE.lock();
    if let Some(ref q) = *queue {
        if let Some(top) = q.peek() {
            let now = now_ns();
            if top.wake_time_ns > now {
                let wait_ns = top.wake_time_ns - now;
                return Some(Duration::from_nanos(wait_ns));
            } else {
                return Some(Duration::ZERO); // Already expired
            }
        }
    }
    None // Empty queue
}

// ============================================================================
// Sleep API
// ============================================================================

/// Sleep the current GVThread for the specified duration.
pub fn sleep(duration: Duration) {
    if !tls::is_in_gvthread() {
        std::thread::sleep(duration);
        return;
    }
    
    let gvthread_id = tls::current_gvthread_id();
    let meta_base = tls::current_gvthread_base();
    
    if meta_base.is_null() {
        std::thread::sleep(duration);
        return;
    }
    
    let meta = unsafe { &*(meta_base as *const gvthread_core::metadata::GVThreadMetadata) };
    let generation = meta.get_generation();
    
    // Calculate wake time
    let wake_time_ns = now_ns() + duration.as_nanos() as u64;
    
    // Store wake time in metadata (for debugging)
    meta.wake_time_ns.store(wake_time_ns, Ordering::Release);
    
    // Add to sleep queue (SpinLock is safe - no syscalls)
    add_to_sleep_queue(SleepEntry {
        wake_time_ns,
        gvthread_id: gvthread_id.as_u32(),
        generation,
    });
    
    // Block and yield
    scheduler::block_current();
}

/// Sleep for the specified number of milliseconds
#[inline]
pub fn sleep_ms(ms: u64) {
    sleep(Duration::from_millis(ms));
}

/// Sleep for the specified number of microseconds
#[inline]
pub fn sleep_us(us: u64) {
    sleep(Duration::from_micros(us));
}

/// Sleep for the specified number of nanoseconds
#[inline]
pub fn sleep_ns(ns: u64) {
    sleep(Duration::from_nanos(ns));
}

// ============================================================================
// Timer Thread
// ============================================================================

pub struct TimerThread {
    handle: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    time_slice_ns: u64,
    grace_period_ns: u64,
    enable_forced_preempt: bool,
}

impl TimerThread {
    pub fn new(config: &SchedulerConfig) -> Self {
        Self {
            handle: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            time_slice_ns: config.time_slice.as_nanos() as u64,
            grace_period_ns: config.grace_period.as_nanos() as u64,
            enable_forced_preempt: config.enable_forced_preempt,
        }
    }
    
    pub fn start(&mut self, num_workers: usize, _max_gvthreads: usize) {
        let shutdown = Arc::clone(&self.shutdown);
        let time_slice_ns = self.time_slice_ns;
        let grace_period_ns = self.grace_period_ns;
        let enable_forced_preempt = self.enable_forced_preempt;
        
        let handle = thread::Builder::new()
            .name("gvthread-timer".to_string())
            .spawn(move || {
                timer_loop(
                    num_workers,
                    time_slice_ns,
                    grace_period_ns,
                    enable_forced_preempt,
                    shutdown,
                );
            })
            .expect("Failed to spawn timer thread");
        
        self.handle = Some(handle);
    }
    
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
    
    pub fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ============================================================================
// Timer Loop
// ============================================================================

struct WorkerWatch {
    last_counter: u32,
    first_stall_time: Option<Instant>,
}

fn timer_loop(
    num_workers: usize,
    time_slice_ns: u64,
    _grace_period_ns: u64,
    enable_forced_preempt: bool,
    shutdown: Arc<AtomicBool>,
) {
    use gvthread_core::env::env_get;
    
    let mut watches: Vec<WorkerWatch> = (0..num_workers)
        .map(|_| WorkerWatch {
            last_counter: 0,
            first_stall_time: None,
        })
        .collect();
    
    let max_sleep_ms: u64 = env_get("GVT_TIMER_MAX_MS", 10);
    
    while !shutdown.load(Ordering::Acquire) {
        // Compute sleep duration based on next wake time
        let sleep_duration = match time_until_next_wake() {
            Some(d) if d.is_zero() => Duration::from_micros(100), // Work ready, quick check
            Some(d) => d.min(Duration::from_millis(max_sleep_ms)),
            None => Duration::from_millis(max_sleep_ms), // Empty queue
        };
        
        thread::sleep(sleep_duration);
        
        update_coarse_time();
        
        // Process sleep queue - wake expired GVThreads
        process_sleep_queue();
        
        // Check for stuck GVThreads (preemption)
        let now_instant = Instant::now();
        
        for i in 0..num_workers {
            let worker = worker_states().get(i);
            let watch = &mut watches[i];
            
            let gthread_id = worker.current_gthread.load(Ordering::Acquire);
            if gthread_id == gvthread_core::constants::GVTHREAD_NONE {
                watch.first_stall_time = None;
                continue;
            }
            
            let counter = worker.activity_counter.load(Ordering::Acquire);
            
            if counter == watch.last_counter {
                if watch.first_stall_time.is_none() {
                    watch.first_stall_time = Some(now_instant);
                } else if let Some(stall_start) = watch.first_stall_time {
                    let stall_ns = now_instant.duration_since(stall_start).as_nanos() as u64;
                    
                    if stall_ns > time_slice_ns {
                        handle_stuck_gvthread(i, gthread_id, enable_forced_preempt);
                        watch.first_stall_time = None;
                    }
                }
            } else {
                watch.last_counter = counter;
                watch.first_stall_time = None;
            }
        }
    }
}

fn handle_stuck_gvthread(
    worker_id: usize,
    gthread_id: u32,
    enable_forced_preempt: bool,
) {
    let meta_ptr = memory::get_metadata_ptr(gthread_id);
    let meta = unsafe { &*meta_ptr };
    
    // Set preempt flag
    meta.preempt_flag.store(1, Ordering::Release);
    
    if enable_forced_preempt {
        #[cfg(unix)]
        {
            let tid = worker_states().get(worker_id).thread_id.load(Ordering::Relaxed);
            if tid != 0 {
                let _ = crate::signal::send_sigurg(tid);
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timer_handle_uniqueness() {
        let h1 = TimerHandle::new();
        let h2 = TimerHandle::new();
        let h3 = TimerHandle::new();

        assert_ne!(h1, h2);
        assert_ne!(h2, h3);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_timer_entry_creation() {
        let entry = TimerEntry::preempt(42, 3, Duration::from_millis(10));
        assert_eq!(entry.gvt_id, 42);
        assert_eq!(entry.worker_affinity, Some(3));
        assert!(matches!(entry.timer_type, TimerType::Preempt));
    }

    #[test]
    fn test_heap_backend_basic() {
        let backend = HeapTimerBackend::new();
        assert!(backend.is_empty());

        let entry = TimerEntry::sleep(1, Duration::from_millis(100), None);
        let handle = backend.insert(entry);

        assert_eq!(backend.len(), 1);
        assert!(backend.next_deadline().is_some());

        // Cancel
        assert!(backend.cancel(handle));

        // Poll should skip cancelled
        let expired = backend.poll_expired(Instant::now() + Duration::from_secs(1));
        assert!(expired.is_empty());
    }

    #[test]
    fn test_now_ns_monotonic() {
        init_time();
        let t1 = now_ns();
        std::thread::sleep(Duration::from_micros(100));
        let t2 = now_ns();
        assert!(t2 > t1);
    }

    #[test]
    fn test_init_sleep_queue() {
        init_sleep_queue_with_capacity(1000);
        // Should not panic on second init
        init_sleep_queue_with_capacity(500);
    }
}