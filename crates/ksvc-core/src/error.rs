//! KSVC error types.

use std::fmt;

#[derive(Debug)]
pub enum KsvcError {
    /// Ring is full, cannot submit/complete.
    RingFull,
    /// io_uring submission failed.
    IoUringSubmit(i32),
    /// io_uring setup failed.
    IoUringSetup(i32),
    /// Worker pool is shut down or full.
    WorkerUnavailable,
    /// Syscall number not routable (Tier 3 / unsupported).
    Unsupported(u32),
    /// The KSVC /dev/ksvc fd is not open or not created.
    NotInitialized,
    /// mmap failed.
    MmapFailed(i32),
    /// ioctl failed.
    IoctlFailed(i32),
    /// OS error with errno.
    Os(i32),
}

impl fmt::Display for KsvcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RingFull => write!(f, "ring full"),
            Self::IoUringSubmit(e) => write!(f, "io_uring submit: errno {}", e),
            Self::IoUringSetup(e) => write!(f, "io_uring setup: errno {}", e),
            Self::WorkerUnavailable => write!(f, "worker pool unavailable"),
            Self::Unsupported(nr) => write!(f, "unsupported syscall {}", nr),
            Self::NotInitialized => write!(f, "KSVC instance not initialized"),
            Self::MmapFailed(e) => write!(f, "mmap failed: errno {}", e),
            Self::IoctlFailed(e) => write!(f, "ioctl failed: errno {}", e),
            Self::Os(e) => write!(f, "OS error: errno {}", e),
        }
    }
}

impl std::error::Error for KsvcError {}

pub type Result<T> = std::result::Result<T, KsvcError>;
