# ksvc-gvthread Integration Guide

## Extract into your gvthread repo

```bash
cd ~/gvthread
tar xzf ksvc-gvthread-patch.tar.gz
```

This creates/modifies:
```
crates/ksvc-gvthread/              ← NEW (bridge crate: 4 .rs files)
cmd/httpd/rust/gvthread/           ← NEW (GVThread HTTP server)
docs/KSVC_GVTHREAD.md             ← NEW (architecture doc)
Cargo.toml                         ← MODIFIED (added ksvc-gvthread dep)
```

## Verify Cargo.toml changes

The `Cargo.toml` workspace should now have:

```toml
[workspace.dependencies]
# Internal — KSVC
ksvc-core = { path = "crates/ksvc-core" }
ksvc-module = { path = "crates/ksvc-module" }
ksvc-executor = { path = "crates/ksvc-executor" }
ksvc-gvthread = { path = "crates/ksvc-gvthread" }   # ← NEW
```

The `crates/*` glob in `[workspace.members]` auto-includes `crates/ksvc-gvthread`.
The `cmd/*/rust/*` glob auto-includes `cmd/httpd/rust/gvthread`.

## Build

```bash
cargo build --workspace
cargo build -p ksvc-gvthread
cargo build -p gvthread-httpd --release
```

## Run the GVThread HTTP server

```bash
# Default: 4 workers, port 8080
cargo run -p gvthread-httpd --release

# Custom:
cargo run -p gvthread-httpd --release -- --port 9090 --workers 8 --sq 2048

# Benchmark:
wrk -t4 -c100 -d10s http://127.0.0.1:8080/
ab -n 100000 -c 100 -k http://127.0.0.1:8080/
```

## Git commit

```bash
git add crates/ksvc-gvthread cmd/httpd/rust/gvthread docs/KSVC_GVTHREAD.md Cargo.toml
git commit -m "feat: add ksvc-gvthread — green threads on io_uring

Bridge crate wiring GVThread scheduler to KSVC io_uring backend.
Go-like programming model: one GVThread per connection, blocking-style I/O.

Architecture:
- Reactor thread: dedicated OS thread driving io_uring
- MPSC queue: lock-free crossbeam ArrayQueue for I/O request submission
- Results slab: O(1) indexed by GVThread slot ID (= io_uring CorrId)
- submit_and_park(): GVThread submits I/O, blocks, wakes on completion

New files:
- crates/ksvc-gvthread: reactor, syscall wrappers, net (GvtListener/GvtStream)
- cmd/httpd/rust/gvthread: HTTP server demo (one GVThread per connection)
- docs/KSVC_GVTHREAD.md: architecture doc
"
```

## Architecture Overview

```
GVThread user code
    │ stream.read(), stream.write(), listener.accept()
    ▼
ksvc-gvthread::syscall::submit_and_park()
    │ push IoRequest to MPSC queue
    │ block_current() → worker runs other GVThreads
    ▼
ksvc-gvthread::reactor (dedicated OS thread)
    │ pop requests → ProbeRouter → BasicIoUring::submit_with_opcode()
    │ flush_and_wait() → poll CQEs → write result → wake_gvthread()
    ▼
io_uring (kernel)
```

## Three HTTP server models comparison

```
cargo run -p ksvc-httpd --release          # Callback (event loop)
cargo run -p gvthread-httpd --release      # Green threads (this)
# (tokio-httpd also available)

wrk -t4 -c100 -d10s http://127.0.0.1:8080/
```
