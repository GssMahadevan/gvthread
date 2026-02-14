# KSVC Trait Map

Every axis of variability in KSVC is behind a trait.
Default = safe + working. Optimize by swapping the impl, not modifying it.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          TRAIT MAP                                      │
│                                                                         │
│  Trait              Default Impl         Optimized Impl(s)              │
│  ─────              ────────────         ──────────────────             │
│                                                                         │
│  SyscallRouter      ProbeRouter          StaticRouter                   │
│    route(nr) → Tier   probes io_uring      compile-time table           │
│                       at create time       for known kernel version     │
│                                                                         │
│  IoBackend          BasicIoUring         SqpollIoUring                  │
│    submit/flush/      io_uring_enter()     IORING_SETUP_SQPOLL          │
│    poll_completions   no fixed files       kernel poller thread          │
│                       no fixed buffers   FixedFileIoUring               │
│                                            IORING_REGISTER_FILES        │
│                                          FixedBufferIoUring             │
│                                            IORING_REGISTER_BUFFERS      │
│                                          CompositeIoUring               │
│                                            any combination above        │
│                                                                         │
│  WorkerPool         FixedPool            LazyPool                       │
│    enqueue/          N threads at start    start 1, grow on demand      │
│    poll_completions  N = min(8, nproc/2)   shrink on idle               │
│                                          InlineWorker (test only)       │
│                                            sync execution               │
│                                                                         │
│  Notifier           EventFdNotifier      FutexNotifier                  │
│    notify()          writes eventfd        wakes futex word              │
│                      epoll-compatible      lower overhead                │
│                                                                         │
│  CompletionSink     RingCompletionSink   DirectWakeSink                 │
│    push/flush        writes mmap'd ring    writes result directly       │
│                      batched notify        into GVThread metadata        │
│                                            + unpark specific GT          │
│                                                                         │
│  BufferProvider     HeapBuffers           RegisteredBuffers              │
│    acquire/release   Vec<u8> per op        IORING_REGISTER_BUFFERS      │
│                      no pinning            pre-pinned pages              │
│                                          ProvidedBufferRing             │
│                                            io_uring selects buffer      │
│                                                                         │
│  SharedPage         MmapSharedPage       CachedSharedPage               │
│    pid/uid/etc       volatile read from    thread-local cache            │
│                      mmap'd kernel page    ~1-2 cycles for hot fields   │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

## Composition via Generics

```rust
// The Instance type composes all traits:
pub struct KsvcInstance<R, B, W, N, P>
where
    R: SyscallRouter,
    B: IoBackend,
    W: WorkerPool,
    N: Notifier,
    P: BufferProvider,
{ ... }

// Default (safe) configuration — type alias:
pub type DefaultInstance = KsvcInstance<
    ProbeRouter,
    BasicIoUring,
    FixedPool,
    EventFdNotifier,
    HeapBuffers,
>;

// High-performance server configuration:
pub type ServerInstance = KsvcInstance<
    ProbeRouter,          // still probe (auto-promote)
    SqpollIoUring,        // kernel poller thread
    LazyPool,             // dynamic scaling
    FutexNotifier,        // lower overhead
    RegisteredBuffers,    // pre-pinned
>;

// The dispatcher is fully generic:
pub fn dispatcher_loop<R, B, W, N>( ... )
where
    R: SyscallRouter,
    B: IoBackend,
    W: WorkerPool,
    N: Notifier,
{ ... }
// Same code, different performance characteristics.
```

## Upgrade Path (per trait)

### SyscallRouter: ProbeRouter → StaticRouter

| Aspect      | ProbeRouter              | StaticRouter           |
|-------------|--------------------------|------------------------|
| When        | Runtime                  | Compile time           |
| Overhead    | one-time probe at init   | zero                   |
| Auto-promote| ✅ yes                   | ❌ no — hardcoded      |
| Use when    | general use              | embedded / known target|

**Migration:** `type MyRouter = StaticRouter;` — done.

### IoBackend: BasicIoUring → SqpollIoUring

| Aspect      | BasicIoUring             | SqpollIoUring          |
|-------------|--------------------------|------------------------|
| Submission  | io_uring_enter()         | kernel thread polls SQ |
| Latency     | ~1μs per flush           | ~0 (already polled)    |
| CPU         | minimal                  | one core dedicated     |
| Use when    | general use              | high-throughput server  |

