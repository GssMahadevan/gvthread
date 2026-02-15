#[cfg(not(feature = "production"))]
use std::collections::BTreeMap;
use std::error::Error;

use crate::GlobalId;

/// Full error context — heap-allocated, used for diagnostic/rich errors.
///
/// In production builds, `message`, `file`, `line`, and `metadata` are
/// stripped at compile time. Only the numeric GlobalId codes, `os_error`,
/// and `source` chain survive.
pub struct ErrorContext {
    // ── Identity ──────────────────────────────────────────────
    /// Application identifier. `GlobalId::UNSET` until app-level ID is assigned.
    pub app:        GlobalId,
    /// System (crate-level) where the error originated.
    pub system:     GlobalId,
    /// Subsystem (module-level) where the error originated.
    pub subsystem:  GlobalId,
    /// The specific error code.
    pub error_code: GlobalId,
    /// Caller-defined operation context (e.g., UC_ACCEPT, UC_READ).
    pub user_code:  GlobalId,

    // ── OS integration ────────────────────────────────────────
    /// Raw OS errno, preserved when wrapping a syscall failure.
    pub os_error:   Option<i32>,

    // ── Debug-only fields ─────────────────────────────────────
    /// Human-readable error message.
    #[cfg(not(feature = "production"))]
    pub message:    String,
    /// Source file where the error was constructed.
    #[cfg(not(feature = "production"))]
    pub file:       &'static str,
    /// Line number where the error was constructed.
    #[cfg(not(feature = "production"))]
    pub line:       u32,
    /// Arbitrary key-value metadata for diagnostics.
    /// `None` by default — zero allocation when unused.
    #[cfg(not(feature = "production"))]
    pub metadata:   Option<BTreeMap<String, String>>,

    // ── Error chain ───────────────────────────────────────────
    /// The underlying cause, if any.
    pub source:     Option<Box<dyn Error + Send + Sync>>,

    // ── Backtrace ─────────────────────────────────────────────
    /// Captured backtrace string (only with `--features backtrace`).
    #[cfg(feature = "backtrace")]
    pub backtrace:  Option<String>,
}

impl ErrorContext {
    /// Attach a source error to this context.
    pub fn with_source<E>(mut self, error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        self.source = Some(Box::new(error));
        self
    }

    /// Add a key-value pair to metadata. Allocates the BTreeMap on first use.
    #[cfg(not(feature = "production"))]
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata
            .get_or_insert_with(BTreeMap::new)
            .insert(key.into(), value.into());
        self
    }

    /// Capture a backtrace if the feature is enabled and none exists yet.
    #[cfg(feature = "backtrace")]
    pub fn capture_backtrace(&mut self) {
        if self.backtrace.is_none() {
            self.backtrace = Some(std::backtrace::Backtrace::capture().to_string());
        }
    }
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self {
            app:        GlobalId::UNSET,
            system:     GlobalId::UNSET,
            subsystem:  GlobalId::UNSET,
            error_code: GlobalId::UNSET,
            user_code:  GlobalId::UNSET,
            os_error:   None,

            #[cfg(not(feature = "production"))]
            message:    String::new(),
            #[cfg(not(feature = "production"))]
            file:       "",
            #[cfg(not(feature = "production"))]
            line:       0,
            #[cfg(not(feature = "production"))]
            metadata:   None,

            source:     None,

            #[cfg(feature = "backtrace")]
            backtrace:  None,
        }
    }
}

impl core::fmt::Debug for ErrorContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut d = f.debug_struct("ErrorContext");
        d.field("app", &self.app);
        d.field("system", &self.system);
        d.field("subsystem", &self.subsystem);
        d.field("error_code", &self.error_code);
        d.field("user_code", &self.user_code);

        if let Some(errno) = self.os_error {
            d.field("os_error", &errno);
        }

        #[cfg(not(feature = "production"))]
        {
            if !self.message.is_empty() {
                d.field("message", &self.message);
            }
            d.field("location", &format_args!("{}:{}", self.file, self.line));
            if self.metadata.is_some() {
                d.field("metadata", &self.metadata);
            }
        }

        if self.source.is_some() {
            d.field("source", &self.source.as_ref().map(|e| e.to_string()));
        }

        #[cfg(feature = "backtrace")]
        if self.backtrace.is_some() {
            d.field("backtrace", &"<captured>");
        }

        d.finish()
    }
}
