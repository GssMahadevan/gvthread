//! GVThread and Worker metadata structures
//!
//! These structures have fixed layouts (repr(C)) for direct memory access
//! from assembly code and signal handlers.

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, AtomicBool, Ordering};
use crate::id::GVThreadId;
use crate::state::{GVThreadState, Priority};
use crate::constants::{CACHE_LINE_SIZE, GVTHREAD_NONE};

/// Size of WorkerState (cache-line aligned = 64 bytes)
pub const WORKER_STATE_SIZE: usize = CACHE_LINE_SIZE;

/// Size of voluntary saved registers (callee-saved only)
pub const VOLUNTARY_SAVE_SIZE: usize = 64;

/// Size of forced saved registers (all registers for SIGURG)
pub const FORCED_SAVE_SIZE: usize = 256;

/// GVThread metadata at the start of each slot
///
/// Layout (offsets are stable for ASM access):
/// ```text
/// 0x00: preempt_flag    (u8)  - Set by timer, checked at safepoints
/// 0x01: cancelled       (u8)  - Cancellation flag
/// 0x02: state           (u8)  - GVThreadState
/// 0x03: priority        (u8)  - Priority level
/// 0x04: gvthread_id     (u32) - Self ID
/// 0x08: parent_id       (u32) - Parent GVThread ID
/// 0x0C: worker_id       (u32) - Current/last worker ID
/// 0x10: entry_fn        (u64) - Entry function pointer
/// 0x18: entry_arg       (u64) - Entry function argument
/// 0x20: result_ptr      (u64) - Pointer to result storage
/// 0x28: reserved        (24 bytes)
/// 0x40: voluntary_regs  (64 bytes)  - Callee-saved registers
/// 0x80: forced_regs     (256 bytes) - All registers (SIGURG)
/// ```
#[repr(C, align(64))]
pub struct GVThreadMetadata {
    // Flags (offset 0x00-0x03)
    pub preempt_flag: AtomicU8,
    pub cancelled: AtomicU8,
    pub state: AtomicU8,
    pub priority: AtomicU8,
    
    // IDs (offset 0x04-0x0F)
    pub gvthread_id: AtomicU32,
    pub parent_id: AtomicU32,
    pub worker_id: AtomicU32,
    
    // Entry point (offset 0x10-0x27)
    pub entry_fn: AtomicU64,
    pub entry_arg: AtomicU64,
    pub result_ptr: AtomicU64,
    
    // Reserved for future use (offset 0x28-0x3F)
    _reserved: [u8; 24],
    
    // Saved registers for voluntary yield (offset 0x40-0x7F)
    // rsp, rip, rbx, rbp, r12, r13, r14, r15
    pub voluntary_regs: VoluntarySavedRegs,
    
    // Saved registers for forced preemption (offset 0x80-0x17F)
    // All general purpose + flags + FPU state pointer
    pub forced_regs: ForcedSavedRegs,
}

/// Saved registers for voluntary yield (callee-saved per System V AMD64 ABI)
#[repr(C)]
pub struct VoluntarySavedRegs {
    pub rsp: u64,
    pub rip: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

impl Default for VoluntarySavedRegs {
    fn default() -> Self {
        Self {
            rsp: 0,
            rip: 0,
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
        }
    }
}

/// Saved registers for forced preemption (all registers)
#[repr(C)]
pub struct ForcedSavedRegs {
    // General purpose registers
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    
    // Instruction pointer and flags
    pub rip: u64,
    pub rflags: u64,
    
    // Segment registers (usually not needed, but for completeness)
    pub cs: u64,
    pub ss: u64,
    
    // FPU/SSE state pointer (points to separate storage if needed)
    pub fpu_state_ptr: u64,
    
