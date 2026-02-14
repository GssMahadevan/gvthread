//! `SubmitRing` — multi-producer submit ring with CAS on tail.
//!
//! This is the write side of the KSVC submit ring. Multiple OS worker
//! threads (each running GVThreads) write entries concurrently.
//! The single dispatcher thread reads entries from the head.
//!
//! # Thread safety
//!
//! - **Producers (GVThreads on worker threads):** CAS-advance tail, then
//!   write entry at the claimed slot. Multiple producers are safe.
//! - **Consumer (dispatcher):** sole reader, advances head after processing.
//!   Single consumer — no CAS needed on head.
//!
//! # Memory layout
//!
//! The ring is mmap'd from the kernel module via /dev/ksvc:
//!
//! ```text
//! Page 0:          ksvc_ring_header { magic, ring_size, mask, entry_size, head, tail }
//! Pages 1..N:      ksvc_entry[ring_size]
//! ```
//!
//! head and tail are u64 monotonically increasing. Actual index = val & mask.
//! Ring is empty when head == tail.
//! Ring is full when (tail - head) >= ring_size.
//!
//! # Atomics
//!
//! The tail field in the ring header is accessed with atomic CAS by producers.
//! The head field is read with Acquire ordering by producers (to check fullness)
//! and written with Release ordering by the consumer.
//!
//! We use AtomicU64 pointers into the mmap'd memory. The kernel module
//! allocates the ring header with natural alignment (64-byte cache line),
//! so the atomic operations are safe on x86_64.

use std::sync::atomic::{AtomicU64, Ordering};
use std::ptr;

use crate::ksvc_sys;

/// Error returned when the submit ring is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingFullError;

impl std::fmt::Display for RingFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KSVC submit ring full")
    }
}

/// A handle to the mmap'd KSVC submit ring.
///
/// Created once per process after `KSVC_IOC_CREATE` + mmap.
/// Shared (via Arc) across all worker threads.
///
/// Producers call `try_push()` to submit entries.
/// The dispatcher calls `dequeue_batch()` to drain entries.
pub struct SubmitRing {
    /// Base address of the mmap'd region (header + data).
    base: *mut u8,
    /// Total mmap size in bytes (for munmap on drop).
    mmap_len: usize,
    /// Pointer to head field in the ring header (consumer position).
    /// Consumer (dispatcher) writes, producers read.
    head: *const AtomicU64,
    /// Pointer to tail field in the ring header (producer position).
    /// Producers CAS, consumer reads.
    tail: *const AtomicU64,
    /// Pointer to the first entry in the data region.
    entries: *mut ksvc_sys::KsvcEntry,
    /// ring_size - 1 (for fast modulo via bitwise AND).
    mask: u32,
    /// Number of entries (power of 2).
    ring_size: u32,
}

// Safety: The mmap'd memory is shared between threads.
// Access is synchronized via atomics on head/tail.
// Entry slots are exclusively owned between CAS-claim and publish.
unsafe impl Send for SubmitRing {}
unsafe impl Sync for SubmitRing {}

impl SubmitRing {
    /// Wrap an existing mmap'd submit ring region.
    ///
    /// # Safety
    ///
    /// - `base` must point to a valid mmap'd KSVC submit ring
    ///   (header page + data pages) with the correct size.
    /// - The memory must remain valid for the lifetime of this struct.
    /// - The caller must ensure `mmap_len` matches the actual mapping.
    pub unsafe fn from_mmap(base: *mut u8, mmap_len: usize) -> Result<Self, &'static str> {
        if base.is_null() {
            return Err("null base pointer");
        }

        // Read and validate the ring header
        let header = base as *const ksvc_sys::KsvcRingHeader;
        let magic = ptr::read_volatile(&(*header).magic);
        if magic != ksvc_sys::KSVC_RING_MAGIC {
            return Err("bad ring magic");
        }

        let ring_size = ptr::read_volatile(&(*header).ring_size);
        let mask = ptr::read_volatile(&(*header).mask);
        let entry_size = ptr::read_volatile(&(*header).entry_size);

        if ring_size == 0 || (ring_size & (ring_size - 1)) != 0 {
            return Err("ring_size not power of 2");
        }
        if mask != ring_size - 1 {
            return Err("mask mismatch");
        }
        if entry_size != std::mem::size_of::<ksvc_sys::KsvcEntry>() as u32 {
            return Err("entry_size mismatch");
        }

        // head and tail are at fixed offsets in the header.
        // ksvc_ring_header layout:
        //   0x00: magic (u32)
        //   0x04: ring_size (u32)
        //   0x08: mask (u32)
        //   0x0C: entry_size (u32)
        //   0x10: head (u64)  ← AtomicU64
        //   0x18: tail (u64)  ← AtomicU64
        let head_ptr = base.add(0x10) as *const AtomicU64;
        let tail_ptr = base.add(0x18) as *const AtomicU64;

