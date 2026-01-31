//! Main scheduler implementation
//!
//! Orchestrates all components: memory, workers, timers, ready queue.

use crate::config::SchedulerConfig;
use crate::memory;
use crate::worker::{WorkerPool, set_current_worker_id, current_worker_state, worker_states};
use crate::timer::TimerThread;
use crate::tls;
use crate::current_arch;
use crate::ready_queue::{ReadyQueue, SimpleQueue};

use gvthread_core::id::GVThreadId;
use gvthread_core::state::{GVThreadState, Priority};
use gvthread_core::metadata::{GVThreadMetadata, VoluntarySavedRegs};
use gvthread_core::constants::GVTHREAD_NONE;

use gvthread_core::slot::SlotAllocator;
use gvthread_core::cancel::CancellationToken;
use gvthread_core::error::{SchedError, SchedResult};

// Use kprint macros for debug output
use gvthread_core::{kprintln, kdebug, kwarn};

use std::sync::atomic::{AtomicBool, Ordering};


/// Global scheduler instance
pub(crate) static mut SCHEDULER: Option<Scheduler> = None;
static SCHEDULER_INIT: AtomicBool = AtomicBool::new(false);
static SCHEDULER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Per-worker scheduler context
/// 
/// Each worker stores its "scheduler context" here - the register state
/// that gets restored when a GVThread yields back to the scheduler loop.
/// 
/// Layout: One VoluntarySavedRegs per worker, cache-line padded.
#[repr(C, align(4096))]
struct WorkerSchedulerContexts {
    /// Scheduler saved regs for each worker (64 bytes each)
    contexts: [SchedulerContext; gvthread_core::constants::MAX_WORKERS],
}

/// Scheduler context for a single worker
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct SchedulerContext {
    /// Saved registers when switching to a GVThread
    regs: VoluntarySavedRegs,
    /// Padding to cache line
    _pad: [u8; 0], // VoluntarySavedRegs is already 64 bytes
}

impl SchedulerContext {
    const fn new() -> Self {
        Self {
            regs: VoluntarySavedRegs {
                rsp: 0, rip: 0, rbx: 0, rbp: 0,
                r12: 0, r13: 0, r14: 0, r15: 0,
            },
            _pad: [],
        }
    }
}

impl WorkerSchedulerContexts {
    const fn new() -> Self {
        Self {
            contexts: [const { SchedulerContext::new() }; gvthread_core::constants::MAX_WORKERS],
        }
    }
    
    pub fn get(&self, worker_id: usize) -> *mut VoluntarySavedRegs {
        &self.contexts[worker_id].regs as *const _ as *mut _
    }
}

/// Global worker scheduler contexts
static mut WORKER_SCHED_CONTEXTS: WorkerSchedulerContexts = WorkerSchedulerContexts::new();

/// Get scheduler context for a worker (for use by arch module)
/// 
/// # Safety
/// 
/// Only call from within the runtime.
#[inline]
pub fn get_worker_sched_context(worker_id: usize) -> *mut VoluntarySavedRegs {
    unsafe { WORKER_SCHED_CONTEXTS.get(worker_id) }
}

/// Main scheduler
pub struct Scheduler {
    /// Configuration
    config: SchedulerConfig,
    
    /// Slot allocator
    slot_allocator: SlotAllocator,
    
    /// Ready queue (per-worker local + global, Go-like)
    pub(crate) ready_queue: Box<dyn ReadyQueue>,
    
    /// Worker thread pool
    worker_pool: Option<WorkerPool>,
    
    /// Timer thread
    timer_thread: Option<TimerThread>,
    
    /// Scheduler is running
    running: AtomicBool,
}

impl Scheduler {
    /// Create a new scheduler with the given configuration
    pub fn new(config: SchedulerConfig) -> Self {
        config.validate().expect("Invalid scheduler configuration");
        
        // Create and initialize ready queue
        let mut ready_queue = SimpleQueue::new();
        ready_queue.init(config.num_workers);
        
        Self {
            slot_allocator: SlotAllocator::new(config.max_gvthreads),
            ready_queue: Box::new(ready_queue),
            worker_pool: None,
            timer_thread: None,
            running: AtomicBool::new(false),
            config,
        }
    }
    
