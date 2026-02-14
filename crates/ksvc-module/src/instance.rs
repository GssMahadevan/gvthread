//! `KsvcInstance` — the compositor that wires all traits together.
//!
//! This is the "dependency injection" point. All trait impls are
//! generic parameters with defaults. To swap an implementation,
//! change the type alias — no other code changes needed.
//!
//! ```text
//! KsvcInstance<
//!     R: SyscallRouter    = ProbeRouter,
//!     B: IoBackend         = BasicIoUring,
//!     W: WorkerPool        = FixedPool,
//!     N: Notifier          = EventFdNotifier,
//!     P: BufferProvider    = HeapBuffers,
//! >
//! ```

use ksvc_core::io_backend::IoBackend;
use ksvc_core::notifier::Notifier;
use ksvc_core::router::SyscallRouter;
use ksvc_core::worker::WorkerPool;
use ksvc_core::buffer::BufferProvider;
use ksvc_core::error::Result;

use crate::basic_iouring::{BasicIoUring, BasicIoUringConfig};
use crate::eventfd_notifier::EventFdNotifier;
use crate::fixed_pool::FixedPool;
use crate::heap_buffers::HeapBuffers;
use crate::probe_router::ProbeRouter;

/// The fully-wired KSVC instance.
///
/// Owns all component implementations. Created once per process.
/// Passed to the dispatcher loop which drives the main event loop.
pub struct KsvcInstance<R, B, W, N, P>
where
    R: SyscallRouter,
    B: IoBackend,
    W: WorkerPool,
    N: Notifier,
    P: BufferProvider,
{
    pub router: R,
    pub io_backend: B,
    pub worker_pool: W,
    pub notifier: N,
    pub buffer_provider: P,
    /// The eventfd for waking userspace.
    pub eventfd_raw: i32,
}

/// Type alias for the default (safe, working) configuration.
pub type DefaultInstance = KsvcInstance<
    ProbeRouter,
    BasicIoUring,
    FixedPool,
    EventFdNotifier,
    HeapBuffers,
>;

/// Builder for constructing a default KSVC instance.
///
/// Each component can be overridden before building.
pub struct InstanceBuilder {
    sq_entries: u32,
    worker_count: usize,
    worker_queue_depth: usize,
    buffer_size: usize,
}

impl Default for InstanceBuilder {
    fn default() -> Self {
        Self {
            sq_entries: 256,
            worker_count: 0, // 0 = auto
            worker_queue_depth: 256,
            buffer_size: 8192,
        }
    }
}

impl InstanceBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sq_entries(mut self, n: u32) -> Self {
        self.sq_entries = n;
        self
    }

    pub fn worker_count(mut self, n: usize) -> Self {
        self.worker_count = n;
        self
    }

    pub fn worker_queue_depth(mut self, n: usize) -> Self {
        self.worker_queue_depth = n;
        self
    }

    pub fn buffer_size(mut self, n: usize) -> Self {
        self.buffer_size = n;
        self
    }

    /// Build the default instance.
    ///
    /// 1. Creates io_uring ring
    /// 2. Probes supported opcodes
    /// 3. Builds routing table
    /// 4. Spawns worker pool
    /// 5. Creates eventfd notifier
    /// 6. Creates buffer provider
    pub fn build(self) -> Result<DefaultInstance> {
        // 1. io_uring
        let io_backend = BasicIoUring::new(BasicIoUringConfig {
            sq_entries: self.sq_entries,
            ..Default::default()
        })?;

        // 2. Probe opcodes from the running kernel
        let supported = io_backend.probe_opcodes();

        // 3. Build routing table
        let router = ProbeRouter::new(&supported);

        // Log tier counts for diagnostics
        let counts = router.tier_counts();
        eprintln!(
            "ksvc: routing table built — T0:{} T1:{} T2:{} T3:{}",
            counts.tier0, counts.tier1, counts.tier2, counts.tier3
        );

        // 4. Worker pool
        let worker_pool = if self.worker_count == 0 {
            FixedPool::auto_sized(self.worker_queue_depth)
        } else {
            FixedPool::new(self.worker_count, self.worker_queue_depth)
        };

        // 5. Eventfd notifier
        let notifier = EventFdNotifier::create()?;
        let eventfd_raw = notifier.fd();

        // 6. Buffer provider
        let buffer_provider = HeapBuffers::new(self.buffer_size);

        Ok(KsvcInstance {
            router,
            io_backend,
            worker_pool,
            notifier,
            buffer_provider,
            eventfd_raw,
        })
    }
}

/// Ordered shutdown: workers first (drain Tier 2), then io_uring (drain Tier 1),
/// then notifier. Rust's default Drop order is field declaration order, but
/// we need workers stopped BEFORE io_uring is torn down (workers might
/// reference in-flight state).
impl<R, B, W, N, P> Drop for KsvcInstance<R, B, W, N, P>
where
    R: SyscallRouter,
    B: IoBackend,
    W: WorkerPool,
    N: Notifier,
    P: BufferProvider,
{
    fn drop(&mut self) {
        // 1. Stop accepting new work
        // 2. Drain Tier 2 worker pool (blocks until workers finish)
        self.worker_pool.shutdown();
        // 3. Drain Tier 1 io_uring completions
        self.io_backend.shutdown();
        // 4. Notifier + buffer_provider + router: dropped automatically by Rust
        eprintln!("ksvc: instance shut down cleanly");
    }
}
