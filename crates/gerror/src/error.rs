use std::error::Error;
use std::fmt;

use crate::context::ErrorContext;
use crate::GlobalId;
use crate::SiteId;

/// Generic Error — a structured, zero-dep error type.
///
/// Two internal representations, same external API:
///
/// - **Simple**: 3 GlobalIds + SiteId on the stack. Zero heap allocation.
///   Use for hot-path errors like `EAGAIN`, `WouldBlock`, `ConnectionReset`.
///
/// - **Full**: Boxed `ErrorContext` with message, source chain, metadata.
///   Use for diagnostic errors, setup failures, configuration errors.
///
/// Users never see `Repr` — they interact through `.system()`, `.error_code()`,
/// `.user_code()`, `.kind()`, and `.site_id()`.
///
/// # Size
///
/// - Production: 32 bytes (16-byte aligned)
/// - Debug:      80 bytes (16-byte aligned)
///
/// # Site-level Metrics (`feature = "metrics"`)
///
/// When the `metrics` feature is enabled, every GError creation with a
/// non-NONE site_id atomically increments a per-site counter.
/// Cost: one `AtomicU64::fetch_add(1, Relaxed)`.
pub struct GError {
    repr: Repr,
}

enum Repr {
    /// Zero-allocation fast path.
    /// 3 × GlobalId (8 bytes each in production) + SiteId (8 bytes).
    Simple {
        system:     GlobalId,
        error_code: GlobalId,
        user_code:  GlobalId,
        site_id:    SiteId,
    },
    /// Heap-allocated full diagnostic context.
    Full(Box<ErrorContext>),
}

// ── Constructors ──────────────────────────────────────────────────

impl GError {
    /// Create a zero-allocation error with just the three identifying codes.
    ///
    /// ```
    /// use gerror::{GError, GlobalId};
    /// const SYS_NET: GlobalId = GlobalId::new("net", 3);
    /// const ERR_EAGAIN: GlobalId = GlobalId::new("eagain", 11);
    /// const UC_ACCEPT: GlobalId = GlobalId::new("accept", 1);
    ///
    /// let err = GError::simple(SYS_NET, ERR_EAGAIN, UC_ACCEPT);
    /// ```
    #[inline]
    pub fn simple(system: GlobalId, error_code: GlobalId, user_code: GlobalId) -> Self {
        Self {
            repr: Repr::Simple {
                system,
                error_code,
                user_code,
                site_id: SiteId::NONE,
            },
        }
    }

    /// Create a zero-allocation error with a site identifier for metrics.
    ///
    /// When `feature = "metrics"` is enabled, this also bumps the
    /// per-site `AtomicU64` counter. Without the feature, site_id is
    /// stored but no counter infrastructure exists.
    #[inline]
    pub fn simple_site(
        system: GlobalId,
        error_code: GlobalId,
        user_code: GlobalId,
        site_id: SiteId,
    ) -> Self {
        #[cfg(feature = "metrics")]
        crate::metrics::bump(site_id);

        Self {
            repr: Repr::Simple {
                system,
                error_code,
                user_code,
                site_id,
            },
        }
    }

    /// Create a full diagnostic error from a pre-built ErrorContext.
    ///
    /// Prefer the `err!` macro over calling this directly.
    pub fn full(ctx: ErrorContext) -> Self {
        #[cfg(feature = "metrics")]
        if !ctx.site_id.is_none() {
            crate::metrics::bump(ctx.site_id);
        }

        Self {
            repr: Repr::Full(Box::new(ctx)),
        }
    }
}

// ── Accessors ─────────────────────────────────────────────────────

impl GError {
    /// The system (crate) where the error originated.
    #[inline]
    pub fn system(&self) -> &GlobalId {
        match &self.repr {
            Repr::Simple { system, .. } => system,
            Repr::Full(ctx) => &ctx.system,
        }
    }

    /// The specific error code.
    #[inline]
    pub fn error_code(&self) -> &GlobalId {
        match &self.repr {
            Repr::Simple { error_code, .. } => error_code,
            Repr::Full(ctx) => &ctx.error_code,
        }
    }

    /// The caller-defined operation context.
    #[inline]
    pub fn user_code(&self) -> &GlobalId {
        match &self.repr {
            Repr::Simple { user_code, .. } => user_code,
            Repr::Full(ctx) => &ctx.user_code,
        }
    }

    /// The triple `(system, error_code, user_code)` for matching.
    ///
    /// ```ignore
    /// let (sys, err, uc) = gerr.kind();
    /// ```
    #[inline]
    pub fn kind(&self) -> (&GlobalId, &GlobalId, &GlobalId) {
        match &self.repr {
            Repr::Simple { system, error_code, user_code, .. } => {
                (system, error_code, user_code)
            }
            Repr::Full(ctx) => {
                (&ctx.system, &ctx.error_code, &ctx.user_code)
            }
        }
    }

