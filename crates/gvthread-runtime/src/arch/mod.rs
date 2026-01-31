//! Architecture-specific context switching
//!
//! Provides assembly implementations for saving and restoring registers
//! during GVThread context switches.

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        pub mod x86_64;
    } else if #[cfg(target_arch = "aarch64")] {
        pub mod aarch64;
    }
}
