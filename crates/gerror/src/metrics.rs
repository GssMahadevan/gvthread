//! Per-site error counters and metadata registry.
//!
//! Enabled with `feature = "metrics"`. Cost per GError creation:
//! one `AtomicU64::fetch_add(1, Relaxed)` — L1-hot for repeated errors.
//!
//! # Architecture
//!
//! ```text
//! GError::simple_site(.., site_id)
//!       │
//!       ▼  counter_index = site_id >> 32
//! COUNTERS[counter_index].fetch_add(1, Relaxed)
//!       │
//!       ▼  Prometheus scrape / bench-runner dump
//! REGISTRY[counter_index] → { subsystem, class, action, ... }
//! ```

use core::sync::atomic::{AtomicU64, Ordering};

use crate::SiteId;

/// Maximum number of error sites. 64K reserved for GVT-Core,
/// rest for user apps. Total counter array = 64K × 8 bytes = 512KB.
pub const MAX_SITES: usize = 65536;

/// Global counter array. Index = `SiteId::counter_index()`.
static COUNTERS: [AtomicU64; MAX_SITES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_SITES]
};

/// Increment the counter for a site. Called from GError constructors.
///
/// Cost: one `fetch_add(1, Relaxed)`. No fence, no CAS loop.
/// Returns the previous count.
#[inline(always)]
pub fn bump(site: SiteId) -> u64 {
    let idx = site.counter_index() as usize;
    if idx > 0 && idx < MAX_SITES {
        COUNTERS[idx].fetch_add(1, Ordering::Relaxed)
    } else {
        0
    }
}

/// Read the current count for a site.
#[inline]
pub fn count(site: SiteId) -> u64 {
    let idx = site.counter_index() as usize;
    if idx < MAX_SITES {
        COUNTERS[idx].load(Ordering::Relaxed)
    } else {
        0
    }
}

/// Reset the counter for a site. Returns the old value.
#[inline]
pub fn reset(site: SiteId) -> u64 {
    let idx = site.counter_index() as usize;
    if idx < MAX_SITES {
        COUNTERS[idx].swap(0, Ordering::Relaxed)
    } else {
        0
    }
}

/// Reset all counters.
pub fn reset_all() {
    for counter in COUNTERS.iter() {
        counter.store(0, Ordering::Relaxed);
    }
}

// ── Registry ──────────────────────────────────────────────────────

/// Metadata for a registered error site.
#[derive(Debug, Clone)]
pub struct SiteInfo {
    pub counter_index: u32,
    pub unique_id: u32,
    pub subsystem: &'static str,
    pub error_class: &'static str,
    pub operation: &'static str,
    pub description: &'static str,
    pub recoverable: bool,
    pub action: &'static str,
}

use std::sync::Mutex;

static REGISTRY: Mutex<Vec<SiteInfo>> = Mutex::new(Vec::new());

/// Register a site's metadata.
pub fn register_site(info: SiteInfo) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.push(info);
    }
}

/// Get a snapshot of all registered sites.
pub fn registered_sites() -> Vec<SiteInfo> {
    REGISTRY.lock().map(|r| r.clone()).unwrap_or_default()
}

/// Get metadata for a specific counter_index.
pub fn site_info(counter_index: u32) -> Option<SiteInfo> {
    let reg = REGISTRY.lock().ok()?;
    reg.iter().find(|s| s.counter_index == counter_index).cloned()
}

// ── Dump ──────────────────────────────────────────────────────────

/// Snapshot of a site's counter + metadata.
#[derive(Debug, Clone)]
pub struct SiteSnapshot {
    pub site_id: SiteId,
    pub count: u64,
    pub info: Option<SiteInfo>,
}

/// Dump all non-zero counters with their registry metadata.
pub fn dump() -> Vec<SiteSnapshot> {
    let reg = REGISTRY.lock().ok();
    let mut result = Vec::new();
    for idx in 1..MAX_SITES {
        let count = COUNTERS[idx].load(Ordering::Relaxed);
        if count > 0 {
            let info = reg.as_ref().and_then(|r| {
                r.iter().find(|s| s.counter_index == idx as u32).cloned()
            });
            result.push(SiteSnapshot {
                site_id: SiteId::new(idx as u32, info.as_ref().map_or(0, |i| i.unique_id)),
                count,
                info,
            });
        }
    }
    result
}

