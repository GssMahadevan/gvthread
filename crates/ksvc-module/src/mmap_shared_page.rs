//! `MmapSharedPage` — default `SharedPage` implementation.
//!
//! Reads process metadata directly from the KSVC shared page mmap'd
//! from the kernel module. Each read is a single volatile memory load.
//! Cost: ~4 cycles (L1 cache hit).

use ksvc_core::shared_page::SharedPage;
use crate::ksvc_sys::KsvcSharedPageLayout;

pub struct MmapSharedPage {
    /// Pointer to the mmap'd shared page.
    ptr: *const KsvcSharedPageLayout,
}

// Safety: the shared page is read-only from userspace,
// written only by the kernel module (single writer).
// Volatile reads ensure we see the latest values.
unsafe impl Send for MmapSharedPage {}
unsafe impl Sync for MmapSharedPage {}

impl MmapSharedPage {
    /// Create from a pointer to mmap'd shared page memory.
    ///
    /// # Safety
    /// - `ptr` must point to a valid KSVC shared page.
    /// - The memory must remain mapped for the lifetime of this struct.
    pub unsafe fn new(ptr: *const u8) -> Self {
        Self {
            ptr: ptr as *const KsvcSharedPageLayout,
        }
    }

    /// Read a field from the shared page via volatile load.
    #[inline(always)]
    unsafe fn read_i32(&self, offset: usize) -> i32 {
        let p = (self.ptr as *const u8).add(offset) as *const i32;
        std::ptr::read_volatile(p)
    }

    #[inline(always)]
    unsafe fn read_u32(&self, offset: usize) -> u32 {
        let p = (self.ptr as *const u8).add(offset) as *const u32;
        std::ptr::read_volatile(p)
    }

    #[inline(always)]
    unsafe fn read_u64(&self, offset: usize) -> u64 {
        let p = (self.ptr as *const u8).add(offset) as *const u64;
        std::ptr::read_volatile(p)
    }
}

// Field offsets matching the C struct layout in ksvc_uapi.h / ksvc_sys.rs
// These are computed from KsvcSharedPageLayout's repr(C) layout.
mod offsets {
    // magic: u32 = 0
    // version: u32 = 4
    pub const PID: usize = 8;
    pub const TGID: usize = 12;
    pub const PPID: usize = 16;
    pub const PGID: usize = 20;
    pub const SID: usize = 24;
    // _pad_id: 28
    pub const UID: usize = 32;
    pub const GID: usize = 36;
    pub const EUID: usize = 40;
    pub const EGID: usize = 44;
    // suid: 48, sgid: 52
    pub const KTHREAD_CPU: usize = 56;
    pub const WORKER_STATE: usize = 60;
    pub const ENTRIES_PROCESSED: usize = 64;
    // batches_processed: 72

    // Extended fields (from DESIGN.md shared page layout)
    pub const RLIMIT_NOFILE: usize = 0x108;
    pub const CLOCK_MONOTONIC_NS: usize = 0x280;
}

impl SharedPage for MmapSharedPage {
    #[inline] fn pid(&self) -> i32 { unsafe { self.read_i32(offsets::PID) } }
    #[inline] fn tgid(&self) -> i32 { unsafe { self.read_i32(offsets::TGID) } }
    #[inline] fn ppid(&self) -> i32 { unsafe { self.read_i32(offsets::PPID) } }
    #[inline] fn pgid(&self) -> i32 { unsafe { self.read_i32(offsets::PGID) } }
    #[inline] fn sid(&self) -> i32 { unsafe { self.read_i32(offsets::SID) } }

    #[inline] fn uid(&self) -> u32 { unsafe { self.read_u32(offsets::UID) } }
    #[inline] fn gid(&self) -> u32 { unsafe { self.read_u32(offsets::GID) } }
    #[inline] fn euid(&self) -> u32 { unsafe { self.read_u32(offsets::EUID) } }
    #[inline] fn egid(&self) -> u32 { unsafe { self.read_u32(offsets::EGID) } }

    #[inline] fn rlimit_nofile(&self) -> u64 { unsafe { self.read_u64(offsets::RLIMIT_NOFILE) } }
    #[inline] fn clock_monotonic_ns(&self) -> u64 { unsafe { self.read_u64(offsets::CLOCK_MONOTONIC_NS) } }
    #[inline] fn entries_processed(&self) -> u64 { unsafe { self.read_u64(offsets::ENTRIES_PROCESSED) } }
    #[inline] fn kthread_cpu(&self) -> u32 { unsafe { self.read_u32(offsets::KTHREAD_CPU) } }
    #[inline] fn worker_state(&self) -> u32 { unsafe { self.read_u32(offsets::WORKER_STATE) } }
}
