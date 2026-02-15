/// A globally unique identifier for error classification.
///
/// In production builds (`--features production`), the `name` field is
/// stripped — only the numeric `code` survives. This keeps error matching
/// fast and binary-size minimal while retaining human-readable names
/// during development.
///
/// # Naming conventions
///
/// | Prefix | Purpose             | Example                              |
/// |--------|---------------------|--------------------------------------|
/// | `SYS_` | System (crate)      | `SYS_NET`, `SYS_RUNTIME`            |
/// | `SUB_` | Subsystem (module)  | `SUB_IOURING`, `SUB_SCHEDULER`       |
/// | `ERR_` | Error code          | `ERR_EAGAIN`, `ERR_BIND_FAILED`      |
/// | `UC_`  | User code (op ctx)  | `UC_ACCEPT`, `UC_READ`, `UC_SUBMIT`  |
#[derive(Clone, Copy)]
pub struct GlobalId {
    #[cfg(not(feature = "production"))]
    pub name: &'static str,
    pub code: u64,
}

impl GlobalId {
    /// Construct a new GlobalId.
    ///
    /// ```
    /// use gerror::GlobalId;
    /// const SYS_NET: GlobalId = GlobalId::new("net", 3);
    /// ```
    #[cfg(not(feature = "production"))]
    pub const fn new(name: &'static str, code: u64) -> Self {
        Self { name, code }
    }

    #[cfg(feature = "production")]
    pub const fn new(_name: &'static str, code: u64) -> Self {
        Self { code }
    }

    /// Sentinel for unset / don't-care fields.
    pub const UNSET: GlobalId = GlobalId::new("unset", 0);
}

impl PartialEq for GlobalId {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code
    }
}

impl Eq for GlobalId {}

impl core::hash::Hash for GlobalId {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.code.hash(state);
    }
}

impl core::fmt::Debug for GlobalId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        #[cfg(not(feature = "production"))]
        {
            write!(f, "{}({})", self.name, self.code)
        }
        #[cfg(feature = "production")]
        {
            write!(f, "{}", self.code)
        }
    }
}

impl core::fmt::Display for GlobalId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        #[cfg(not(feature = "production"))]
        {
            write!(f, "{}", self.name)
        }
        #[cfg(feature = "production")]
        {
            write!(f, "{}", self.code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ID: GlobalId = GlobalId::new("test_sys", 42);

    #[test]
    fn equality_by_code() {
        let a = GlobalId::new("alpha", 10);
        let b = GlobalId::new("beta", 10);
        assert_eq!(a, b); // same code = equal, names don't matter
    }

    #[test]
    fn inequality() {
        let a = GlobalId::new("alpha", 10);
        let b = GlobalId::new("alpha", 11);
        assert_ne!(a, b);
    }

    #[test]
    fn unset_is_zero() {
        assert_eq!(GlobalId::UNSET.code, 0);
    }

    #[test]
    fn const_construction() {
        assert_eq!(TEST_ID.code, 42);
    }

    #[test]
    fn display_shows_name() {
        let id = GlobalId::new("ksvc", 1);
        let s = format!("{}", id);
        #[cfg(not(feature = "production"))]
        assert_eq!(s, "ksvc");
        #[cfg(feature = "production")]
        assert_eq!(s, "1");
    }

    #[test]
    fn debug_shows_both() {
        let id = GlobalId::new("ksvc", 1);
        let s = format!("{:?}", id);
        #[cfg(not(feature = "production"))]
        assert_eq!(s, "ksvc(1)");
        #[cfg(feature = "production")]
        assert_eq!(s, "1");
    }

    #[test]
    fn copy_semantics() {
        let a = GlobalId::new("x", 5);
        let b = a; // Copy
        assert_eq!(a, b);
    }
}