/// Dump all non-zero counters as a formatted string.
pub fn dump_string() -> String {
    let snapshots = dump();
    let mut out = String::new();
    for snap in &snapshots {
        let idx = snap.site_id.counter_index();
        match &snap.info {
            Some(info) => {
                out.push_str(&format!(
                    "[{:>5}] count={:<10} subsys={:<20} class={:<16} op={:<12} recover={}\n",
                    idx, snap.count, info.subsystem, info.error_class,
                    info.operation, info.recoverable
                ));
            }
            None => {
                out.push_str(&format!(
                    "[{:>5}] count={:<10} (unregistered)\n",
                    idx, snap.count
                ));
            }
        }
    }
    out
}

/// Dump counters in OpenMetrics/Prometheus exposition format.
pub fn dump_prometheus() -> String {
    let snapshots = dump();
    let mut out = String::from(
        "# HELP gerror_total Per-site error counter\n\
         # TYPE gerror_total counter\n"
    );
    for snap in &snapshots {
        let idx = snap.site_id.counter_index();
        match &snap.info {
            Some(info) => {
                out.push_str(&format!(
                    "gerror_total{{site=\"{}\",subsys=\"{}\",class=\"{}\",op=\"{}\",recover=\"{}\"}} {}\n",
                    idx, info.subsystem, info.error_class,
                    info.operation, info.recoverable, snap.count
                ));
            }
            None => {
                out.push_str(&format!(
                    "gerror_total{{site=\"{}\"}} {}\n",
                    idx, snap.count
                ));
            }
        }
    }
    out
}

// ── Register macro ────────────────────────────────────────────────

/// Register an error site with metadata and get a `SiteId`.
///
/// ```ignore
/// use gerror::{SiteId, register_error_site};
///
/// const SITE_ACCEPT_EAGAIN: SiteId = register_error_site! {
///     counter_index: 101,
///     unique_id:     1,
///     subsystem:     "net.listener",
///     error_class:   "EAGAIN",
///     operation:     "accept",
///     description:   "accept backpressure under load",
///     recoverable:   true,
///     action:        "retry after yield",
/// };
/// ```
#[macro_export]
macro_rules! register_error_site {
    (
        counter_index: $idx:expr,
        unique_id:     $uid:expr,
        subsystem:     $subsys:expr,
        error_class:   $class:expr,
        operation:     $op:expr,
        description:   $desc:expr,
        recoverable:   $recov:expr,
        action:        $action:expr $(,)?
    ) => {{
        const __SITE: $crate::SiteId = $crate::SiteId::new($idx, $uid);

        #[allow(dead_code)]
        const __SITE_INFO: $crate::metrics::SiteInfo = $crate::metrics::SiteInfo {
            counter_index: $idx,
            unique_id:     $uid,
            subsystem:     $subsys,
            error_class:   $class,
            operation:     $op,
            description:   $desc,
            recoverable:   $recov,
            action:        $action,
        };

        __SITE
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_and_count() {
        let site = SiteId::new(10001, 1);
        let before = count(site);
        bump(site);
        bump(site);
        bump(site);
        assert_eq!(count(site), before + 3);
    }

    #[test]
    fn reset_counter() {
        let site = SiteId::new(10002, 1);
        bump(site);
        bump(site);
        let old = reset(site);
        assert!(old >= 2);
        assert_eq!(count(site), 0);
    }

    #[test]
    fn zero_site_is_noop() {
        let prev = bump(SiteId::NONE);
        assert_eq!(prev, 0);
    }

    #[test]
    fn out_of_bounds_is_noop() {
        let big = SiteId::new(MAX_SITES as u32 + 100, 1);
        let prev = bump(big);
        assert_eq!(prev, 0);
    }

    #[test]
    fn register_and_lookup() {
        register_site(SiteInfo {
            counter_index: 10003,
            unique_id: 42,
            subsystem: "test",
            error_class: "TEST_ERR",
            operation: "testing",
            description: "test error site",
            recoverable: true,
            action: "do nothing",
        });
        let info = site_info(10003);
        assert!(info.is_some());
        assert_eq!(info.unwrap().subsystem, "test");
    }

    #[test]
    fn dump_non_zero() {
        let site = SiteId::new(10004, 99);
        bump(site);
        let snapshots = dump();
        assert!(snapshots.iter().any(|s| s.site_id.counter_index() == 10004));
    }
}
