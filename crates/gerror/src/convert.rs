use std::io;

use crate::{GError, GlobalId, GResult};
use crate::context::ErrorContext;

// ── Well-known system/error codes for conversions ─────────────────

/// System code used when converting from `std::io::Error`.
pub const SYS_IO: GlobalId = GlobalId::new("io", 100);

/// Maps `io::ErrorKind` to a GlobalId error code.
fn io_error_code(kind: io::ErrorKind) -> GlobalId {
    match kind {
        io::ErrorKind::NotFound         => GlobalId::new("not_found", 101),
        io::ErrorKind::PermissionDenied => GlobalId::new("permission_denied", 102),
        io::ErrorKind::ConnectionRefused => GlobalId::new("conn_refused", 103),
        io::ErrorKind::ConnectionReset  => GlobalId::new("conn_reset", 104),
        io::ErrorKind::ConnectionAborted => GlobalId::new("conn_aborted", 105),
        io::ErrorKind::NotConnected     => GlobalId::new("not_connected", 106),
        io::ErrorKind::AddrInUse        => GlobalId::new("addr_in_use", 107),
        io::ErrorKind::AddrNotAvailable => GlobalId::new("addr_not_available", 108),
        io::ErrorKind::BrokenPipe       => GlobalId::new("broken_pipe", 109),
        io::ErrorKind::AlreadyExists    => GlobalId::new("already_exists", 110),
        io::ErrorKind::WouldBlock       => GlobalId::new("would_block", 111),
        io::ErrorKind::InvalidInput     => GlobalId::new("invalid_input", 112),
        io::ErrorKind::InvalidData      => GlobalId::new("invalid_data", 113),
        io::ErrorKind::TimedOut         => GlobalId::new("timed_out", 114),
        io::ErrorKind::Interrupted      => GlobalId::new("interrupted", 115),
        io::ErrorKind::UnexpectedEof    => GlobalId::new("unexpected_eof", 116),
        io::ErrorKind::OutOfMemory      => GlobalId::new("out_of_memory", 117),
        _                               => GlobalId::new("other", 199),
    }
}

// ── From<io::Error> ───────────────────────────────────────────────

impl From<io::Error> for GError {
    /// Convert an `io::Error` into a `GError`.
    ///
    /// Uses the Simple representation for raw OS errors (zero alloc).
    /// Custom io errors use Full to preserve the source chain.
    fn from(err: io::Error) -> Self {
        let error_code = io_error_code(err.kind());

        // Raw OS error → zero-alloc Simple path
        if err.raw_os_error().is_some() {
            return GError::simple(SYS_IO, error_code, GlobalId::UNSET);
        }

        // Custom io::Error → Full path to preserve source
        let ctx = ErrorContext {
            system: SYS_IO,
            error_code,
            #[cfg(not(feature = "production"))]
            message: err.to_string(),
            #[cfg(not(feature = "production"))]
            file: "",
            #[cfg(not(feature = "production"))]
            line: 0,
            ..Default::default()
        };

        GError::full(ErrorContext::with_source(ctx, err))
    }
}

// ── Into<io::Error> ───────────────────────────────────────────────

impl From<GError> for io::Error {
    /// Convert a `GError` back into `io::Error`.
    ///
    /// Wraps the GError as a custom io::Error.
    fn from(err: GError) -> Self {
        io::Error::new(io::ErrorKind::Other, err)
    }
}

// ── ResultExt — context annotation on Results ─────────────────────

/// Extension trait for adding `GError` context to any `Result`.
///
/// Inspired by `anyhow::Context`. Allows annotating errors during
/// propagation without defining new error constants for every call site.
///
/// ```ignore
/// use gerror::ResultExt;
///
/// // Simple string context (wraps any error into GError):
/// std::fs::read("config.toml").gerr_context("reading config")?;
///
/// // Structured context with codes:
/// socket.bind(addr).gerr_ctx(SYS_NET, ERR_BIND, UC_LISTEN, "binding port 8080")?;
/// ```
pub trait ResultExt<T> {
    /// Attach a string context message, converting any error into `GError`.
    fn gerr_context(self, msg: &str) -> GResult<T>;

