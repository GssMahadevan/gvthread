//! Buffer management abstraction.
//!
//! A `BufferProvider` manages I/O buffers for read/write operations.
//!
//! # Implementors
//!
//! - `HeapBuffers` (default): each I/O operation uses a userspace-allocated
//!   buffer. No pre-registration. Simple, safe, works everywhere.
//!   The buffer pointer is passed directly in the syscall args.
//!
//! - `RegisteredBuffers` (feature = "fixed-buffers"): pre-pins a set of
//!   buffers via `IORING_REGISTER_BUFFERS`. Uses `IORING_OP_READ_FIXED` /
//!   `IORING_OP_WRITE_FIXED`. Eliminates per-I/O page pinning.
//!   Major win for O_DIRECT workloads.
//!
//! - `ProvidedBufferRing` (future): io_uring selects buffers from a
//!   pre-registered ring. Eliminates buffer allocation per read.
//!   Requires `IOSQE_BUFFER_SELECT` flag. Available since 5.7.

/// Handle to a buffer managed by a `BufferProvider`.
#[derive(Debug, Clone, Copy)]
pub struct BufferHandle {
    /// Pointer to the buffer data (userspace address).
    pub ptr: *mut u8,
    /// Length of the buffer in bytes.
    pub len: usize,
    /// Index into the registered buffer table (for fixed buffers).
    /// `u16::MAX` means "not a registered buffer" (heap buffer).
    pub buf_index: u16,
}

// Safety: buffer handles are just pointers + metadata
unsafe impl Send for BufferHandle {}
unsafe impl Sync for BufferHandle {}

/// Manages I/O buffer lifecycle.
///
/// **Contract:**
/// - `acquire()` returns a buffer suitable for I/O.
/// - `release()` returns the buffer to the pool.
/// - Buffers must remain valid (pinned if necessary) for the duration
///   of the I/O operation.
pub trait BufferProvider: Send + Sync {
    /// Acquire a buffer of at least `min_size` bytes.
    ///
    /// Returns `None` if no buffer is available (pool exhausted).
    fn acquire(&self, min_size: usize) -> Option<BufferHandle>;

    /// Release a previously acquired buffer back to the pool.
    fn release(&self, handle: BufferHandle);

    /// Whether this provider uses registered (fixed) buffers.
    ///
    /// If true, the IoBackend should use `IORING_OP_READ_FIXED` etc.
    /// If false, use `IORING_OP_READ` with regular buffer pointers.
    fn is_registered(&self) -> bool {
        false
    }

    /// Total number of buffers in the pool.
    fn pool_size(&self) -> usize;

    /// Number of buffers currently in use.
    fn in_use(&self) -> usize;
}