    // Padding to 256 bytes
    _padding: [u64; 11],
}

impl Default for ForcedSavedRegs {
    fn default() -> Self {
        Self {
            rax: 0, rbx: 0, rcx: 0, rdx: 0,
            rsi: 0, rdi: 0, rbp: 0, rsp: 0,
            r8: 0, r9: 0, r10: 0, r11: 0,
            r12: 0, r13: 0, r14: 0, r15: 0,
            rip: 0, rflags: 0,
            cs: 0, ss: 0,
            fpu_state_ptr: 0,
            _padding: [0; 11],
        }
    }
}

impl GVThreadMetadata {
    /// Create default metadata (zeroed)
    pub const fn new() -> Self {
        Self {
            preempt_flag: AtomicU8::new(0),
            cancelled: AtomicU8::new(0),
            state: AtomicU8::new(GVThreadState::Created as u8),
            priority: AtomicU8::new(Priority::Normal as u8),
            gvthread_id: AtomicU32::new(GVTHREAD_NONE),
            parent_id: AtomicU32::new(GVTHREAD_NONE),
            worker_id: AtomicU32::new(GVTHREAD_NONE),
            entry_fn: AtomicU64::new(0),
            entry_arg: AtomicU64::new(0),
            result_ptr: AtomicU64::new(0),
            _reserved: [0; 24],
            voluntary_regs: VoluntarySavedRegs {
                rsp: 0, rip: 0, rbx: 0, rbp: 0,
                r12: 0, r13: 0, r14: 0, r15: 0,
            },
            forced_regs: ForcedSavedRegs {
                rax: 0, rbx: 0, rcx: 0, rdx: 0,
                rsi: 0, rdi: 0, rbp: 0, rsp: 0,
                r8: 0, r9: 0, r10: 0, r11: 0,
                r12: 0, r13: 0, r14: 0, r15: 0,
                rip: 0, rflags: 0, cs: 0, ss: 0,
                fpu_state_ptr: 0,
                _padding: [0; 11],
            },
        }
    }
    
    /// Initialize metadata for a new GVThread
    pub fn init(&self, id: GVThreadId, parent: GVThreadId, priority: Priority) {
        self.preempt_flag.store(0, Ordering::Relaxed);
        self.cancelled.store(0, Ordering::Relaxed);
        self.state.store(GVThreadState::Created as u8, Ordering::Relaxed);
        self.priority.store(priority as u8, Ordering::Relaxed);
        self.gvthread_id.store(id.as_u32(), Ordering::Relaxed);
        self.parent_id.store(parent.as_u32(), Ordering::Relaxed);
        self.worker_id.store(GVTHREAD_NONE, Ordering::Relaxed);
    }
    
    // Accessor methods
    
    #[inline]
    pub fn get_state(&self) -> GVThreadState {
        GVThreadState::from(self.state.load(Ordering::Acquire))
    }
    
    #[inline]
    pub fn set_state(&self, state: GVThreadState) {
        self.state.store(state as u8, Ordering::Release);
    }
    
    #[inline]
    pub fn get_priority(&self) -> Priority {
        Priority::from(self.priority.load(Ordering::Relaxed))
    }
    
    #[inline]
    pub fn is_preempt_requested(&self) -> bool {
        self.preempt_flag.load(Ordering::Acquire) != 0
    }
    
    #[inline]
    pub fn request_preempt(&self) {
        self.preempt_flag.store(1, Ordering::Release);
    }
    
    #[inline]
    pub fn clear_preempt(&self) {
        self.preempt_flag.store(0, Ordering::Relaxed);
    }
    
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) != 0
    }
    
    #[inline]
    pub fn request_cancel(&self) {
        self.cancelled.store(1, Ordering::Release);
    }
    
    #[inline]
    pub fn get_id(&self) -> GVThreadId {
        GVThreadId::new(self.gvthread_id.load(Ordering::Relaxed))
    }
}

/// Worker state - stored in contiguous array for cache efficiency
///
/// Each worker has its own cache line to avoid false sharing.
/// Timer thread scans this array to detect stuck GVThreads.
///
/// Layout (64 bytes, cache-line aligned):
/// ```text
/// 0x00: current_gthread   (u32) - Currently running GVThread ID
/// 0x04: activity_counter  (u32) - Incremented at safepoints
/// 0x08: run_start_ns      (u64) - When current GVThread started
/// 0x10: last_activity_ns  (u64) - Last safepoint/yield time
/// 0x18: thread_id         (u64) - pthread_t / OS thread ID
/// 0x20: is_parked         (u8)  - Worker is parked (no work)
/// 0x21: is_low_priority   (u8)  - Dedicated LOW priority worker
/// 0x22: worker_index      (u8)  - Index in worker array
/// 0x23: padding           (29 bytes)
/// ```
#[repr(C, align(64))]
pub struct WorkerState {
    /// Currently running GVThread (GVTHREAD_NONE if idle)
    pub current_gthread: AtomicU32,
    
    /// Activity counter - incremented at safepoints
    /// Timer compares to detect stuck GVThreads
    pub activity_counter: AtomicU32,
    
    /// Timestamp when current GVThread started running
    pub run_start_ns: AtomicU64,
    
    /// Timestamp of last activity (safepoint/yield)
    pub last_activity_ns: AtomicU64,
    
    /// OS thread ID (pthread_t on Unix)
    pub thread_id: AtomicU64,
    
