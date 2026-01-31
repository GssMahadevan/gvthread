//! Scheduler configuration

use gvthread_core::constants::{DEFAULT_MAX_GVTHREADS, MAX_WORKERS};
use std::time::Duration;

/// Configuration for the scheduler
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Maximum number of GVThreads
    pub max_gvthreads: usize,
    
    /// Number of worker threads (defaults to CPU count)
    pub num_workers: usize,
    
    /// Number of workers dedicated to LOW priority only
    pub num_low_priority_workers: usize,
    
    /// Time slice before considering preemption (default: 10ms)
    pub time_slice: Duration,
    
    /// Grace period after setting preempt flag before SIGURG (default: 1ms)
    pub grace_period: Duration,
    
    /// How often timer thread checks workers (default: 1ms)
    pub timer_interval: Duration,
    
    /// Enable SIGURG-based forced preemption
    pub enable_forced_preempt: bool,
    
    /// Enable debug logging
    pub debug_logging: bool,
    
    /// Stack size per GVThread (within the 16MB slot)
    pub stack_size: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        
        Self {
            max_gvthreads: DEFAULT_MAX_GVTHREADS,
            num_workers: num_cpus.min(MAX_WORKERS),
            num_low_priority_workers: 1, // One dedicated LOW worker
            time_slice: Duration::from_millis(10),
            grace_period: Duration::from_millis(1),
            timer_interval: Duration::from_millis(1),
            enable_forced_preempt: true,
            debug_logging: false,
            stack_size: gvthread_core::constants::STACK_SIZE,
        }
    }
}

impl SchedulerConfig {
    /// Create a new configuration with custom settings
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set maximum number of GVThreads
    pub fn max_gvthreads(mut self, n: usize) -> Self {
        self.max_gvthreads = n;
        self
    }
    
    /// Set number of worker threads
    pub fn num_workers(mut self, n: usize) -> Self {
        self.num_workers = n.min(MAX_WORKERS);
        self
    }
    
    /// Set number of LOW priority dedicated workers
    pub fn num_low_priority_workers(mut self, n: usize) -> Self {
        self.num_low_priority_workers = n;
        self
    }
    
    /// Set time slice for preemption
    pub fn time_slice(mut self, d: Duration) -> Self {
        self.time_slice = d;
        self
    }
    
    /// Set grace period before SIGURG
    pub fn grace_period(mut self, d: Duration) -> Self {
        self.grace_period = d;
        self
    }
    
    /// Enable or disable forced preemption
    pub fn enable_forced_preempt(mut self, enable: bool) -> Self {
        self.enable_forced_preempt = enable;
        self
    }
    
    /// Enable debug logging
    pub fn debug_logging(mut self, enable: bool) -> Self {
        self.debug_logging = enable;
        self
    }
    
    /// Validate configuration
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.num_workers == 0 {
            return Err("num_workers must be at least 1");
        }
        if self.num_workers > MAX_WORKERS {
            return Err("num_workers exceeds maximum");
        }
        if self.num_low_priority_workers >= self.num_workers {
            return Err("num_low_priority_workers must be less than num_workers");
        }
        if self.max_gvthreads == 0 {
            return Err("max_gvthreads must be at least 1");
        }
        Ok(())
    }
}
