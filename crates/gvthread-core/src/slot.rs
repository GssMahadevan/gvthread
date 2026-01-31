//! Slot allocator for GVThread memory slots
//!
//! Manages allocation and deallocation of fixed-size slots.
//! Uses a LIFO free stack for cache-friendly reuse of recently freed slots.

use core::sync::atomic::{AtomicU32, Ordering};
use crate::id::GVThreadId;
use crate::spinlock::SpinLock;
use crate::error::{SchedError, SchedResult};

/// Slot allocator for GVThread memory management
pub struct SlotAllocator {
    /// LIFO stack of free slot IDs (for reuse)
    free_stack: SpinLock<Vec<u32>>,
    
    /// Next fresh slot ID to allocate (never used before)
    next_fresh: AtomicU32,
    
    /// Maximum number of slots
    max_slots: u32,
    
    /// Number of currently allocated slots
    allocated_count: AtomicU32,
}

impl SlotAllocator {
    /// Create a new slot allocator
    pub fn new(max_slots: usize) -> Self {
        Self {
            // Pre-allocate to max capacity to avoid reallocation from GVThread stack
            free_stack: SpinLock::new(Vec::with_capacity(max_slots)),
            next_fresh: AtomicU32::new(0),
            max_slots: max_slots as u32,
            allocated_count: AtomicU32::new(0),
        }
    }
    
    /// Allocate a slot, returning its ID
    ///
    /// Prefers reusing recently freed slots (LIFO) for better cache behavior.
    /// Falls back to fresh slot IDs if free stack is empty.
    pub fn allocate(&self) -> SchedResult<GVThreadId> {
        // First, try to get a recycled slot from free stack
        {
            let mut free = self.free_stack.lock();
            if let Some(id) = free.pop() {
                self.allocated_count.fetch_add(1, Ordering::Relaxed);
                return Ok(GVThreadId::new(id));
            }
        }
        
        // No recycled slots, allocate fresh
        loop {
            let current = self.next_fresh.load(Ordering::Acquire);
            if current >= self.max_slots {
                return Err(SchedError::NoSlotsAvailable);
            }
            
            // Try to claim this slot
            match self.next_fresh.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.allocated_count.fetch_add(1, Ordering::Relaxed);
                    return Ok(GVThreadId::new(current));
                }
                Err(_) => continue, // Another thread claimed it, retry
            }
        }
    }
    
    /// Release a slot back to the allocator
    ///
    /// The slot will be reused by subsequent allocations.
    pub fn release(&self, id: GVThreadId) {
        if id.is_none() {
            return;
        }
        
        let mut free = self.free_stack.lock();
        free.push(id.as_u32());
        self.allocated_count.fetch_sub(1, Ordering::Relaxed);
    }
    
    /// Release multiple slots at once (for batch cleanup)
    pub fn release_batch(&self, ids: &[GVThreadId]) {
        if ids.is_empty() {
            return;
        }
        
        let mut free = self.free_stack.lock();
        for id in ids {
            if !id.is_none() {
                free.push(id.as_u32());
            }
        }
        self.allocated_count.fetch_sub(ids.len() as u32, Ordering::Relaxed);
    }
    
    /// Get the number of currently allocated slots
    #[inline]
    pub fn allocated_count(&self) -> u32 {
        self.allocated_count.load(Ordering::Relaxed)
    }
    
    /// Get the maximum number of slots
    #[inline]
    pub fn max_slots(&self) -> u32 {
        self.max_slots
    }
    
    /// Get the number of fresh (never-used) slots remaining
    #[inline]
    pub fn fresh_remaining(&self) -> u32 {
        let next = self.next_fresh.load(Ordering::Relaxed);
        self.max_slots.saturating_sub(next)
    }
    
    /// Get the number of slots in the free stack
    pub fn free_stack_size(&self) -> usize {
        self.free_stack.lock().len()
    }
    
    /// Check if a slot ID is valid (within range)
    #[inline]
    pub fn is_valid(&self, id: GVThreadId) -> bool {
        !id.is_none() && id.as_u32() < self.max_slots
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_allocate_sequential() {
        let alloc = SlotAllocator::new(100);
        
        let id1 = alloc.allocate().unwrap();
        let id2 = alloc.allocate().unwrap();
        let id3 = alloc.allocate().unwrap();
        
        assert_eq!(id1.as_u32(), 0);
        assert_eq!(id2.as_u32(), 1);
        assert_eq!(id3.as_u32(), 2);
        assert_eq!(alloc.allocated_count(), 3);
    }
    
    #[test]
    fn test_allocate_release_reuse() {
        let alloc = SlotAllocator::new(100);
        
        let id1 = alloc.allocate().unwrap();
        let id2 = alloc.allocate().unwrap();
        
        assert_eq!(alloc.allocated_count(), 2);
        
        // Release id1
        alloc.release(id1);
        assert_eq!(alloc.allocated_count(), 1);
        
        // Next allocation should reuse id1's slot (LIFO)
        let id3 = alloc.allocate().unwrap();
        assert_eq!(id3, id1);
        assert_eq!(alloc.allocated_count(), 2);
    }
    
    #[test]
    fn test_allocate_exhaustion() {
        let alloc = SlotAllocator::new(3);
        
        let _id1 = alloc.allocate().unwrap();
        let _id2 = alloc.allocate().unwrap();
        let _id3 = alloc.allocate().unwrap();
        
        // Should fail - no slots left
        let result = alloc.allocate();
        assert!(matches!(result, Err(SchedError::NoSlotsAvailable)));
    }
    
    #[test]
    fn test_release_batch() {
        let alloc = SlotAllocator::new(100);
        
        let ids: Vec<_> = (0..10).map(|_| alloc.allocate().unwrap()).collect();
        assert_eq!(alloc.allocated_count(), 10);
        
        alloc.release_batch(&ids);
        assert_eq!(alloc.allocated_count(), 0);
        assert_eq!(alloc.free_stack_size(), 10);
    }
    
    #[test]
    fn test_concurrent_allocation() {
        use std::sync::Arc;
        use std::thread;
        
        let alloc = Arc::new(SlotAllocator::new(10000));
        let mut handles = vec![];
        
        // Spawn threads to allocate concurrently
        for _ in 0..4 {
            let alloc = Arc::clone(&alloc);
            handles.push(thread::spawn(move || {
                let mut ids = vec![];
                for _ in 0..1000 {
                    ids.push(alloc.allocate().unwrap());
                }
                ids
            }));
        }
        
        // Collect all allocated IDs
        let mut all_ids: Vec<GVThreadId> = vec![];
        for h in handles {
            all_ids.extend(h.join().unwrap());
        }
        
        // Should have 4000 unique IDs
        assert_eq!(all_ids.len(), 4000);
        all_ids.sort();
        all_ids.dedup();
        assert_eq!(all_ids.len(), 4000);
    }
}