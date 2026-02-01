//! GVThread Configuration File
//!
//! This is an example configuration file for GVThread.
//! Copy this file to your project and modify as needed.
//!
//! Usage:
//!   GVT_CONFIG_RS=./gvt_config.rs cargo build --features custom-config
//!
//! You only need to include parameters you want to change.
//! All other parameters will use library defaults.
//!
//! These values can still be overridden at runtime via environment variables:
//!   GVT_NUM_WORKERS=16 ./my-app

// Number of worker threads for running GVThreads
pub const NUM_WORKERS: usize = 4;

// Number of workers dedicated to low priority GVThreads
pub const NUM_LOW_PRIORITY_WORKERS: usize = 1;

// Maximum concurrent GVThreads (affects memory reservation)
pub const MAX_GVTHREADS: usize = 1_048_576;

// Time slice before setting preempt flag (ms)
pub const TIME_SLICE_MS: u64 = 10;

// Grace period before forced preemption via SIGURG (ms)
pub const GRACE_PERIOD_MS: u64 = 1;

// Timer thread check interval (ms)
pub const TIMER_INTERVAL_MS: u64 = 1;

// Maximum timer thread sleep duration (ms)
pub const TIMER_MAX_SLEEP_MS: u64 = 10;

// Enable SIGURG-based forced preemption
pub const ENABLE_FORCED_PREEMPT: bool = true;

// Enable debug logging
pub const DEBUG_LOGGING: bool = false;

// Virtual stack size per GVThread (16MB default)
pub const STACK_SIZE: usize = 16 * 1024 * 1024;

// Per-worker local queue capacity
pub const LOCAL_QUEUE_CAPACITY: usize = 256;

// Global queue capacity
pub const GLOBAL_QUEUE_CAPACITY: usize = 65536;

// Spins before parking an idle worker
pub const IDLE_SPINS: u32 = 10;

// Worker park timeout (ms)
pub const PARK_TIMEOUT_MS: u64 = 100;