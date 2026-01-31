//! Worker thread management
//!
//! Workers are OS threads that run GVThreads. Each worker has its own
//! state stored in a contiguous array for efficient timer thread scanning.

use gvthread_core::constants::MAX_WORKERS;
use gvthread_core::metadata::WorkerState;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};

/// Contiguous array of worker states (cache-line aligned)
/// 
/// This is a single 4KB allocation that the timer thread can scan
/// efficiently without page faults.
#[repr(C, align(4096))]
pub struct WorkerStates {
    states: [WorkerState; MAX_WORKERS],
}

impl WorkerStates {
    /// Create new WorkerStates with all workers initialized
    fn new_boxed() -> Box<Self> {
        // Safety: WorkerState is all atomics/primitives, zeroing is safe
        let mut states: Box<Self> = unsafe {
            let layout = std::alloc::Layout::new::<Self>();
            let ptr = std::alloc::alloc_zeroed(layout) as *mut Self;
            Box::from_raw(ptr)
        };
        // Initialize each worker state properly
        for i in 0..MAX_WORKERS {
            states.states[i] = WorkerState::new();
        }
        states
    }
    
    #[inline]
    pub fn get(&self, id: usize) -> &WorkerState {
        &self.states[id]
    }
}

/// Global worker states - lazily initialized
static WORKER_STATES_CELL: OnceLock<Box<WorkerStates>> = OnceLock::new();

/// Get the global worker states array
#[inline]
pub fn worker_states() -> &'static WorkerStates {
    WORKER_STATES_CELL.get_or_init(|| WorkerStates::new_boxed())
}

/// Pool of worker threads
pub struct WorkerPool {
    /// Join handles for worker threads
    handles: Vec<JoinHandle<()>>,
    
    /// Number of active workers
    num_workers: usize,
    
    /// Number of LOW priority dedicated workers
    num_low_priority_workers: usize,
    
    /// Shutdown flag
    shutdown: AtomicBool,
    
    /// Number of workers that have started
    started_count: AtomicUsize,
}

impl WorkerPool {
    /// Create a new worker pool
    pub fn new(num_workers: usize, num_low_priority_workers: usize) -> Self {
        Self {
            handles: Vec::with_capacity(num_workers),
            num_workers,
            num_low_priority_workers,
            shutdown: AtomicBool::new(false),
            started_count: AtomicUsize::new(0),
        }
    }
    
    /// Start all worker threads
    pub fn start<F>(&mut self, worker_fn: F) 
    where
        F: Fn(usize, bool) + Send + Sync + Clone + 'static,
    {
        for i in 0..self.num_workers {
            let is_low_priority = i >= (self.num_workers - self.num_low_priority_workers);
            let worker_fn = worker_fn.clone();
            
            // Initialize worker state
            worker_states().get(i).init(i as u8, is_low_priority);
            
            let handle = thread::Builder::new()
                .name(format!("gvthread-worker-{}", i))
                .spawn(move || {
                    worker_fn(i, is_low_priority);
                })
                .expect("Failed to spawn worker thread");
            
            self.handles.push(handle);
        }
    }
    
    /// Signal shutdown to all workers
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
    
    /// Check if shutdown was requested
    #[inline]
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }
    
    /// Wait for all workers to finish
    pub fn join(self) {
        for handle in self.handles {
            let _ = handle.join();
        }
    }
    
    /// Get number of workers
    #[inline]
    pub fn num_workers(&self) -> usize {
        self.num_workers
    }
    
    /// Get worker state by index
    #[inline]
    pub fn get_worker_state(&self, id: usize) -> &WorkerState {
        worker_states().get(id)
    }
}

/// Thread-local worker ID
thread_local! {
    static CURRENT_WORKER_ID: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
}

/// Set the current worker ID for this thread
pub fn set_current_worker_id(id: usize) {
    CURRENT_WORKER_ID.with(|cell| cell.set(id));
}

/// Get the current worker ID for this thread
#[inline]
pub fn current_worker_id() -> usize {
    CURRENT_WORKER_ID.with(|cell| cell.get())
}

/// Get the current worker's state
#[inline]
pub fn current_worker_state() -> &'static WorkerState {
    let id = current_worker_id();
    debug_assert!(id < MAX_WORKERS, "Worker ID not set or invalid");
    worker_states().get(id)
}