    /// Initialize and start the scheduler
    pub fn start(&mut self) -> SchedResult<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(SchedError::AlreadyInitialized);
        }
        
        // Initialize memory region
        memory::init_memory_region(self.config.max_gvthreads)?;
        
        // Set the global running flag BEFORE starting workers
        SCHEDULER_RUNNING.store(true, Ordering::Release);
        
        // Start timer thread
        let mut timer = TimerThread::new(&self.config);
        timer.start(self.config.num_workers, self.config.max_gvthreads);
        self.timer_thread = Some(timer);
        
        // Start worker threads
        let mut workers = WorkerPool::new(
            self.config.num_workers,
            self.config.num_low_priority_workers,
        );
        
        // Clone values needed by worker closure
        let debug = self.config.debug_logging;
        
        workers.start(move |worker_id, is_low_priority| {
            worker_main_loop(worker_id, is_low_priority, debug);
        });
        
        self.worker_pool = Some(workers);
        
        Ok(())
    }
    
    /// Spawn a new GVThread
    pub fn spawn<F>(&self, f: F, priority: Priority) -> GVThreadId
    where
        F: FnOnce(&CancellationToken) + Send + 'static,
    {
        // Allocate a slot
        let id = self.slot_allocator.allocate()
            .expect("No slots available");
        
        // Activate the slot's memory
        memory::memory_region().activate_slot(id.as_u32())
            .expect("Failed to activate slot");
        
        // Get metadata pointer
        let meta_ptr = memory::get_metadata_ptr(id.as_u32());
        let meta = unsafe { &*meta_ptr };
        
        // Get parent ID (if spawning from within a GVThread)
        let parent = if tls::is_in_gvthread() {
            tls::current_gvthread_id()
        } else {
            GVThreadId::NONE
        };
        
        // Initialize metadata
        meta.init(id, parent, priority);
        
        // Box the closure and store pointer in metadata
        let boxed: Box<dyn FnOnce(&CancellationToken) + Send> = Box::new(f);
        let closure_ptr = Box::into_raw(Box::new(boxed));
        meta.entry_fn.store(gvthread_entry as usize as u64, Ordering::Relaxed);
        meta.entry_arg.store(closure_ptr as usize as u64, Ordering::Relaxed);
        
        // Initialize stack context
        let stack_top = memory::get_stack_top(id.as_u32());
        let regs_ptr = unsafe {
            (meta_ptr as *mut u8).add(0x40) as *mut gvthread_core::metadata::VoluntarySavedRegs
        };
        
        unsafe {
            crate::current_arch::init_context(
                regs_ptr,
                stack_top,
                gvthread_entry as usize,
                closure_ptr as usize,
            );
        }
        
        // Mark as ready and add to queue
        meta.set_state(GVThreadState::Ready);
        self.ready_queue.push(id, priority, None);  // No worker hint for spawn
        
        id
    }
    
    /// Get next ready GVThread for a worker
    pub fn get_next(&self, worker_id: usize, _low_priority_only: bool) -> Option<(GVThreadId, Priority)> {
        self.ready_queue.pop(worker_id)
    }
    
    /// Mark a GVThread as ready
    pub fn mark_ready(&self, id: GVThreadId, priority: Priority) {
        let meta_ptr = memory::get_metadata_ptr(id.as_u32());
        let meta = unsafe { &*meta_ptr };
        meta.set_state(GVThreadState::Ready);
        // Use current worker as hint if available
        let hint = tls::try_current_worker_id();
        self.ready_queue.push(id, priority, hint);
    }
    
    /// Mark a GVThread as blocked
    pub fn mark_blocked(&self, id: GVThreadId, _priority: Priority) {
        // Queue-based: no need to remove, it was already popped
        let meta_ptr = memory::get_metadata_ptr(id.as_u32());
        let meta = unsafe { &*meta_ptr };
        meta.set_state(GVThreadState::Blocked);
    }
    
    /// Wake a blocked GVThread (make it ready again)
    pub fn wake_gvthread(&self, id: GVThreadId, priority: Priority) {
        let meta_ptr = memory::get_metadata_ptr(id.as_u32());
        let meta = unsafe { &*meta_ptr };
        
        // Only wake if currently blocked
        if meta.get_state() == GVThreadState::Blocked {
            meta.set_state(GVThreadState::Ready);
            // Use current worker as hint for locality
            let hint = tls::try_current_worker_id();
            self.ready_queue.push(id, priority, hint);
        }
    }
    
    /// Mark a GVThread as finished and clean up
    pub fn mark_finished(&self, id: GVThreadId) {
        let meta_ptr = memory::get_metadata_ptr(id.as_u32());
        let meta = unsafe { &*meta_ptr };
        
        meta.set_state(GVThreadState::Finished);
        // Queue-based: no need to remove, it was already popped
        
        // Deactivate slot memory
        let _ = memory::memory_region().deactivate_slot(id.as_u32());
        
        // Return slot to allocator
        self.slot_allocator.release(id);
    }
    
    /// Check if scheduler is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
    
    /// Shutdown the scheduler
    pub fn shutdown(&mut self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return; // Already stopped
        }
        
        // Clear the global running flag FIRST to signal workers to exit
        SCHEDULER_RUNNING.store(false, Ordering::Release);
        
        // Wake all parked workers so they can see the shutdown flag
        self.ready_queue.wake_all();
        
        // Give workers a moment to notice the flag
        std::thread::sleep(std::time::Duration::from_millis(10));
        
        // Stop timer thread
        if let Some(timer) = self.timer_thread.take() {
            timer.shutdown();
            timer.join();
        }
        
        // Stop worker threads
        if let Some(workers) = self.worker_pool.take() {
            workers.shutdown();
            workers.join();
        }
    }
}

