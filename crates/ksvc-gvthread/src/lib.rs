//! # ksvc-gvthread — Green threads on io_uring
//!
//! This crate bridges GVThread green threads with KSVC's io_uring backend.
//! GVThreads make blocking-style I/O calls that are transparently multiplexed
//! onto io_uring by a reactor thread — the same pattern as Go's netpoller.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │          GVThread User Code                              │
//! │   stream.read(&mut buf)    ← looks like blocking I/O    │
//! │   stream.write_all(&resp)  ← but frees the worker       │
//! └──────────────────┬──────────────────────────────────────┘
//!                    │ submit_and_park()
//!                    ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │         ksvc-gvthread Bridge                             │
//! │   IoRequest → MPSC queue → Reactor thread               │
//! │   block_current() ─── worker runs other GVThreads       │
//! │   on completion: write result + wake_gvthread()          │
//! └──────────────────┬──────────────────────────────────────┘
//!                    │
//! ┌──────────────────▼──────────────────────────────────────┐
//! │         Reactor Thread (dedicated OS thread)             │
//! │   BasicIoUring → submit SQEs → flush_and_wait()         │
//! │   poll CQEs → results slab → wake GVThread              │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Quick Start
//!
//! ```ignore
//! use gvthread::{Runtime, spawn, SchedulerConfig};
//! use ksvc_gvthread::{Reactor, ReactorConfig, net::GvtListener};
//!
//! fn main() {
//!     let config = SchedulerConfig::default().num_workers(4);
//!     let mut runtime = Runtime::new(config);
//!
//!     // Start io_uring reactor
//!     let mut reactor = Reactor::start(ReactorConfig::default());
//!     let shared = reactor.shared();
//!
//!     runtime.block_on(|| {
//!         let listener = GvtListener::bind(shared.clone(), 8080).unwrap();
//!
//!         // Accept loop — one GVThread per connection (like goroutines)
//!         loop {
//!             let stream = listener.accept().unwrap();
//!             let r = shared.clone();
//!             spawn(move |_token| {
//!                 let mut buf = [0u8; 4096];
//!                 let n = stream.read(&mut buf);
//!                 if n > 0 {
//!                     let response = b"HTTP/1.1 200 OK\r\n\r\nHello!\n";
//!                     stream.write_all(response);
//!                 }
//!             });
//!         }
//!     });
//!
//!     reactor.shutdown();
//! }
//! ```
//!
//! ## Design Decisions
//!
//! - **CorrId = GVThread slot ID**: Zero-lookup completion routing
//! - **Dedicated reactor thread**: Always available, never starved by GVThread work
//! - **Results slab**: O(1) result delivery indexed by slot, no hashmap
//! - **MPSC queue**: Lock-free crossbeam `ArrayQueue` for request submission
//! - **Stack-safe**: GVThread stacks are stable while blocked (no move/invalidation)

pub mod reactor;
pub mod syscall;
pub mod net;

// Re-export the main types
pub use reactor::{Reactor, ReactorConfig, ReactorShared};
pub use syscall::*;
pub use net::{GvtListener, GvtStream};