    /// Attach structured context with system, error_code, user_code, and message.
    fn gerr_ctx(
        self,
        system: GlobalId,
        error_code: GlobalId,
        user_code: GlobalId,
        msg: &str,
    ) -> GResult<T>;
}

impl<T, E> ResultExt<T> for Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn gerr_context(self, _msg: &str) -> GResult<T> {
        self.map_err(|e| {
            let ctx = ErrorContext {
                system: SYS_IO,
                #[cfg(not(feature = "production"))]
                message: _msg.to_string(),
                #[cfg(not(feature = "production"))]
                file: "",
                #[cfg(not(feature = "production"))]
                line: 0,
                ..Default::default()
            }
            .with_source(e);
            GError::full(ctx)
        })
    }

    fn gerr_ctx(
        self,
        system: GlobalId,
        error_code: GlobalId,
        user_code: GlobalId,
        _msg: &str,
    ) -> GResult<T> {
        self.map_err(|e| {
            let ctx = ErrorContext {
                system,
                error_code,
                user_code,
                #[cfg(not(feature = "production"))]
                message: _msg.to_string(),
                #[cfg(not(feature = "production"))]
                file: "",
                #[cfg(not(feature = "production"))]
                line: 0,
                ..Default::default()
            }
            .with_source(e);
            GError::full(ctx)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn from_io_error_os() {
        // Raw OS error → Simple path
        let io_err = io::Error::from_raw_os_error(104); // ECONNRESET
        let gerr = GError::from(io_err);
        assert!(gerr.is_simple());
        assert_eq!(gerr.system(), &SYS_IO);
    }

    #[test]
    fn from_io_error_custom() {
        // Custom io::Error → Full path
        let io_err = io::Error::new(io::ErrorKind::AddrInUse, "port taken");
        let gerr = GError::from(io_err);
        assert!(!gerr.is_simple());
        assert_eq!(gerr.error_code().code, 107); // addr_in_use
        assert!(gerr.source().is_some());
    }

    #[test]
    fn into_io_error() {
        let gerr = GError::simple(SYS_IO, GlobalId::UNSET, GlobalId::UNSET);
        let io_err: io::Error = gerr.into();
        assert_eq!(io_err.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn question_mark_conversion() {
        fn inner() -> Result<(), io::Error> {
            Err(io::Error::new(io::ErrorKind::NotFound, "gone"))
        }
        fn outer() -> GResult<()> {
            inner()?; // auto-converts via From<io::Error>
            Ok(())
        }
        let result = outer();
        assert!(result.is_err());
        let e = result.unwrap_err();
        assert_eq!(e.error_code().code, 101); // not_found
    }

    #[test]
    fn result_ext_context() {
        fn failing() -> Result<(), io::Error> {
            Err(io::Error::new(io::ErrorKind::NotFound, "missing"))
        }
        let result = failing().gerr_context("reading config");
        assert!(result.is_err());
        let e = result.unwrap_err();
        assert!(e.source().is_some());
    }

    #[test]
    fn result_ext_ctx_structured() {
        const SYS_APP: GlobalId = GlobalId::new("app", 1);
        const ERR_INIT: GlobalId = GlobalId::new("init", 1);
        const UC_CONFIG: GlobalId = GlobalId::new("config", 1);

        fn failing() -> Result<(), io::Error> {
            Err(io::Error::new(io::ErrorKind::NotFound, "missing"))
        }

        let result = failing().gerr_ctx(SYS_APP, ERR_INIT, UC_CONFIG, "loading config");
        assert!(result.is_err());
        let e = result.unwrap_err();
        assert_eq!(e.system(), &SYS_APP);
        assert_eq!(e.error_code(), &ERR_INIT);
        assert_eq!(e.user_code(), &UC_CONFIG);
    }
}