    /// The subsystem, if available (only in Full variant).
    /// Returns `GlobalId::UNSET` for Simple errors.
    #[inline]
    pub fn subsystem(&self) -> &GlobalId {
        match &self.repr {
            Repr::Simple { .. } => &GlobalId::UNSET,
            Repr::Full(ctx) => &ctx.subsystem,
        }
    }

    /// The error site identifier for metrics.
    /// Returns `SiteId::NONE` if not set.
    #[inline]
    pub fn site_id(&self) -> SiteId {
        match &self.repr {
            Repr::Simple { site_id, .. } => *site_id,
            Repr::Full(ctx) => ctx.site_id,
        }
    }

    /// Returns `true` if this is a zero-allocation Simple error.
    #[inline]
    pub fn is_simple(&self) -> bool {
        matches!(&self.repr, Repr::Simple { .. })
    }

    /// Access the full ErrorContext, if available.
    /// Returns `None` for Simple errors.
    pub fn context(&self) -> Option<&ErrorContext> {
        match &self.repr {
            Repr::Simple { .. } => None,
            Repr::Full(ctx) => Some(ctx),
        }
    }

    /// Consume this error and return the ErrorContext.
    /// For Simple errors, constructs a minimal ErrorContext.
    pub fn into_context(self) -> ErrorContext {
        match self.repr {
            Repr::Simple { system, error_code, user_code, site_id } => {
                ErrorContext {
                    system,
                    error_code,
                    user_code,
                    site_id,
                    ..Default::default()
                }
            }
            Repr::Full(ctx) => *ctx,
        }
    }
}

// ── std::error::Error ─────────────────────────────────────────────

impl Error for GError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.repr {
            Repr::Simple { .. } => None,
            Repr::Full(ctx) => ctx.source.as_ref().map(|e| e.as_ref() as &(dyn Error + 'static)),
        }
    }
}

// ── Display ───────────────────────────────────────────────────────

impl fmt::Display for GError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            Repr::Simple { system, error_code, user_code, site_id } => {
                write!(f, "[{}/{}] {}", system, error_code, user_code)?;
                if !site_id.is_none() {
                    write!(f, " ({})", site_id)?;
                }
                Ok(())
            }
            Repr::Full(ctx) => {
                write!(f, "[{}/{}/{}] {}",
                    ctx.system, ctx.subsystem, ctx.error_code, ctx.user_code)?;

                #[cfg(not(feature = "production"))]
                if !ctx.message.is_empty() {
                    write!(f, ": {}", ctx.message)?;
                }

                if !ctx.site_id.is_none() {
                    write!(f, " ({})", ctx.site_id)?;
                }

                if let Some(src) = &ctx.source {
                    write!(f, " — caused by: {}", src)?;
                }

                #[cfg(not(feature = "production"))]
                if !ctx.file.is_empty() {
                    write!(f, " at {}:{}", ctx.file, ctx.line)?;
                }

                Ok(())
            }
        }
    }
}

// ── Debug ─────────────────────────────────────────────────────────

impl fmt::Debug for GError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            Repr::Simple { system, error_code, user_code, site_id } => {
                f.debug_struct("GError::Simple")
                    .field("system", system)
                    .field("error_code", error_code)
                    .field("user_code", user_code)
                    .field("site_id", site_id)
                    .finish()
            }
            Repr::Full(ctx) => {
                f.debug_struct("GError::Full")
                    .field("context", ctx)
                    .finish()
            }
        }
    }
}

// ── Send + Sync ───────────────────────────────────────────────────

// GError is Send + Sync:
// - Simple variant: GlobalId is Copy, SiteId is Copy — no references
// - Full variant: ErrorContext contains Box<dyn Error + Send + Sync>
unsafe impl Send for GError {}
unsafe impl Sync for GError {}

#[cfg(test)]
mod tests {
    use super::*;

    const SYS_NET: GlobalId = GlobalId::new("net", 3);
    const ERR_EAGAIN: GlobalId = GlobalId::new("eagain", 11);
    const ERR_BIND: GlobalId = GlobalId::new("bind_failed", 8);
    const UC_ACCEPT: GlobalId = GlobalId::new("accept", 1);
    const UC_LISTEN: GlobalId = GlobalId::new("listen", 2);
    const SUB_LISTENER: GlobalId = GlobalId::new("listener", 5);

