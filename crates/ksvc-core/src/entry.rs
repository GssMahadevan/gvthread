//! Submission and completion entry types.
//!
//! These mirror the C structs in ksvc_uapi.h but are safe Rust types.
//! They are the *lingua franca* between all KSVC components.

/// Correlation ID — maps 1:1 to a GVThread ID.
/// Stored in io_uring's `user_data` field for zero-lookup routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct CorrId(pub u64);

impl CorrId {
    pub const NONE: Self = Self(u64::MAX);

    #[inline]
    pub fn from_gvthread_id(id: u32) -> Self {
        Self(id as u64)
    }

    #[inline]
    pub fn as_gvthread_id(self) -> u32 {
        self.0 as u32
    }
}

/// A syscall submission entry.
///
/// Written by userspace GVThread into the KSVC submit ring.
/// Read by the dispatcher kthread.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
pub struct SubmitEntry {
    /// Correlation ID = GVThread ID of the submitter.
    pub corr_id: CorrId,
    /// Linux syscall number (__NR_read, __NR_write, etc.)
    pub syscall_nr: u32,
    /// Flags (KSVC_FLAG_*)
    pub flags: u32,
    /// Standard syscall arguments (up to 6).
    pub args: [u64; 6],
}

/// A completion entry.
///
/// Written by the dispatcher/worker into the KSVC completion ring.
/// Read by the userspace completion handler GVThread.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(32))]
pub struct CompletionEntry {
    /// Matches the submission's corr_id.
    pub corr_id: CorrId,
    /// Syscall return value, or negative errno.
    pub result: i64,
    /// Flags (KSVC_COMP_*)
    pub flags: u32,
    pub _pad: u32,
}

/// Submission flags.
pub mod submit_flags {
    /// This entry depends on the previous entry completing first.
    pub const LINKED: u32 = 1 << 0;
    /// Wait for all prior entries to complete before processing this one.
    pub const DRAIN: u32 = 1 << 1;
}

/// Completion flags.
pub mod comp_flags {
    /// More completions are available (hint to keep polling).
    pub const MORE: u32 = 1 << 0;
}
