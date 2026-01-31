//! Platform and architecture traits
//!
//! These traits define the interface between platform-agnostic core
//! and platform-specific runtime implementations.

use crate::id::GVThreadId;
use crate::error::SchedResult;

/// Platform-specific memory operations
pub trait PlatformMemory: Send + Sync {
    /// Reserve virtual address space for GVThread slots
    fn reserve_region(&self, size: usize) -> SchedResult<*mut u8>;
    
    /// Release virtual address space
    fn release_region(&self, base: *mut u8, size: usize) -> SchedResult<()>;
    
    /// Make a memory region readable/writable
    fn protect_rw(&self, base: *mut u8, size: usize) -> SchedResult<()>;
    
    /// Make a memory region inaccessible (guard page)
    fn protect_none(&self, base: *mut u8, size: usize) -> SchedResult<()>;
    
    /// Advise kernel that memory is not needed (release physical pages)
    fn advise_dontneed(&self, base: *mut u8, size: usize) -> SchedResult<()>;
}

/// Platform-specific signal/interrupt handling
pub trait PlatformSignal: Send + Sync {
    /// Install the preemption signal handler (e.g., SIGURG)
    fn install_preempt_handler(&self) -> SchedResult<()>;
    
    /// Send preemption signal to a worker thread
    fn send_preempt_signal(&self, thread_id: u64) -> SchedResult<()>;
    
    /// Block all signals except preemption on current thread
    fn block_signals_except_preempt(&self) -> SchedResult<()>;
}

/// Platform-specific threading operations
pub trait PlatformThread: Send + Sync {
    /// Spawn a new OS thread
    fn spawn_thread<F>(&self, name: &str, f: F) -> SchedResult<u64>
    where
        F: FnOnce() + Send + 'static;
    
    /// Get current thread ID
    fn current_thread_id(&self) -> u64;
    
    /// Set thread CPU affinity (optional, may not be supported)
    fn set_affinity(&self, thread_id: u64, cpu: usize) -> SchedResult<()>;
    
    /// Yield the current OS thread
    fn yield_thread(&self);
    
    /// Sleep for the specified duration in nanoseconds
    fn sleep_ns(&self, ns: u64);
}

/// Platform-specific time operations
pub trait PlatformTime: Send + Sync {
    /// Get current time in nanoseconds (monotonic)
    fn now_ns(&self) -> u64;
    
    /// Get coarse time in nanoseconds (faster, less precise)
    fn coarse_now_ns(&self) -> u64;
}

/// Architecture-specific context switch operations
pub trait ArchContext: Send + Sync {
    /// Initialize context for a new GVThread
    ///
    /// Sets up the initial stack and registers so that when the context
    /// is switched to, execution begins at `entry_fn(entry_arg)`.
    fn init_context(
        &self,
        regs: *mut u8,      // Pointer to saved registers area
        stack_top: *mut u8, // Top of stack (highest address)
        entry_fn: usize,    // Entry function pointer
        entry_arg: usize,   // Argument to entry function
    );
    
    /// Perform voluntary context switch (callee-saved registers only)
    ///
    /// Saves current context to `old_regs` and loads context from `new_regs`.
    /// Returns when this context is switched back to.
    ///
    /// # Safety
    ///
    /// Both register areas must be valid and properly aligned.
    unsafe fn switch_voluntary(
        &self,
        old_regs: *mut u8, // Where to save current context
        new_regs: *mut u8, // Where to load new context from
    );
    
    /// Perform forced context switch (all registers)
    ///
    /// Used when resuming a GVThread that was forcibly preempted via SIGURG.
    ///
    /// # Safety
    ///
    /// Register area must contain valid state from SIGURG handler.
    unsafe fn switch_forced(
        &self,
        regs: *mut u8, // Full register state to restore
    );
}

/// Combined platform interface
pub trait Platform: PlatformMemory + PlatformSignal + PlatformThread + PlatformTime {
    /// Platform name (e.g., "linux", "macos", "windows")
    fn name(&self) -> &'static str;
}

/// Callback interface for platform to notify scheduler
pub trait SchedulerCallback: Send + Sync {
    /// Called when a GVThread yields voluntarily
    fn on_yield(&self, id: GVThreadId);
    
    /// Called when a GVThread is forcibly preempted
    fn on_preempt(&self, id: GVThreadId);
    
    /// Called when a GVThread finishes
    fn on_finish(&self, id: GVThreadId);
    
    /// Get the next GVThread to run on this worker
    fn get_next(&self, worker_id: usize, low_priority_only: bool) -> Option<GVThreadId>;
}
