//! # gvthread-runtime
//!
//! Platform-specific runtime implementation for the GVThread scheduler.
//!
//! This crate provides:
//! - Memory management (mmap/VirtualAlloc)
//! - Signal handling (SIGURG for preemption)
//! - Worker thread management
//! - Context switching (architecture-specific assembly)
//! - Timer thread for preemption monitoring

#![allow(dead_code)]
#![allow(unused_variables)]
#![cfg_attr(feature = "nightly", feature(naked_functions))]
#![cfg_attr(feature = "nightly", feature(asm_const))]

pub mod config;
pub mod memory;
pub mod signal;
pub mod arch;
pub mod worker;
pub mod timer;
pub mod scheduler;
pub mod tls;
pub mod parking;

// Re-exports
pub use config::SchedulerConfig;
pub use scheduler::Scheduler;
pub use worker::{WorkerPool, worker_states};
pub use timer::{sleep, sleep_ms, sleep_us};
pub use parking::{WorkerParking, new_parking};

// Platform detection
cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        mod platform_linux;
        pub use platform_linux::LinuxPlatform as CurrentPlatform;
    } else if #[cfg(target_os = "macos")] {
        mod platform_macos;
        pub use platform_macos::MacOSPlatform as CurrentPlatform;
    } else if #[cfg(target_os = "windows")] {
        mod platform_windows;
        pub use platform_windows::WindowsPlatform as CurrentPlatform;
    } else {
        compile_error!("Unsupported platform");
    }
}

// Architecture detection
cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        pub use arch::x86_64 as current_arch;
    } else if #[cfg(target_arch = "aarch64")] {
        pub use arch::aarch64 as current_arch;
    } else {
        compile_error!("Unsupported architecture");
    }
}