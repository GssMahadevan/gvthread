//! # gerror — Generic Error
//!
//! A zero-dependency, structured error crate with numeric `GlobalId` codes
//! for fast matching and optional rich diagnostics.
//!
//! ## Design
//!
//! `GError` has two internal representations:
//!
//! - **Simple** (32 bytes prod, zero heap allocation): three `GlobalId` codes
//!   identifying the system, error, and user operation, plus a `SiteId`
//!   for per-call-site metrics.
//!
//! - **Full** (boxed `ErrorContext`): message, source chain, metadata,
//!   backtrace. Use for diagnostic errors, setup failures, config errors.
//!
//! Both variants expose the same API: `.system()`, `.error_code()`,
//! `.user_code()`, `.kind()`, `.site_id()`.
//!
//! ## Quick Start
//!
//! ```rust
//! use gerror::{GError, GlobalId, GResult, err, match_error};
//!
//! // Define your domain codes
//! const SYS_NET: GlobalId = GlobalId::new("net", 3);
//! const SUB_LISTENER: GlobalId = GlobalId::new("listener", 5);
//! const ERR_EAGAIN: GlobalId = GlobalId::new("eagain", 11);
//! const ERR_BIND: GlobalId = GlobalId::new("bind_failed", 8);
//! const UC_ACCEPT: GlobalId = GlobalId::new("accept", 1);
//! const UC_LISTEN: GlobalId = GlobalId::new("listen", 2);
//!
//! // Fast path — zero allocation
//! fn accept_conn() -> GResult<i32> {
//!     Err(GError::simple(SYS_NET, ERR_EAGAIN, UC_ACCEPT))
//! }
//!
//! // Diagnostic path — full context
//! fn bind_port(port: u16) -> GResult<()> {
//!     Err(err!(SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN,
//!              "port already in use"))
//! }
//!
//! // Matching
//! fn handle(err: GError) -> &'static str {
//!     match_error!(err, {
//!         (SYS_NET, ERR_EAGAIN, UC_ACCEPT) => "backoff on listener",
//!         (SYS_NET, ERR_EAGAIN, _)         => "eagain on net",
//!         (SYS_NET, _, _)                  => "some net error",
//!         (_, _, _)                        => "unknown",
//!     })
//! }
//! ```
//!
//! ## Feature Flags
//!
//! | Flag         | Effect |
//! |--------------|--------|
//! | `production` | Strips `message`, `file`, `line`, `metadata` at compile time |
//! | `backtrace`  | Captures `std::backtrace::Backtrace` on error construction |
//! | `metrics`    | Per-site AtomicU64 counters, registry, Prometheus dump |
//!
//! ## Dependencies
//!
//! Zero. By design.

mod id;
mod site;
mod context;
mod error;
#[macro_use]
mod macros;
mod convert;

#[cfg(feature = "metrics")]
pub mod metrics;

// ── Public API ────────────────────────────────────────────────────

pub use id::GlobalId;
pub use site::SiteId;
pub use context::ErrorContext;
pub use error::GError;
pub use convert::{ResultExt, SYS_IO};

// Re-export macros (they use #[macro_export] so they're already at crate root,
// but explicit re-export makes documentation clearer).

/// Convenience Result alias.
pub type GResult<T> = Result<T, GError>;
