use std::error::Error;
use std::fmt;

use crate::context::ErrorContext;
use crate::GlobalId;

/// Generic Error — a structured, zero-dep error type.
///
/// Two internal representations, same external API:
///
/// - **Simple**: 3 GlobalIds on the stack. Zero heap allocation.  
///   Use for hot-path errors like `EAGAIN`, `WouldBlock`, `ConnectionReset`.
///
/// - **Full**: Boxed `ErrorContext` with message, source chain, metadata.  
///   Use for diagnostic errors, setup failures, configuration errors.
///
/// Users never see `Repr` — they interact through `.system()`, `.error_code()`,
/// `.user_code()`, `.kind()`, and `.os_error()`.
///
/// # Size
///
/// - Production: 32 bytes (16-byte aligned)
/// - Debug:      80 bytes (16-byte aligned)
///
/// Both variants coexist — pick the right constructor for the situation.
pub struct GError {
    repr: Repr,
}

enum Repr {
    /// Zero-allocation fast path.
    /// 3 × GlobalId: system, error_code, user_code.
    /// Optional os_error packed alongside.
    Simple {
        system:     GlobalId,
        error_code: GlobalId,
        user_code:  GlobalId,
        os_error:   Option<i32>,
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
                os_error: None,
            },
        }
    }

    /// Create a zero-allocation error with an OS errno attached.
    ///
    /// Use when wrapping raw io_uring CQE results or syscall failures
    /// where the caller may need to inspect the raw errno.
    #[inline]
    pub fn simple_os(
        system: GlobalId,
        error_code: GlobalId,
        user_code: GlobalId,
        os_error: i32,
    ) -> Self {
        Self {
            repr: Repr::Simple {
                system,
                error_code,
                user_code,
                os_error: Some(os_error),
            },
        }
    }

    /// Create a full diagnostic error from a pre-built ErrorContext.
    ///
    /// Prefer the `err!` macro over calling this directly.
    pub fn full(ctx: ErrorContext) -> Self {
        Self {
            repr: Repr::Full(Box::new(ctx)),
        }
    }

    /// Convert any `std::error::Error` into a `GError`.
    ///
    /// Low-level escape hatch for callbacks or contexts where you have
    /// a bare error value rather than a `Result`. Uses `UNSET` codes —
    /// prefer `ResultExt::gerr_ctx()` when you have a `Result` and want
    /// structured codes.
    ///
    /// ```ignore
    /// // In a callback where you only have the error value:
    /// let gerr = GError::from_std(some_toml_error);
    ///
    /// // Prefer this instead when you have a Result:
    /// toml::from_str(&data)
    ///     .gerr_ctx(SYS_APP, ERR_PARSE, UC_CONFIG, "parsing config")?;
    /// ```
    pub fn from_std<E>(err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        let ctx = ErrorContext {
            system: crate::convert::SYS_IO,
            #[cfg(not(feature = "production"))]
            message: err.to_string(),
            ..Default::default()
        }
        .with_source(err);
        Self::full(ctx)
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

    /// Raw OS errno, if this error wraps a syscall failure.
    #[inline]
    pub fn os_error(&self) -> Option<i32> {
        match &self.repr {
            Repr::Simple { os_error, .. } => *os_error,
            Repr::Full(ctx) => ctx.os_error,
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

    /// Consume this error and return the ErrorContext, if available.
    /// For Simple errors, constructs a minimal ErrorContext.
    pub fn into_context(self) -> ErrorContext {
        match self.repr {
            Repr::Simple { system, error_code, user_code, os_error } => {
                ErrorContext {
                    system,
                    error_code,
                    user_code,
                    os_error,
                    ..Default::default()
                }
            }
            Repr::Full(ctx) => *ctx,
        }
    }

    /// Attempt to downcast the source error to a concrete type.
    ///
    /// This is the equivalent of Java's `instanceof` check on a cause.
    ///
    /// ```ignore
    /// if let Some(io_err) = gerr.downcast_ref::<std::io::Error>() {
    ///     eprintln!("raw os error: {:?}", io_err.raw_os_error());
    /// }
    /// ```
    pub fn downcast_ref<E: std::error::Error + 'static>(&self) -> Option<&E> {
        match &self.repr {
            Repr::Simple { .. } => None,
            Repr::Full(ctx) => ctx.source.as_ref()?.downcast_ref::<E>(),
        }
    }

    /// Re-tag this error at a higher layer, preserving the original as source.
    ///
    /// Use when catching a lower-layer GError and re-classifying it for
    /// the caller. The original error becomes the `.source()`.
    ///
    /// ```ignore
    /// // net layer returns (SYS_NET, ERR_ECONNRESET, UC_READ)
    /// // app layer wraps it as (SYS_APP, ERR_REQUEST_FAILED, UC_HANDLE)
    /// let app_err = net_err.wrap(SYS_APP, ERR_REQUEST_FAILED, UC_HANDLE);
    /// // app_err.source() → the original net_err
    /// ```
    pub fn wrap(self, system: GlobalId, error_code: GlobalId, user_code: GlobalId) -> Self {
        let ctx = ErrorContext {
            system,
            error_code,
            user_code,
            #[cfg(not(feature = "production"))]
            message: format!("{}", &self),
            #[cfg(not(feature = "production"))]
            file: "",
            #[cfg(not(feature = "production"))]
            line: 0,
            ..Default::default()
        }
        .with_source(self);
        Self::full(ctx)
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
            Repr::Simple { system, error_code, user_code, os_error } => {
                write!(f, "[{}/{}] {}", system, error_code, user_code)?;
                if let Some(errno) = os_error {
                    write!(f, " (os error {})", errno)?;
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

                if let Some(errno) = ctx.os_error {
                    write!(f, " (os error {})", errno)?;
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
            Repr::Simple { system, error_code, user_code, os_error } => {
                f.debug_struct("GError::Simple")
                    .field("system", system)
                    .field("error_code", error_code)
                    .field("user_code", user_code)
                    .field("os_error", os_error)
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
// - Simple variant: GlobalId is Copy (no references), Option<i32> is trivial
// - Full variant: ErrorContext contains Box<dyn Error + Send + Sync>
// Both are safe to send across threads.
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
        assert_eq!(err.os_error(), None);
        assert!(err.context().is_none());
    }

    #[test]
    fn simple_with_os_error() {
        let err = GError::simple_os(SYS_NET, ERR_EAGAIN, UC_ACCEPT, 11);
        assert_eq!(err.os_error(), Some(11));
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
    fn display_simple_with_errno() {
        let err = GError::simple_os(SYS_NET, ERR_EAGAIN, UC_ACCEPT, 11);
        let s = format!("{}", err);
        assert!(s.contains("os error 11"));
    }

    #[test]
    fn into_context_from_simple() {
        let err = GError::simple_os(SYS_NET, ERR_EAGAIN, UC_ACCEPT, 11);
        let ctx = err.into_context();
        assert_eq!(ctx.system, SYS_NET);
        assert_eq!(ctx.error_code, ERR_EAGAIN);
        assert_eq!(ctx.user_code, UC_ACCEPT);
        assert_eq!(ctx.os_error, Some(11));
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
        // Verify alignment
        assert_eq!(size % 16, 0, "GError should be 16-byte aligned, got {} bytes", size);
        // Print for visibility
        eprintln!("GError size: {} bytes", size);
        eprintln!("Repr size:   {} bytes", std::mem::size_of::<Repr>());
        eprintln!("GlobalId:    {} bytes", std::mem::size_of::<GlobalId>());
    }

    #[test]
    fn send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GError>();
    }

    #[test]
    fn from_std_any_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let gerr = GError::from_std(io_err);
        assert!(!gerr.is_simple());
        assert!(gerr.source().is_some());
    }

    #[test]
    fn downcast_ref_succeeds() {
        let io_err = std::io::Error::new(std::io::ErrorKind::AddrInUse, "taken");
        let gerr = GError::from_std(io_err);
        let downcasted = gerr.downcast_ref::<std::io::Error>();
        assert!(downcasted.is_some());
        assert_eq!(downcasted.unwrap().kind(), std::io::ErrorKind::AddrInUse);
    }

    #[test]
    fn downcast_ref_fails_on_simple() {
        let gerr = GError::simple(SYS_NET, ERR_EAGAIN, UC_ACCEPT);
        let downcasted = gerr.downcast_ref::<std::io::Error>();
        assert!(downcasted.is_none());
    }

    #[test]
    fn downcast_ref_wrong_type() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let gerr = GError::from_std(io_err);
        // Try downcasting to a different type
        let downcasted = gerr.downcast_ref::<std::fmt::Error>();
        assert!(downcasted.is_none());
    }

    #[test]
    fn wrap_preserves_source() {
        const SYS_APP: GlobalId = GlobalId::new("app", 100);
        const ERR_REQ: GlobalId = GlobalId::new("request_failed", 101);
        const UC_HANDLE: GlobalId = GlobalId::new("handle", 102);

        let net_err = GError::simple(SYS_NET, ERR_EAGAIN, UC_ACCEPT);
        let app_err = net_err.wrap(SYS_APP, ERR_REQ, UC_HANDLE);

        // New codes at outer level
        assert_eq!(app_err.system(), &SYS_APP);
        assert_eq!(app_err.error_code(), &ERR_REQ);
        assert_eq!(app_err.user_code(), &UC_HANDLE);

        // Original preserved as source
        assert!(app_err.source().is_some());

        // Can downcast to original GError
        let original = app_err.downcast_ref::<GError>();
        assert!(original.is_some());
        assert_eq!(original.unwrap().system(), &SYS_NET);
        assert_eq!(original.unwrap().error_code(), &ERR_EAGAIN);
    }
}
