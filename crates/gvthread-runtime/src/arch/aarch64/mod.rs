//! aarch64 context switching implementation
//!
//! TODO: Implement for ARM64 (macOS Apple Silicon, Linux ARM, etc.)

use gvthread_core::metadata::{VoluntarySavedRegs, ForcedSavedRegs};

/// Initialize a new GVThread's context
pub unsafe fn init_context(
    _regs: *mut VoluntarySavedRegs,
    _stack_top: *mut u8,
    _entry_fn: usize,
    _entry_arg: usize,
) {
    todo!("aarch64 init_context not yet implemented")
}

/// Perform a voluntary context switch
pub unsafe extern "C" fn context_switch_voluntary(
    _old_regs: *mut VoluntarySavedRegs,
    _new_regs: *const VoluntarySavedRegs,
) {
    todo!("aarch64 context_switch_voluntary not yet implemented")
}

/// Restore from forced preemption
pub unsafe extern "C" fn context_restore_forced(_regs: *const ForcedSavedRegs) {
    todo!("aarch64 context_restore_forced not yet implemented")
}
