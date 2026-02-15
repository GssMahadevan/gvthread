//! OS-level system identifiers.
//!
//! Use these as the `system` field in GError when the error originates
//! from an OS syscall or platform-specific API.

use crate::GlobalId;

/// Generic POSIX-compatible OS error.
pub const SYS_POSIX:   GlobalId = GlobalId::new("posix", 1000);

/// Linux-specific error.
pub const SYS_LINUX:   GlobalId = GlobalId::new("linux", 1001);

/// macOS-specific error.
pub const SYS_MACOS:   GlobalId = GlobalId::new("macos", 1002);

/// Windows-specific error.
pub const SYS_WINDOWS: GlobalId = GlobalId::new("windows", 1003);

/// FreeBSD-specific error.
pub const SYS_FREEBSD: GlobalId = GlobalId::new("freebsd", 1004);

/// Generic I/O subsystem (used by `From<io::Error>` conversion).
pub const SYS_IO:      GlobalId = GlobalId::new("io", 1010);

/// Generic network subsystem.
pub const SYS_NET:     GlobalId = GlobalId::new("net", 1011);

/// Generic filesystem subsystem.
pub const SYS_FS:      GlobalId = GlobalId::new("fs", 1012);