/// Entry point for GVThread execution
extern "C" fn gvthread_entry(closure_ptr: usize) {
    // CRITICAL: No heap allocations in this function!
    // We're running on GVThread's custom stack, malloc gets confused.
    
    // Reconstruct the boxed closure (this is from_raw, not allocating)
    let boxed: Box<Box<dyn FnOnce(&CancellationToken) + Send>> = 
        unsafe { Box::from_raw(closure_ptr as *mut _) };
    
    // Get cancellation token from metadata (no allocation!)
    // The token was created in spawn() and stored in metadata
    let meta_base = crate::tls::current_gvthread_base();
    let token = if !meta_base.is_null() {
        let meta = unsafe { &*(meta_base as *const GVThreadMetadata) };
        // Create a lightweight token that reads from metadata's cancelled field
        CancellationToken::from_metadata(meta)
    } else {
        // Fallback - should not happen
        CancellationToken::dummy()
    };
    
    // Run the closure
    (*boxed)(&token);
    
    // GVThread finished - will be handled by trampoline cleanup
}

/// Main worker loop
fn worker_main_loop(worker_id: usize, is_low_priority: bool, debug: bool) {
    // Set up TLS
    set_current_worker_id(worker_id);
    
    // Set kprint context for this worker thread
    gvthread_core::kprint::set_worker_id(worker_id as u32);
    
    // Store thread ID in worker state
    let worker = current_worker_state();
    worker.thread_id.store(
        unsafe { libc::pthread_self() as u64 },
        Ordering::Relaxed,
    );
    
    if debug {
        kdebug!("Started (low_priority={})", is_low_priority);
    }
    
    // Idle configuration from environment
    use gvthread_core::env::env_get;
    let spin_limit: u32 = env_get("GVT_IDLE_SPINS", 10);
    let park_timeout_ms: u64 = env_get("GVT_PARK_TIMEOUT_MS", 100); // Default 100ms
    
    // Idle state
    let mut idle_spins: u32 = 0;
    
    // Main loop
    loop {
        // Check for shutdown
        if !SCHEDULER_RUNNING.load(Ordering::Acquire) {
            if debug {
                kdebug!("Shutdown signaled, exiting");
            }
            break;
        }
        
        // Try to get next GVThread
        let next = unsafe {
            if let Some(ref sched) = SCHEDULER {
                sched.get_next(worker_id, is_low_priority)
            } else {
                None
            }
        };
        
        match next {
            Some((id, priority)) => {
                // Got work - reset idle state
                idle_spins = 0;
                worker.is_parked.store(false, Ordering::Relaxed);
                run_gvthread(worker_id, id, priority, debug);
            }
            None => {
                // No work available
                if idle_spins < spin_limit {
                    // Quick spin first (catch fast ready→run cycles)
                    idle_spins += 1;
                    for _ in 0..32 {
                        std::hint::spin_loop();
                    }
                    std::thread::yield_now();
                } else {
                    // Park via ready_queue's condvar
                    worker.is_parked.store(true, Ordering::Relaxed);
                    unsafe {
                        if let Some(ref sched) = SCHEDULER {
                            sched.ready_queue.park(worker_id, park_timeout_ms);
                        }
                    }
                    worker.is_parked.store(false, Ordering::Relaxed);
                    idle_spins = 0; // Reset after park
                }
            }
        }
    }
    
    // Clear kprint context on exit
    gvthread_core::kprint::clear_worker_id();
}

