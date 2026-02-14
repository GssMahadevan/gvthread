# ksvc-gvthread: Green Threads on io_uring

> Bridge crate: GVThread cooperative scheduler + KSVC io_uring backend

## The Programming Model

GVThreads make **blocking-style** I/O calls. Under the hood, the reactor
submits them to io_uring and wakes the GVThread on completion. The worker
OS thread is freed to run other GVThreads while I/O is in flight.

**Same pattern as Go's netpoller, but on io_uring instead of epoll.**

```rust
// Inside a GVThread — reads like synchronous code:
fn handle_connection(stream: GvtStream) {
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf);     // blocks GVThread, not OS thread
        if n <= 0 { break; }
        stream.write_all(&response);       // same — io_uring underneath
    }
}

// Accept loop — one GVThread per connection:
fn accept_loop(listener: GvtListener) {
    loop {
        let stream = listener.accept();    // blocks on io_uring accept
        spawn(move |_| handle_connection(stream));
    }
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    User GVThread Code                            │
│   stream.read()  stream.write()  listener.accept()              │
│   (blocking-style API — looks like synchronous I/O)             │
└──────────────────────┬──────────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────────┐
│              ksvc-gvthread::syscall                              │
│   submit_and_park():                                            │
│     1. Build IoRequest (syscall_nr + args + CorrId=GVThread ID) │
│     2. Push to MPSC queue (crossbeam ArrayQueue)                │
│     3. block_current() → context switch to scheduler            │
│     4. ... worker runs other GVThreads ...                      │
│     5. GVThread wakes, reads result from slab                   │
└──────────────────────┬──────────────────────────────────────────┘
                       │ MPSC queue (lock-free)
                       ▼
┌─────────────────────────────────────────────────────────────────┐
│              ksvc-gvthread::reactor                              │
│   Dedicated OS thread ("ksvc-reactor"):                         │
│     1. Pop IoRequests from MPSC queue                           │
│     2. Route via ProbeRouter → io_uring opcode                  │
│     3. Submit SQEs via BasicIoUring::submit_with_opcode()       │
│     4. flush_and_wait(1) — kernel blocks until CQE ready        │
│     5. Poll CQEs → write result to slab → wake_gvthread()      │
└──────────────────────┬──────────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────────┐
│              io_uring (kernel)                                   │
│   SQ → kernel io-wq workers → CQ                               │
│   accept, recv, send, read, write, close, openat, connect...   │
└─────────────────────────────────────────────────────────────────┘
```

## Key Design Decisions

### 1. CorrId = GVThread slot ID
The KSVC `CorrId` was designed from day one to map 1:1 to GVThread IDs.
io_uring stores user_data in each SQE/CQE — we use the GVThread's slot
index. Completion → wake is a direct index lookup, zero hashtable.

### 2. Results slab (not hashmap)
`Box<[AtomicI64]>` indexed by GVThread slot. O(1) write by reactor,
O(1) read by woken GVThread. Cache-friendly for sequential access.

### 3. Dedicated reactor OS thread
The reactor never competes with GVThread workers for CPU. It's always
available to poll completions and submit new requests. This avoids the
problem of "who polls io_uring?" that plagues async runtimes where
polling happens on worker threads.

### 4. Lock-free MPSC queue
crossbeam `ArrayQueue` — bounded, lock-free, cache-friendly.
GVThreads push (multiple producers), reactor pops (single consumer).
If full, GVThread yields and retries (backpressure).

### 5. Buffer safety via GVThread stack stability
When a GVThread blocks, its 16MB stack slot is stable. Buffers on the
stack remain valid until the GVThread resumes. This means we can safely
pass stack buffer pointers through io_uring without pinning.

## Crate Map

```
crates/ksvc-gvthread/
├── Cargo.toml
└── src/
    ├── lib.rs          # Re-exports, crate docs
    ├── reactor.rs      # Reactor thread (io_uring driver + GVThread waker)
    ├── syscall.rs      # Blocking syscall wrappers (ksvc_read, ksvc_write, ...)
    └── net.rs          # GvtListener, GvtStream (high-level networking)
```

## Comparison: Three HTTP Server Models

| Aspect                | ksvc-httpd (callback)  | tokio-httpd (async)    | gvthread-httpd (green) |
|-----------------------|------------------------|------------------------|------------------------|
| I/O backend           | io_uring               | epoll (mio)            | **io_uring**           |
| Programming model     | Event loop + callbacks | async/await            | **Blocking-style**     |
| Concurrency unit      | State machine per conn | Future per conn        | **GVThread per conn**  |
| Context switch        | N/A (single-threaded)  | Future poll overhead   | **~20ns voluntary**    |
| Stack per connection  | Shared (state machine) | Shared (pinned future) | **16MB virtual**       |
| Code complexity       | High (manual state)    | Medium (async/await)   | **Low (sequential)**   |
| Debuggability         | Low (callbacks)        | Medium (async traces)  | **High (real stacks)** |
| Throughput            | Highest (no overhead)  | High                   | **High (io_uring)**    |
| Memory per 10K conns  | ~40MB (buffers)        | ~100MB (futures)       | **~40MB (on-demand)**  |

## Dependencies

```
ksvc-gvthread
├── gvthread-core     (GVThreadId, Priority, CancellationToken)
├── gvthread-runtime  (scheduler::block_current, scheduler::wake_gvthread, tls)
├── gvthread          (spawn, yield_now, sleep)
├── ksvc-core         (CorrId, SubmitEntry, IoBackend trait, SyscallRouter trait)
├── ksvc-module       (BasicIoUring, ProbeRouter)
├── crossbeam-queue   (ArrayQueue for MPSC)
└── libc              (constants: EAGAIN, ENOSYS, etc.)
```

## Future Enhancements

1. **Tier 2 fallback**: Route unsupported syscalls to the WorkerPool
2. **Multi-ring**: One io_uring ring per worker for NUMA affinity
3. **DirectWake**: Write result directly to GVThread metadata, skip slab
4. **SQPOLL**: Enable kernel-side SQ polling for lower latency
5. **Multishot accept**: `IORING_ACCEPT_MULTISHOT` for zero-syscall accepts
6. **Buffer ring**: `IORING_OP_PROVIDE_BUFFERS` for zero-copy reads
7. **Generation checking**: Prevent stale wakes after GVThread slot reuse
8. **Cancel support**: Cancel in-flight I/O on GVThread cancellation
