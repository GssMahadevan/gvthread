//! Timer thread for preemption monitoring
//!
//! The timer thread periodically checks all workers to detect
//! GVThreads that have been running too long without yielding.
//! Also handles the sleep queue for GVThread-aware sleeping.

use crate::worker::worker_states;
use crate::config::SchedulerConfig;
use crate::tls;
use crate::scheduler;
use gvthread_core::id::GVThreadId;
use gvthread_core::state::Priority;
use gvthread_core::SpinLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::collections::BinaryHeap;
use std::cmp::Ordering as CmpOrdering;

/// Timer thread state
pub struct TimerThread {
    /// Handle to the timer thread
    handle: Option<JoinHandle<()>>,
    
    /// Shutdown flag (shared with timer loop)
    shutdown: Arc<AtomicBool>,
    
    /// Configuration
    time_slice_ns: u64,
    grace_period_ns: u64,
    check_interval: Duration,
    enable_forced_preempt: bool,
}

/// Per-worker tracking state (owned by timer thread)
struct WorkerWatch {
    /// Last seen activity counter
    last_counter: u32,
    
    /// Time when we first saw the counter unchanged
    first_stall_time: Option<Instant>,
}

// ============================================================================
// Pending Sleep Ring Buffer (lock-free from GVThread side)
// ============================================================================

/// Ring buffer for pending sleep notifications
/// GVThreads atomically push, timer thread drains
const PENDING_SLEEP_CAPACITY: usize = 65536;

struct PendingSleepRing {
    /// Each slot: 0 = empty, or (gvthread_id << 32) | generation
    slots: Vec<AtomicU64>,
    /// Write position (only GVThreads increment)
    write_pos: AtomicUsize,
    /// Read position (only timer thread increments)  
    read_pos: AtomicUsize,
}

static PENDING_SLEEPS: OnceLock<PendingSleepRing> = OnceLock::new();

fn init_pending_sleeps(capacity: usize) {
    let _ = PENDING_SLEEPS.get_or_init(|| {
        let mut slots = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            slots.push(AtomicU64::new(0));
        }
        PendingSleepRing {
            slots,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
        }
    });
}

/// Notify timer thread that a GVThread wants to sleep
/// Called from GVThread stack - must be lock-free!
fn notify_pending_sleep(gvthread_id: u32, generation: u32) {
    let ring = PENDING_SLEEPS.get().expect("Pending sleeps not initialized");
    
    // Claim a slot atomically
    let pos = ring.write_pos.fetch_add(1, Ordering::AcqRel) % ring.slots.len();
    
    // Write the notification (GVThread ID + generation for validation)
    let value = ((gvthread_id as u64) << 32) | (generation as u64);
    ring.slots[pos].store(value, Ordering::Release);
}

/// Drain pending sleep notifications into the sleep queue
/// Called from timer thread only
fn drain_pending_sleeps() {
    let ring = match PENDING_SLEEPS.get() {
        Some(r) => r,
        None => return,
    };
    
    let write_pos = ring.write_pos.load(Ordering::Acquire);
    let mut read_pos = ring.read_pos.load(Ordering::Relaxed);
    
    // Process all pending notifications
    while read_pos != write_pos {
        let pos = read_pos % ring.slots.len();
        let value = ring.slots[pos].swap(0, Ordering::AcqRel);
        
        if value != 0 {
            let gvthread_id = (value >> 32) as u32;
            let generation = value as u32;
            
            // Get wake_time from metadata
            let meta_ptr = crate::memory::get_metadata_ptr(gvthread_id);
            let meta = unsafe { &*meta_ptr };
            
            // Verify generation matches (slot wasn't reused)
            if meta.generation.load(Ordering::Acquire) == generation {
                let wake_time = meta.wake_time_ns.load(Ordering::Acquire);
                let priority = meta.get_priority();
                
                // Clear sleep flag
                meta.sleep_flag.store(0, Ordering::Release);
                
                // Add to actual sleep queue (now on timer thread stack - safe!)
                add_to_sleep_queue(SleepEntry {
                    wake_time_ns: wake_time,
                    gvthread_id: GVThreadId::new(gvthread_id),
                    priority,
                    generation,
                });
            }
        }
        
        read_pos += 1;
    }
    
    ring.read_pos.store(read_pos, Ordering::Release);
}

// ============================================================================
// Sleep Queue (only accessed from timer thread now)
// ============================================================================

/// Entry in the sleep queue
#[derive(Debug, Clone, Copy)]
struct SleepEntry {
    wake_time_ns: u64,
    gvthread_id: GVThreadId,
    priority: Priority,
    generation: u32,  // To detect stale wakes after slot reuse
}

// Ordering for min-heap (smallest wake_time first)
impl Ord for SleepEntry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // Reverse order for min-heap
        other.wake_time_ns.cmp(&self.wake_time_ns)
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

