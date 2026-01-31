//! Ready queue bitmaps for O(1) scheduling
//!
//! Uses atomic bitmaps to track which GVThreads are ready to run.
//! Separate bitmaps for each priority level, scanned in order.
//! Random starting block for fairness across GVThreads.

use core::sync::atomic::{AtomicU64, Ordering};
use crate::id::GVThreadId;
use crate::state::Priority;
use crate::spinlock::SpinLock;

/// Number of bits per block
const BITS_PER_BLOCK: usize = 64;

/// Single priority level bitmap
pub struct ReadyBitmap {
    /// Bitmap blocks (each u64 holds 64 GVThread ready bits)
    blocks: Box<[AtomicU64]>,
    
    /// Number of blocks
    num_blocks: usize,
    
    /// Maximum GVThread ID this bitmap can hold
    max_id: u32,
}

impl ReadyBitmap {
    /// Create a new bitmap for the given maximum GVThread count
    pub fn new(max_gvthreads: usize) -> Self {
        let num_blocks = (max_gvthreads + BITS_PER_BLOCK - 1) / BITS_PER_BLOCK;
        let blocks: Vec<AtomicU64> = (0..num_blocks)
            .map(|_| AtomicU64::new(0))
            .collect();
        
        Self {
            blocks: blocks.into_boxed_slice(),
            num_blocks,
            max_id: max_gvthreads as u32,
        }
    }
    
    /// Set a GVThread as ready
    #[inline]
    pub fn set(&self, id: GVThreadId) {
        let idx = id.as_usize();
        if idx >= self.max_id as usize {
            return;
        }
        
        let block_idx = idx / BITS_PER_BLOCK;
        let bit_idx = idx % BITS_PER_BLOCK;
        let mask = 1u64 << bit_idx;
        
        self.blocks[block_idx].fetch_or(mask, Ordering::Release);
    }
    
    /// Clear a GVThread from ready
    #[inline]
    pub fn clear(&self, id: GVThreadId) {
        let idx = id.as_usize();
        if idx >= self.max_id as usize {
            return;
        }
        
        let block_idx = idx / BITS_PER_BLOCK;
        let bit_idx = idx % BITS_PER_BLOCK;
        let mask = !(1u64 << bit_idx);
        
        self.blocks[block_idx].fetch_and(mask, Ordering::Release);
    }
    
    /// Check if a GVThread is ready
    #[inline]
    pub fn is_set(&self, id: GVThreadId) -> bool {
        let idx = id.as_usize();
        if idx >= self.max_id as usize {
            return false;
        }
        
        let block_idx = idx / BITS_PER_BLOCK;
        let bit_idx = idx % BITS_PER_BLOCK;
        let mask = 1u64 << bit_idx;
        
        (self.blocks[block_idx].load(Ordering::Acquire) & mask) != 0
    }
    
    /// Find and atomically claim a ready GVThread
    ///
    /// Uses random starting block for fairness.
    /// Returns None if no ready GVThreads.
    pub fn find_and_claim(&self, start_hint: usize) -> Option<GVThreadId> {
        let start_block = start_hint % self.num_blocks;
        
        // Scan from start_block, wrapping around
        for i in 0..self.num_blocks {
            let block_idx = (start_block + i) % self.num_blocks;
            
            // Quick check if block has any bits set
            let block_val = self.blocks[block_idx].load(Ordering::Acquire);
            if block_val == 0 {
                continue;
            }
            
            // Try to claim a bit from this block
            if let Some(bit_idx) = self.try_claim_from_block(block_idx) {
                let gvthread_idx = block_idx * BITS_PER_BLOCK + bit_idx;
                return Some(GVThreadId::new(gvthread_idx as u32));
            }
        }
        
        None
    }
    
