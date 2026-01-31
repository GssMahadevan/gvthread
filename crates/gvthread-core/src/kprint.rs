//! Kernel-style print macros for gvthread
//!
//! Provides thread-safe, optionally-flushing debug output similar to Linux kernel's printk.
//!
//! # Environment Variables
//!
//! - `GVT_FLUSH_EPRINT=1` - Flush stderr after each print (useful for debugging crashes)
//! - `GVT_LOG_LEVEL=<level>` - Set log level: 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
//!
//! # Usage
//!
//! ```ignore
//! use gvthread_core::kprint::{kprintln, kdebug, kinfo, kwarn, kerror};
//!
//! kprintln!("Simple message");
//! kdebug!("Debug info: x={}", x);
//! kinfo!("Worker {} started", id);
//! kwarn!("Unexpected state: {:?}", state);
//! kerror!("Critical failure!");
//! ```

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Log levels (matches common conventions)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl LogLevel {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => LogLevel::Off,
            1 => LogLevel::Error,
            2 => LogLevel::Warn,
            3 => LogLevel::Info,
            4 => LogLevel::Debug,
            _ => LogLevel::Trace,
        }
    }
    
    pub fn prefix(&self) -> &'static str {
        match self {
            LogLevel::Off => "",
            LogLevel::Error => "[ERROR]",
            LogLevel::Warn => "[WARN] ",
            LogLevel::Info => "[INFO] ",
            LogLevel::Debug => "[DEBUG]",
            LogLevel::Trace => "[TRACE]",
        }
    }
}

// Global configuration (initialized once)
static FLUSH_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize logging from environment variables
/// 
/// Called automatically on first log, but can be called explicitly for
/// deterministic initialization.
pub fn init() {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return; // Already initialized
    }
    
    // Check GVT_FLUSH_EPRINT
    if let Ok(val) = std::env::var("GVT_FLUSH_EPRINT") {
        let flush = matches!(val.as_str(), "1" | "true" | "yes" | "on");
        FLUSH_ENABLED.store(flush, Ordering::Relaxed);
    }
    
    // Check GVT_LOG_LEVEL
    if let Ok(val) = std::env::var("GVT_LOG_LEVEL") {
        let level = match val.to_lowercase().as_str() {
            "off" | "0" => LogLevel::Off,
            "error" | "1" => LogLevel::Error,
            "warn" | "2" => LogLevel::Warn,
            "info" | "3" => LogLevel::Info,
            "debug" | "4" => LogLevel::Debug,
            "trace" | "5" => LogLevel::Trace,
            _ => LogLevel::Info,
        };
        LOG_LEVEL.store(level as u8, Ordering::Relaxed);
    }
}

/// Check if flush is enabled
#[inline]
pub fn flush_enabled() -> bool {
    if !INITIALIZED.load(Ordering::Relaxed) {
        init();
    }
    FLUSH_ENABLED.load(Ordering::Relaxed)
}

/// Get current log level
#[inline]
pub fn log_level() -> LogLevel {
    if !INITIALIZED.load(Ordering::Relaxed) {
        init();
    }
    LogLevel::from_u8(LOG_LEVEL.load(Ordering::Relaxed))
}

/// Set log level programmatically
pub fn set_log_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Set flush mode programmatically  
pub fn set_flush_enabled(enabled: bool) {
    FLUSH_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Check if a log level is enabled
#[inline]
pub fn level_enabled(level: LogLevel) -> bool {
    level as u8 <= log_level() as u8
}

/// Internal: Write and optionally flush
/// 
/// Uses a lock on stderr to ensure atomic line output.
#[doc(hidden)]
pub fn _kprint_impl(args: std::fmt::Arguments<'_>) {
    let stderr = std::io::stderr();
    let mut handle = stderr.lock(); // Mutex lock for atomic output
    let _ = handle.write_fmt(args);
    if flush_enabled() {
        let _ = handle.flush();
    }
}

/// Internal: Write with newline and optionally flush
#[doc(hidden)]
pub fn _kprintln_impl(args: std::fmt::Arguments<'_>) {
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = handle.write_fmt(args);
    let _ = handle.write_all(b"\n");
    if flush_enabled() {
        let _ = handle.flush();
    }
}

/// Internal: Leveled print
#[doc(hidden)]
pub fn _klog_impl(level: LogLevel, args: std::fmt::Arguments<'_>) {
    if !level_enabled(level) {
        return;
    }
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = write!(handle, "{} ", level.prefix());
    let _ = handle.write_fmt(args);
    let _ = handle.write_all(b"\n");
    if flush_enabled() {
        let _ = handle.flush();
    }
}

// ============================================================================
// Public Macros
// ============================================================================

/// Print to stderr (no newline)
/// 
/// Like `eprint!` but with optional auto-flush and mutex protection.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        $crate::kprint::_kprint_impl(format_args!($($arg)*));
    }};
}

/// Print to stderr with newline
/// 
/// Like `eprintln!` but with optional auto-flush and mutex protection.
#[macro_export]
macro_rules! kprintln {
    () => {{
        $crate::kprint::_kprintln_impl(format_args!(""));
    }};
    ($($arg:tt)*) => {{
        $crate::kprint::_kprintln_impl(format_args!($($arg)*));
    }};
}

/// Error level log (always shown unless logging is off)
#[macro_export]
macro_rules! kerror {
    ($($arg:tt)*) => {{
        $crate::kprint::_klog_impl(
            $crate::kprint::LogLevel::Error,
            format_args!($($arg)*)
        );
    }};
}

/// Warning level log
#[macro_export]
macro_rules! kwarn {
    ($($arg:tt)*) => {{
        $crate::kprint::_klog_impl(
            $crate::kprint::LogLevel::Warn,
            format_args!($($arg)*)
        );
    }};
}

/// Info level log
#[macro_export]
macro_rules! kinfo {
    ($($arg:tt)*) => {{
        $crate::kprint::_klog_impl(
            $crate::kprint::LogLevel::Info,
            format_args!($($arg)*)
        );
    }};
}

/// Debug level log
#[macro_export]
macro_rules! kdebug {
    ($($arg:tt)*) => {{
        $crate::kprint::_klog_impl(
            $crate::kprint::LogLevel::Debug,
            format_args!($($arg)*)
        );
    }};
}

/// Trace level log (most verbose)
#[macro_export]
macro_rules! ktrace {
    ($($arg:tt)*) => {{
        $crate::kprint::_klog_impl(
            $crate::kprint::LogLevel::Trace,
            format_args!($($arg)*)
        );
    }};
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_log_levels() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }
    
    #[test]
    fn test_level_from_u8() {
        assert_eq!(LogLevel::from_u8(0), LogLevel::Off);
        assert_eq!(LogLevel::from_u8(1), LogLevel::Error);
        assert_eq!(LogLevel::from_u8(4), LogLevel::Debug);
        assert_eq!(LogLevel::from_u8(99), LogLevel::Trace);
    }
    
    #[test]
    fn test_macros_compile() {
        // Just verify macros compile - actual output tested manually
        set_log_level(LogLevel::Off); // Suppress output during test
        
        kprint!("test");
        kprintln!("test {}", 42);
        kerror!("error {}", "msg");
        kwarn!("warn");
        kinfo!("info");
        kdebug!("debug");
        ktrace!("trace");
    }
}