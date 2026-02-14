//! # ksvc-module — Default (safe) implementations
//!
//! This crate provides the default implementation for every KSVC trait.
//! Each impl prioritizes correctness and simplicity over performance.
//! Optimized impls live behind feature flags or in separate crates.
//!
//! ## Default stack
//!
//! | Trait           | Default Impl       | Feature-gated alternative |
//! |-----------------|--------------------|---------------------------|
//! | IoBackend       | BasicIoUring       | SqpollIoUring (sqpoll)    |
//! | WorkerPool      | FixedPool          | LazyPool (future)         |
//! | CompletionSink  | RingCompletionSink | DirectWakeSink (future)   |
//! | Notifier        | EventFdNotifier    | FutexNotifier (future)    |
//! | BufferProvider  | HeapBuffers        | RegisteredBuffers (fixed) |
//! | SyscallRouter   | ProbeRouter        | StaticRouter (compile)    |
//! | SharedPage      | MmapSharedPage     | CachedSharedPage (future) |

pub mod basic_iouring;
pub mod probe_router;
pub mod fixed_pool;
pub mod eventfd_notifier;
pub mod ring_completion;
pub mod heap_buffers;
pub mod mmap_shared_page;
pub mod submit_ring;
pub mod instance;
pub mod ksvc_sys;
