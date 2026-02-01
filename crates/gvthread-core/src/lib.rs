//! # gvthread-core
//!
//! Core types and traits for the GVThread (Generic Virtual Thread) scheduler.
//!
//! This crate is platform-agnostic and contains no OS-specific code.
//! All platform-specific implementations are in `gvthread-runtime`.
//!
//! ## Modules
//!
//! - `id` - GVThread identifier type
//! - `state` - GVThread state and priority enums
//! - `metadata` - GVThread metadata layout (repr(C))
//! - `bitmap` - Ready queue bitmaps for O(1) scheduling
//! - `slot` - Slot allocator for GVThread memory
//! - `channel` - MPMC channel for GVThread communication
//! - `mutex` - GVThread-aware mutex
//! - `cancel` - Cancellation token for cooperative cancellation
//! - `error` - Error types
//! - `spinlock` - Internal spinlock primitive
//! - `traits` - Platform and architecture traits
//! - `kprint` - Kernel-style debug printing macros
//! - `env` - Environment variable utilities

#![allow(dead_code)]

pub mod id;
pub mod state;
pub mod metadata;
pub mod bitmap;
pub mod slot;
pub mod channel;
pub mod mutex;
pub mod cancel;
pub mod error;
pub mod spinlock;
pub mod traits;
pub mod kprint;
pub mod env;

// Re-exports for convenience
pub use id::GVThreadId;
pub use state::{GVThreadState, Priority};
pub use metadata::{GVThreadMetadata, WorkerState, WORKER_STATE_SIZE};
pub use bitmap::ReadyBitmaps;
pub use slot::SlotAllocator;
pub use channel::{channel, Sender, Receiver};
pub use mutex::SchedMutex;
pub use cancel::CancellationToken;
pub use error::{SchedError, SchedResult};
pub use spinlock::SpinLock;
pub use env::{env_get, env_get_bool, env_get_opt, env_get_str, env_is_set};

/// Constants for memory layout
pub mod constants {
    /// Slot size - configurable via feature flag
    /// Default: 16KB (4 pages) for debugging, gives ~8KB usable stack
    /// For production: rebuild with larger size or use GVT_SLOT_PAGES env
    #[cfg(feature = "large-stack")]
    pub const SLOT_SIZE: usize = 16 * 1024 * 1024;  // 16 MB
    
    #[cfg(not(feature = "large-stack"))]
    pub const SLOT_SIZE: usize = 16 * 1024;  // 16 KB (4 pages)
    
    /// Guard page size (4 KB)
    pub const GUARD_SIZE: usize = 4096;
    
    /// Metadata size at start of slot (4 KB, one page)
    pub const METADATA_SIZE: usize = 4096;
    
    /// Stack size within slot (slot - metadata - guard)
    pub const STACK_SIZE: usize = SLOT_SIZE - METADATA_SIZE - GUARD_SIZE;
    
    /// Maximum workers (OS threads)
    pub const MAX_WORKERS: usize = 64;
    
    /// Default maximum GVThreads
    pub const DEFAULT_MAX_GVTHREADS: usize = 65536;
    
    /// No GVThread sentinel value
    pub const GVTHREAD_NONE: u32 = u32::MAX;
    
    /// Cache line size for alignment
    pub const CACHE_LINE_SIZE: usize = 64;
}