/// Run a GVThread on this worker
/// 
/// This is the core of the scheduler. We:
/// 1. Set up TLS and worker state
/// 2. Save the scheduler context (current registers)
/// 3. Load the GVThread's context and jump to it
/// 
/// When the GVThread yields or finishes, context_switch_voluntary
/// will restore our scheduler context and we return here.
fn run_gvthread(worker_id: usize, id: GVThreadId, priority: Priority, debug: bool) {
    let worker = current_worker_state();
    
    // Get GVThread metadata
    let meta_ptr = memory::get_metadata_ptr(id.as_u32());
    let meta = unsafe { &*meta_ptr };
    
    // Update worker state (now_ns is safe here - we're on worker stack)
    let now_ns = crate::timer::now_ns();
    worker.start_running(id, now_ns);
    
    // Update TLS
    tls::set_current_gvthread(id, meta_ptr as *mut u8);
    
    // Set kprint gvthread context
    gvthread_core::kprint::set_gvthread_id(id.as_u32());
    
    // Update GVThread state
    meta.set_state(GVThreadState::Running);
    meta.worker_id.store(worker_id as u32, Ordering::Relaxed);
    meta.clear_preempt();
    
    if debug {
        kdebug!("Running GVThread {} ({:?})", id, priority);
    }
    
    // Get scheduler context save area for this worker
    let sched_ctx = get_worker_sched_context(worker_id);
    
    // Get GVThread's saved registers (at offset 0x40 in metadata)
    let gvthread_regs = unsafe {
        (meta_ptr as *mut u8).add(0x40) as *mut VoluntarySavedRegs
    };
    
    // Perform the context switch!
    unsafe {
        current_arch::context_switch_voluntary(sched_ctx, gvthread_regs);
    }
    
    // We're back from GVThread - clear gvthread context for kprint
    gvthread_core::kprint::clear_gvthread_id();
    
    // Handle based on GVThread state
    let state = meta.get_state();
    
    if debug {
        kdebug!("GVThread {} returned ({:?})", id, state);
    }
    
    match state {
        GVThreadState::Ready => {
            // GVThread yielded - add back to ready queue
            // Use current worker as hint for locality
            unsafe {
                if let Some(ref sched) = SCHEDULER {
                    sched.ready_queue.push(id, priority, Some(worker_id));
                }
            }
        }
        GVThreadState::Finished => {
            // GVThread completed - clean it up
            unsafe {
                if let Some(ref sched) = SCHEDULER {
                    sched.mark_finished(id);
                }
            }
        }
        GVThreadState::Blocked => {
            // GVThread is blocked - blocking op already called mark_blocked()
        }
        GVThreadState::Preempted => {
            // GVThread was forcibly preempted - re-add to queue
            unsafe {
                if let Some(ref sched) = SCHEDULER {
                    sched.mark_ready(id, priority);
                }
            }
        }
        _ => {
            if debug {
                kwarn!("unexpected state {:?}", state);
            }
        }
    }
    
    // Clear worker state
    worker.stop_running();
    tls::clear_current_gvthread();
}

/// Yield the current GVThread
/// 
/// Saves the GVThread's context, marks it as Ready, and switches
/// back to the scheduler context. The scheduler will then pick
/// the next GVThread to run.
/// 
/// IMPORTANT: We set state = Ready but do NOT add to the ready bitmap here.
/// The run_gvthread() function adds us back after the context switch completes.
/// This prevents a race where another worker picks us up before we've saved our context.
pub fn yield_now() {
    if !tls::is_in_gvthread() {
        // Not in a GVThread, just yield OS thread
        std::thread::yield_now();
        return;
    }
    
    // Get current GVThread info from TLS
    let gvthread_id = tls::current_gvthread_id();
    let meta_base = tls::current_gvthread_base();
    let worker_id = crate::worker::current_worker_id();
    
    // Safety check
    if meta_base.is_null() || gvthread_id.is_none() {
        std::thread::yield_now();
        return;
    }
    
    let meta = unsafe { &*(meta_base as *const GVThreadMetadata) };
    
    // Mark as Ready - but do NOT add to bitmap yet!
    // run_gvthread() will add us after context switch returns.
    meta.set_state(GVThreadState::Ready);
    
    // Bump activity counter for preemption tracking
    let worker = current_worker_state();
    worker.record_activity(crate::timer::now_ns());
    
    // Get our saved registers (at offset 0x40 in metadata)
    let gvthread_regs = unsafe {
        (meta_base).add(0x40) as *mut VoluntarySavedRegs
    };
    
    // Get scheduler context for this worker
    let sched_ctx = get_worker_sched_context(worker_id);
    
    // Switch back to scheduler
    unsafe {
        current_arch::context_switch_voluntary(gvthread_regs, sched_ctx);
    }
    
    // When we get here, we've been resumed by a worker.
    // Clear preempt flag in case it was set
    meta.clear_preempt();
}

