//! Timer thread for preemption monitoring
//!
//! The timer thread periodically checks all workers to detect
//! GVThreads that have been running too long without yielding.

use crate::worker::worker_states;
use crate::config::SchedulerConfig;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

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
    check_interval: Duration,
    enable_forced_preempt: bool,
    shutdown: Arc<AtomicBool>,
) {
    // Per-worker tracking
    let mut watches: Vec<WorkerWatch> = (0..num_workers)
        .map(|_| WorkerWatch {
            last_counter: 0,
            first_stall_time: None,
        })
        .collect();
    
    while !shutdown.load(Ordering::Acquire) {
        thread::sleep(check_interval);
        
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
#[inline]
pub fn now_ns() -> u64 {
    // Using std::time for now, could use RDTSC for lower overhead
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
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
