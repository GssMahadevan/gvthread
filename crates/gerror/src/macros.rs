/// Construct a full diagnostic `GError` with location tracking.
///
/// # Forms
///
/// ```ignore
/// // Basic: system, subsystem, error_code, user_code, message
/// err!(SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN, "port 8080 in use")
///
/// // With source error:
/// err!(SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN, "bind failed", source = io_err)
///
/// // With field overrides:
/// err!(SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN, "bind failed", {
///     os_error: Some(98),
/// })
///
/// // With source + fields:
/// err!(SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN, "bind failed", source = io_err, {
///     os_error: Some(98),
/// })
/// ```
#[macro_export]
macro_rules! err {
    // ── Basic ─────────────────────────────────────────────────
    ($system:expr, $subsystem:expr, $error_code:expr, $user_code:expr, $msg:expr) => {{
        #[allow(unused_mut)]
        let mut ctx = $crate::ErrorContext {
            system: $system,
            subsystem: $subsystem,
            error_code: $error_code,
            user_code: $user_code,

            #[cfg(not(feature = "production"))]
            message: $msg.to_string(),
            #[cfg(not(feature = "production"))]
            file: file!(),
            #[cfg(not(feature = "production"))]
            line: line!(),

            ..Default::default()
        };

        #[cfg(feature = "backtrace")]
        ctx.capture_backtrace();

        $crate::GError::full(ctx)
    }};

    // ── With source error ─────────────────────────────────────
    ($system:expr, $subsystem:expr, $error_code:expr, $user_code:expr, $msg:expr,
     source = $source:expr) => {{
        #[allow(unused_mut)]
        let mut ctx = $crate::ErrorContext {
            system: $system,
            subsystem: $subsystem,
            error_code: $error_code,
            user_code: $user_code,

            #[cfg(not(feature = "production"))]
            message: $msg.to_string(),
            #[cfg(not(feature = "production"))]
            file: file!(),
            #[cfg(not(feature = "production"))]
            line: line!(),

            ..Default::default()
        }
        .with_source($source);

        #[cfg(feature = "backtrace")]
        ctx.capture_backtrace();

        $crate::GError::full(ctx)
    }};

    // ── With field overrides ──────────────────────────────────
    ($system:expr, $subsystem:expr, $error_code:expr, $user_code:expr, $msg:expr,
     { $($field:ident : $value:expr),* $(,)? }) => {{
        #[allow(unused_mut)]
        let mut ctx = $crate::ErrorContext {
            system: $system,
            subsystem: $subsystem,
            error_code: $error_code,
            user_code: $user_code,

            #[cfg(not(feature = "production"))]
            message: $msg.to_string(),
            #[cfg(not(feature = "production"))]
            file: file!(),
            #[cfg(not(feature = "production"))]
            line: line!(),

            ..Default::default()
        };

        $( ctx.$field = $value; )*

        #[cfg(feature = "backtrace")]
        ctx.capture_backtrace();

        $crate::GError::full(ctx)
    }};

    // ── With source + field overrides ─────────────────────────
    ($system:expr, $subsystem:expr, $error_code:expr, $user_code:expr, $msg:expr,
     source = $source:expr, { $($field:ident : $value:expr),* $(,)? }) => {{
        #[allow(unused_mut)]
        let mut ctx = $crate::ErrorContext {
            system: $system,
            subsystem: $subsystem,
            error_code: $error_code,
            user_code: $user_code,

            #[cfg(not(feature = "production"))]
            message: $msg.to_string(),
            #[cfg(not(feature = "production"))]
            file: file!(),
            #[cfg(not(feature = "production"))]
            line: line!(),

            ..Default::default()
        }
        .with_source($source);

        $( ctx.$field = $value; )*

        #[cfg(feature = "backtrace")]
        ctx.capture_backtrace();

        $crate::GError::full(ctx)
    }};
}

