//! Timer thread for preemption monitoring and sleep handling
//!
//! Sleep queue design:
//! - BinaryHeap (min-heap) for O(1) next wake time peek
//! - SpinLock protection (no syscalls, safe from GVThread stack)
//! - Pre-allocated capacity to avoid allocation during push

use crate::worker::worker_states;
use crate::config::SchedulerConfig;
use crate::tls;
use crate::scheduler;
use crate::memory;
use gvthread_core::id::GVThreadId;
use gvthread_core::SpinLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::collections::BinaryHeap;
use std::cmp::Ordering as CmpOrdering;

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

#[inline]
pub fn coarse_now_ns() -> u64 {
    COARSE_TIME_NS.load(Ordering::Acquire)
}

#[inline]
pub fn now_ns() -> u64 {
    START_INSTANT.get()
        .map(|s| s.elapsed().as_nanos() as u64)
        .unwrap_or(0)
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

pub fn init_sleep_queue_with_capacity(capacity: usize) {
    init_time();
    let mut queue = SLEEP_QUEUE.lock();
    *queue = Some(BinaryHeap::with_capacity(capacity));
}

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
    
    let meta = unsafe { &*(meta_base as *const gvthread_core::GVThreadMetadata) };
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

#[inline]
pub fn sleep_ms(ms: u64) {
    sleep(Duration::from_millis(ms));
}

#[inline]
pub fn sleep_us(us: u64) {
    sleep(Duration::from_micros(us));
}

// ============================================================================
// Timer Thread
// ============================================================================

pub struct TimerThread {
    handle: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    time_slice_ns: u64,
    grace_period_ns: u64,
    check_interval: Duration,
    enable_forced_preempt: bool,
}

impl TimerThread {
    pub fn new(config: &SchedulerConfig) -> Self {
        Self {
            handle: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            time_slice_ns: config.time_slice.as_nanos() as u64,
            grace_period_ns: config.grace_period.as_nanos() as u64,
            check_interval: config.timer_interval,
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
    
    pub fn join(mut self) {
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