/// Global sleep queue
static SLEEP_QUEUE: SpinLock<Option<BinaryHeap<SleepEntry>>> = SpinLock::new(None);

/// Initialize the sleep queue with given capacity
pub fn init_sleep_queue_with_capacity(capacity: usize) {
    // Initialize pending sleeps ring buffer
    init_pending_sleeps(capacity);
    
    // Initialize the actual sleep queue
    let mut queue = SLEEP_QUEUE.lock();
    // Pre-allocate to avoid reallocation
    *queue = Some(BinaryHeap::with_capacity(capacity));
}

/// Initialize the sleep queue with default capacity
pub fn init_sleep_queue() {
    init_sleep_queue_with_capacity(65536);
}

/// Add a GVThread to the sleep queue
/// NOTE: Only called from timer thread now (safe to use SpinLock)
fn add_to_sleep_queue(entry: SleepEntry) {
    let mut queue = SLEEP_QUEUE.lock();
    if let Some(ref mut q) = *queue {
        q.push(entry);
    }
}

/// Process sleep queue - wake up GVThreads whose time has come
/// Called by timer thread
fn process_sleep_queue() {
    let now = now_ns();
    
    loop {
        // Peek at the next entry
        let entry = {
            let mut queue = SLEEP_QUEUE.lock();
            if let Some(ref mut q) = *queue {
                if let Some(entry) = q.peek() {
                    if entry.wake_time_ns <= now {
                        q.pop()
                    } else {
                        None // Not time yet
                    }
                } else {
                    None // Queue empty
                }
            } else {
                None
            }
        };
        
        match entry {
            Some(entry) => {
                // Wake up this GVThread (with generation check)
                scheduler::wake_gvthread_checked(
                    entry.gvthread_id, 
                    entry.priority,
                    entry.generation,
                );
            }
            None => break, // No more entries ready
        }
    }
}

/// Sleep the current GVThread for the specified duration
/// 
/// This yields the GVThread and schedules it to wake up after the duration.
/// Unlike `std::thread::sleep`, this does NOT block the worker thread.
///
/// This function is LOCK-FREE from GVThread stack - only atomic operations.
///
/// # Panics
/// Panics if called from outside a GVThread.
pub fn sleep(duration: Duration) {
    if !tls::is_in_gvthread() {
        // Fallback for non-GVThread context
        std::thread::sleep(duration);
        return;
    }
    
    let gvthread_id = tls::current_gvthread_id();
    let meta_base = tls::current_gvthread_base();
    
    if meta_base.is_null() {
        std::thread::sleep(duration);
        return;
    }
    
    // Get metadata pointer
    let meta = unsafe { &*(meta_base as *const gvthread_core::GVThreadMetadata) };
    let generation = meta.get_generation();
    
    // Calculate wake time
    let wake_time_ns = now_ns() + duration.as_nanos() as u64;
    
    // Store wake time in metadata (atomic - safe from GVThread stack)
    meta.wake_time_ns.store(wake_time_ns, Ordering::Release);
    
    // Set sleep flag (atomic - safe from GVThread stack)
    meta.sleep_flag.store(1, Ordering::Release);
    
    // Notify timer thread (lock-free ring buffer - safe from GVThread stack)
    notify_pending_sleep(gvthread_id.as_u32(), generation);
    
    // Mark as blocked and yield
    scheduler::block_current();
}

/// Sleep for specified milliseconds
#[inline]
pub fn sleep_ms(ms: u64) {
    sleep(Duration::from_millis(ms));
}

/// Sleep for specified microseconds
#[inline]
pub fn sleep_us(us: u64) {
    sleep(Duration::from_micros(us));
}

impl TimerThread {
    /// Create a new timer thread (not started)
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
    
    /// Start the timer thread
    pub fn start(&mut self, num_workers: usize) {
        let shutdown = Arc::clone(&self.shutdown);
        let time_slice_ns = self.time_slice_ns;
        let grace_period_ns = self.grace_period_ns;
        let check_interval = self.check_interval;
        let enable_forced_preempt = self.enable_forced_preempt;
        
        let handle = thread::Builder::new()
            .name("gvthread-timer".to_string())
            .spawn(move || {
                timer_loop(
                    num_workers,
                    time_slice_ns,
                    grace_period_ns,
                    check_interval,
                    enable_forced_preempt,
                    shutdown,
                );
            })
            .expect("Failed to spawn timer thread");
        
        self.handle = Some(handle);
    }
    
    /// Signal shutdown
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
    
