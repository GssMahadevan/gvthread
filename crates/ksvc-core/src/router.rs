//! Syscall routing abstraction.
//!
//! A `SyscallRouter` maps a Linux syscall number to a tier and,
//! for Tier 1, the corresponding io_uring opcode.
//!
//! # Implementors
//!
//! - `ProbeRouter` (default): at create time, probes io_uring for supported
//!   opcodes via `IORING_REGISTER_PROBE`. Builds the table dynamically.
//!   Syscalls with a supported opcode → Tier 1. Known-delegatable syscalls
//!   without an opcode → Tier 2. Everything else → Tier 3 (Legacy).
//!
//! - `StaticRouter`: compile-time table for a known kernel version.
//!   Zero runtime cost, but won't auto-promote when kernel upgrades.

use crate::tier::Tier;

/// Routing decision for a single syscall.
#[derive(Debug, Clone, Copy)]
pub struct RouteInfo {
    /// Which tier handles this syscall.
    pub tier: Tier,
    /// io_uring opcode (only meaningful when tier == Tier::IoUring).
    /// Stored as u8 matching the IORING_OP_* enum values.
    pub iouring_opcode: u8,
}

impl RouteInfo {
    pub const LEGACY: Self = Self {
        tier: Tier::Legacy,
        iouring_opcode: 0,
    };

    pub const fn iouring(opcode: u8) -> Self {
        Self {
            tier: Tier::IoUring,
            iouring_opcode: opcode,
        }
    }

    pub const fn worker() -> Self {
        Self {
            tier: Tier::WorkerPool,
            iouring_opcode: 0,
        }
    }

    pub const fn shared_page() -> Self {
        Self {
            tier: Tier::SharedPage,
            iouring_opcode: 0,
        }
    }
}

/// Maps syscall numbers to routing decisions.
///
/// Implementations must be cheap to query (O(1) table lookup).
/// The table is built once at KSVC instance creation and never changes.
pub trait SyscallRouter: Send + Sync {
    /// Look up the routing decision for a syscall number.
    ///
    /// Syscall numbers beyond the table size return `RouteInfo::LEGACY`.
    fn route(&self, syscall_nr: u32) -> RouteInfo;

    /// How many syscalls are routable (table size).
    fn table_size(&self) -> usize;

    /// Count of syscalls per tier (for diagnostics/logging).
    fn tier_counts(&self) -> TierCounts {
        let mut counts = TierCounts::default();
        for nr in 0..self.table_size() as u32 {
            match self.route(nr).tier {
                Tier::SharedPage => counts.tier0 += 1,
                Tier::IoUring => counts.tier1 += 1,
                Tier::WorkerPool => counts.tier2 += 1,
                Tier::Legacy => counts.tier3 += 1,
            }
        }
        counts
    }
}

#[derive(Debug, Default)]
pub struct TierCounts {
    pub tier0: usize,
    pub tier1: usize,
    pub tier2: usize,
    pub tier3: usize,
}
