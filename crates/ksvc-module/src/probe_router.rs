//! `ProbeRouter` — default `SyscallRouter` implementation.
//!
//! At creation time, probes io_uring via `IORING_REGISTER_PROBE` to discover
//! which opcodes the running kernel supports. Builds the routing table:
//!   - Syscall has matching io_uring opcode AND opcode is probed-supported → Tier 1
//!   - Syscall is delegatable but no opcode → Tier 2
//!   - Syscall is process-altering / undelegatable → Tier 3 (Legacy)
//!   - Syscall is a Tier 0 identity read → SharedPage
//!
//! The table is a flat array indexed by `__NR_*` syscall number.
//! Lookup is O(1): one array index.

use ksvc_core::router::{RouteInfo, SyscallRouter, TierCounts};
use ksvc_core::tier::Tier;

/// Maximum syscall number we track. Linux x86_64 has ~450 syscalls.
const TABLE_SIZE: usize = 512;

/// The "candidate" mapping: syscall_nr → (io_uring opcode, is this Tier 2 if no opcode?).
/// Built at compile time. The probe step then filters by what's actually supported.
struct Candidate {
    syscall_nr: u32,
    iouring_opcode: u8,   // 0 = no opcode exists → Tier 2 candidate
    tier2_fallback: bool, // true = use Tier 2 if opcode missing or unsupported
}

// ── io_uring opcode constants (from linux/io_uring.h) ──
// We define them here to avoid pulling in kernel headers.
pub mod op {
    pub const NOP: u8 = 0;
    pub const READV: u8 = 1;
    pub const WRITEV: u8 = 2;
    pub const FSYNC: u8 = 3;
    pub const READ_FIXED: u8 = 4;
    pub const WRITE_FIXED: u8 = 5;
    pub const POLL_ADD: u8 = 6;
    pub const POLL_REMOVE: u8 = 7;
    pub const SYNC_FILE_RANGE: u8 = 8;
    pub const SENDMSG: u8 = 9;
    pub const RECVMSG: u8 = 10;
    pub const TIMEOUT: u8 = 11;
    pub const TIMEOUT_REMOVE: u8 = 12;
    pub const ACCEPT: u8 = 13;
    pub const ASYNC_CANCEL: u8 = 14;
    pub const LINK_TIMEOUT: u8 = 15;
    pub const CONNECT: u8 = 16;
    pub const FALLOCATE: u8 = 17;
    pub const OPENAT: u8 = 18;
    pub const CLOSE: u8 = 19;
    pub const FILES_UPDATE: u8 = 20;
    pub const STATX: u8 = 21;
    pub const READ: u8 = 22;
    pub const WRITE: u8 = 23;
    pub const FADVISE: u8 = 24;
    pub const MADVISE: u8 = 25;
    pub const SEND: u8 = 26;
    pub const RECV: u8 = 27;
    pub const OPENAT2: u8 = 28;
    pub const EPOLL_CTL: u8 = 29;
    pub const SPLICE: u8 = 30;
    pub const PROVIDE_BUFFERS: u8 = 31;
    pub const REMOVE_BUFFERS: u8 = 32;
    pub const TEE: u8 = 33;
    pub const SHUTDOWN: u8 = 34;
    pub const RENAMEAT: u8 = 35;
    pub const UNLINKAT: u8 = 36;
    pub const MKDIRAT: u8 = 37;
    pub const SYMLINKAT: u8 = 38;
    pub const LINKAT: u8 = 39;
    // 5.18+
    pub const MSG_RING: u8 = 40;
    pub const FSETXATTR: u8 = 41;
    pub const SETXATTR: u8 = 42;
    pub const FGETXATTR: u8 = 43;
    pub const GETXATTR: u8 = 44;
    // 5.19+
    pub const SOCKET: u8 = 45;
    pub const URING_CMD: u8 = 46;
    pub const SEND_ZC: u8 = 47;
    // 6.1+
    pub const SENDMSG_ZC: u8 = 48;
    // 6.5+
    pub const WAITID: u8 = 53;
    // 6.7+
    pub const FUTEX_WAIT: u8 = 54;
    pub const FUTEX_WAKE: u8 = 55;
    pub const FUTEX_WAITV: u8 = 56;
    // 6.8+
    pub const FIXED_FD_INSTALL: u8 = 57;
    pub const FTRUNCATE: u8 = 58;
    // 6.11+
    pub const BIND: u8 = 59;
    pub const LISTEN: u8 = 60;
}

