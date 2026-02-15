//! POSIX errno values as `GlobalId` constants.
//!
//! Code formula: `2000 + errno_value`. This makes the mapping predictable
//! and debuggable — seeing code `2011` immediately tells you EAGAIN(11).
//!
//! Only the most commonly handled errnos are defined as named constants.
//! For any errno not listed here, use [`errno_to_global_id`] at runtime.

use crate::GlobalId;

// ── Process / permission ──────────────────────────────────────────

pub const ERR_EPERM:           GlobalId = GlobalId::new("EPERM", 2001);
pub const ERR_ENOENT:          GlobalId = GlobalId::new("ENOENT", 2002);
pub const ERR_ESRCH:           GlobalId = GlobalId::new("ESRCH", 2003);
pub const ERR_EINTR:           GlobalId = GlobalId::new("EINTR", 2004);
pub const ERR_EIO:             GlobalId = GlobalId::new("EIO", 2005);
pub const ERR_ENXIO:           GlobalId = GlobalId::new("ENXIO", 2006);
pub const ERR_EACCES:          GlobalId = GlobalId::new("EACCES", 2013);
pub const ERR_EEXIST:          GlobalId = GlobalId::new("EEXIST", 2017);

// ── Memory / resources ────────────────────────────────────────────

pub const ERR_ENOMEM:          GlobalId = GlobalId::new("ENOMEM", 2012);
pub const ERR_EAGAIN:          GlobalId = GlobalId::new("EAGAIN", 2011);
pub const ERR_EWOULDBLOCK:     GlobalId = GlobalId::new("EWOULDBLOCK", 2011); // same as EAGAIN on Linux
pub const ERR_EMFILE:          GlobalId = GlobalId::new("EMFILE", 2024);
pub const ERR_ENFILE:          GlobalId = GlobalId::new("ENFILE", 2023);

// ── I/O ───────────────────────────────────────────────────────────

pub const ERR_EBADF:           GlobalId = GlobalId::new("EBADF", 2009);
pub const ERR_EINVAL:          GlobalId = GlobalId::new("EINVAL", 2022);
pub const ERR_EPIPE:           GlobalId = GlobalId::new("EPIPE", 2032);
pub const ERR_EFBIG:           GlobalId = GlobalId::new("EFBIG", 2027);
pub const ERR_ENOSPC:          GlobalId = GlobalId::new("ENOSPC", 2028);

// ── Networking ────────────────────────────────────────────────────

pub const ERR_EADDRINUSE:      GlobalId = GlobalId::new("EADDRINUSE", 2098);
pub const ERR_EADDRNOTAVAIL:   GlobalId = GlobalId::new("EADDRNOTAVAIL", 2099);
pub const ERR_ENETDOWN:        GlobalId = GlobalId::new("ENETDOWN", 2100);
pub const ERR_ENETUNREACH:     GlobalId = GlobalId::new("ENETUNREACH", 2101);
pub const ERR_ECONNABORTED:    GlobalId = GlobalId::new("ECONNABORTED", 2103);
pub const ERR_ECONNRESET:      GlobalId = GlobalId::new("ECONNRESET", 2104);
pub const ERR_ENOBUFS:         GlobalId = GlobalId::new("ENOBUFS", 2105);
pub const ERR_EISCONN:         GlobalId = GlobalId::new("EISCONN", 2106);
pub const ERR_ENOTCONN:        GlobalId = GlobalId::new("ENOTCONN", 2107);
pub const ERR_ETIMEDOUT:       GlobalId = GlobalId::new("ETIMEDOUT", 2110);
pub const ERR_ECONNREFUSED:    GlobalId = GlobalId::new("ECONNREFUSED", 2111);
pub const ERR_EHOSTUNREACH:    GlobalId = GlobalId::new("EHOSTUNREACH", 2113);
pub const ERR_EALREADY:        GlobalId = GlobalId::new("EALREADY", 2114);
pub const ERR_EINPROGRESS:     GlobalId = GlobalId::new("EINPROGRESS", 2115);

// ── Catch-all ─────────────────────────────────────────────────────

/// Generic / unrecognized OS error.
pub const ERR_EOTHER:          GlobalId = GlobalId::new("EOTHER", 2999);

// ── Runtime helper ────────────────────────────────────────────────

/// Convert a raw errno into a `GlobalId` at runtime.
///
/// Named constants are returned for well-known values.
/// Unknown errnos get a deterministic code (`2000 + errno`) with
/// the name `"errno_<N>"`.
///
/// ```
/// use gerror::codes::errno_to_global_id;
///
/// let id = errno_to_global_id(11);
/// assert_eq!(id.code, 2011); // EAGAIN
/// ```
pub fn errno_to_global_id(errno: i32) -> GlobalId {
    // Return named constant for well-known values
    match errno {
        1   => ERR_EPERM,
        2   => ERR_ENOENT,
        3   => ERR_ESRCH,
        4   => ERR_EINTR,
        5   => ERR_EIO,
        6   => ERR_ENXIO,
        9   => ERR_EBADF,
        11  => ERR_EAGAIN,
        12  => ERR_ENOMEM,
        13  => ERR_EACCES,
        17  => ERR_EEXIST,
        22  => ERR_EINVAL,
        23  => ERR_ENFILE,
        24  => ERR_EMFILE,
        27  => ERR_EFBIG,
        28  => ERR_ENOSPC,
        32  => ERR_EPIPE,
        98  => ERR_EADDRINUSE,
        99  => ERR_EADDRNOTAVAIL,
        100 => ERR_ENETDOWN,
        101 => ERR_ENETUNREACH,
        103 => ERR_ECONNABORTED,
        104 => ERR_ECONNRESET,
        105 => ERR_ENOBUFS,
        106 => ERR_EISCONN,
        107 => ERR_ENOTCONN,
        110 => ERR_ETIMEDOUT,
        111 => ERR_ECONNREFUSED,
        113 => ERR_EHOSTUNREACH,
        114 => ERR_EALREADY,
        115 => ERR_EINPROGRESS,
        _   => GlobalId::new("errno", 2000 + errno as u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eagain_ewouldblock_same_code() {
        assert_eq!(ERR_EAGAIN, ERR_EWOULDBLOCK);
    }

    #[test]
    fn errno_to_id_known() {
        let id = errno_to_global_id(11);
        assert_eq!(id, ERR_EAGAIN);
        assert_eq!(id.code, 2011);
    }

    #[test]
    fn errno_to_id_unknown() {
        let id = errno_to_global_id(255);
        assert_eq!(id.code, 2255);
    }

    #[test]
    fn code_formula_predictable() {
        assert_eq!(ERR_ECONNRESET.code, 2104);  // 2000 + 104
        assert_eq!(ERR_EADDRINUSE.code, 2098);   // 2000 + 98
        assert_eq!(ERR_EPERM.code, 2001);         // 2000 + 1
    }
}
