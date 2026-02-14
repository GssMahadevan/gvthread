//! Shared page abstraction (Tier 0 reads).
//!
//! A `SharedPage` provides zero-cost access to kernel-maintained
//! process metadata. The page is mmap'd read-only into userspace
//! and updated by the kernel module.
//!
//! # Implementors
//!
//! - `MmapSharedPage` (default): reads from the mmap'd KSVC shared page
//!   at `KSVC_OFF_SHARED_PAGE`. Direct memory access, ~4 cycles.
//!
//! - `CachedSharedPage` (future): caches frequently-read fields in
//!   thread-local storage. Avoids even the L1 cache line fetch for
//!   truly hot fields (pid, uid). ~1-2 cycles for cached fields.
//!   Falls through to mmap for uncached fields.

/// Process metadata available via Tier 0 (zero-cost reads).
///
/// All methods are `#[inline]` because they're single memory loads.
/// The backing store is a volatile read from mmap'd memory.
pub trait SharedPage: Send + Sync {
    /// Process ID.
    fn pid(&self) -> i32;
    /// Thread group ID (usually same as pid for main thread).
    fn tgid(&self) -> i32;
    /// Parent process ID.
    fn ppid(&self) -> i32;
    /// Process group ID.
    fn pgid(&self) -> i32;
    /// Session ID.
    fn sid(&self) -> i32;

    /// Real user ID.
    fn uid(&self) -> u32;
    /// Real group ID.
    fn gid(&self) -> u32;
    /// Effective user ID.
    fn euid(&self) -> u32;
    /// Effective group ID.
    fn egid(&self) -> u32;

    /// RLIMIT_NOFILE (max open file descriptors).
    fn rlimit_nofile(&self) -> u64;

    /// Monotonic clock timestamp (nanoseconds). Updated per batch.
    fn clock_monotonic_ns(&self) -> u64;

    /// Number of entries the kthread has processed (monotonic counter).
    fn entries_processed(&self) -> u64;

    /// Current CPU the kthread is running on.
    fn kthread_cpu(&self) -> u32;

    /// Whether the kthread is currently processing (0=sleeping, 1=processing).
    fn worker_state(&self) -> u32;
}
