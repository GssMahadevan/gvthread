//! x86_64 context switching implementation
//!
//! Uses inline assembly for context switch.
//! Now stable in Rust 1.88+

use gvthread_core::metadata::{VoluntarySavedRegs, ForcedSavedRegs};
use std::arch::naked_asm;

/// Initialize a new GVThread's context
///
/// Sets up the stack so that when switched to, execution begins at entry_fn.
///
/// # Safety
///
/// `regs` must point to valid VoluntarySavedRegs memory.
/// `stack_top` must be a valid stack pointer (16-byte aligned).
#[inline]
pub unsafe fn init_context(
    regs: *mut VoluntarySavedRegs,
    stack_top: *mut u8,
    entry_fn: usize,
    entry_arg: usize,
) {
    // Stack must be 16-byte aligned per System V AMD64 ABI
    let sp = stack_top as usize;
    
    // Align stack to 16 bytes, then subtract 8 for the "call" alignment
    let aligned_sp = (sp & !0xF) - 8;
    
    // Set up initial register state
    let regs = &mut *regs;
    regs.rsp = aligned_sp as u64;
    regs.rip = gvthread_entry_trampoline as usize as u64;
    regs.rbx = 0;
    regs.rbp = 0;
    regs.r12 = entry_fn as u64;    // Entry function
    regs.r13 = entry_arg as u64;   // Entry argument
    regs.r14 = 0;
    regs.r15 = 0;
}

/// Trampoline that calls the entry function with its argument
#[unsafe(naked)]
pub unsafe extern "C" fn gvthread_entry_trampoline() {
    naked_asm!(
        "mov rdi, r13",
        "call r12",
        "call {cleanup}",
        "ud2",
        cleanup = sym gvthread_finished,
    );
}

/// Perform a voluntary context switch
///
/// Saves callee-saved registers to `old_regs` and loads from `new_regs`.
#[unsafe(naked)]
pub unsafe extern "C" fn context_switch_voluntary(
    _old_regs: *mut VoluntarySavedRegs,
    _new_regs: *const VoluntarySavedRegs,
) {
    naked_asm!(
        // Save callee-saved registers to old_regs (RDI)
        "mov [rdi + 0x00], rsp",
        "lea rax, [rip + 1f]",
        "mov [rdi + 0x08], rax",
        "mov [rdi + 0x10], rbx",
        "mov [rdi + 0x18], rbp",
        "mov [rdi + 0x20], r12",
        "mov [rdi + 0x28], r13",
        "mov [rdi + 0x30], r14",
        "mov [rdi + 0x38], r15",
        // Load callee-saved registers from new_regs (RSI)
        "mov rsp, [rsi + 0x00]",
        "mov rax, [rsi + 0x08]",
        "mov rbx, [rsi + 0x10]",
        "mov rbp, [rsi + 0x18]",
        "mov r12, [rsi + 0x20]",
        "mov r13, [rsi + 0x28]",
        "mov r14, [rsi + 0x30]",
        "mov r15, [rsi + 0x38]",
        // Jump to new RIP
        "jmp rax",
        // Return point for saved context
        "1:",
        "ret",
    );
}

/// Restore from forced preemption (all registers)
#[unsafe(naked)]
pub unsafe extern "C" fn context_restore_forced(_regs: *const ForcedSavedRegs) {
    naked_asm!(
        // RDI contains pointer to ForcedSavedRegs
        "mov rax, [rdi + 0x00]",
        "mov rbx, [rdi + 0x08]",
        "mov rcx, [rdi + 0x10]",
        "mov rdx, [rdi + 0x18]",
        "mov rsi, [rdi + 0x20]",
        "mov rbp, [rdi + 0x30]",
        "mov rsp, [rdi + 0x38]",
        "mov r8,  [rdi + 0x40]",
        "mov r9,  [rdi + 0x48]",
        "mov r10, [rdi + 0x50]",
        "mov r11, [rdi + 0x58]",
        "mov r12, [rdi + 0x60]",
        "mov r13, [rdi + 0x68]",
        "mov r14, [rdi + 0x70]",
        "mov r15, [rdi + 0x78]",
        // Push RIP and RFLAGS for return
        "push qword ptr [rdi + 0x80]",
        "push qword ptr [rdi + 0x88]",
        // Now restore RDI
        "mov rdi, [rdi + 0x28]",
        // Restore flags and return
        "popfq",
        "ret",
    );
}

/// Called when a GVThread's entry function returns
/// 
/// This is called from the trampoline after the user's closure completes.
/// We mark the GVThread as Finished and switch back to the scheduler.
extern "C" fn gvthread_finished() {
    use crate::tls;
    use crate::scheduler::get_worker_sched_context;
    use crate::worker::current_worker_id;
    use gvthread_core::state::GVThreadState;
    use gvthread_core::metadata::GVThreadMetadata;
    
    // Get current GVThread info
    let meta_base = tls::current_gvthread_base();
    let worker_id = current_worker_id();
    
    if meta_base.is_null() {
        // Something went wrong - spin forever (will trigger SIGURG eventually)
        loop { std::hint::spin_loop(); }
    }
    
    // Mark as finished
    let meta = unsafe { &*(meta_base as *const GVThreadMetadata) };
    meta.set_state(GVThreadState::Finished);
    
    // Get our saved registers and scheduler context
    let gvthread_regs = unsafe {
        (meta_base).add(0x40) as *mut VoluntarySavedRegs
    };
    let sched_ctx = get_worker_sched_context(worker_id);
    
    // Switch back to scheduler - we will NOT return from this
    // because run_gvthread() will clean us up
    unsafe {
        context_switch_voluntary(gvthread_regs, sched_ctx);
    }
    
    // Should never reach here, but just in case
    unreachable!("gvthread_finished returned after context switch");
}
