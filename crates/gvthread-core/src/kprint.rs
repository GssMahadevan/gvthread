//! Kernel-style print macros for gvthread
//!
//! Provides thread-safe, context-aware debug output similar to Linux kernel's printk.
//! Automatically includes worker ID, GVThread ID, and optional timestamp.
//!
//! # Environment Variables
//!
//! - `GVT_FLUSH_EPRINT=1` - Flush stderr after each print (useful for debugging crashes)
//! - `GVT_LOG_LEVEL=<level>` - Set log level: 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
//! - `GVT_KPRINT_TIME=1` - Include nanosecond timestamp in output
//!
//! # Output Format
//!
//! Without timestamp: `[LEVEL] [w<worker>:g<gvthread>] message`
//! With timestamp:    `[LEVEL] [<ns>] [w<worker>:g<gvthread>] message`
//!
//! Examples:
//! - `[DEBUG] [w0:g5] Started processing`
//! - `[INFO]  [12345678] [w2:g--] Worker idle`
//! - `[ERROR] [w--:g--] Not in runtime context`
//!
//! # Usage
//!
//! ```ignore
//! use gvthread_core::{kprintln, kdebug, kinfo, kwarn, kerror};
//!
//! // User just provides message - context is automatic
//! kdebug!("Processing item {}", item_id);
//! kinfo!("Task completed");
//! kwarn!("Unexpected state: {:?}", state);
//! kerror!("Critical failure!");
//! ```

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Instant;
use crate::env::env_get_bool;

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
static TIME_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

// Start time for relative timestamps
static START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Initialize logging from environment variables
/// 
/// Called automatically on first log, but can be called explicitly for
/// deterministic initialization.
pub fn init() {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return; // Already initialized
    }
    
    // Initialize start time
    START_TIME.get_or_init(Instant::now);
    
    // Check GVT_FLUSH_EPRINT
    FLUSH_ENABLED.store(env_get_bool("GVT_FLUSH_EPRINT", false), Ordering::Relaxed);
    
    // Check GVT_KPRINT_TIME
    TIME_ENABLED.store(env_get_bool("GVT_KPRINT_TIME", false), Ordering::Relaxed);
    
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

/// Check if timestamp is enabled
#[inline]
pub fn time_enabled() -> bool {
    if !INITIALIZED.load(Ordering::Relaxed) {
        init();
    }
    TIME_ENABLED.load(Ordering::Relaxed)
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

/// Set time display programmatically
pub fn set_time_enabled(enabled: bool) {
    TIME_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Check if a log level is enabled
#[inline]
pub fn level_enabled(level: LogLevel) -> bool {
    level as u8 <= log_level() as u8
}

/// Get elapsed nanoseconds since start (safe for any stack)
#[inline]
pub fn elapsed_ns() -> u64 {
    let start = START_TIME.get_or_init(Instant::now);
    start.elapsed().as_nanos() as u64
}

// Thread-local for worker ID (set by runtime)
thread_local! {
    static WORKER_ID: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    static GVTHREAD_ID: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

/// Set current worker ID for this thread (called by runtime)
pub fn set_worker_id(id: u32) {
    WORKER_ID.with(|w| w.set(Some(id)));
}

/// Clear worker ID (called by runtime on thread exit)
pub fn clear_worker_id() {
    WORKER_ID.with(|w| w.set(None));
}

/// Set current GVThread ID (called by runtime during context switch)
pub fn set_gvthread_id(id: u32) {
    GVTHREAD_ID.with(|g| g.set(Some(id)));
}

/// Clear GVThread ID (called by runtime when not in GVThread)
pub fn clear_gvthread_id() {
    GVTHREAD_ID.with(|g| g.set(None));
}

/// Get current worker ID
#[inline]
pub fn get_worker_id() -> Option<u32> {
    WORKER_ID.with(|w| w.get())
}

/// Get current GVThread ID
#[inline]
pub fn get_gvthread_id() -> Option<u32> {
    GVTHREAD_ID.with(|g| g.get())
}

/// Format context string [w<id>:g<id>]
fn format_context() -> String {
    let worker = match get_worker_id() {
        Some(id) => format!("w{}", id),
        None => "w--".to_string(),
    };
    let gvthread = match get_gvthread_id() {
        Some(id) => format!("g{}", id),
        None => "g--".to_string(),
    };
    format!("[{}:{}]", worker, gvthread)
}

/// Internal: Write and optionally flush (no context)
#[doc(hidden)]
pub fn _kprint_impl(args: std::fmt::Arguments<'_>) {
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = handle.write_fmt(args);
    if flush_enabled() {
        let _ = handle.flush();
    }
}

/// Internal: Write with newline and optionally flush (no context)
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

/// Internal: Leveled print with context
#[doc(hidden)]
pub fn _klog_impl(level: LogLevel, args: std::fmt::Arguments<'_>) {
    if !level_enabled(level) {
        return;
    }
    
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    
    // Level prefix
    let _ = write!(handle, "{} ", level.prefix());
    
    // Optional timestamp
    if time_enabled() {
        let _ = write!(handle, "[{}] ", elapsed_ns());
    }
    
    // Context [worker:gvthread]
    let _ = write!(handle, "{} ", format_context());
    
    // User message
    let _ = handle.write_fmt(args);
    let _ = handle.write_all(b"\n");
    
    if flush_enabled() {
        let _ = handle.flush();
    }
}

// ============================================================================
// Public Macros
// ============================================================================

/// Print to stderr (no newline, no context)
/// 
/// Like `eprint!` but with optional auto-flush and mutex protection.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        $crate::kprint::_kprint_impl(format_args!($($arg)*));
    }};
}

/// Print to stderr with newline (no context)
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

/// Error level log with context
#[macro_export]
macro_rules! kerror {
    ($($arg:tt)*) => {{
        $crate::kprint::_klog_impl(
            $crate::kprint::LogLevel::Error,
            format_args!($($arg)*)
        );
    }};
}

/// Warning level log with context
#[macro_export]
macro_rules! kwarn {
    ($($arg:tt)*) => {{
        $crate::kprint::_klog_impl(
            $crate::kprint::LogLevel::Warn,
            format_args!($($arg)*)
        );
    }};
}

/// Info level log with context
#[macro_export]
macro_rules! kinfo {
    ($($arg:tt)*) => {{
        $crate::kprint::_klog_impl(
            $crate::kprint::LogLevel::Info,
            format_args!($($arg)*)
        );
    }};
}

/// Debug level log with context
#[macro_export]
macro_rules! kdebug {
    ($($arg:tt)*) => {{
        $crate::kprint::_klog_impl(
            $crate::kprint::LogLevel::Debug,
            format_args!($($arg)*)
        );
    }};
}

/// Trace level log with context
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
    fn test_context() {
        // No context set
        assert_eq!(get_worker_id(), None);
        assert_eq!(get_gvthread_id(), None);
        
        // Set worker
        set_worker_id(5);
        assert_eq!(get_worker_id(), Some(5));
        
        // Set gvthread
        set_gvthread_id(42);
        assert_eq!(get_gvthread_id(), Some(42));
        
        // Clear
        clear_worker_id();
        clear_gvthread_id();
        assert_eq!(get_worker_id(), None);
        assert_eq!(get_gvthread_id(), None);
    }
    
    #[test]
    fn test_elapsed_ns() {
        let t1 = elapsed_ns();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let t2 = elapsed_ns();
        assert!(t2 > t1);
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