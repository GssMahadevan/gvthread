//! Standard operation codes (user codes / verbs).
//!
//! These represent _what the caller was doing_ when the error occurred.
//! Use as the `user_code` field in GError so that the same errno
//! (e.g., EAGAIN) can be distinguished by operation context.
//!
//! Standardizing these across crates means `match_error!` arms like
//! `(_, ERR_EAGAIN, UC_ACCEPT)` work regardless of which crate
//! created the error.

use crate::GlobalId;

// ── File / I/O operations ─────────────────────────────────────────

pub const UC_OPEN:       GlobalId = GlobalId::new("open", 4001);
pub const UC_CLOSE:      GlobalId = GlobalId::new("close", 4002);
pub const UC_READ:       GlobalId = GlobalId::new("read", 4003);
pub const UC_WRITE:      GlobalId = GlobalId::new("write", 4004);
pub const UC_SEEK:       GlobalId = GlobalId::new("seek", 4005);
pub const UC_FLUSH:      GlobalId = GlobalId::new("flush", 4006);
pub const UC_SYNC:       GlobalId = GlobalId::new("sync", 4007);
pub const UC_STAT:       GlobalId = GlobalId::new("stat", 4008);

// ── Network operations ────────────────────────────────────────────

pub const UC_BIND:       GlobalId = GlobalId::new("bind", 4020);
pub const UC_LISTEN:     GlobalId = GlobalId::new("listen", 4021);
pub const UC_ACCEPT:     GlobalId = GlobalId::new("accept", 4022);
pub const UC_CONNECT:    GlobalId = GlobalId::new("connect", 4023);
pub const UC_SEND:       GlobalId = GlobalId::new("send", 4024);
pub const UC_RECV:       GlobalId = GlobalId::new("recv", 4025);
pub const UC_SHUTDOWN:   GlobalId = GlobalId::new("shutdown", 4026);

// ── Process / runtime operations ──────────────────────────────────

pub const UC_SPAWN:      GlobalId = GlobalId::new("spawn", 4040);
pub const UC_JOIN:       GlobalId = GlobalId::new("join", 4041);
pub const UC_CANCEL:     GlobalId = GlobalId::new("cancel", 4042);
pub const UC_PARK:       GlobalId = GlobalId::new("park", 4043);
pub const UC_WAKE:       GlobalId = GlobalId::new("wake", 4044);

// ── I/O subsystem operations ─────────────────────────────────────

pub const UC_SUBMIT:     GlobalId = GlobalId::new("submit", 4060);
pub const UC_POLL:       GlobalId = GlobalId::new("poll", 4061);
pub const UC_COMPLETE:   GlobalId = GlobalId::new("complete", 4062);

// ── Lifecycle operations ──────────────────────────────────────────

pub const UC_INIT:       GlobalId = GlobalId::new("init", 4080);
pub const UC_START:      GlobalId = GlobalId::new("start", 4081);
pub const UC_STOP:       GlobalId = GlobalId::new("stop", 4082);
pub const UC_CONFIGURE:  GlobalId = GlobalId::new("configure", 4083);
pub const UC_ALLOCATE:   GlobalId = GlobalId::new("allocate", 4084);

// ── Data operations ───────────────────────────────────────────────

pub const UC_PARSE:      GlobalId = GlobalId::new("parse", 4100);
pub const UC_SERIALIZE:  GlobalId = GlobalId::new("serialize", 4101);
pub const UC_VALIDATE:   GlobalId = GlobalId::new("validate", 4102);
pub const UC_ENCODE:     GlobalId = GlobalId::new("encode", 4103);
pub const UC_DECODE:     GlobalId = GlobalId::new("decode", 4104);
