//! GVThread identifier type

use core::fmt;

/// Unique identifier for a GVThread
///
/// This is a 32-bit value that indexes into the slot array.
/// The maximum value (u32::MAX) is reserved as a sentinel for "no GVThread".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct GVThreadId(u32);

impl GVThreadId {
    /// Sentinel value indicating no GVThread
    pub const NONE: GVThreadId = GVThreadId(u32::MAX);
    
    /// Create a new GVThreadId from a raw value
    #[inline]
    pub const fn new(id: u32) -> Self {
        GVThreadId(id)
    }
    
    /// Get the raw u32 value
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    
    /// Get as usize for indexing
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
    
    /// Check if this is the NONE sentinel
    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 == u32::MAX
    }
    
    /// Check if this is a valid GVThread ID
    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }
    
    /// Convert to Option
    #[inline]
    pub const fn to_option(self) -> Option<GVThreadId> {
        if self.is_none() {
            None
        } else {
            Some(self)
        }
    }
}

impl From<u32> for GVThreadId {
    #[inline]
    fn from(id: u32) -> Self {
        GVThreadId(id)
    }
}

impl From<GVThreadId> for u32 {
    #[inline]
    fn from(id: GVThreadId) -> Self {
        id.0
    }
}

impl fmt::Debug for GVThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            write!(f, "GVThreadId(NONE)")
        } else {
            write!(f, "GVThreadId({})", self.0)
        }
    }
}

impl fmt::Display for GVThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            write!(f, "none")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

impl Default for GVThreadId {
    fn default() -> Self {
        GVThreadId::NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_gvthread_id_basics() {
        let id = GVThreadId::new(42);
        assert_eq!(id.as_u32(), 42);
        assert_eq!(id.as_usize(), 42);
        assert!(!id.is_none());
        assert!(id.is_some());
    }
    
    #[test]
    fn test_gvthread_id_none() {
        let none = GVThreadId::NONE;
        assert!(none.is_none());
        assert!(!none.is_some());
        assert_eq!(none.to_option(), None);
    }
    
    #[test]
    fn test_gvthread_id_conversions() {
        let id: GVThreadId = 100u32.into();
        let raw: u32 = id.into();
        assert_eq!(raw, 100);
    }
}
