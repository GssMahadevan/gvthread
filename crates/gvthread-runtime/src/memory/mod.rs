//! Memory region management for GVThread slots
//!
//! Platform-specific implementations handle virtual memory allocation.

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix;
        pub use unix::*;
    } else if #[cfg(windows)] {
        mod windows;
        pub use windows::*;
    }
}

use gvthread_core::constants::{SLOT_SIZE, METADATA_SIZE, GUARD_SIZE};

use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::ptr;

/// Memory region for all GVThread slots
pub struct MemoryRegion {
    /// Base address of the region
    base: AtomicPtr<u8>,
    
    /// Total size of the region
    total_size: usize,
    
    /// Number of slots
    max_slots: usize,
    
    /// Whether region is initialized
    initialized: AtomicBool,
}

impl MemoryRegion {
    /// Create a new uninitialized memory region
    pub const fn new() -> Self {
        Self {
            base: AtomicPtr::new(ptr::null_mut()),
            total_size: 0,
            max_slots: 0,
            initialized: AtomicBool::new(false),
        }
    }
    
    /// Check if region is initialized
    #[inline]
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }
    
    /// Get base address
    #[inline]
    pub fn base(&self) -> *mut u8 {
        self.base.load(Ordering::Acquire)
    }
    
    /// Get maximum number of slots
    #[inline]
    pub fn max_slots(&self) -> usize {
        self.max_slots
    }
    
    /// Calculate the base address of a slot
    #[inline]
    pub fn slot_base(&self, slot_id: u32) -> *mut u8 {
        debug_assert!((slot_id as usize) < self.max_slots);
        unsafe { self.base().add(slot_id as usize * SLOT_SIZE) }
    }
    
    /// Calculate the metadata address for a slot
    #[inline]
    pub fn metadata_addr(&self, slot_id: u32) -> *mut u8 {
        self.slot_base(slot_id)
    }
    
    /// Calculate the stack top address for a slot (stack grows down)
    #[inline]
    pub fn stack_top(&self, slot_id: u32) -> *mut u8 {
        unsafe {
            self.slot_base(slot_id)
                .add(SLOT_SIZE)
                .sub(GUARD_SIZE)
        }
    }
    
    /// Calculate the stack bottom address for a slot
    #[inline]
    pub fn stack_bottom(&self, slot_id: u32) -> *mut u8 {
        unsafe {
            self.slot_base(slot_id)
                .add(METADATA_SIZE)
        }
    }
}

// Global memory region instance
static mut MEMORY_REGION: MemoryRegion = MemoryRegion::new();

/// Get the global memory region
/// 
/// # Safety
/// 
/// Must be initialized before use via `init_memory_region`.
#[inline]
pub fn memory_region() -> &'static MemoryRegion {
    unsafe { &MEMORY_REGION }
}

/// Get mutable access to the global memory region
/// 
/// # Safety
/// 
/// Only call during initialization, before any GVThreads are running.
#[inline]
pub unsafe fn memory_region_mut() -> &'static mut MemoryRegion {
    &mut MEMORY_REGION
}