    #[test]
    fn simple_zero_alloc() {
        let err = GError::simple(SYS_NET, ERR_EAGAIN, UC_ACCEPT);
        assert!(err.is_simple());
        assert_eq!(err.system(), &SYS_NET);
        assert_eq!(err.error_code(), &ERR_EAGAIN);
        assert_eq!(err.user_code(), &UC_ACCEPT);
        assert!(err.site_id().is_none());
        assert!(err.context().is_none());
    }

    #[test]
    fn simple_with_site() {
        let site = SiteId::new(42, 1001);
        let err = GError::simple_site(SYS_NET, ERR_EAGAIN, UC_ACCEPT, site);
        assert_eq!(err.site_id(), site);
        assert_eq!(err.site_id().counter_index(), 42);
        assert_eq!(err.site_id().unique_id(), 1001);
    }

    #[test]
    fn full_error() {
        let ctx = ErrorContext {
            system: SYS_NET,
            subsystem: SUB_LISTENER,
            error_code: ERR_BIND,
            user_code: UC_LISTEN,
            #[cfg(not(feature = "production"))]
            message: "port 8080 in use".to_string(),
            #[cfg(not(feature = "production"))]
            file: file!(),
            #[cfg(not(feature = "production"))]
            line: line!(),
            ..Default::default()
        };
        let err = GError::full(ctx);
        assert!(!err.is_simple());
        assert_eq!(err.system(), &SYS_NET);
        assert_eq!(err.subsystem(), &SUB_LISTENER);
        assert_eq!(err.error_code(), &ERR_BIND);
        assert!(err.context().is_some());
    }

    #[test]
    fn kind_triple() {
        let err = GError::simple(SYS_NET, ERR_EAGAIN, UC_ACCEPT);
        let (sys, code, uc) = err.kind();
        assert_eq!(sys, &SYS_NET);
        assert_eq!(code, &ERR_EAGAIN);
        assert_eq!(uc, &UC_ACCEPT);
    }

    #[test]
    fn display_simple() {
        let err = GError::simple(SYS_NET, ERR_EAGAIN, UC_ACCEPT);
        let s = format!("{}", err);
        #[cfg(not(feature = "production"))]
        {
            assert!(s.contains("net"), "expected 'net' in: {}", s);
            assert!(s.contains("eagain"), "expected 'eagain' in: {}", s);
            assert!(s.contains("accept"), "expected 'accept' in: {}", s);
        }
        #[cfg(feature = "production")]
        {
            assert!(s.contains("3"), "expected system code '3' in: {}", s);
            assert!(s.contains("11"), "expected error code '11' in: {}", s);
            assert!(s.contains("1"), "expected user code '1' in: {}", s);
        }
    }

    #[test]
    fn display_simple_with_site() {
        let site = SiteId::new(42, 1001);
        let err = GError::simple_site(SYS_NET, ERR_EAGAIN, UC_ACCEPT, site);
        let s = format!("{}", err);
        assert!(s.contains("site:"), "expected site info in: {}", s);
    }

    #[test]
    fn into_context_from_simple() {
        let site = SiteId::new(7, 99);
        let err = GError::simple_site(SYS_NET, ERR_EAGAIN, UC_ACCEPT, site);
        let ctx = err.into_context();
        assert_eq!(ctx.system, SYS_NET);
        assert_eq!(ctx.error_code, ERR_EAGAIN);
        assert_eq!(ctx.user_code, UC_ACCEPT);
        assert_eq!(ctx.site_id, site);
    }

    #[test]
    fn with_source_chain() {
        let io_err = std::io::Error::new(std::io::ErrorKind::AddrInUse, "port taken");
        let ctx = ErrorContext {
            system: SYS_NET,
            error_code: ERR_BIND,
            user_code: UC_LISTEN,
            ..Default::default()
        }
        .with_source(io_err);
        let err = GError::full(ctx);
        assert!(err.source().is_some());
    }

    #[test]
    fn size_check() {
        let size = std::mem::size_of::<GError>();
        assert_eq!(size % 16, 0, "GError should be 16-byte aligned, got {} bytes", size);
        // Production: GlobalId = 8 bytes (just u64) → Simple = 3×8 + 8(SiteId) = 32
        // Debug: GlobalId = 24 bytes (u64 + &str) → Simple = 3×24 + 8(SiteId) = 80
        eprintln!("GError size: {} bytes", size);
        eprintln!("Repr size:   {} bytes", std::mem::size_of::<Repr>());
        eprintln!("GlobalId:    {} bytes", std::mem::size_of::<GlobalId>());
        eprintln!("SiteId:      {} bytes", std::mem::size_of::<SiteId>());
    }

    #[test]
    fn send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GError>();
    }
}
