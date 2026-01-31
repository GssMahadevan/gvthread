//! Thread-local storage for GVThread context
//!
//! Provides fast access to current worker and GVThread state.

use gvthread_core::id::GVThreadId;
use gvthread_core::constants::GVTHREAD_NONE;
use std::cell::Cell;

thread_local! {
    /// Current worker ID for this OS thread
    static WORKER_ID: Cell<usize> = const { Cell::new(usize::MAX) };
    
    /// Current GVThread ID running on this worker
    static CURRENT_GVTHREAD: Cell<u32> = const { Cell::new(GVTHREAD_NONE) };
    
    /// Base address of current GVThread's metadata
    static GVTHREAD_BASE: Cell<*mut u8> = const { Cell::new(std::ptr::null_mut()) };
}

/// Set the current worker ID
#[inline]
pub fn set_worker_id(id: usize) {
    WORKER_ID.with(|cell| cell.set(id));
}

/// Get the current worker ID
#[inline]
pub fn worker_id() -> usize {
    WORKER_ID.with(|cell| cell.get())
}

/// Set the current GVThread
#[inline]
pub fn set_current_gvthread(id: GVThreadId, base: *mut u8) {
    CURRENT_GVTHREAD.with(|cell| cell.set(id.as_u32()));
    GVTHREAD_BASE.with(|cell| cell.set(base));
}

/// Clear the current GVThread (worker going idle)
#[inline]
pub fn clear_current_gvthread() {
    CURRENT_GVTHREAD.with(|cell| cell.set(GVTHREAD_NONE));
    GVTHREAD_BASE.with(|cell| cell.set(std::ptr::null_mut()));
}

/// Get the current GVThread ID
#[inline]
pub fn current_gvthread_id() -> GVThreadId {
    GVThreadId::new(CURRENT_GVTHREAD.with(|cell| cell.get()))
}

/// Get the current GVThread's base address
#[inline]
pub fn current_gvthread_base() -> *mut u8 {
    GVTHREAD_BASE.with(|cell| cell.get())
}

/// Check if we're running inside a GVThread
#[inline]
pub fn is_in_gvthread() -> bool {
    CURRENT_GVTHREAD.with(|cell| cell.get() != GVTHREAD_NONE)
}

/// Try to get current worker ID, returns None if not on a worker thread
#[inline]
pub fn try_current_worker_id() -> Option<usize> {
    let id = WORKER_ID.with(|cell| cell.get());
    if id == usize::MAX {
        None
    } else {
        Some(id)
    }
}