// ── Linux x86_64 syscall numbers (from asm/unistd_64.h) ──
mod nr {
    pub const READ: u32 = 0;
    pub const WRITE: u32 = 1;
    pub const OPEN: u32 = 2;
    pub const CLOSE: u32 = 3;
    pub const STAT: u32 = 4;
    pub const FSTAT: u32 = 5;
    pub const LSTAT: u32 = 6;
    pub const POLL: u32 = 7;
    pub const LSEEK: u32 = 8;
    pub const MMAP: u32 = 9;
    pub const MPROTECT: u32 = 10;
    pub const MUNMAP: u32 = 11;
    pub const BRK: u32 = 12;
    pub const IOCTL: u32 = 16;
    pub const PREAD64: u32 = 17;
    pub const PWRITE64: u32 = 18;
    pub const READV: u32 = 19;
    pub const WRITEV: u32 = 20;
    pub const ACCESS: u32 = 21;
    pub const PIPE: u32 = 22;
    pub const DUP: u32 = 32;
    pub const DUP2: u32 = 33;
    pub const NANOSLEEP: u32 = 35;
    pub const GETPID: u32 = 39;
    pub const SOCKET: u32 = 41;
    pub const CONNECT: u32 = 42;
    pub const ACCEPT: u32 = 43;
    pub const SENDTO: u32 = 44;
    pub const RECVFROM: u32 = 45;
    pub const SENDMSG: u32 = 46;
    pub const RECVMSG: u32 = 47;
    pub const SHUTDOWN: u32 = 48;
    pub const BIND: u32 = 49;
    pub const LISTEN: u32 = 50;
    pub const GETSOCKNAME: u32 = 51;
    pub const GETPEERNAME: u32 = 52;
    pub const SETSOCKOPT: u32 = 54;
    pub const GETSOCKOPT: u32 = 55;
    pub const CLONE: u32 = 56;
    pub const FORK: u32 = 57;
    pub const VFORK: u32 = 58;
    pub const EXECVE: u32 = 59;
    pub const EXIT: u32 = 60;
    pub const WAIT4: u32 = 61;
    pub const KILL: u32 = 62;
    pub const UNAME: u32 = 63;
    pub const FCNTL: u32 = 72;
    pub const FLOCK: u32 = 73;
    pub const FSYNC: u32 = 74;
    pub const FDATASYNC: u32 = 75;
    pub const FTRUNCATE: u32 = 77;
    pub const GETDENTS: u32 = 78;
    pub const GETCWD: u32 = 79;
    pub const CHDIR: u32 = 80;
    pub const FCHDIR: u32 = 81;
    pub const RENAME: u32 = 82;
    pub const MKDIR: u32 = 83;
    pub const RMDIR: u32 = 84;
    pub const LINK: u32 = 86;
    pub const UNLINK: u32 = 87;
    pub const SYMLINK: u32 = 88;
    pub const READLINK: u32 = 89;
    pub const CHMOD: u32 = 90;
    pub const FCHMOD: u32 = 91;
    pub const CHOWN: u32 = 92;
    pub const FCHOWN: u32 = 93;
    pub const UMASK: u32 = 95;
    pub const GETUID: u32 = 102;
    pub const SYSLOG: u32 = 103;
    pub const GETGID: u32 = 104;
    pub const SETUID: u32 = 105;
    pub const SETGID: u32 = 106;
    pub const GETEUID: u32 = 107;
    pub const GETEGID: u32 = 108;
    pub const SETPGID: u32 = 109;
    pub const GETPPID: u32 = 110;
    pub const GETPGRP: u32 = 111;
    pub const SETSID: u32 = 112;
    pub const GETGROUPS: u32 = 115;
    pub const SIGALTSTACK: u32 = 131;
    pub const MLOCK: u32 = 149;
    pub const MUNLOCK: u32 = 150;
    pub const PRCTL: u32 = 157;
    pub const ARCH_PRCTL: u32 = 158;
    pub const SYNC_FILE_RANGE: u32 = 277;
    pub const SPLICE: u32 = 275;
    pub const TEE: u32 = 276;
    pub const FALLOCATE: u32 = 285;
    pub const ACCEPT4: u32 = 288;
    pub const DUP3: u32 = 292;
    pub const PIPE2: u32 = 293;
    pub const PREADV: u32 = 295;
    pub const PWRITEV: u32 = 296;
    pub const SENDMMSG: u32 = 307;
    pub const RENAMEAT2: u32 = 316;
    pub const GETRANDOM: u32 = 318;
    pub const STATX: u32 = 332;
    pub const CLONE3: u32 = 435;
    pub const OPENAT: u32 = 257;
    pub const MKDIRAT: u32 = 258;
    pub const FCHOWNAT: u32 = 260;
    pub const UNLINKAT: u32 = 263;
    pub const RENAMEAT: u32 = 264;
    pub const LINKAT: u32 = 265;
    pub const SYMLINKAT: u32 = 266;
    pub const READLINKAT: u32 = 267;
    pub const FCHMODAT: u32 = 268;
    pub const FACCESSAT: u32 = 269;
    pub const UTIMENSAT: u32 = 280;
    pub const OPENAT2: u32 = 437;
    pub const GETDENTS64: u32 = 217;
    pub const FADVISE64: u32 = 221;
    pub const MADVISE: u32 = 28;
    pub const GETSID: u32 = 124;
    pub const EXIT_GROUP: u32 = 231;
    pub const EPOLL_CTL: u32 = 233;
    pub const TGKILL: u32 = 234;
    pub const WAITID: u32 = 247;
    pub const FSETXATTR: u32 = 190;
    pub const FGETXATTR: u32 = 193;
    pub const SETXATTR: u32 = 188;
    pub const GETXATTR: u32 = 191;
    pub const SET_TID_ADDRESS: u32 = 218;
    pub const SET_ROBUST_LIST: u32 = 273;
    pub const GETRLIMIT: u32 = 97;
    pub const SETRLIMIT: u32 = 160;
    pub const PRLIMIT64: u32 = 302;
}