    /// Worker is parked (waiting for work)
    pub is_parked: AtomicBool,
    
    /// This worker only runs LOW priority GVThreads
    pub is_low_priority: AtomicBool,
    
    /// Index in the worker array
    pub worker_index: AtomicU8,
    
    /// Padding to fill cache line
    _padding: [u8; 29],
}

impl WorkerState {
    /// Create a new worker state
    pub const fn new() -> Self {
        Self {
            current_gthread: AtomicU32::new(GVTHREAD_NONE),
            activity_counter: AtomicU32::new(0),
            run_start_ns: AtomicU64::new(0),
            last_activity_ns: AtomicU64::new(0),
            thread_id: AtomicU64::new(0),
            is_parked: AtomicBool::new(true),
            is_low_priority: AtomicBool::new(false),
            worker_index: AtomicU8::new(0),
            _padding: [0; 29],
        }
    }
    
    /// Initialize worker state
    pub fn init(&self, index: u8, is_low_priority: bool) {
        self.current_gthread.store(GVTHREAD_NONE, Ordering::Relaxed);
        self.activity_counter.store(0, Ordering::Relaxed);
        self.run_start_ns.store(0, Ordering::Relaxed);
        self.last_activity_ns.store(0, Ordering::Relaxed);
        self.is_parked.store(true, Ordering::Relaxed);
        self.is_low_priority.store(is_low_priority, Ordering::Relaxed);
        self.worker_index.store(index, Ordering::Relaxed);
    }
    
    /// Record that a GVThread has started running
    #[inline]
    pub fn start_running(&self, gvthread_id: GVThreadId, now_ns: u64) {
        self.activity_counter.store(0, Ordering::Relaxed);
        self.run_start_ns.store(now_ns, Ordering::Relaxed);
        self.last_activity_ns.store(now_ns, Ordering::Relaxed);
        self.current_gthread.store(gvthread_id.as_u32(), Ordering::Release);
        self.is_parked.store(false, Ordering::Relaxed);
    }
    
    /// Record that a GVThread has stopped running
    #[inline]
    pub fn stop_running(&self) {
        self.current_gthread.store(GVTHREAD_NONE, Ordering::Release);
    }
    
    /// Bump activity counter and update last activity time
    #[inline]
    pub fn record_activity(&self, now_ns: u64) {
        self.activity_counter.fetch_add(1, Ordering::Relaxed);
        self.last_activity_ns.store(now_ns, Ordering::Relaxed);
    }
    
    /// Get current GVThread ID
    #[inline]
    pub fn get_current_gthread(&self) -> GVThreadId {
        GVThreadId::new(self.current_gthread.load(Ordering::Acquire))
    }
    
    /// Check if worker is idle
    #[inline]
    pub fn is_idle(&self) -> bool {
        self.current_gthread.load(Ordering::Relaxed) == GVTHREAD_NONE
    }
}

// Verify sizes at compile time
const _: () = {
    assert!(core::mem::size_of::<WorkerState>() == 64);
    assert!(core::mem::align_of::<WorkerState>() == 64);
    assert!(core::mem::size_of::<VoluntarySavedRegs>() == 64);
    assert!(core::mem::size_of::<ForcedSavedRegs>() == 256);
};

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_worker_state_size() {
        assert_eq!(core::mem::size_of::<WorkerState>(), 64);
        assert_eq!(core::mem::align_of::<WorkerState>(), 64);
    }
    
    #[test]
    fn test_gvthread_metadata_offsets() {
        let meta = GVThreadMetadata::new();
        let base = &meta as *const _ as usize;
        
        // Verify critical offsets for ASM access
        assert_eq!(&meta.preempt_flag as *const _ as usize - base, 0x00);
        assert_eq!(&meta.cancelled as *const _ as usize - base, 0x01);
        assert_eq!(&meta.state as *const _ as usize - base, 0x02);
        assert_eq!(&meta.priority as *const _ as usize - base, 0x03);
        assert_eq!(&meta.gvthread_id as *const _ as usize - base, 0x04);
    }
    
    #[test]
    fn test_worker_state_operations() {
        let worker = WorkerState::new();
        worker.init(0, false);
        
        assert!(worker.is_idle());
        
        let id = GVThreadId::new(42);
        worker.start_running(id, 1000);
        
        assert!(!worker.is_idle());
        assert_eq!(worker.get_current_gthread(), id);
        
        worker.record_activity(2000);
        assert_eq!(worker.activity_counter.load(Ordering::Relaxed), 1);
        
        worker.stop_running();
        assert!(worker.is_idle());
    }
}