/// Match on a `GError`'s `(system, error_code, user_code)` triple.
///
/// ```ignore
/// match_error!(err, {
///     (SYS_NET, ERR_EAGAIN, UC_ACCEPT) => { /* backoff on listener */ },
///     (SYS_NET, ERR_EAGAIN, _)         => { /* any EAGAIN on net */ },
///     (SYS_KSVC, _, _)                 => { /* any ksvc error */ },
///     _                                => { /* fallback */ },
/// })
/// ```
///
/// Each arm's identifiers are compared by `.code` (u64) against the error's
/// GlobalIds. Use `_` as a wildcard for any position.
#[macro_export]
macro_rules! match_error {
    ($error:expr, {
        $( ($sys:tt, $err:tt, $uc:tt) => $handler:expr ),*
        $(,)*
    }) => {{
        let __e = &$error;
        let (__sys, __err, __uc) = __e.kind();
        $crate::__match_error_arms!(__sys, __err, __uc; $( ($sys, $err, $uc) => $handler ),* )
    }};
}

/// Internal helper for match_error! — handles wildcards.
#[doc(hidden)]
#[macro_export]
macro_rules! __match_error_arms {
    // Terminal: no arms left → unreachable / default
    ($sys:ident, $err:ident, $uc:ident; ) => {
        unreachable!("unhandled GError: {:?}", ($sys, $err, $uc))
    };

    // Wildcard-all arm: _
    ($sys:ident, $err:ident, $uc:ident;
     (_, _, _) => $handler:expr
     $(, ($sys2:tt, $err2:tt, $uc2:tt) => $handler2:expr)*
    ) => {
        $handler
    };

    // (SYS, _, _) — match system only
    ($sys:ident, $err:ident, $uc:ident;
     ($s:expr, _, _) => $handler:expr
     $(, ($sys2:tt, $err2:tt, $uc2:tt) => $handler2:expr)*
    ) => {
        if $sys.code == $s.code {
            $handler
        } else {
            $crate::__match_error_arms!($sys, $err, $uc; $( ($sys2, $err2, $uc2) => $handler2 ),* )
        }
    };

    // (SYS, ERR, _) — match system + error_code
    ($sys:ident, $err:ident, $uc:ident;
     ($s:expr, $e:expr, _) => $handler:expr
     $(, ($sys2:tt, $err2:tt, $uc2:tt) => $handler2:expr)*
    ) => {
        if $sys.code == $s.code && $err.code == $e.code {
            $handler
        } else {
            $crate::__match_error_arms!($sys, $err, $uc; $( ($sys2, $err2, $uc2) => $handler2 ),* )
        }
    };

    // (SYS, ERR, UC) — match all three
    ($sys:ident, $err:ident, $uc:ident;
     ($s:expr, $e:expr, $u:expr) => $handler:expr
     $(, ($sys2:tt, $err2:tt, $uc2:tt) => $handler2:expr)*
    ) => {
        if $sys.code == $s.code && $err.code == $e.code && $uc.code == $u.code {
            $handler
        } else {
            $crate::__match_error_arms!($sys, $err, $uc; $( ($sys2, $err2, $uc2) => $handler2 ),* )
        }
    };
}

/// Quick fire-and-forget error with just system + error_code.
///
/// ```ignore
/// return Err(quick_err!(SYS_NET, ERR_BIND, "port in use"));
/// ```
#[macro_export]
macro_rules! quick_err {
    ($system:expr, $error_code:expr, $msg:expr) => {
        $crate::err!(
            $system,
            $crate::GlobalId::UNSET,
            $error_code,
            $crate::GlobalId::UNSET,
            $msg
        )
    };
}

