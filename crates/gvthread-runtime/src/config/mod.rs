//! GVThread Configuration
//!
//! Provides compile-time defaults with runtime environment overrides.
//!
//! # Configuration Priority (highest wins)
//!
//! 1. Environment variables (runtime)
//! 2. User's gvt_config.rs (compile-time, feature-gated)
//! 3. Library defaults
//!
//! # Example
//!
//! ```rust,ignore
//! use gvthread_runtime::config::SchedulerConfig;
//!
//! // Use defaults with env overrides
//! let config = SchedulerConfig::from_env();
//!
//! // Or customize programmatically
//! let config = SchedulerConfig::from_env()
//!     .num_workers(8)
//!     .time_slice(Duration::from_millis(5));
//! ```

pub mod defaults;

use std::time::Duration;
use gvthread_core::env::env_get;

/// Scheduler configuration with builder pattern.
///
/// Use `from_env()` to start with compile-time defaults and apply
/// any environment variable overrides.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Number of worker threads
    pub num_workers: usize,
    /// Number of workers dedicated to low priority GVThreads
    pub num_low_priority_workers: usize,
    /// Maximum concurrent GVThreads
    pub max_gvthreads: usize,
    /// Time slice before setting preempt flag
    pub time_slice: Duration,
    /// Grace period before forced preemption (SIGURG)
    pub grace_period: Duration,
    /// Timer thread check interval
    pub timer_interval: Duration,
    /// Enable SIGURG-based forced preemption
    pub enable_forced_preempt: bool,
    /// Enable debug logging
    pub debug_logging: bool,
    /// Virtual stack size per GVThread
    pub stack_size: usize,
    /// Per-worker local queue capacity
    pub local_queue_capacity: usize,
    /// Global queue capacity
    pub global_queue_capacity: usize,
    /// Spins before parking worker
    pub idle_spins: u32,
    /// Worker park timeout
    pub park_timeout: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl SchedulerConfig {
    /// Create config from compile-time defaults with environment overrides.
    ///
    /// Environment variables (all optional):
    /// - `GVT_NUM_WORKERS` - Number of worker threads
    /// - `GVT_NUM_LOW_PRIORITY_WORKERS` - Low priority workers
    /// - `GVT_MAX_GVTHREADS` - Max concurrent GVThreads
    /// - `GVT_TIME_SLICE_MS` - Time slice in milliseconds
    /// - `GVT_GRACE_PERIOD_MS` - Grace period in milliseconds
    /// - `GVT_TIMER_INTERVAL_MS` - Timer interval in milliseconds
    /// - `GVT_ENABLE_FORCED_PREEMPT` - Enable SIGURG (0/1)
    /// - `GVT_DEBUG` - Enable debug logging (0/1)
    /// - `GVT_STACK_SIZE` - Stack size per GVThread
    /// - `GVT_LOCAL_QUEUE_CAPACITY` - Per-worker queue size
    /// - `GVT_GLOBAL_QUEUE_CAPACITY` - Global queue size
    /// - `GVT_IDLE_SPINS` - Spins before parking
    /// - `GVT_PARK_TIMEOUT_MS` - Park timeout in milliseconds
    pub fn from_env() -> Self {
        Self {
            num_workers: env_get("GVT_NUM_WORKERS", defaults::NUM_WORKERS),
            num_low_priority_workers: env_get(
                "GVT_NUM_LOW_PRIORITY_WORKERS",
                defaults::NUM_LOW_PRIORITY_WORKERS,
            ),
            max_gvthreads: env_get("GVT_MAX_GVTHREADS", defaults::MAX_GVTHREADS),
            time_slice: Duration::from_millis(env_get(
                "GVT_TIME_SLICE_MS",
                defaults::TIME_SLICE_MS,
            )),
            grace_period: Duration::from_millis(env_get(
                "GVT_GRACE_PERIOD_MS",
                defaults::GRACE_PERIOD_MS,
            )),
            timer_interval: Duration::from_millis(env_get(
                "GVT_TIMER_INTERVAL_MS",
                defaults::TIMER_INTERVAL_MS,
            )),
            enable_forced_preempt: env_get(
                "GVT_ENABLE_FORCED_PREEMPT",
                if defaults::ENABLE_FORCED_PREEMPT { 1usize } else { 0 },
            ) != 0,
            debug_logging: env_get(
                "GVT_DEBUG",
                if defaults::DEBUG_LOGGING { 1usize } else { 0 },
            ) != 0,
            stack_size: env_get("GVT_STACK_SIZE", defaults::STACK_SIZE),
            local_queue_capacity: env_get(
                "GVT_LOCAL_QUEUE_CAPACITY",
                defaults::LOCAL_QUEUE_CAPACITY,
            ),
            global_queue_capacity: env_get(
                "GVT_GLOBAL_QUEUE_CAPACITY",
                defaults::GLOBAL_QUEUE_CAPACITY,
            ),
            idle_spins: env_get("GVT_IDLE_SPINS", defaults::IDLE_SPINS as usize) as u32,
            park_timeout: Duration::from_millis(env_get(
                "GVT_PARK_TIMEOUT_MS",
                defaults::PARK_TIMEOUT_MS,
            )),
        }
    }

    /// Create config with explicit defaults (no env override).
    /// Useful for testing or when you want full control.
    pub fn new() -> Self {
        Self {
            num_workers: defaults::NUM_WORKERS,
            num_low_priority_workers: defaults::NUM_LOW_PRIORITY_WORKERS,
            max_gvthreads: defaults::MAX_GVTHREADS,
            time_slice: Duration::from_millis(defaults::TIME_SLICE_MS),
            grace_period: Duration::from_millis(defaults::GRACE_PERIOD_MS),
            timer_interval: Duration::from_millis(defaults::TIMER_INTERVAL_MS),
            enable_forced_preempt: defaults::ENABLE_FORCED_PREEMPT,
            debug_logging: defaults::DEBUG_LOGGING,
            stack_size: defaults::STACK_SIZE,
            local_queue_capacity: defaults::LOCAL_QUEUE_CAPACITY,
            global_queue_capacity: defaults::GLOBAL_QUEUE_CAPACITY,
            idle_spins: defaults::IDLE_SPINS,
            park_timeout: Duration::from_millis(defaults::PARK_TIMEOUT_MS),
        }
    }

    // Builder methods

    pub fn num_workers(mut self, n: usize) -> Self {
        self.num_workers = n;
        self
    }

    pub fn num_low_priority_workers(mut self, n: usize) -> Self {
        self.num_low_priority_workers = n;
        self
    }

    pub fn max_gvthreads(mut self, n: usize) -> Self {
        self.max_gvthreads = n;
        self
    }

    pub fn time_slice(mut self, d: Duration) -> Self {
        self.time_slice = d;
        self
    }

    pub fn grace_period(mut self, d: Duration) -> Self {
        self.grace_period = d;
        self
    }

    pub fn timer_interval(mut self, d: Duration) -> Self {
        self.timer_interval = d;
        self
    }

    pub fn enable_forced_preempt(mut self, enable: bool) -> Self {
        self.enable_forced_preempt = enable;
        self
    }

    pub fn debug_logging(mut self, enable: bool) -> Self {
        self.debug_logging = enable;
        self
    }

    pub fn stack_size(mut self, size: usize) -> Self {
        self.stack_size = size;
        self
    }

    pub fn local_queue_capacity(mut self, cap: usize) -> Self {
        self.local_queue_capacity = cap;
        self
    }

    pub fn global_queue_capacity(mut self, cap: usize) -> Self {
        self.global_queue_capacity = cap;
        self
    }

    pub fn idle_spins(mut self, spins: u32) -> Self {
        self.idle_spins = spins;
        self
    }

    pub fn park_timeout(mut self, d: Duration) -> Self {
        self.park_timeout = d;
        self
    }

    /// Validate configuration and return errors if invalid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.num_workers == 0 {
            return Err(ConfigError::InvalidValue("num_workers must be > 0"));
        }
        if self.num_workers > 256 {
            return Err(ConfigError::InvalidValue("num_workers must be <= 256"));
        }
        if self.num_low_priority_workers >= self.num_workers {
            return Err(ConfigError::InvalidValue(
                "num_low_priority_workers must be < num_workers",
            ));
        }
        if self.max_gvthreads == 0 {
            return Err(ConfigError::InvalidValue("max_gvthreads must be > 0"));
        }
        if self.stack_size < 64 * 1024 {
            return Err(ConfigError::InvalidValue("stack_size must be >= 64KB"));
        }
        if self.local_queue_capacity == 0 {
            return Err(ConfigError::InvalidValue("local_queue_capacity must be > 0"));
        }
        if self.global_queue_capacity == 0 {
            return Err(ConfigError::InvalidValue("global_queue_capacity must be > 0"));
        }
        Ok(())
    }

    /// Print configuration (for debugging)
    pub fn print(&self) {
        eprintln!("GVThread Configuration:");
        eprintln!("  num_workers:            {}", self.num_workers);
        eprintln!("  num_low_priority:       {}", self.num_low_priority_workers);
        eprintln!("  max_gvthreads:          {}", self.max_gvthreads);
        eprintln!("  time_slice:             {:?}", self.time_slice);
        eprintln!("  grace_period:           {:?}", self.grace_period);
        eprintln!("  timer_interval:         {:?}", self.timer_interval);
        eprintln!("  enable_forced_preempt:  {}", self.enable_forced_preempt);
        eprintln!("  debug_logging:          {}", self.debug_logging);
        eprintln!("  stack_size:             {}", self.stack_size);
        eprintln!("  local_queue_capacity:   {}", self.local_queue_capacity);
        eprintln!("  global_queue_capacity:  {}", self.global_queue_capacity);
        eprintln!("  idle_spins:             {}", self.idle_spins);
        eprintln!("  park_timeout:           {:?}", self.park_timeout);
    }
}

/// Configuration error
#[derive(Debug, Clone)]
pub enum ConfigError {
    InvalidValue(&'static str),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::InvalidValue(msg) => write!(f, "Invalid config: {}", msg),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env() {
        let config = SchedulerConfig::from_env();
        assert!(config.num_workers >= 1);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_builder() {
        let config = SchedulerConfig::from_env()
            .num_workers(8)
            .time_slice(Duration::from_millis(5))
            .enable_forced_preempt(false);

        assert_eq!(config.num_workers, 8);
        assert_eq!(config.time_slice, Duration::from_millis(5));
        assert!(!config.enable_forced_preempt);
    }

    #[test]
    fn test_validation() {
        let config = SchedulerConfig::from_env().num_workers(0);
        assert!(config.validate().is_err());

        let config = SchedulerConfig::from_env().num_workers(1000);
        assert!(config.validate().is_err());
    }
}