pub struct ProbeRouter {
    table: [RouteInfo; TABLE_SIZE],
}

impl ProbeRouter {
    /// Build routing table from a set of probed io_uring opcodes.
    ///
    /// `supported_opcodes` is a slice of IORING_OP_* values that the
    /// running kernel's io_uring supports (from `IORING_REGISTER_PROBE`).
    pub fn new(supported_opcodes: &[u8]) -> Self {
        let mut table = [RouteInfo::LEGACY; TABLE_SIZE];

        // ── Tier 0: Shared page (never reaches ring) ──
        let tier0 = [
            nr::GETPID, nr::GETPPID, nr::GETUID, nr::GETGID,
            nr::GETEUID, nr::GETEGID, nr::GETSID, nr::UNAME,
            nr::GETPGRP, nr::GETRLIMIT,
        ];
        for &s in &tier0 {
            table[s as usize] = RouteInfo::shared_page();
        }

        // ── Tier 1 candidates: syscall → io_uring opcode ──
        // Each entry: (syscall_nr, opcode, fallback_to_tier2_if_unsupported)
        let tier1_candidates: &[(u32, u8, bool)] = &[
            // File I/O
            (nr::READ,           op::READ,       false),
            (nr::WRITE,          op::WRITE,      false),
            (nr::PREAD64,        op::READ,       false),
            (nr::PWRITE64,       op::WRITE,      false),
            (nr::READV,          op::READV,      false),
            (nr::WRITEV,         op::WRITEV,     false),
            (nr::PREADV,         op::READV,      false),
            (nr::PWRITEV,        op::WRITEV,     false),
            // File lifecycle
            (nr::OPENAT,         op::OPENAT,     false),
            (nr::OPENAT2,        op::OPENAT2,    false),
            (nr::CLOSE,          op::CLOSE,      false),
            (nr::STATX,          op::STATX,      false),
            (nr::FALLOCATE,      op::FALLOCATE,  false),
            (nr::FTRUNCATE,      op::FTRUNCATE,  false),
            // Sync
            (nr::FSYNC,          op::FSYNC,      false),
            (nr::FDATASYNC,      op::FSYNC,      false), // DATASYNC flag variant
            (nr::SYNC_FILE_RANGE,op::SYNC_FILE_RANGE, false),
            // Metadata
            (nr::RENAMEAT2,      op::RENAMEAT,   false),
            (nr::RENAMEAT,       op::RENAMEAT,   false),
            (nr::UNLINKAT,       op::UNLINKAT,   false),
            (nr::MKDIRAT,        op::MKDIRAT,    false),
            (nr::SYMLINKAT,      op::SYMLINKAT,  false),
            (nr::LINKAT,         op::LINKAT,     false),
            (nr::FADVISE64,      op::FADVISE,    false),
            (nr::MADVISE,        op::MADVISE,    false),
            // xattr (6.0+)
            (nr::SETXATTR,       op::SETXATTR,   true),
            (nr::GETXATTR,       op::GETXATTR,   true),
            (nr::FSETXATTR,      op::FSETXATTR,  true),
            (nr::FGETXATTR,      op::FGETXATTR,  true),
            // Network
            (nr::ACCEPT4,        op::ACCEPT,     false),
            (nr::CONNECT,        op::CONNECT,    false),
            (nr::SENDTO,         op::SEND,       false),
            (nr::RECVFROM,       op::RECV,       false),
            (nr::SENDMSG,        op::SENDMSG,    false),
            (nr::RECVMSG,        op::RECVMSG,    false),
            (nr::SHUTDOWN,       op::SHUTDOWN,   false),
            (nr::SOCKET,         op::SOCKET,     true), // 5.19+
            // Network setup (6.11+, Tier 2 fallback on 6.8)
            (nr::BIND,           op::BIND,       true),
            (nr::LISTEN,         op::LISTEN,     true),
            // Splice
            (nr::SPLICE,         op::SPLICE,     false),
            (nr::TEE,            op::TEE,        false),
            // Process sync
            (nr::WAITID,         op::WAITID,     true), // 6.5+
            // epoll
            (nr::EPOLL_CTL,      op::EPOLL_CTL,  false),
        ];

        // Build a lookup set for O(1) probe checking
        let mut opcode_supported = [false; 256];
        for &opc in supported_opcodes {
            opcode_supported[opc as usize] = true;
        }

        for &(syscall_nr, opcode, tier2_fallback) in tier1_candidates {
            if opcode_supported[opcode as usize] {
                table[syscall_nr as usize] = RouteInfo::iouring(opcode);
            } else if tier2_fallback {
                table[syscall_nr as usize] = RouteInfo::worker();
            }
            // else: stays LEGACY (the default)
        }

        // ── Tier 2: always worker pool (no io_uring opcode in any kernel) ──
        let tier2_always: &[u32] = &[
            nr::DUP, nr::DUP2, nr::DUP3,
            nr::FCNTL,
            nr::IOCTL,
            nr::LSEEK,
            nr::SETSOCKOPT, nr::GETSOCKOPT,
            nr::GETSOCKNAME, nr::GETPEERNAME,
            nr::GETDENTS64,
            nr::ACCESS, nr::FACCESSAT,
            nr::FCHMOD, nr::FCHMODAT,
            nr::FCHOWN, nr::FCHOWNAT,
            nr::UTIMENSAT,
            nr::FLOCK,
            nr::READLINKAT, nr::READLINK,
            nr::PIPE2,
            nr::GETCWD,
            nr::GETRANDOM,
        ];
        for &s in tier2_always {
            // Only set if not already promoted to Tier 1 by probe
            if table[s as usize].tier == Tier::Legacy {
                table[s as usize] = RouteInfo::worker();
            }
        }

        // ── Legacy syscalls mapped to newer *at() variants ──
        // Old-style open/rename/unlink etc. → route same as their *at version
        // if the at version is Tier 1. These old syscalls get translated
        // in the dispatcher (e.g., open → openat with AT_FDCWD).
        let legacy_to_at: &[(u32, u32)] = &[
            (nr::OPEN,    nr::OPENAT),
            (nr::RENAME,  nr::RENAMEAT),
            (nr::UNLINK,  nr::UNLINKAT),
            (nr::MKDIR,   nr::MKDIRAT),
            (nr::SYMLINK, nr::SYMLINKAT),
            (nr::LINK,    nr::LINKAT),
            (nr::RMDIR,   nr::UNLINKAT), // rmdir → unlinkat(AT_REMOVEDIR)
        ];
        for &(old, at_ver) in legacy_to_at {
            table[old as usize] = table[at_ver as usize];
        }

        // stat/fstat/lstat → statx
        table[nr::STAT as usize] = table[nr::STATX as usize];
        table[nr::FSTAT as usize] = table[nr::STATX as usize];
        table[nr::LSTAT as usize] = table[nr::STATX as usize];

        ProbeRouter { table }
    }

