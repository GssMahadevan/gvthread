//! `HeapBuffers` — default `BufferProvider` implementation.
//!
//! Each buffer is a standard heap allocation via `Vec<u8>`.
//! No pre-registration, no page pinning.
//! Simple, safe, correct. Use `RegisteredBuffers` for O_DIRECT perf.

use ksvc_core::buffer::{BufferHandle, BufferProvider};

use std::sync::atomic::{AtomicUsize, Ordering};

pub struct HeapBuffers {
    /// Default buffer size for allocations.
    default_size: usize,
    /// Number of buffers currently in use (for diagnostics).
    in_use: AtomicUsize,
    /// Total allocated (for diagnostics).
    total: AtomicUsize,
}

impl HeapBuffers {
    pub fn new(default_size: usize) -> Self {
        Self {
            default_size,
            in_use: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
        }
    }
}

impl Default for HeapBuffers {
    fn default() -> Self {
        Self::new(8192) // 8 KiB default
    }
}

impl BufferProvider for HeapBuffers {
    fn acquire(&self, min_size: usize) -> Option<BufferHandle> {
        let size = min_size.max(self.default_size);
        let mut buf = Vec::<u8>::with_capacity(size);
        // Safety: we need the raw pointer. The Vec stays alive via leak.
        let ptr = buf.as_mut_ptr();
        let len = buf.capacity();
        std::mem::forget(buf); // Leak — will be reclaimed in release()

        self.in_use.fetch_add(1, Ordering::Relaxed);
        self.total.fetch_add(1, Ordering::Relaxed);

        Some(BufferHandle {
            ptr,
            len,
            buf_index: u16::MAX, // not a registered buffer
        })
    }

    fn release(&self, handle: BufferHandle) {
        // Reconstruct the Vec and let it drop.
        unsafe {
            let _ = Vec::from_raw_parts(handle.ptr, 0, handle.len);
        }
        self.in_use.fetch_sub(1, Ordering::Relaxed);
    }

    fn is_registered(&self) -> bool {
        false
    }

    fn pool_size(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    fn in_use(&self) -> usize {
        self.in_use.load(Ordering::Relaxed)
    }
}
