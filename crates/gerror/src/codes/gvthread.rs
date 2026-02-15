//! Reserved code namespace for GVThread runtime.
//!
//! These are placeholders — filled during GVThread integration.
//! Other projects should not use codes in the 5000–7999 range.

use crate::GlobalId;

// ── Systems (5000–5099) ───────────────────────────────────────────

pub const SYS_GVT_RUNTIME:  GlobalId = GlobalId::new("gvt_runtime", 5001);
pub const SYS_GVT_KSVC:     GlobalId = GlobalId::new("gvt_ksvc", 5002);
pub const SYS_GVT_NET:      GlobalId = GlobalId::new("gvt_net", 5003);

// ── Subsystems (5100–5199) ────────────────────────────────────────

pub const SUB_GVT_SCHEDULER: GlobalId = GlobalId::new("gvt_scheduler", 5100);
pub const SUB_GVT_WORKER:    GlobalId = GlobalId::new("gvt_worker", 5101);
pub const SUB_GVT_STACK:     GlobalId = GlobalId::new("gvt_stack", 5102);
pub const SUB_GVT_IOURING:   GlobalId = GlobalId::new("gvt_io_uring", 5103);
pub const SUB_GVT_REACTOR:   GlobalId = GlobalId::new("gvt_reactor", 5104);
pub const SUB_GVT_LISTENER:  GlobalId = GlobalId::new("gvt_listener", 5105);
pub const SUB_GVT_STREAM:    GlobalId = GlobalId::new("gvt_stream", 5106);

// ── Error codes (6000–6099) ───────────────────────────────────────

pub const ERR_GVT_RING_SETUP:       GlobalId = GlobalId::new("gvt_ring_setup", 6001);
pub const ERR_GVT_SQE_SUBMIT:       GlobalId = GlobalId::new("gvt_sqe_submit", 6002);
pub const ERR_GVT_CQE_ERROR:        GlobalId = GlobalId::new("gvt_cqe_error", 6003);
pub const ERR_GVT_PROBE_UNSUPPORTED: GlobalId = GlobalId::new("gvt_probe_unsupported", 6004);
pub const ERR_GVT_SPAWN_FAILED:     GlobalId = GlobalId::new("gvt_spawn_failed", 6005);
pub const ERR_GVT_POOL_EXHAUSTED:   GlobalId = GlobalId::new("gvt_pool_exhausted", 6006);
pub const ERR_GVT_STACK_ALLOC:      GlobalId = GlobalId::new("gvt_stack_alloc", 6007);
pub const ERR_GVT_SHUTDOWN:         GlobalId = GlobalId::new("gvt_shutdown", 6008);