    /// Convenience: create with ALL opcodes supported (for testing
    /// against a "latest kernel" scenario).
    pub fn all_opcodes() -> Self {
        let all: Vec<u8> = (0..=80).collect();
        Self::new(&all)
    }

    /// Convenience: create with 6.8 GA baseline opcodes.
    pub fn kernel_6_8() -> Self {
        let opcodes: Vec<u8> = (0..=op::FTRUNCATE).collect();
        Self::new(&opcodes)
    }
}

impl SyscallRouter for ProbeRouter {
    fn route(&self, syscall_nr: u32) -> RouteInfo {
        if (syscall_nr as usize) < TABLE_SIZE {
            self.table[syscall_nr as usize]
        } else {
            RouteInfo::LEGACY
        }
    }

    fn table_size(&self) -> usize {
        TABLE_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier0_getpid_is_shared_page() {
        let router = ProbeRouter::kernel_6_8();
        assert_eq!(router.route(nr::GETPID).tier, Tier::SharedPage);
        assert_eq!(router.route(nr::GETUID).tier, Tier::SharedPage);
        assert_eq!(router.route(nr::GETEUID).tier, Tier::SharedPage);
    }

    #[test]
    fn tier1_read_write_on_6_8() {
        let router = ProbeRouter::kernel_6_8();
        assert_eq!(router.route(nr::READ).tier, Tier::IoUring);
        assert_eq!(router.route(nr::WRITE).tier, Tier::IoUring);
        assert_eq!(router.route(nr::OPENAT).tier, Tier::IoUring);
        assert_eq!(router.route(nr::CLOSE).tier, Tier::IoUring);
        assert_eq!(router.route(nr::ACCEPT4).tier, Tier::IoUring);
    }

    #[test]
    fn tier2_dup_always_worker() {
        let router = ProbeRouter::kernel_6_8();
        assert_eq!(router.route(nr::DUP).tier, Tier::WorkerPool);
        assert_eq!(router.route(nr::FCNTL).tier, Tier::WorkerPool);
        assert_eq!(router.route(nr::IOCTL).tier, Tier::WorkerPool);
        assert_eq!(router.route(nr::LSEEK).tier, Tier::WorkerPool);
    }

    #[test]
    fn bind_listen_tier2_on_6_8_tier1_on_6_11() {
        // 6.8: bind/listen opcodes NOT supported → Tier 2 fallback
        let router_68 = ProbeRouter::kernel_6_8();
        assert_eq!(router_68.route(nr::BIND).tier, Tier::WorkerPool);
        assert_eq!(router_68.route(nr::LISTEN).tier, Tier::WorkerPool);

        // 6.11+: bind/listen opcodes supported → auto-promoted to Tier 1
        let router_611 = ProbeRouter::all_opcodes();
        assert_eq!(router_611.route(nr::BIND).tier, Tier::IoUring);
        assert_eq!(router_611.route(nr::LISTEN).tier, Tier::IoUring);
    }

    #[test]
    fn tier3_fork_exec_mmap() {
        let router = ProbeRouter::kernel_6_8();
        assert_eq!(router.route(nr::FORK).tier, Tier::Legacy);
        assert_eq!(router.route(nr::EXECVE).tier, Tier::Legacy);
        assert_eq!(router.route(nr::MMAP).tier, Tier::Legacy);
        assert_eq!(router.route(nr::CLONE).tier, Tier::Legacy);
        assert_eq!(router.route(nr::EXIT).tier, Tier::Legacy);
    }

    #[test]
    fn legacy_open_maps_to_openat() {
        let router = ProbeRouter::kernel_6_8();
        let r = router.route(nr::OPEN);
        assert_eq!(r.tier, Tier::IoUring);
        assert_eq!(r.iouring_opcode, op::OPENAT);
    }

    #[test]
    fn tier_counts_reasonable() {
        let router = ProbeRouter::kernel_6_8();
        let counts = router.tier_counts();
        assert!(counts.tier0 >= 8, "should have ≥8 Tier 0 syscalls");
        assert!(counts.tier1 >= 30, "should have ≥30 Tier 1 syscalls");
        assert!(counts.tier2 >= 10, "should have ≥10 Tier 2 syscalls");
    }
}
