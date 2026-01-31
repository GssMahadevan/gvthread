//! Main scheduler implementation
//!
//! Orchestrates all components: memory, workers, timers, bitmaps.

use crate::config::SchedulerConfig;
use crate::memory;
use crate::worker::{WorkerPool, set_current_worker_id, current_worker_state, worker_states};
use crate::timer::TimerThread;
use crate::tls;
use crate::current_arch;

use gvthread_core::id::GVThreadId;
use gvthread_core::state::{GVThreadState, Priority};
use gvthread_core::metadata::{GVThreadMetadata, VoluntarySavedRegs};
use gvthread_core::constants::GVTHREAD_NONE;

use gvthread_core::bitmap::ReadyBitmaps;
use gvthread_core::slot::SlotAllocator;
use gvthread_core::cancel::CancellationToken;
use gvthread_core::error::{SchedError, SchedResult};

use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::ptr;


/// Global scheduler instance
static mut SCHEDULER: Option<Scheduler> = None;
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
    
    /// Ready bitmaps (one per priority)
    ready_bitmaps: ReadyBitmaps,
    
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
        
        Self {
            slot_allocator: SlotAllocator::new(config.max_gvthreads),
            ready_bitmaps: ReadyBitmaps::new(config.max_gvthreads, config.num_workers),
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
        timer.start(self.config.num_workers);
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
        
        // Mark as ready
        meta.set_state(GVThreadState::Ready);
        self.ready_bitmaps.set_ready(id, priority);
        
        id
    }
    
    /// Get next ready GVThread for a worker
    pub fn get_next(&self, worker_id: usize, low_priority_only: bool) -> Option<(GVThreadId, Priority)> {
        self.ready_bitmaps.find_and_claim(worker_id, low_priority_only)
    }
    
    /// Mark a GVThread as ready
    pub fn mark_ready(&self, id: GVThreadId, priority: Priority) {
        let meta_ptr = memory::get_metadata_ptr(id.as_u32());
        let meta = unsafe { &*meta_ptr };
        meta.set_state(GVThreadState::Ready);
        self.ready_bitmaps.set_ready(id, priority);
    }
    
    /// Mark a GVThread as blocked
    pub fn mark_blocked(&self, id: GVThreadId, priority: Priority) {
        self.ready_bitmaps.clear_ready(id, priority);
        let meta_ptr = memory::get_metadata_ptr(id.as_u32());
        let meta = unsafe { &*meta_ptr };
        meta.set_state(GVThreadState::Blocked);
    }
    
    /// Mark a GVThread as finished and clean up
    pub fn mark_finished(&self, id: GVThreadId) {
        let meta_ptr = memory::get_metadata_ptr(id.as_u32());
        let meta = unsafe { &*meta_ptr };
        let priority = meta.get_priority();
        
        meta.set_state(GVThreadState::Finished);
        self.ready_bitmaps.clear_ready(id, priority);
        
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
    // Reconstruct the boxed closure
    let boxed: Box<Box<dyn FnOnce(&CancellationToken) + Send>> = 
        unsafe { Box::from_raw(closure_ptr as *mut _) };
    
    // Create cancellation token for this GVThread
    let token = CancellationToken::new();
    
    // Run the closure
    (*boxed)(&token);
    
    // GVThread finished - will be handled by trampoline cleanup
}