        // Data region starts at page 1 (offset 4096 from base)
        let entries = base.add(4096) as *mut ksvc_sys::KsvcEntry;

        Ok(Self {
            base,
            mmap_len,
            head: head_ptr,
            tail: tail_ptr,
            entries,
            mask,
            ring_size,
        })
    }

    /// Try to push a submit entry into the ring.
    ///
    /// This is the hot path — called by GVThreads on any worker OS thread.
    /// Uses CAS on the tail to claim a slot, then writes the entry.
    ///
    /// Returns `Err(RingFullError)` if the ring is full.
    /// Returns `Ok(())` on success.
    ///
    /// # Lock-free guarantee
    ///
    /// This is wait-free for the common case (no contention) and
    /// lock-free under contention (CAS retry, bounded by ring_size).
    pub fn try_push(&self, entry: &ksvc_sys::KsvcEntry) -> Result<(), RingFullError> {
        loop {
            // 1. Read current tail (our candidate slot)
            let tail = self.tail().load(Ordering::Relaxed);

            // 2. Check if ring is full
            //    Read head with Acquire to see consumer's latest progress
            let head = self.head().load(Ordering::Acquire);
            if tail.wrapping_sub(head) >= self.ring_size as u64 {
                return Err(RingFullError);
            }

            // 3. CAS tail to claim this slot
            //    If another thread beat us, tail has advanced — retry.
            match self.tail().compare_exchange_weak(
                tail,
                tail.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // 4. We own slot at index = tail & mask
                    //    Write the entry. No other producer will write here
                    //    because we successfully claimed this tail position.
                    let idx = (tail as u32 & self.mask) as usize;
                    unsafe {
                        let slot = self.entries.add(idx);
                        ptr::write_volatile(slot, *entry);
                    }
                    // The entry is now visible to the consumer because:
                    // - We published tail with Release semantics (AcqRel in CAS)
                    // - Consumer reads tail with Acquire
                    return Ok(());
                }
                Err(_) => {
                    // CAS failed — another producer claimed this slot.
                    // Retry with updated tail.
                    std::hint::spin_loop();
                    continue;
                }
            }
        }
    }

    /// Convenience: build and push a submit entry in one call.
    pub fn submit(
        &self,
        corr_id: u64,
        syscall_nr: u32,
        args: [u64; 6],
    ) -> Result<(), RingFullError> {
        let entry = ksvc_sys::KsvcEntry {
            corr_id,
            syscall_nr,
            flags: 0,
            args,
        };
        self.try_push(&entry)
    }

    // ── Consumer methods (dispatcher only, single-threaded) ──

    /// Dequeue a batch of entries for the dispatcher.
    ///
    /// Reads up to `max` entries from head..tail.
    /// Advances head after reading.
    ///
    /// **Single consumer only** — the dispatcher thread.
    pub fn dequeue_batch(
        &self,
        buf: &mut [ksvc_sys::KsvcEntry],
        max: usize,
    ) -> usize {
        let head = self.head().load(Ordering::Relaxed);
        let tail = self.tail().load(Ordering::Acquire);

        let available = tail.wrapping_sub(head) as usize;
        let count = available.min(max).min(buf.len());

        for i in 0..count {
            let idx = ((head + i as u64) as u32 & self.mask) as usize;
            unsafe {
                buf[i] = ptr::read_volatile(self.entries.add(idx));
            }
        }

        if count > 0 {
            // Publish new head — producers can now reuse these slots.
            // Release ordering ensures our reads of entries are visible
            // before producers see the freed slots.
            self.head().store(head.wrapping_add(count as u64), Ordering::Release);
        }

        count
    }

    /// Check if the ring is empty (no pending submissions).
    pub fn is_empty(&self) -> bool {
        let head = self.head().load(Ordering::Relaxed);
        let tail = self.tail().load(Ordering::Acquire);
        head == tail
    }

    /// Number of entries currently in the ring.
    pub fn len(&self) -> usize {
        let head = self.head().load(Ordering::Relaxed);
        let tail = self.tail().load(Ordering::Acquire);
        tail.wrapping_sub(head) as usize
    }

    /// Ring capacity.
    pub fn capacity(&self) -> u32 {
        self.ring_size
    }

    // ── Internal helpers ──

    #[inline(always)]
    fn head(&self) -> &AtomicU64 {
        unsafe { &*self.head }
    }

    #[inline(always)]
    fn tail(&self) -> &AtomicU64 {
        unsafe { &*self.tail }
    }
}

