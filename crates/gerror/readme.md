# gerror — Generic Error

A zero-dependency, structured error crate for Rust. Errors carry numeric `GlobalId` codes for fast programmatic matching and optional rich diagnostics for debugging.

## Why gerror?

Most Rust error crates force a choice: structured enums (thiserror) or opaque context chains (anyhow). gerror gives you both in one type — a zero-allocation fast path for hot errors like `EAGAIN`, and a boxed diagnostic path for setup/config failures, all behind the same `GError` type and `match_error!` macro.

```
                    ┌─────────────────────────────────┐
                    │            GError               │
                    │  (opaque struct, like io::Error) │
                    ├─────────────┬───────────────────┤
                    │   Simple    │       Full        │
                    │  32 bytes   │   Box<Context>    │
                    │  0 alloc    │   rich diagnostics│
                    │  hot path   │   setup/diag path │
                    └─────────────┴───────────────────┘
```

## Quick Start

### 1. Define your domain codes

Each crate defines its own constants. The naming convention is `SYS_` for systems, `SUB_` for subsystems, `ERR_` for error codes, and `UC_` for user/operation codes.

```rust
use gerror::GlobalId;

// System (which crate/layer)
pub const SYS_NET: GlobalId = GlobalId::new("net", 3);

// Subsystem (which module)
pub const SUB_LISTENER: GlobalId = GlobalId::new("listener", 5);
pub const SUB_STREAM:   GlobalId = GlobalId::new("stream", 6);

// Error codes (what went wrong)
pub const ERR_EAGAIN:   GlobalId = GlobalId::new("eagain", 11);
pub const ERR_BIND:     GlobalId = GlobalId::new("bind_failed", 8);
pub const ERR_CONNRESET: GlobalId = GlobalId::new("conn_reset", 10);

// User codes (what operation was happening)
pub const UC_ACCEPT: GlobalId = GlobalId::new("accept", 1);
pub const UC_LISTEN: GlobalId = GlobalId::new("listen", 2);
pub const UC_READ:   GlobalId = GlobalId::new("read", 3);
```

### 2. Create errors

**Fast path** — zero heap allocation, just three codes on the stack:

```rust
use gerror::{GError, GResult};

fn accept_conn(fd: i32) -> GResult<i32> {
    // io_uring returned EAGAIN during accept
    Err(GError::simple(SYS_NET, ERR_EAGAIN, UC_ACCEPT))
}

// With raw OS errno preserved:
fn read_data(fd: i32) -> GResult<usize> {
    Err(GError::simple_os(SYS_NET, ERR_EAGAIN, UC_READ, 11))
}
```

**Diagnostic path** — full context with message, source chain, location:

```rust
use gerror::err;

fn bind_port(port: u16) -> GResult<()> {
    let addr = format!("0.0.0.0:{}", port);
    match socket_bind(&addr) {
        Ok(()) => Ok(()),
        Err(io_err) => Err(err!(
            SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN,
            "failed to bind port",
            source = io_err
        )),
    }
}
```

### 3. Match errors

The `match_error!` macro matches on the `(system, error_code, user_code)` triple. Use `_` as a wildcard at any position.

```rust
use gerror::match_error;

fn handle_error(err: GError) {
    match_error!(err, {
        // Exact match — EAGAIN during accept
        (SYS_NET, ERR_EAGAIN, UC_ACCEPT) => {
            eprintln!("listener backpressure, will retry");
        },
        // Any EAGAIN on net subsystem
        (SYS_NET, ERR_EAGAIN, _) => {
            eprintln!("net EAGAIN, re-enqueue");
        },
        // Any net error
        (SYS_NET, _, _) => {
            eprintln!("net error: {}", err);
        },
        // Catch-all
        (_, _, _) => {
            eprintln!("unhandled: {}", err);
        },
    });
}
```

### 4. Use the `?` operator

`GError` implements `From<std::io::Error>`, so `?` works seamlessly:

```rust
use gerror::GResult;

fn read_config() -> GResult<String> {
    let content = std::fs::read_to_string("config.toml")?; // auto-converts
    Ok(content)
}
```

