//! Environment variable utilities
//!
//! Generic `env_get<T>` function for parsing environment variables with defaults.
//!
//! # Usage
//!
//! ```ignore
//! use gvthread_core::env::{env_get, env_get_bool};
//!
//! // Get with type inference
//! let workers: usize = env_get("GVT_WORKERS", 4);
//! let timeout: u64 = env_get("GVT_TIMEOUT_MS", 1000);
//! let name: String = env_get("GVT_NAME", "default".to_string());
//!
//! // Boolean helper (accepts "1", "true", "yes", "on")
//! let debug: bool = env_get_bool("GVT_DEBUG", false);
//! ```

use std::str::FromStr;

/// Get environment variable parsed as type T, or return default
///
/// Works with any type that implements `FromStr`.
///
/// # Examples
///
/// ```ignore
/// let count: usize = env_get("COUNT", 10);
/// let ratio: f64 = env_get("RATIO", 0.5);
/// let name: String = env_get("NAME", "default".to_string());
/// ```
#[inline]
pub fn env_get<T>(key: &str, default: T) -> T
where
    T: FromStr,
{
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Get environment variable as boolean
///
/// Accepts: "1", "true", "yes", "on" (case-insensitive) as true.
/// Everything else (including unset) returns the default.
///
/// # Examples
///
/// ```ignore
/// let debug = env_get_bool("DEBUG", false);
/// let verbose = env_get_bool("VERBOSE", true);
/// ```
#[inline]
pub fn env_get_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(val) => matches!(val.to_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}

/// Get environment variable as optional value
///
/// Returns `Some(T)` if the variable is set and parses successfully,
/// `None` otherwise.
///
/// # Examples
///
/// ```ignore
/// let maybe_port: Option<u16> = env_get_opt("PORT");
/// if let Some(port) = maybe_port {
///     println!("Using port {}", port);
/// }
/// ```
#[inline]
pub fn env_get_opt<T>(key: &str) -> Option<T>
where
    T: FromStr,
{
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

/// Get environment variable as string, or return default
///
/// Convenience wrapper that doesn't require `FromStr`.
#[inline]
pub fn env_get_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Check if environment variable is set (regardless of value)
#[inline]
pub fn env_is_set(key: &str) -> bool {
    std::env::var(key).is_ok()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_env_get_default() {
        // Unset variable should return default
        let val: usize = env_get("__TEST_UNSET_VAR_12345__", 42);
        assert_eq!(val, 42);
    }
    
    #[test]
    fn test_env_get_bool_default() {
        let val = env_get_bool("__TEST_UNSET_VAR_12345__", true);
        assert!(val);
        
        let val = env_get_bool("__TEST_UNSET_VAR_12345__", false);
        assert!(!val);
    }
    
    #[test]
    fn test_env_get_opt_none() {
        let val: Option<usize> = env_get_opt("__TEST_UNSET_VAR_12345__");
        assert!(val.is_none());
    }
    
    #[test]
    fn test_env_get_str_default() {
        let val = env_get_str("__TEST_UNSET_VAR_12345__", "hello");
        assert_eq!(val, "hello");
    }
    
    #[test]
    fn test_env_is_set() {
        assert!(!env_is_set("__TEST_UNSET_VAR_12345__"));
        // PATH should always be set
        assert!(env_is_set("PATH"));
    }
    
    #[test]
    fn test_env_get_with_set_var() {
        // Set a test variable
        std::env::set_var("__TEST_VAR_NUM__", "123");
        let val: usize = env_get("__TEST_VAR_NUM__", 0);
        assert_eq!(val, 123);
        std::env::remove_var("__TEST_VAR_NUM__");
    }
    
    #[test]
    fn test_env_get_bool_variants() {
        std::env::set_var("__TEST_BOOL__", "1");
        assert!(env_get_bool("__TEST_BOOL__", false));
        
        std::env::set_var("__TEST_BOOL__", "true");
        assert!(env_get_bool("__TEST_BOOL__", false));
        
        std::env::set_var("__TEST_BOOL__", "TRUE");
        assert!(env_get_bool("__TEST_BOOL__", false));
        
        std::env::set_var("__TEST_BOOL__", "yes");
        assert!(env_get_bool("__TEST_BOOL__", false));
        
        std::env::set_var("__TEST_BOOL__", "on");
        assert!(env_get_bool("__TEST_BOOL__", false));
        
        std::env::set_var("__TEST_BOOL__", "0");
        assert!(!env_get_bool("__TEST_BOOL__", true));
        
        std::env::set_var("__TEST_BOOL__", "false");
        assert!(!env_get_bool("__TEST_BOOL__", true));
        
        std::env::set_var("__TEST_BOOL__", "garbage");
        assert!(!env_get_bool("__TEST_BOOL__", false));
        
        std::env::remove_var("__TEST_BOOL__");
    }
    
    #[test]
    fn test_env_get_invalid_parse() {
        std::env::set_var("__TEST_INVALID__", "not_a_number");
        let val: usize = env_get("__TEST_INVALID__", 99);
        assert_eq!(val, 99); // Should return default on parse failure
        std::env::remove_var("__TEST_INVALID__");
    }
}