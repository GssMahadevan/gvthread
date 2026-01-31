//! Cancellation token for cooperative cancellation
//!
//! GVThreads can check for cancellation via their token and exit gracefully.
//! Tokens can be linked to form parent-child relationships.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use crate::error::{SchedError, SchedResult};
use crate::metadata::GVThreadMetadata;

/// Token for checking and triggering cancellation
///
/// Each GVThread receives a cancellation token. The token can be checked
/// at any point to see if cancellation was requested. When cancelled,
/// operations should return `Err(SchedError::Cancelled)`.
///
/// Tokens can have parents, allowing cancellation to propagate from
/// parent GVThreads to children.
#[derive(Clone)]
pub struct CancellationToken {
    inner: CancellationInner,
}

#[derive(Clone)]
enum CancellationInner {
    /// Heap-allocated token with Arc (for tokens created outside GVThread)
    Owned(Arc<OwnedCancellation>),
    /// Reference to metadata's cancelled field (no allocation)
    Metadata(*const AtomicU8),
    /// Dummy token that never cancels
    Dummy,
}

struct OwnedCancellation {
    /// Cancellation flag
    cancelled: AtomicBool,
    
    /// Parent token (if any)
    parent: Option<CancellationToken>,
}

// Safety: CancellationInner::Metadata points to GVThreadMetadata which is thread-safe
unsafe impl Send for CancellationToken {}
unsafe impl Sync for CancellationToken {}

impl CancellationToken {
    /// Create a new independent cancellation token (heap allocated)
    /// 
    /// WARNING: Do not call from GVThread stack! Use from_metadata instead.
    pub fn new() -> Self {
        Self {
            inner: CancellationInner::Owned(Arc::new(OwnedCancellation {
                cancelled: AtomicBool::new(false),
                parent: None,
            })),
        }
    }
    
    /// Create a token that reads from GVThreadMetadata's cancelled field
    /// 
    /// This does NOT allocate and is safe to call from GVThread stack.
    pub fn from_metadata(meta: &GVThreadMetadata) -> Self {
        Self {
            inner: CancellationInner::Metadata(&meta.cancelled as *const AtomicU8),
        }
    }
    
    /// Create a dummy token that never cancels
    /// 
    /// This does NOT allocate and is safe to call from GVThread stack.
    pub fn dummy() -> Self {
        Self {
            inner: CancellationInner::Dummy,
        }
    }
    
    /// Create a child token linked to this one
    ///
    /// If this token is cancelled, checking the child will also return cancelled.
    /// 
    /// WARNING: This allocates! Do not call from GVThread stack.
    pub fn child(&self) -> Self {
        Self {
            inner: CancellationInner::Owned(Arc::new(OwnedCancellation {
                cancelled: AtomicBool::new(false),
                parent: Some(self.clone()),
            })),
        }
    }
    
    /// Check if cancellation was requested
    ///
    /// Also checks parent tokens recursively.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        match &self.inner {
            CancellationInner::Owned(arc) => {
                // Check own flag first (most common case)
                if arc.cancelled.load(Ordering::Acquire) {
                    return true;
                }
                // Check parent chain
                if let Some(ref parent) = arc.parent {
                    return parent.is_cancelled();
                }
                false
            }
            CancellationInner::Metadata(ptr) => {
                // Read from metadata's cancelled field
                unsafe { (**ptr).load(Ordering::Acquire) != 0 }
            }
            CancellationInner::Dummy => false,
        }
    }
    
    /// Request cancellation
    ///
    /// This only sets this token's flag, not parent's.
    /// Child tokens will see cancellation when they check.
    pub fn cancel(&self) {
        match &self.inner {
            CancellationInner::Owned(arc) => {
                arc.cancelled.store(true, Ordering::Release);
            }
            CancellationInner::Metadata(ptr) => {
                unsafe { (**ptr).store(1, Ordering::Release); }
            }
            CancellationInner::Dummy => {}
        }
    }
    
    /// Check if cancelled and return error if so
    ///
    /// This is the typical usage pattern:
    /// ```ignore
    /// fn my_gvthread(token: &CancellationToken) -> SchedResult<()> {
    ///     loop {
    ///         token.check()?;  // Returns Err(Cancelled) if cancelled
    ///         // ... do work ...\
    ///     }
    /// }
    /// ```
    #[inline]
    pub fn check(&self) -> SchedResult<()> {
        if self.is_cancelled() {
            Err(SchedError::Cancelled)
        } else {
            Ok(())
        }
    }
    
    /// Check cancellation and also bump activity counter
    ///
    /// Combines cancellation check with safepoint activity tracking.
    /// This is the preferred method in hot loops.
    #[inline]
    pub fn check_and_yield(&self) -> SchedResult<()> {
        // TODO: Also bump worker activity counter and check preempt flag
        self.check()
    }
    
    /// Reset cancellation (for token reuse)
    ///
    /// Warning: This does not affect child tokens or parent tokens.
    pub fn reset(&self) {
        match &self.inner {
            CancellationInner::Owned(arc) => {
                arc.cancelled.store(false, Ordering::Release);
            }
            CancellationInner::Metadata(ptr) => {
                unsafe { (**ptr).store(0, Ordering::Release); }
            }
            CancellationInner::Dummy => {}
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_cancellation() {
        let token = CancellationToken::new();
        
        assert!(!token.is_cancelled());
        assert!(token.check().is_ok());
        
        token.cancel();
        
        assert!(token.is_cancelled());
        assert!(matches!(token.check(), Err(SchedError::Cancelled)));
    }
    
    #[test]
    fn test_child_token() {
        let parent = CancellationToken::new();
        let child = parent.child();
        
        assert!(!child.is_cancelled());
        
        // Cancelling parent affects child
        parent.cancel();
        assert!(child.is_cancelled());
    }
    
    #[test]
    fn test_child_independent_cancel() {
        let parent = CancellationToken::new();
        let child = parent.child();
        
        // Cancelling child does NOT affect parent
        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
    }
    
    #[test]
    fn test_deep_hierarchy() {
        let root = CancellationToken::new();
        let level1 = root.child();
        let level2 = level1.child();
        let level3 = level2.child();
        
        assert!(!level3.is_cancelled());
        
        // Cancel at root propagates all the way down
        root.cancel();
        assert!(level1.is_cancelled());
        assert!(level2.is_cancelled());
        assert!(level3.is_cancelled());
    }
    
    #[test]
    fn test_reset() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
        
        token.reset();
        assert!(!token.is_cancelled());
    }
    
    #[test]
    fn test_clone_shares_state() {
        let token1 = CancellationToken::new();
        let token2 = token1.clone();
        
        token1.cancel();
        assert!(token2.is_cancelled());
    }
    
    #[test]
    fn test_dummy_token() {
        let token = CancellationToken::dummy();
        assert!(!token.is_cancelled());
        token.cancel(); // Should be no-op
        assert!(!token.is_cancelled());
    }
}