For richer context on the conversion, use `ResultExt`:

```rust
use gerror::ResultExt;

fn read_config() -> GResult<String> {
    // Simple context string:
    let content = std::fs::read_to_string("config.toml")
        .gerr_context("reading config file")?;

    // Structured context with codes:
    let parsed = parse(&content)
        .gerr_ctx(SYS_APP, ERR_PARSE, UC_CONFIG, "parsing config")?;

    Ok(parsed)
}
```

## API Reference

### `GlobalId`

A compile-time identifier with a name (stripped in production) and a numeric code.

```rust
const SYS_NET: GlobalId = GlobalId::new("net", 3);

// Sentinel for unset fields
GlobalId::UNSET  // code = 0

// Equality is by code only
GlobalId::new("alpha", 10) == GlobalId::new("beta", 10)  // true
```

### `GError`

The main error type. Two internal representations, one external API.

#### Constructors

| Constructor | Allocation | Use case |
|-------------|-----------|----------|
| `GError::simple(sys, err, uc)` | None | Hot-path errors (EAGAIN, WouldBlock) |
| `GError::simple_os(sys, err, uc, errno)` | None | Syscall failures with raw errno |
| `GError::full(ctx)` | Box | Diagnostic errors with message/source |
| `err!(...)` macro | Box | Diagnostic errors with auto file/line |

#### Accessors

```rust
err.system()      // &GlobalId — which crate/layer
err.error_code()  // &GlobalId — what went wrong
err.user_code()   // &GlobalId — what operation was happening
err.subsystem()   // &GlobalId — which module (UNSET for Simple)
err.kind()        // (&GlobalId, &GlobalId, &GlobalId) — the triple
err.os_error()    // Option<i32> — raw errno if available
err.is_simple()   // bool — true if zero-allocation variant
err.context()     // Option<&ErrorContext> — full context if Full variant
err.source()      // Option<&dyn Error> — underlying cause (std::error::Error)
```

### `ErrorContext`

The full diagnostic payload, used inside `GError::Full`.

```rust
let ctx = ErrorContext {
    system: SYS_NET,
    subsystem: SUB_LISTENER,
    error_code: ERR_BIND,
    user_code: UC_LISTEN,
    os_error: Some(98),
    message: "port 8080 already in use".to_string(),  // stripped in production
    file: file!(),                                     // stripped in production
    line: line!(),                                     // stripped in production
    ..Default::default()
};

// Chain a source error
let ctx = ctx.with_source(io_err);

// Add metadata (lazy-allocated BTreeMap)
let ctx = ctx.with_meta("port", "8080").with_meta("addr", "0.0.0.0");
```

### Macros

#### `err!` — construct a full diagnostic GError

```rust
// Basic
err!(SYS, SUB, ERR, UC, "message")

// With source error
err!(SYS, SUB, ERR, UC, "message", source = some_err)

// With field overrides
err!(SYS, SUB, ERR, UC, "message", {
    os_error: Some(98),
})

// With source + fields
err!(SYS, SUB, ERR, UC, "message", source = some_err, {
    os_error: Some(98),
})
```

All forms auto-capture `file!()` and `line!()`.

#### `match_error!` — match on the (system, error_code, user_code) triple

```rust
match_error!(err, {
    (SYS_NET, ERR_EAGAIN, UC_ACCEPT) => { /* exact */ },
    (SYS_NET, ERR_EAGAIN, _)         => { /* wildcard user_code */ },
    (SYS_NET, _, _)                  => { /* wildcard err + uc */ },
    (_, _, _)                        => { /* catch-all */ },
})
```

Returns the value of the matched arm, so it can be used in expressions.

#### `quick_err!` — shorthand with just system + error_code

```rust
return Err(quick_err!(SYS_NET, ERR_BIND, "port in use"));
// subsystem and user_code default to GlobalId::UNSET
```

#### `ensure!` — early return if condition is false

```rust
ensure!(port > 0, SYS_NET, SUB_LISTENER, ERR_BIND, UC_LISTEN, "invalid port");
// expands to: if !cond { return Err(err!(...)); }
```

### `ResultExt` — context annotation

