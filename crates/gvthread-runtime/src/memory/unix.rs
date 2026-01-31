//! Unix memory implementation using mmap

use super::MemoryRegion;
use gvthread_core::constants::{SLOT_SIZE, METADATA_SIZE, GUARD_SIZE};
use gvthread_core::error::{MemoryError, SchedResult};
use std::sync::atomic::Ordering;

/// Hint for region start address (high address to avoid conflicts)
const REGION_START_HINT: usize = 0x7000_0000_0000;

impl MemoryRegion {
    /// Initialize the memory region
    ///
    /// Reserves virtual address space for `max_slots` GVThread slots.
    /// Memory is reserved with PROT_NONE (no access) initially.
    pub fn init(&mut self, max_slots: usize) -> SchedResult<()> {
        if self.initialized.load(Ordering::SeqCst) {
            return Err(MemoryError::AlreadyInitialized.into());
        }
        
        let total_size = max_slots.checked_mul(SLOT_SIZE)
            .ok_or(MemoryError::TooManySlots)?;
        
        // Reserve virtual address space with PROT_NONE
        let base = unsafe {
            libc::mmap(
                REGION_START_HINT as *mut libc::c_void,
                total_size,
                libc::PROT_NONE,  // No access initially
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        
        if base == libc::MAP_FAILED {
            return Err(MemoryError::AllocationFailed.into());
        }
        
        self.base.store(base as *mut u8, Ordering::Release);
        self.total_size = total_size;
        self.max_slots = max_slots;
        self.initialized.store(true, Ordering::SeqCst);
        
        Ok(())
    }
    
    /// Activate a slot (make it readable/writable)
    ///
    /// Called when allocating a new GVThread.
    pub fn activate_slot(&self, slot_id: u32) -> SchedResult<()> {
        if !self.is_initialized() {
            return Err(MemoryError::AllocationFailed.into());
        }
        
        if slot_id as usize >= self.max_slots {
            return Err(MemoryError::InvalidSlot.into());
        }
        
        let base = self.slot_base(slot_id);
        
        // Make metadata region accessible
        let ret = unsafe {
            libc::mprotect(
                base as *mut libc::c_void,
                METADATA_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
            )
        };
        if ret != 0 {
            return Err(MemoryError::ProtectionFailed.into());
        }
        
        // Make stack region accessible (between metadata and guard)
        let stack_base = unsafe { base.add(METADATA_SIZE) };
        let stack_size = SLOT_SIZE - METADATA_SIZE - GUARD_SIZE;
        let ret = unsafe {
            libc::mprotect(
                stack_base as *mut libc::c_void,
                stack_size,
                libc::PROT_READ | libc::PROT_WRITE,
            )
        };
        if ret != 0 {
            return Err(MemoryError::ProtectionFailed.into());
        }
        
        // Guard page at the end remains PROT_NONE (from initial mmap)
        // This will cause SIGSEGV on stack overflow
        
        Ok(())
    }
    
    /// Deactivate a slot (release physical memory)
    ///
    /// Called when a GVThread is finished and its slot is being recycled.
    pub fn deactivate_slot(&self, slot_id: u32) -> SchedResult<()> {
        if !self.is_initialized() {
            return Err(MemoryError::AllocationFailed.into());
        }
        
        if slot_id as usize >= self.max_slots {
            return Err(MemoryError::InvalidSlot.into());
        }
        
        let base = self.slot_base(slot_id);
        let usable_size = SLOT_SIZE - GUARD_SIZE;
        
        // Tell kernel we don't need the physical pages
        let ret = unsafe {
            libc::madvise(
                base as *mut libc::c_void,
                usable_size,
                libc::MADV_DONTNEED,
            )
        };
        if ret != 0 {
            return Err(MemoryError::AdviseFailed.into());
        }
        
        Ok(())
    }
    
    /// Release the entire memory region
    pub fn release(&mut self) -> SchedResult<()> {
        if !self.is_initialized() {
            return Ok(());
        }
        
        let base = self.base();
        if !base.is_null() {
            let ret = unsafe {
                libc::munmap(base as *mut libc::c_void, self.total_size)
            };
            if ret != 0 {
                return Err(MemoryError::AllocationFailed.into());
            }
        }
        
        self.base.store(std::ptr::null_mut(), Ordering::Release);
        self.total_size = 0;
        self.max_slots = 0;
        self.initialized.store(false, Ordering::SeqCst);
        
        Ok(())
    }
}

/// Initialize the global memory region
pub fn init_memory_region(max_slots: usize) -> SchedResult<()> {
    unsafe {
        super::memory_region_mut().init(max_slots)
    }
}

/// Get a pointer to GVThread metadata
#[inline]
pub fn get_metadata_ptr(slot_id: u32) -> *mut gvthread_core::metadata::GVThreadMetadata {
    super::memory_region().metadata_addr(slot_id) as *mut _
}

/// Get a pointer to stack top for a slot
#[inline]
pub fn get_stack_top(slot_id: u32) -> *mut u8 {
    super::memory_region().stack_top(slot_id)
}
