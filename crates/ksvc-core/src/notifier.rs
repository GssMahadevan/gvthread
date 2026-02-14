//! Userspace notification abstraction.
//!
//! A `Notifier` wakes the userspace completion handler when
//! new completions are available in the KSVC completion ring.
//!
//! # Implementors
//!
//! - `EventFdNotifier` (default): writes 1 to an eventfd.
//!   The completion handler GVThread polls/reads the eventfd.
//!   Simple, well-understood, compatible with epoll/io_uring poll.
//!
//! - `FutexNotifier` (future): wakes a futex word.
//!   Lower overhead than eventfd for high-frequency notifications.
//!   Requires the completion handler to futex_wait on the word.

use crate::error::Result;

/// Wakes userspace when completions are ready.
///
/// **Contract:**
/// - `notify()` must NEVER block.
/// - Multiple calls before the consumer wakes are coalesced
///   (eventfd semantics: counter increments, one read drains).
/// - Called once per dispatcher loop iteration, not per completion.
pub trait Notifier: Send + Sync {
    /// Signal that new completions are available.
    fn notify(&self) -> Result<()>;
}
