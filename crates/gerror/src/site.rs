//! Error site identification for per-call-site metrics.
//!
//! A `SiteId` is a `u64` packed as:
//!
//! ```text
//! ┌────────────────────────┬────────────────────────┐
//! │  counter_index (u32)   │   unique_id (u32)      │
//! │  MSB — array index     │   LSB — global unique   │
//! │  into AtomicU64[]      │   app_id + this = GUID  │
//! └────────────────────────┴────────────────────────┘
//! ```
//!
//! # Reserved Ranges (counter_index)
//!
//! | Range               | Owner                        |
//! |---------------------|------------------------------|
//! | `0`                 | No site (default, unset)     |
//! | `1 — 0xFFFF`        | GVThread-core reserved (64K) |
//! | `0x0001_0000+`      | User application space       |

/// Packed site identifier: counter_index (MSB u32) + unique_id (LSB u32).
///
/// Every distinct error site in the codebase gets its own `SiteId`.
/// Same errno at different call sites → different `SiteId` values →
/// different counters → you know exactly where the error came from.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SiteId(pub u64);

impl SiteId {
    /// No site assigned.
    pub const NONE: SiteId = SiteId(0);

    /// Create a SiteId from counter_index and unique_id.
    ///
    /// ```
    /// use gerror::SiteId;
    /// let site = SiteId::new(42, 1001);
    /// assert_eq!(site.counter_index(), 42);
    /// assert_eq!(site.unique_id(), 1001);
    /// ```
    #[inline]
    pub const fn new(counter_index: u32, unique_id: u32) -> Self {
        Self(((counter_index as u64) << 32) | (unique_id as u64))
    }

    /// Index into the static `AtomicU64` counter array.
    #[inline]
    pub const fn counter_index(&self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// Globally unique identifier within the org/app namespace.
    #[inline]
    pub const fn unique_id(&self) -> u32 {
        self.0 as u32
    }

    /// Raw packed value.
    #[inline]
    pub const fn raw(&self) -> u64 {
        self.0
    }

    /// True if no site is assigned.
    #[inline]
    pub const fn is_none(&self) -> bool {
        self.0 == 0
    }
}

impl core::fmt::Display for SiteId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_none() {
            write!(f, "site:none")
        } else {
            write!(f, "site:{:#x}:{:#x}", self.counter_index(), self.unique_id())
        }
    }
}

impl core::fmt::Debug for SiteId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SiteId")
            .field("counter_index", &self.counter_index())
            .field("unique_id", &self.unique_id())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_zero() {
        assert!(SiteId::NONE.is_none());
        assert_eq!(SiteId::NONE.counter_index(), 0);
        assert_eq!(SiteId::NONE.unique_id(), 0);
    }

    #[test]
    fn pack_unpack() {
        let site = SiteId::new(42, 1001);
        assert_eq!(site.counter_index(), 42);
        assert_eq!(site.unique_id(), 1001);
        assert!(!site.is_none());
    }

    #[test]
    fn large_values() {
        let site = SiteId::new(0xFFFF_FFFF, 0xFFFF_FFFF);
        assert_eq!(site.counter_index(), 0xFFFF_FFFF);
        assert_eq!(site.unique_id(), 0xFFFF_FFFF);
    }

    #[test]
    fn gvt_reserved_range() {
        let gvt_site = SiteId::new(100, 1);
        assert!(gvt_site.counter_index() <= 0xFFFF);
        let user_site = SiteId::new(0x1_0000, 1);
        assert!(user_site.counter_index() > 0xFFFF);
    }

    #[test]
    fn display_format() {
        assert_eq!(format!("{}", SiteId::NONE), "site:none");
        let site = SiteId::new(42, 1001);
        assert_eq!(format!("{}", site), "site:0x2a:0x3e9");
    }
}