```rust
use gerror::ResultExt;

// Simple context (uses SYS_IO as default system):
result.gerr_context("while doing X")?;

// Structured context with codes:
result.gerr_ctx(SYS_NET, ERR_BIND, UC_LISTEN, "binding port")?;
```

### Conversions

```rust
// io::Error → GError (auto via From, works with ?)
let gerr: GError = io_err.into();

// GError → io::Error (preserves raw errno when available)
let io_err: io::Error = gerr.into();
```

Raw OS errors use the `Simple` path (zero allocation). Custom `io::Error` values use the `Full` path to preserve the source chain.

### `GResult<T>`

Convenience alias:

```rust
pub type GResult<T> = Result<T, GError>;
```

## Size & Performance

| Mode | `GError` size | `GlobalId` size | Heap allocation |
|------|--------------|----------------|-----------------|
| Debug | 80 bytes | 24 bytes | Only for `Full` variant |
| Production | **32 bytes** | **8 bytes** | Only for `Full` variant |

The `Simple` variant never allocates. Both variants are 16-byte aligned.

Comparison: `std::io::Error` is 8 bytes (always boxed). gerror trades a larger stack footprint (32 bytes) for zero allocation on the hot path — the right tradeoff for runtime libraries where errors like `EAGAIN` are frequent under load.

## Feature Flags

```toml
[features]
default = []
production = []   # strips message, file, line, metadata from ErrorContext
backtrace = []    # captures std::backtrace::Backtrace on err!() construction
```

**Production mode** reduces `GError` from 80 to 32 bytes and eliminates all string allocations for debug fields. Only the numeric `GlobalId` codes, `os_error`, and source chain survive. Errors remain fully matchable — only the human-readable decorations are stripped.

## Organizing Codes Across Crates

A recommended pattern for multi-crate projects:

```
crates/
  gerror/             ← this crate (zero deps)
  my-io-layer/        ← defines SYS_IO, ERR_*, UC_* for I/O ops
  my-runtime/         ← defines SYS_RT, ERR_*, UC_* for scheduler ops
  my-net/             ← defines SYS_NET, ERR_*, UC_* for networking ops
  my-app/             ← uses GResult<T>, matches with match_error!
```

Each crate defines its own constants in a `codes` module:

```rust
// my-net/src/codes.rs
use gerror::GlobalId;

pub const SYS_NET:        GlobalId = GlobalId::new("net", 3);
pub const SUB_LISTENER:   GlobalId = GlobalId::new("listener", 1);
pub const SUB_STREAM:     GlobalId = GlobalId::new("stream", 2);
pub const ERR_BIND:       GlobalId = GlobalId::new("bind_failed", 1);
pub const ERR_ACCEPT:     GlobalId = GlobalId::new("accept_failed", 2);
pub const ERR_CONNRESET:  GlobalId = GlobalId::new("conn_reset", 3);
pub const UC_LISTEN:      GlobalId = GlobalId::new("listen", 1);
pub const UC_ACCEPT:      GlobalId = GlobalId::new("accept", 2);
pub const UC_READ:        GlobalId = GlobalId::new("read", 3);
pub const UC_WRITE:       GlobalId = GlobalId::new("write", 4);
```

The app layer imports codes from whichever crates it needs and matches uniformly:

```rust
use my_net::codes::*;
use my_runtime::codes::*;

fn run() -> GResult<()> {
    let result = start_server()?;
    Ok(result)
}

fn main() {
    if let Err(e) = run() {
        match_error!(e, {
            (SYS_NET, ERR_BIND, _)     => eprintln!("port conflict: {}", e),
            (SYS_RT, ERR_SPAWN, _)     => eprintln!("worker pool full: {}", e),
            (_, _, _)                  => eprintln!("fatal: {}", e),
        });
        std::process::exit(1);
    }
}
```

## Dependencies

**Zero.** By design. No `thiserror`, no `serde`, no `anyhow`. The crate uses only `std`.

Serialization is the consumer's responsibility — if you need to serialize `GError`, implement `serde::Serialize` in your application using the accessor methods.

## License

MIT OR Apache-2.0