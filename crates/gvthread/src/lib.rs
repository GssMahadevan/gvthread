//! # gvthread - Green Virtual Thread Scheduler
//!
//! High-performance userspace threading library for Rust.
//!
//! Named in memory of Gorti Viswanadham (GVThread = Green Virtual Thread).
//!
//! ## Features
//!
//! - **Lightweight**: 16MB virtual address space per GVThread, physical memory on-demand
//! - **Fast Context Switch**: ~20ns voluntary yield via hand-written assembly
//! - **Preemption**: Cooperative (safepoints) + Forced (SIGURG) for CPU-bound code
//! - **Priority Scheduling**: Critical, High, Normal, Low with bitmap-based O(1) lookup
//! - **Synchronization**: Channels, Mutex, Sleep primitives
//! - **Cancellation**: Result-based cancellation with token propagation
//!
//! ## Quick Start
//!
//! ```ignore
//! use gvthread::{Runtime, spawn, yield_now, channel};
//!
//! fn main() {
//!     // Create runtime with default config
//!     let mut runtime = Runtime::new(Default::default());
//!     
//!     // Run some GVThreads
//!     runtime.block_on(|| {
//!         // Spawn a GVThread
//!         spawn(|token| {
//!             println!("Hello from GVThread!");
//!             yield_now();
//!             println!("Back again!");
//!         });
//!         
//!         // Spawn with channel communication
//!         let (tx, rx) = channel(10);
//!         
//!         spawn(move |_| {
//!             for i in 0..5 {
//!                 tx.try_send(i).unwrap();
//!             }
//!         });
//!         
//!         spawn(move |_| {
//!             while let Ok(val) = rx.try_recv() {
//!                 println!("Received: {}", val);
//!             }
//!         });
//!     });
//! }
//! ```
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      User Code                              │
//! │              spawn(), yield_now(), channel                  │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      Scheduler                              │
//! │         Bitmap scan, priority, worker coordination          │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!          ┌───────────────────┼───────────────────┐
//!          ▼                   ▼                   ▼
//!    ┌───────────┐      ┌───────────┐      ┌───────────┐
//!    │  Worker   │      │  Worker   │      │   Timer   │
//!    │  Thread   │      │  Thread   │      │   Thread  │
//!    └───────────┘      └───────────┘      └───────────┘
//!          │                   │                   │
//!          └───────────────────┼───────────────────┘
//!                              ▼
//!    ┌─────────────────────────────────────────────────────────┐
//!    │                  Memory Region                          │
//!    │     16MB slots × N GVThreads, guard pages, mmap         │
//!    └─────────────────────────────────────────────────────────┘
//! ```

// Re-export core types
pub use gvthread_core::{
    GVThreadId,
    GVThreadState,
    Priority,
    CancellationToken,
    SchedError,
    SchedResult,
    channel,
    Sender,
    Receiver,
    SchedMutex,
};

// Re-export kprint macros for debug logging
pub use gvthread_core::{kprint, kprintln, kerror, kwarn, kinfo, kdebug, ktrace};
pub use gvthread_core::kprint::{LogLevel, init as init_logging, set_log_level, set_flush_enabled, set_time_enabled};

// Re-export env utilities
pub use gvthread_core::{env_get, env_get_bool, env_get_opt, env_get_str, env_is_set};

// Re-export runtime types
pub use gvthread_runtime::{
    SchedulerConfig,
    Scheduler,
    sleep,
    sleep_ms,
    sleep_us,
};

use gvthread_runtime::scheduler;
use std::sync::atomic::{AtomicBool, Ordering};

/// Runtime handle for managing the GVThread scheduler
///
/// The runtime manages the lifecycle of worker threads, timer threads,
/// and coordinates GVThread scheduling.
pub struct Runtime {
    started: AtomicBool,
}

impl Runtime {
    /// Create a new runtime with the given configuration
    ///
    /// This does not start the scheduler. Call `start()` or `block_on()` to begin.
    pub fn new(config: SchedulerConfig) -> Self {
        scheduler::init_global_scheduler(config)
            .expect("Failed to initialize scheduler");
        
        Self {
            started: AtomicBool::new(false),
        }
    }
    
    /// Start the scheduler
    ///
    /// This spawns worker threads and begins processing GVThreads.
    pub fn start(&mut self) -> SchedResult<()> {
        if self.started.swap(true, Ordering::SeqCst) {
            return Err(SchedError::AlreadyInitialized);
        }
        
        // Start the global scheduler
        scheduler::start_global_scheduler()
    }
    
    /// Run a function with the scheduler active, then shutdown
    ///
    /// This is the typical entry point for applications.
    pub fn block_on<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        let _ = self.start();
        let result = f();
        self.shutdown();
        result
    }
    
    /// Spawn a new GVThread with normal priority
    pub fn spawn<F>(&self, f: F) -> GVThreadId
    where
        F: FnOnce(&CancellationToken) + Send + 'static,
    {
        spawn(f)
    }
    
    /// Spawn a new GVThread with specified priority
    pub fn spawn_with_priority<F>(&self, f: F, priority: Priority) -> GVThreadId
    where
        F: FnOnce(&CancellationToken) + Send + 'static,
    {
        spawn_with_priority(f, priority)
    }
    
    /// Shutdown the scheduler
    pub fn shutdown(&mut self) {
        if self.started.swap(false, Ordering::SeqCst) {
            scheduler::shutdown_global_scheduler();
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Spawn a new GVThread with normal priority
///
/// The closure receives a `CancellationToken` that can be checked
/// for cooperative cancellation.
///
/// # Example
///
/// ```ignore
/// use gvthread::spawn;
///
/// spawn(|token| {
///     loop {
///         if token.is_cancelled() {
///             break;
///         }
///         // Do work...
///     }
/// });
/// ```
pub fn spawn<F>(f: F) -> GVThreadId
where
    F: FnOnce(&CancellationToken) + Send + 'static,
{
    scheduler::spawn(f, Priority::Normal)
}

/// Spawn a new GVThread with specified priority
pub fn spawn_with_priority<F>(f: F, priority: Priority) -> GVThreadId
where
    F: FnOnce(&CancellationToken) + Send + 'static,
{
    scheduler::spawn(f, priority)
}

/// Yield execution to the scheduler
///
/// The current GVThread will be placed back in the ready queue
/// and another GVThread will run. This is a voluntary yield point.
///
/// If called from outside a GVThread, this yields the OS thread.
#[inline]
pub fn yield_now() {
    scheduler::yield_now()
}

/// Get the current GVThread's ID
///
/// Returns `GVThreadId::NONE` if not running in a GVThread.
#[inline]
pub fn current_id() -> GVThreadId {
    gvthread_runtime::tls::current_gvthread_id()
}

/// Check if currently executing within a GVThread
#[inline]
pub fn is_in_gvthread() -> bool {
    gvthread_runtime::tls::is_in_gvthread()
}

/// Safepoint macro for cooperative preemption
///
/// Insert this in long-running loops to allow preemption.
/// It bumps the activity counter and checks the preempt flag.
///
/// # Example
///
/// ```ignore
/// use gvthread::safepoint;
///
/// for i in 0..1_000_000 {
///     safepoint!();
///     // Do work...
/// }
/// ```
#[macro_export]
macro_rules! safepoint {
    () => {
        // Bump activity counter
        if $crate::is_in_gvthread() {
            // TODO: Implement actual safepoint
            // 1. Bump worker.activity_counter
            // 2. Check preempt_flag
            // 3. If set, yield
        }
    };
}