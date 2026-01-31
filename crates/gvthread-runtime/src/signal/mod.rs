//! Signal handling for preemption
//!
//! Uses SIGURG on Unix systems to force preemption of CPU-bound GVThreads.

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix;
        pub use unix::*;
    } else if #[cfg(windows)] {
        mod windows;
        pub use windows::*;
    }
}