    /// Wait for timer thread to finish
    pub fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Main timer loop
fn timer_loop(
    num_workers: usize,
    time_slice_ns: u64,
    grace_period_ns: u64,
    _check_interval: Duration,  // Ignored - we compute dynamically
    enable_forced_preempt: bool,
    shutdown: Arc<AtomicBool>,
) {
    use gvthread_core::env::env_get;
    
    // Per-worker tracking
    let mut watches: Vec<WorkerWatch> = (0..num_workers)
        .map(|_| WorkerWatch {
            last_counter: 0,
            first_stall_time: None,
        })
        .collect();
    
    // Configurable min/max sleep
    let min_sleep_us: u64 = env_get("GVT_TIMER_MIN_US", 100);   // 100us min
    let max_sleep_ms: u64 = env_get("GVT_TIMER_MAX_MS", 10);    // 10ms max
    
    while !shutdown.load(Ordering::Acquire) {
        // Check sleep queue for next wake time
        let sleep_duration = {
            let queue = SLEEP_QUEUE.lock();
            if let Some(ref q) = *queue {
                if let Some(entry) = q.peek() {
                    let now = now_ns();
                    if entry.wake_time_ns <= now {
                        // Work ready now - don't sleep
                        Duration::from_micros(0)
                    } else {
                        // Sleep until next wake, capped
                        let wait_ns = entry.wake_time_ns - now;
                        let wait_ms = (wait_ns / 1_000_000).min(max_sleep_ms);
                        Duration::from_millis(wait_ms.max(min_sleep_us / 1000))
                    }
                } else {
                    // Queue empty - sleep max
                    Duration::from_millis(max_sleep_ms)
                }
            } else {
                Duration::from_millis(max_sleep_ms)
            }
        };
        
        if !sleep_duration.is_zero() {
            thread::sleep(sleep_duration);
        }
        
        // Update coarse time for low-overhead access
        update_coarse_time();
        
        // Drain pending sleep notifications from GVThreads
        // This moves them from lock-free ring buffer to BinaryHeap
        drain_pending_sleeps();
        
        // Process sleep queue - wake up sleeping GVThreads
        process_sleep_queue();
        
        let now = Instant::now();
        
        for i in 0..num_workers {
            let worker = worker_states().get(i);
            let watch = &mut watches[i];
            
            // Get current GVThread
            let gthread_id = worker.current_gthread.load(Ordering::Acquire);
            if gthread_id == gvthread_core::constants::GVTHREAD_NONE {
                // Worker is idle, reset watch
                watch.first_stall_time = None;
                continue;
            }
            
            // Check activity counter
            let counter = worker.activity_counter.load(Ordering::Acquire);
            
            if counter == watch.last_counter {
                // No activity since last check
                if watch.first_stall_time.is_none() {
                    watch.first_stall_time = Some(now);
                } else if let Some(stall_start) = watch.first_stall_time {
                    let stall_duration = now.duration_since(stall_start);
                    
                    if stall_duration.as_nanos() as u64 > time_slice_ns {
                        // GVThread has been running too long
                        handle_stuck_gvthread(
                            i,
                            gthread_id,
                            enable_forced_preempt,
                            grace_period_ns,
                        );
                        
                        // Reset watch after handling
                        watch.first_stall_time = None;
                    }
                }
            } else {
                // Activity detected, reset watch
                watch.last_counter = counter;
                watch.first_stall_time = None;
            }
        }
    }
}

/// Handle a stuck GVThread
fn handle_stuck_gvthread(
    worker_id: usize,
    gthread_id: u32,
    enable_forced_preempt: bool,
    grace_period_ns: u64,
) {
    // Step 1: Set preempt flag (cooperative)
    // TODO: Get metadata pointer and set flag
    // let meta = memory::get_metadata_ptr(gthread_id);
    // unsafe { (*meta).request_preempt(); }
    
    if !enable_forced_preempt {
        return;
    }
    
    // Step 2: Wait grace period
    thread::sleep(Duration::from_nanos(grace_period_ns));
    
    // Step 3: Check if still stuck
    let worker = worker_states().get(worker_id);
    if worker.current_gthread.load(Ordering::Acquire) == gthread_id {
        // Still stuck, send SIGURG
        let thread_id = worker.thread_id.load(Ordering::Acquire);
        if thread_id != 0 {
            // TODO: Send SIGURG
            // crate::signal::send_sigurg(thread_id);
        }
    }
}

/// Get current time in nanoseconds
/// 
/// Uses Instant for monotonic time - more reliable than SystemTime
/// and works better on custom stacks.
#[inline]
pub fn now_ns() -> u64 {
    use std::time::Instant;
    
    // Use a static start time to convert Instant to nanoseconds
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    
    start.elapsed().as_nanos() as u64
}

/// Coarse timestamp (updated periodically for low-overhead access)
static COARSE_TIME_NS: AtomicU64 = AtomicU64::new(0);

/// Get coarse time (faster but less precise)
#[inline]
pub fn coarse_now_ns() -> u64 {
    let t = COARSE_TIME_NS.load(Ordering::Relaxed);
    if t == 0 {
        // Not yet initialized, use precise time
        now_ns()
    } else {
        t
    }
}

/// Update coarse time (called by timer thread)
pub fn update_coarse_time() {
    COARSE_TIME_NS.store(now_ns(), Ordering::Relaxed);
}