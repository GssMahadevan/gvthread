//! Tier classification for syscall routing.

/// Which execution tier handles a given syscall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Tier {
    /// Tier 0: Shared page. Never reaches the ring.
    /// Handled entirely in userspace by reading mmap'd memory.
    SharedPage = 0,

    /// Tier 1: io_uring backed. Dispatcher translates to SQE.
    /// io-wq handles blocking. Zero head-of-line blocking.
    IoUring = 1,

    /// Tier 2: Worker pool. Dispatcher enqueues to bounded kthread pool.
    /// Workers can block independently. For syscalls without io_uring opcode.
    WorkerPool = 2,

    /// Tier 3: Unsupported by KSVC. GVThread falls back to traditional
    /// syscall() directly. Not submitted to the ring at all.
    Legacy = 3,
}