    /// Try to atomically claim a bit from a specific block
    fn try_claim_from_block(&self, block_idx: usize) -> Option<usize> {
        loop {
            let current = self.blocks[block_idx].load(Ordering::Acquire);
            if current == 0 {
                return None;
            }
            
            // Find lowest set bit
            let bit_idx = current.trailing_zeros() as usize;
            let mask = 1u64 << bit_idx;
            
            // Try to clear it atomically
            match self.blocks[block_idx].compare_exchange_weak(
                current,
                current & !mask,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(bit_idx),
                Err(_) => continue, // Another thread modified, retry
            }
        }
    }
    
    /// Check if any GVThread is ready
    pub fn any_ready(&self) -> bool {
        for block in self.blocks.iter() {
            if block.load(Ordering::Relaxed) != 0 {
                return true;
            }
        }
        false
    }
    
    /// Count ready GVThreads (for debugging/stats)
    pub fn count_ready(&self) -> usize {
        let mut count = 0;
        for block in self.blocks.iter() {
            count += block.load(Ordering::Relaxed).count_ones() as usize;
        }
        count
    }
}

/// Collection of bitmaps for all priority levels
pub struct ReadyBitmaps {
    /// One bitmap per priority level
    bitmaps: [ReadyBitmap; Priority::COUNT],
    
    /// Per-worker random state for fair block selection
    worker_rng: SpinLock<Vec<u64>>,
}

impl ReadyBitmaps {
    /// Create new ready bitmaps for the given max GVThread count
    pub fn new(max_gvthreads: usize, num_workers: usize) -> Self {
        Self {
            bitmaps: [
                ReadyBitmap::new(max_gvthreads), // Critical
                ReadyBitmap::new(max_gvthreads), // High
                ReadyBitmap::new(max_gvthreads), // Normal
                ReadyBitmap::new(max_gvthreads), // Low
            ],
            worker_rng: SpinLock::new(vec![12345u64; num_workers]),
        }
    }
    
    /// Mark a GVThread as ready at the given priority
    #[inline]
    pub fn set_ready(&self, id: GVThreadId, priority: Priority) {
        self.bitmaps[priority.as_index()].set(id);
    }
    
    /// Clear a GVThread from ready (all priorities)
    #[inline]
    pub fn clear_ready(&self, id: GVThreadId, priority: Priority) {
        self.bitmaps[priority.as_index()].clear(id);
    }
    
    /// Find and claim a ready GVThread
    ///
    /// Checks priorities in order: Critical -> High -> Normal -> Low
    /// Uses per-worker random state for fairness within each priority.
    ///
    /// If `low_priority_only` is true, only checks LOW priority bitmap.
    pub fn find_and_claim(&self, worker_id: usize, low_priority_only: bool) -> Option<(GVThreadId, Priority)> {
        // Get and update worker's random state
        let start_hint = {
            let mut rng = self.worker_rng.lock();
            if worker_id < rng.len() {
                // Simple xorshift for fast random
                let mut x = rng[worker_id];
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                rng[worker_id] = x;
                x as usize
            } else {
                0
            }
        };
        
        if low_priority_only {
            // Dedicated LOW priority worker
            if let Some(id) = self.bitmaps[Priority::Low.as_index()].find_and_claim(start_hint) {
                return Some((id, Priority::Low));
            }
        } else {
            // Normal worker: check all priorities in order
            for priority in Priority::iter() {
                if let Some(id) = self.bitmaps[priority.as_index()].find_and_claim(start_hint) {
                    return Some((id, priority));
                }
            }
        }
        
        None
    }
    
    /// Check if any GVThread is ready at any priority
    pub fn any_ready(&self) -> bool {
        for bitmap in &self.bitmaps {
            if bitmap.any_ready() {
                return true;
            }
        }
        false
    }
    
    /// Check if any GVThread is ready at specific priority
    pub fn any_ready_at(&self, priority: Priority) -> bool {
        self.bitmaps[priority.as_index()].any_ready()
    }
    
