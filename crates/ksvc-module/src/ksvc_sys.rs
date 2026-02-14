//! Raw bindings to the KSVC kernel module.
//!
//! Mirrors `ksvc_uapi.h` — the shared contract between kernel and userspace.

use std::os::unix::io::RawFd;

// ── Magic numbers ──

pub const KSVC_MAGIC: u32 = 0x4B535643; // "KSVC"
pub const KSVC_RING_MAGIC: u32 = 0x4B52494E; // "KRIN"
pub const KSVC_SHARED_MAGIC: u32 = 0x4B534850; // "KSHP"
pub const KSVC_VERSION: u32 = 2;

// ── mmap offsets ──

pub const KSVC_OFF_SUBMIT_RING: u64 = 0x0000_0000;
pub const KSVC_OFF_COMPLETE_RING: u64 = 0x0010_0000; // 1MB
pub const KSVC_OFF_SHARED_PAGE: u64 = 0x0020_0000; // 2MB

// ── Ring sizes ──

pub const KSVC_MAX_RING_ENTRIES: u32 = 4096;
pub const KSVC_MIN_RING_ENTRIES: u32 = 16;
pub const KSVC_MAX_BATCH: u32 = 64;

// ── ioctl ──

const KSVC_IOC_MAGIC: u8 = b'K';

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KsvcCreateParams {
    pub submit_ring_entries: u32,
    pub complete_ring_entries: u32,
    pub flags: u32,
    pub eventfd: i32,
    pub _reserved: [u32; 4],
}

impl Default for KsvcCreateParams {
    fn default() -> Self {
        Self {
            submit_ring_entries: 256,
            complete_ring_entries: 256,
            flags: 0,
            eventfd: -1,
            _reserved: [0; 4],
        }
    }
}

// ioctl number: _IOWR('K', 1, struct ksvc_create_params)
nix::ioctl_readwrite!(ksvc_ioc_create, KSVC_IOC_MAGIC, 1, KsvcCreateParams);

// ── Submit entry (64 bytes, cache-line aligned) ──

#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct KsvcEntry {
    pub corr_id: u64,
    pub syscall_nr: u32,
    pub flags: u32,
    pub args: [u64; 6],
}

impl KsvcEntry {
    pub fn zeroed() -> Self {
        Self {
            corr_id: 0,
            syscall_nr: 0,
            flags: 0,
            args: [0; 6],
        }
    }
}

// ── Completion entry (32 bytes, aligned) ──

#[repr(C, align(32))]
#[derive(Debug, Clone, Copy)]
pub struct KsvcCompletion {
    pub corr_id: u64,
    pub result: i64,
    pub flags: u32,
    pub _pad: u32,
}

impl KsvcCompletion {
    pub fn zeroed() -> Self {
        Self {
            corr_id: 0,
            result: 0,
            flags: 0,
            _pad: 0,
        }
    }
}

// ── Ring header (mmap'd, 64-byte cache line) ──

#[repr(C, align(64))]
#[derive(Debug)]
pub struct KsvcRingHeader {
    pub magic: u32,
    pub ring_size: u32,
    pub mask: u32,
    pub entry_size: u32,
    pub head: u64,
    pub tail: u64,
    pub _reserved: [u64; 3],
}

// ── Shared page layout (mmap'd read-only) ──

#[repr(C)]
#[derive(Debug)]
pub struct KsvcSharedPageLayout {
    pub magic: u32,
    pub version: u32,

    // Process identity
    pub pid: i32,
    pub tgid: i32,
    pub ppid: i32,
    pub pgid: i32,
    pub sid: i32,
    pub _pad_id: i32,

    // Credentials
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
    pub suid: u32,
    pub sgid: u32,

    // KSVC instance state
    pub kthread_cpu: u32,
    pub worker_state: u32,
    pub entries_processed: u64,
    pub batches_processed: u64,
}

// ── Helper: open /dev/ksvc ──

pub fn open_ksvc() -> std::io::Result<RawFd> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open("/dev/ksvc")?;
    // Don't close on drop — we need the raw fd
    use std::os::unix::io::IntoRawFd;
    Ok(file.into_raw_fd())
}