/// Main worker loop
fn worker_main_loop(worker_id: usize, is_low_priority: bool, debug: bool) {
    // Set up TLS
    set_current_worker_id(worker_id);
    
    // Store thread ID in worker state
    let worker = current_worker_state();
    worker.thread_id.store(
        unsafe { libc::pthread_self() as u64 },
        Ordering::Relaxed,
    );
    
    if debug {
        eprintln!("[worker-{}] Started (low_priority={})", worker_id, is_low_priority);
    }
    
    // Main loop
    loop {
        // Check for shutdown
        if !SCHEDULER_RUNNING.load(Ordering::Acquire) {
            if debug {
                eprintln!("[worker-{}] Shutdown signaled, exiting", worker_id);
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
                // Run the GVThread
                worker.is_parked.store(false, Ordering::Relaxed);
                run_gvthread(worker_id, id, priority, debug);
            }
            None => {
                // No work available, park
                worker.is_parked.store(true, Ordering::Relaxed);
                std::thread::yield_now();
                std::hint::spin_loop();
            }
        }
    }
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
    
    // Update worker state
    let now_ns = crate::timer::now_ns();
    worker.start_running(id, now_ns);
    
    // Update TLS
    tls::set_current_gvthread(id, meta_ptr as *mut u8);
    
    // Update GVThread state
    meta.set_state(GVThreadState::Running);
    meta.worker_id.store(worker_id as u32, Ordering::Relaxed);
    meta.clear_preempt(); // Clear any preempt flag from previous run
    
    // Get scheduler context save area for this worker
    let sched_ctx = get_worker_sched_context(worker_id);
    
    // Get GVThread's saved registers (at offset 0x40 in metadata)
    let gvthread_regs = unsafe {
        (meta_ptr as *mut u8).add(0x40) as *mut VoluntarySavedRegs
    };
    
    if debug {
        // Debug: print the RIP we're about to jump to
        let regs = unsafe { &*gvthread_regs };
        eprintln!("[worker-{}] Switching to GVThread {} (priority={:?}) rip=0x{:x} rsp=0x{:x}", 
                  worker_id, id, priority, regs.rip, regs.rsp);
    }
    
    // Perform the context switch!
    // This saves our current state to sched_ctx, then loads from gvthread_regs.
    // When the GVThread yields, it will save to gvthread_regs and restore sched_ctx,
    // returning us to right after this call.
    unsafe {
        current_arch::context_switch_voluntary(sched_ctx, gvthread_regs);
    }
    
    // We're back! The GVThread either:
    // 1. Called yield_now() - state is Ready, re-add to queue
    // 2. Finished - state is Finished, clean up
    // 3. Was preempted - state is Preempted, re-add to queue
    
    let state = meta.get_state();
    
    if debug {
        eprintln!("[worker-{}] GVThread {} returned with state {:?}", worker_id, id, state);
    }
    
    match state {
        GVThreadState::Ready => {
            // GVThread yielded voluntarily.
            // Now that context is safely saved, add back to ready queue.
            // (yield_now() intentionally doesn't add to queue to avoid race condition)
            unsafe {
                if let Some(ref sched) = SCHEDULER {
                    sched.ready_bitmaps.set_ready(id, priority);
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
            // GVThread is blocked on something (channel, mutex, etc.)
            // The blocking operation already called mark_blocked()
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
            // Unexpected state - log it
            if debug {
                eprintln!("[worker-{}] WARNING: GVThread {} in unexpected state {:?}", 
                         worker_id, id, state);
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
        eprintln!("[yield_now] ERROR: null meta_base or invalid gvthread_id!");
        std::thread::yield_now();
        return;
    }
    
    let meta = unsafe { &*(meta_base as *const GVThreadMetadata) };
    
    // Mark as Ready - but do NOT add to bitmap yet!
    // run_gvthread() will add us after context switch returns.
    // This prevents the race where another worker picks us up
    // before we've saved our context.
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
    
    // DEBUG: Print addresses before context switch
    eprintln!("[yield_now] gvthread={} worker={} regs_ptr={:p} sched_ptr={:p}",
              gvthread_id, worker_id, gvthread_regs, sched_ctx);
    
    // DEBUG: Print current RIP in gvthread_regs BEFORE we save
    let regs_before = unsafe { &*gvthread_regs };
    eprintln!("[yield_now] BEFORE: rip=0x{:x} rsp=0x{:x}", 
              regs_before.rip, regs_before.rsp);
    
    // Switch back to scheduler
    // This saves our state to gvthread_regs and restores sched_ctx,
    // returning control to run_gvthread() right after its context_switch call.
    unsafe {
        current_arch::context_switch_voluntary(gvthread_regs, sched_ctx);
    }
    
    // When we get here, we've been resumed by a worker.
    // DEBUG: Print that we resumed
    eprintln!("[yield_now] RESUMED on worker={}", crate::worker::current_worker_id());
    
    // Clear preempt flag in case it was set
    meta.clear_preempt();
}

/// Spawn a new GVThread (uses global scheduler)
pub fn spawn<F>(f: F, priority: Priority) -> GVThreadId
where
    F: FnOnce(&CancellationToken) + Send + 'static,
{
    unsafe {
        SCHEDULER.as_ref()
            .expect("Scheduler not initialized")
            .spawn(f, priority)
    }
}

/// Initialize the global scheduler
pub fn init_global_scheduler(config: SchedulerConfig) -> SchedResult<()> {
    if SCHEDULER_INIT.swap(true, Ordering::SeqCst) {
        return Err(SchedError::AlreadyInitialized);
    }
    
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