/// Block the current GVThread
/// 
/// Marks the GVThread as Blocked and yields to the scheduler.
/// The GVThread will NOT be added to the ready queue - it must be
/// explicitly woken by calling `wake_gvthread`.
///
/// Used by sleep, channels, mutexes, etc.
pub fn block_current() {
    if !tls::is_in_gvthread() {
        return;
    }
    
    let gvthread_id = tls::current_gvthread_id();
    let meta_base = tls::current_gvthread_base();
    let worker_id = crate::worker::current_worker_id();
    
    if meta_base.is_null() || gvthread_id.is_none() {
        return;
    }
    
    let meta = unsafe { &*(meta_base as *const GVThreadMetadata) };
    
    // Mark as Blocked - scheduler will NOT re-add to ready queue
    meta.set_state(GVThreadState::Blocked);
    
    // Get our saved registers
    let gvthread_regs = unsafe {
        (meta_base).add(0x40) as *mut VoluntarySavedRegs
    };
    
    // Get scheduler context for this worker
    let sched_ctx = get_worker_sched_context(worker_id);
    
    // Switch back to scheduler
    unsafe {
        current_arch::context_switch_voluntary(gvthread_regs, sched_ctx);
    }
    
    // When we get here, we've been woken and resumed
    meta.clear_preempt();
}

/// Wake a blocked GVThread (make it ready again)
/// 
/// Called by timer (for sleep) or synchronization primitives.
pub fn wake_gvthread(id: GVThreadId, priority: Priority) {
    unsafe {
        if let Some(ref sched) = SCHEDULER {
            sched.wake_gvthread(id, priority);
            // Note: ready_queue.push() already wakes a parked worker
        }
    }
}

/// Wake a blocked GVThread with generation check
/// 
/// Only wakes if the current generation matches. This prevents stale
/// wakes after a slot has been reused for a new GVThread.
pub fn wake_gvthread_checked(id: GVThreadId, priority: Priority, expected_generation: u32) {
    let meta_ptr = memory::get_metadata_ptr(id.as_u32());
    let meta = unsafe { &*meta_ptr };
    
    // Check generation to avoid waking wrong GVThread after slot reuse
    if meta.get_generation() != expected_generation {
        return; // Stale wake - slot was reused
    }
    
    // Delegate to normal wake
    wake_gvthread(id, priority);
}

/// Spawn a new GVThread (uses global scheduler)
pub fn spawn<F>(f: F, priority: Priority) -> GVThreadId
where
    F: FnOnce(&CancellationToken) + Send + 'static,
{
    let id = unsafe {
        SCHEDULER.as_ref()
            .expect("Scheduler not initialized")
            .spawn(f, priority)
    };
    
    // Note: ready_queue.push() already wakes a parked worker
    
    id
}

/// Initialize the global scheduler
pub fn init_global_scheduler(config: SchedulerConfig) -> SchedResult<()> {
    if SCHEDULER_INIT.swap(true, Ordering::SeqCst) {
        return Err(SchedError::AlreadyInitialized);
    }
    
    // Initialize the sleep queue with capacity for all possible GVThreads
    crate::timer::init_sleep_queue_with_capacity(config.max_gvthreads);
    
    unsafe {
        SCHEDULER = Some(Scheduler::new(config));
    }
    
    Ok(())
}

/// Get the global scheduler
pub fn global_scheduler() -> Option<&'static Scheduler> {
    unsafe { SCHEDULER.as_ref() }
}

/// Start the global scheduler
pub fn start_global_scheduler() -> SchedResult<()> {
    unsafe {
        if let Some(ref mut sched) = SCHEDULER {
            sched.start()
        } else {
            Err(SchedError::NotInitialized)
        }
    }
}

/// Shutdown the global scheduler
pub fn shutdown_global_scheduler() {
    unsafe {
        if let Some(ref mut sched) = SCHEDULER {
            sched.shutdown();
        }
    }
}