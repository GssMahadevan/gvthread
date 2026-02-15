//! Standard `GlobalId` constants for cross-crate error interop.
//!
//! # Code Ranges
//!
//! The `u64` code space is partitioned to prevent collisions:
//!
//! | Range             | Purpose                                      |
//! |-------------------|----------------------------------------------|
//! | `0`               | `UNSET` — sentinel / don't-care              |
//! | `1 — 999`         | gerror reserved (future)                     |
//! | `1000 — 1999`     | OS system identifiers                        |
//! | `2000 — 2999`     | OS error codes (POSIX errno)                 |
//! | `3000 — 3999`     | OS subsystems                                |
//! | `4000 — 4999`     | Standard operation codes (verbs)             |
//! | `5000 — 5999`     | GVThread systems/subsystems (reserved)       |
//! | `6000 — 6999`     | GVThread error codes (reserved)              |
//! | `7000 — 7999`     | GVThread user codes (reserved)               |
//! | `10000 — 99999`   | Framework namespace (reserved for future)    |
//! | `100000+`         | **User application space** (free for all)    |
//!
//! # Usage
//!
//! ```rust
//! use gerror::codes::*;
//! use gerror::{GError, match_error};
//!
//! let err = GError::simple_os(SYS_LINUX, ERR_EAGAIN, UC_ACCEPT, 11);
//!
//! match_error!(err, {
//!     (SYS_LINUX, ERR_EAGAIN, UC_ACCEPT) => println!("backoff"),
//!     (_, _, _) => println!("other"),
//! });
//! ```

mod os;
mod errno;
mod ops;
mod gvthread;

pub use os::*;
pub use errno::*;
pub use ops::*;
pub use gvthread::*;