    /// Get total ready count across all priorities
    pub fn total_ready(&self) -> usize {
        self.bitmaps.iter().map(|b| b.count_ready()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bitmap_set_clear() {
        let bitmap = ReadyBitmap::new(1000);
        let id = GVThreadId::new(42);
        
        assert!(!bitmap.is_set(id));
        
        bitmap.set(id);
        assert!(bitmap.is_set(id));
        
        bitmap.clear(id);
        assert!(!bitmap.is_set(id));
    }
    
    #[test]
    fn test_bitmap_find_and_claim() {
        let bitmap = ReadyBitmap::new(1000);
        
        bitmap.set(GVThreadId::new(10));
        bitmap.set(GVThreadId::new(20));
        bitmap.set(GVThreadId::new(30));
        
        assert_eq!(bitmap.count_ready(), 3);
        
        let claimed1 = bitmap.find_and_claim(0);
        assert!(claimed1.is_some());
        assert_eq!(bitmap.count_ready(), 2);
        
        let claimed2 = bitmap.find_and_claim(0);
        assert!(claimed2.is_some());
        assert_eq!(bitmap.count_ready(), 1);
        
        let claimed3 = bitmap.find_and_claim(0);
        assert!(claimed3.is_some());
        assert_eq!(bitmap.count_ready(), 0);
        
        let claimed4 = bitmap.find_and_claim(0);
        assert!(claimed4.is_none());
    }
    
    #[test]
    fn test_ready_bitmaps_priority() {
        let bitmaps = ReadyBitmaps::new(1000, 4);
        
        // Add GVThreads at different priorities
        bitmaps.set_ready(GVThreadId::new(1), Priority::Low);
        bitmaps.set_ready(GVThreadId::new(2), Priority::Normal);
        bitmaps.set_ready(GVThreadId::new(3), Priority::High);
        bitmaps.set_ready(GVThreadId::new(4), Priority::Critical);
        
        // Should get Critical first
        let (id1, p1) = bitmaps.find_and_claim(0, false).unwrap();
        assert_eq!(id1, GVThreadId::new(4));
        assert_eq!(p1, Priority::Critical);
        
        // Then High
        let (id2, p2) = bitmaps.find_and_claim(0, false).unwrap();
        assert_eq!(id2, GVThreadId::new(3));
        assert_eq!(p2, Priority::High);
        
        // Then Normal
        let (id3, p3) = bitmaps.find_and_claim(0, false).unwrap();
        assert_eq!(id3, GVThreadId::new(2));
        assert_eq!(p3, Priority::Normal);
        
        // Then Low
        let (id4, p4) = bitmaps.find_and_claim(0, false).unwrap();
        assert_eq!(id4, GVThreadId::new(1));
        assert_eq!(p4, Priority::Low);
        
        // No more
        assert!(bitmaps.find_and_claim(0, false).is_none());
    }
    
    #[test]
    fn test_concurrent_claim() {
        use std::sync::Arc;
        use std::thread;
        
        let bitmap = Arc::new(ReadyBitmap::new(10000));
        
        // Set many bits
        for i in 0..1000 {
            bitmap.set(GVThreadId::new(i));
        }
        
        // Spawn threads to claim concurrently
        let mut handles = vec![];
        for t in 0..4 {
            let bitmap = Arc::clone(&bitmap);
            handles.push(thread::spawn(move || {
                let mut claimed = vec![];
                loop {
                    match bitmap.find_and_claim(t * 100) {
                        Some(id) => claimed.push(id),
                        None => break,
                    }
                }
                claimed
            }));
        }
        
        // Collect results
        let mut all_claimed: Vec<GVThreadId> = vec![];
        for h in handles {
            all_claimed.extend(h.join().unwrap());
        }
        
        // Should have claimed all 1000, no duplicates
        assert_eq!(all_claimed.len(), 1000);
        all_claimed.sort();
        all_claimed.dedup();
        assert_eq!(all_claimed.len(), 1000);
    }
}