/// Early-return if a condition is false.
///
/// ```ignore
/// ensure!(user_id > 0, SYS_APP, SUB_AUTH, ERR_INVALID, UC_LOGIN, "bad user id");
/// ```
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $system:expr, $subsystem:expr, $error_code:expr, $user_code:expr, $msg:expr) => {
        if !$cond {
            return Err($crate::err!($system, $subsystem, $error_code, $user_code, $msg));
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::{GError, GlobalId, GResult};

    const SYS_NET: GlobalId = GlobalId::new("net", 3);
    const SYS_RT: GlobalId = GlobalId::new("runtime", 2);
    const SUB_LISTENER: GlobalId = GlobalId::new("listener", 5);
    const SUB_SCHED: GlobalId = GlobalId::new("scheduler", 2);
    const ERR_BIND: GlobalId = GlobalId::new("bind_failed", 8);
    const ERR_EAGAIN: GlobalId = GlobalId::new("eagain", 11);
    const ERR_SPAWN: GlobalId = GlobalId::new("spawn_failed", 5);
    const UC_LISTEN: GlobalId = GlobalId::new("listen", 2);
    const UC_ACCEPT: GlobalId = GlobalId::new("accept", 1);
    const UC_CREATE: GlobalId = GlobalId::new("create", 3);

    #[test]
    fn err_basic() {
        let e = err!(SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN, "port 8080 in use");
        assert_eq!(e.system(), &SYS_NET);
        assert_eq!(e.subsystem(), &SUB_LISTENER);
        assert_eq!(e.error_code(), &ERR_BIND);
        assert_eq!(e.user_code(), &UC_LISTEN);
        assert!(!e.is_simple());
    }

    #[test]
    fn err_with_source() {
        let io_err = std::io::Error::new(std::io::ErrorKind::AddrInUse, "taken");
        let e = err!(SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN, "bind failed",
                     source = io_err);
        assert!(e.source().is_some());
    }

    #[test]
    fn err_with_fields() {
        let e = err!(SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN, "bind failed", {
            os_error: Some(98),
        });
        assert_eq!(e.os_error(), Some(98));
    }

    #[test]
    fn err_with_source_and_fields() {
        let io_err = std::io::Error::new(std::io::ErrorKind::AddrInUse, "taken");
        let e = err!(SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN, "bind failed",
                     source = io_err, {
            os_error: Some(98),
        });
        assert!(e.source().is_some());
        assert_eq!(e.os_error(), Some(98));
    }

    #[test]
    fn quick_err_macro() {
        let e = quick_err!(SYS_NET, ERR_BIND, "port in use");
        assert_eq!(e.system(), &SYS_NET);
        assert_eq!(e.error_code(), &ERR_BIND);
        assert_eq!(e.subsystem(), &GlobalId::UNSET);
    }

    #[test]
    fn ensure_passes() {
        fn check(val: i32) -> GResult<()> {
            ensure!(val > 0, SYS_RT, SUB_SCHED, ERR_SPAWN, UC_CREATE, "bad value");
            Ok(())
        }
        assert!(check(5).is_ok());
    }

    #[test]
    fn ensure_fails() {
        fn check(val: i32) -> GResult<()> {
            ensure!(val > 0, SYS_RT, SUB_SCHED, ERR_SPAWN, UC_CREATE, "bad value");
            Ok(())
        }
        let result = check(-1);
        assert!(result.is_err());
        let e = result.unwrap_err();
        assert_eq!(e.system(), &SYS_RT);
        assert_eq!(e.error_code(), &ERR_SPAWN);
    }

    #[test]
    fn match_error_exact() {
        let e = GError::simple(SYS_NET, ERR_EAGAIN, UC_ACCEPT);
        let result = match_error!(e, {
            (SYS_NET, ERR_EAGAIN, UC_ACCEPT) => "accept_eagain",
            (SYS_NET, ERR_EAGAIN, _) => "net_eagain",
            (_, _, _) => "other",
        });
        assert_eq!(result, "accept_eagain");
    }

    #[test]
    fn match_error_wildcard_uc() {
        let e = GError::simple(SYS_NET, ERR_EAGAIN, UC_LISTEN);
        let result = match_error!(e, {
            (SYS_NET, ERR_EAGAIN, UC_ACCEPT) => "accept",
            (SYS_NET, ERR_EAGAIN, _) => "net_eagain_any",
            (_, _, _) => "other",
        });
        assert_eq!(result, "net_eagain_any");
    }

    #[test]
    fn match_error_wildcard_system() {
        let e = GError::simple(SYS_RT, ERR_SPAWN, UC_CREATE);
        let result = match_error!(e, {
            (SYS_NET, _, _) => "net",
            (SYS_RT, _, _) => "runtime_any",
            (_, _, _) => "other",
        });
        assert_eq!(result, "runtime_any");
    }

    #[test]
    fn match_error_catchall() {
        let e = GError::simple(SYS_RT, ERR_SPAWN, UC_CREATE);
        let result = match_error!(e, {
            (SYS_NET, ERR_EAGAIN, UC_ACCEPT) => "specific",
            (_, _, _) => "fallback",
        });
        assert_eq!(result, "fallback");
    }

    // Needed for source() in test
    use std::error::Error;
}