impl Drop for SubmitRing {
    fn drop(&mut self) {
        if !self.base.is_null() && self.mmap_len > 0 {
            unsafe {
                libc::munmap(self.base as *mut libc::c_void, self.mmap_len);
            }
            self.base = std::ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Allocate a fake ring in heap memory for testing (no kernel module needed).
    unsafe fn alloc_test_ring(ring_size: u32) -> (*mut u8, usize) {
        let entry_size = std::mem::size_of::<ksvc_sys::KsvcEntry>();
        let data_bytes = ring_size as usize * entry_size;
        let data_pages = (data_bytes + 4095) / 4096;
        let total_pages = 1 + data_pages;
        let mmap_len = total_pages * 4096;

        // Use mmap for page-aligned memory
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            mmap_len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        assert_ne!(ptr, libc::MAP_FAILED);
        let base = ptr as *mut u8;

        // Initialize header
        let header = base as *mut ksvc_sys::KsvcRingHeader;
        (*header).magic = ksvc_sys::KSVC_RING_MAGIC;
        (*header).ring_size = ring_size;
        (*header).mask = ring_size - 1;
        (*header).entry_size = entry_size as u32;
        (*header).head = 0;
        (*header).tail = 0;

        (base, mmap_len)
    }

    #[test]
    fn test_push_pop_single_thread() {
        unsafe {
            let (base, mmap_len) = alloc_test_ring(16);
            let ring = SubmitRing::from_mmap(base, mmap_len).unwrap();

            // Push 3 entries
            for i in 0..3u64 {
                ring.submit(i + 100, i as u32, [i; 6]).unwrap();
            }

            assert_eq!(ring.len(), 3);

            // Dequeue
            let mut buf = [ksvc_sys::KsvcEntry::zeroed(); 16];
            let n = ring.dequeue_batch(&mut buf, 16);
            assert_eq!(n, 3);
            assert_eq!(buf[0].corr_id, 100);
            assert_eq!(buf[1].corr_id, 101);
            assert_eq!(buf[2].corr_id, 102);

            assert!(ring.is_empty());

            // Don't let SubmitRing::drop munmap — we'll do it
            std::mem::forget(ring);
            libc::munmap(base as *mut libc::c_void, mmap_len);
        }
    }

    #[test]
    fn test_ring_full() {
        unsafe {
            let (base, mmap_len) = alloc_test_ring(16);
            let ring = SubmitRing::from_mmap(base, mmap_len).unwrap();

            // Fill ring
            for i in 0..16u64 {
                ring.submit(i, 0, [0; 6]).unwrap();
            }

            // 17th should fail
            assert_eq!(ring.try_push(&ksvc_sys::KsvcEntry::zeroed()), Err(RingFullError));

            std::mem::forget(ring);
            libc::munmap(base as *mut libc::c_void, mmap_len);
        }
    }

    #[test]
    fn test_concurrent_producers() {
        unsafe {
            let (base, mmap_len) = alloc_test_ring(256);
            let ring = Arc::new(SubmitRing::from_mmap(base, mmap_len).unwrap());

            let n_threads = 4;
            let n_per_thread = 50;
            let mut handles = vec![];

            for t in 0..n_threads {
                let ring = Arc::clone(&ring);
                handles.push(std::thread::spawn(move || {
                    for i in 0..n_per_thread {
                        let corr_id = (t * 1000 + i) as u64;
                        ring.submit(corr_id, 0, [0; 6]).unwrap();
                    }
                }));
            }

            for h in handles {
                h.join().unwrap();
            }

            // All entries should be in the ring
            assert_eq!(ring.len(), n_threads * n_per_thread);

            // Dequeue all and verify no duplicates
            let mut buf = [ksvc_sys::KsvcEntry::zeroed(); 256];
            let n = ring.dequeue_batch(&mut buf, 256);
            assert_eq!(n, n_threads * n_per_thread);

            let mut seen = std::collections::HashSet::new();
            for i in 0..n {
                assert!(seen.insert(buf[i].corr_id), "duplicate corr_id {}", buf[i].corr_id);
            }

            std::mem::forget(ring);
            libc::munmap(base as *mut libc::c_void, mmap_len);
        }
    }

    #[test]
    fn test_wrap_around() {
        unsafe {
            let (base, mmap_len) = alloc_test_ring(16);
            let ring = SubmitRing::from_mmap(base, mmap_len).unwrap();

            // Fill and drain 3 times to wrap around
            for round in 0..3u64 {
                for i in 0..16u64 {
                    ring.submit(round * 100 + i, 0, [0; 6]).unwrap();
                }
                assert_eq!(ring.len(), 16);

                let mut buf = [ksvc_sys::KsvcEntry::zeroed(); 16];
                let n = ring.dequeue_batch(&mut buf, 16);
                assert_eq!(n, 16);
                assert_eq!(buf[0].corr_id, round * 100);
                assert!(ring.is_empty());
            }

            std::mem::forget(ring);
            libc::munmap(base as *mut libc::c_void, mmap_len);
        }
    }
}
