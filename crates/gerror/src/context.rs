#[cfg(not(feature = "production"))]
use std::collections::BTreeMap;
use std::error::Error;

use crate::GlobalId;
use crate::SiteId;

/// Full error context — heap-allocated, used for diagnostic/rich errors.
///
/// In production builds, `message`, `file`, `line`, and `metadata` are
/// stripped at compile time. Only the numeric GlobalId codes, `site_id`,
/// and `source` chain survive.
pub struct ErrorContext {
    // ── Identity ──────────────────────────────────────────────
    pub app:        GlobalId,
    pub system:     GlobalId,
    pub subsystem:  GlobalId,
    pub error_code: GlobalId,
    pub user_code:  GlobalId,

    // ── Site metrics ───────────────────────────────────────────
    /// Error site identifier. Indexes into the metrics counter array.
    pub site_id:    SiteId,

    // ── Debug-only fields ─────────────────────────────────────
    #[cfg(not(feature = "production"))]
    pub message:    String,
    #[cfg(not(feature = "production"))]
    pub file:       &'static str,
    #[cfg(not(feature = "production"))]
    pub line:       u32,
    #[cfg(not(feature = "production"))]
    pub metadata:   Option<BTreeMap<String, String>>,

    // ── Error chain ───────────────────────────────────────────
    pub source:     Option<Box<dyn Error + Send + Sync>>,

    // ── Backtrace ─────────────────────────────────────────────
    #[cfg(feature = "backtrace")]
    pub backtrace:  Option<String>,
}

impl ErrorContext {
    pub fn with_source<E>(mut self, error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        self.source = Some(Box::new(error));
        self
    }

    #[cfg(not(feature = "production"))]
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata
            .get_or_insert_with(BTreeMap::new)
            .insert(key.into(), value.into());
        self
    }

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
            site_id:    SiteId::NONE,

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

        if !self.site_id.is_none() {
            d.field("site_id", &self.site_id);
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