**Migration:** `cargo build --features sqpoll` + type alias change.

### WorkerPool: FixedPool → LazyPool

| Aspect      | FixedPool                | LazyPool               |
|-------------|--------------------------|------------------------|
| Threads     | fixed at creation        | 1 → max, on demand    |
| Idle cost   | N sleeping threads       | 1 sleeping thread      |
| Complexity  | trivial                  | moderate (scaling)     |
| Use when    | always                   | mixed workloads        |

**Migration:** swap `FixedPool` for `LazyPool` in builder.

### Notifier: EventFdNotifier → FutexNotifier

| Aspect      | EventFdNotifier          | FutexNotifier          |
|-------------|--------------------------|------------------------|
| Mechanism   | write(eventfd, 1)        | futex_wake(&word)      |
| Cost        | ~200ns (write syscall)   | ~50ns (futex)          |
| Compat      | epoll/io_uring poll      | custom wait loop       |
| Use when    | always (safe default)    | ultra-low-latency      |

### BufferProvider: HeapBuffers → RegisteredBuffers

| Aspect      | HeapBuffers              | RegisteredBuffers      |
|-------------|--------------------------|------------------------|
| Allocation  | per-op Vec               | pre-allocated pool     |
| Page pin    | per-op by kernel         | once at registration   |
| O_DIRECT    | slow (pin + unpin each)  | fast (already pinned)  |
| Use when    | general use              | O_DIRECT / high-IOPS   |

## Feature Flags

```toml
[features]
default = []
sqpoll = []            # SqpollIoUring
fixed-files = []       # IORING_REGISTER_FILES
fixed-buffers = []     # IORING_REGISTER_BUFFERS
multishot-accept = []  # IORING_ACCEPT_MULTISHOT
send-zc = []           # IORING_OP_SEND_ZC
```

Features compose orthogonally. `sqpoll + fixed-files + fixed-buffers`
all combine cleanly. Each only affects the IoBackend impl.

## Runtime Detection (zero config upgrades)

The `ProbeRouter` queries io_uring at instance creation:

```text
Kernel 6.8  GA:  bind→Tier2, listen→Tier2, pipe2→Tier2
Kernel 6.11 HWE: bind→Tier1, listen→Tier1, pipe2→Tier2
Kernel 6.14 HWE: bind→Tier1, listen→Tier1, pipe2→Tier1
```

**Zero code changes.** Just upgrade the kernel. The routing table
auto-promotes syscalls to Tier 1 when their io_uring opcode appears.

## Crate Structure

```
ksvc-rs/
├── ksvc-core/         # Trait definitions only. Zero dependencies.
│   └── src/
│       ├── lib.rs
│       ├── entry.rs          # SubmitEntry, CompletionEntry, CorrId
│       ├── completion.rs     # CompletionSink trait
│       ├── tier.rs           # Tier enum
│       ├── router.rs         # SyscallRouter trait
│       ├── io_backend.rs     # IoBackend trait
│       ├── worker.rs         # WorkerPool trait
│       ├── notifier.rs       # Notifier trait
│       ├── buffer.rs         # BufferProvider trait
│       ├── shared_page.rs    # SharedPage trait
│       └── error.rs          # KsvcError
│
├── ksvc-module/       # Default implementations.
│   └── src/
│       ├── lib.rs
│       ├── ksvc_sys.rs       # Raw /dev/ksvc bindings (ioctl, mmap offsets)
│       ├── probe_router.rs   # ProbeRouter (default SyscallRouter)
│       ├── basic_iouring.rs  # BasicIoUring (default IoBackend)
│       ├── fixed_pool.rs     # FixedPool (default WorkerPool)
│       ├── eventfd_notifier.rs # EventFdNotifier (default Notifier)
│       ├── ring_completion.rs  # RingCompletionSink (default CompletionSink)
│       ├── heap_buffers.rs   # HeapBuffers (default BufferProvider)
│       ├── mmap_shared_page.rs # MmapSharedPage (default SharedPage)
│       └── instance.rs       # KsvcInstance compositor + builder
│
├── ksvc-executor/     # Dispatcher loop.
│   └── src/
│       └── lib.rs            # dispatcher_loop() — fully generic
│
└── [future crates]
    ├── ksvc-gvthread/        # GVThread integration (park/unpark)
    └── ksvc-bench/           # Benchmarks
```
