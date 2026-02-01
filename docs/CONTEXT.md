# GVThread Project Context

> Copy this entire file into a new Claude chat to continue development.
> Last updated: 2025-02-01

---

## Project Summary

**gvthread** is a high-performance userspace green thread library for Rust, named in memory of Gorti Viswanadham (GV). It provides:

- 16MB virtual address slots per GVThread (physical memory on-demand via mmap)
- ~20ns voluntary context switch (hand-written x86_64 assembly)
- Hybrid preemption: cooperative (safepoints) + forced (SIGURG signal)
- Go-like scheduling: per-worker local queues + global queue
- 2M+ concurrent GVThreads supported
- **CPU performance now matches Go's goroutines**

**Developer:** GssMahadevan  
**Repository:** https://github.com/GssMahadevan/gvthread  
**Environment:** macOS → SSH → Ubuntu Linux VM (8 cores, 16GB RAM, Rust 1.88+)  

---

## Repository Structure

```
gvthread/
├── Cargo.toml                    # Workspace root
├── README.md
├── docs/
│   ├── ARCHITECTURE.md           # Full technical reference
│   ├── TODO.md                   # Checklist of all tasks
│   └── CONTEXT.md                # This file
├── crates/
│   ├── gvthread-core/            # Platform-agnostic types (0 deps)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── id.rs             # GVThreadId (u32, NONE=u32::MAX)
│   │       ├── state.rs          # GVThreadState, Priority enums
│   │       ├── error.rs          # SchedError, MemoryError, WorkerError
│   │       ├── metadata.rs       # GVThreadMetadata (repr(C), 64-byte aligned)
│   │       ├── slot.rs           # SlotAllocator (LIFO free stack)
│   │       ├── channel.rs        # Bounded MPMC channel
│   │       ├── mutex.rs          # SchedMutex<T>
│   │       ├── cancel.rs         # CancellationToken
│   │       ├── spinlock.rs       # Internal SpinLock<T>
│   │       └── traits.rs         # Platform/Arch abstraction
│   ├── gvthread-runtime/         # Platform-specific implementation
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs         # SchedulerConfig
│   │       ├── scheduler.rs      # Main Scheduler struct
│   │       ├── worker.rs         # WorkerPool, WorkerStates
│   │       ├── ready_queue.rs    # Go-like local + global queues
│   │       ├── timer/            # Timer subsystem (see below)
│   │       │   ├── mod.rs        # Sleep queue, preemption, TimerThread
│   │       │   ├── entry.rs      # TimerEntry, TimerHandle, TimerType
│   │       │   ├── registry.rs   # TimerRegistry API
│   │       │   ├── worker.rs     # Timer thread utilities
│   │       │   └── impls/
│   │       │       ├── mod.rs    # Backend factory
│   │       │       └── heap.rs   # HeapTimerBackend
│   │       ├── tls.rs            # Thread-local storage
│   │       ├── memory/
│   │       │   ├── mod.rs
│   │       │   └── unix.rs       # mmap-based MemoryRegion
│   │       ├── signal/
│   │       │   ├── mod.rs
│   │       │   └── unix.rs       # SIGURG handler
│   │       └── arch/
│   │           ├── mod.rs
│   │           ├── x86_64/mod.rs # Context switch (naked_asm!)
│   │           └── aarch64/mod.rs# Stubs
│   └── gvthread/                 # Public facade API
│       └── src/lib.rs
├── cmd/                          # Example binaries
│   ├── basic/src/main.rs
│   ├── benchmark/src/main.rs
│   ├── channel/src/main.rs
│   ├── preemption/src/main.rs
│   ├── stress/src/main.rs
│   └── playground/src/main.rs
└── scripts/
```

---

## Recent Changes (2025-02-01)

### Ready Queue Refactor
- **Before:** Bitmap-based O(1) scheduling
- **After:** Go-like queue-based scheduling
  - Per-worker local queues (fast path, no contention)
  - Global queue (overflow, new spawns)
  - Work stealing from other workers
- **Result:** CPU performance now matches Go's goroutines

### Timer Module Refactor
- **Before:** Single `timer.rs` file
- **After:** `timer/` directory with modular structure:
  - `mod.rs` - Main logic: sleep queue (BinaryHeap), preemption monitoring, TimerThread
  - `entry.rs` - TimerEntry, TimerHandle, TimerType (for future use)
  - `registry.rs` - TimerRegistry high-level API (for future use)
  - `worker.rs` - Timer thread spawning utilities (for future use)
  - `impls/heap.rs` - HeapTimerBackend (for future use)

### Timer Design
- Single `SLEEP_QUEUE` BinaryHeap for all sleeping GVThreads
- Timer thread loop:
  1. Process sleep queue → wake expired GVTs via `scheduler::wake_gvthread()`
  2. Monitor workers for preemption (activity counter stall detection)
- Worker affinity support (GVTs can prefer specific workers for cache locality)

---

## Key Data Structures

### GVThreadMetadata (repr(C), stable offsets for ASM)

```
Offset  Field               Size   Type
0x00    preempt_flag        1      AtomicU8    ← Timer sets this
0x01    cancelled           1      AtomicU8
0x02    state               1      AtomicU8
0x03    priority            1      AtomicU8
0x04    gvthread_id         4      AtomicU32
0x08    parent_id           4      AtomicU32
0x0C    worker_id           4      AtomicU32
0x10    entry_fn            8      AtomicU64
0x18    entry_arg           8      AtomicU64
0x20    result_ptr          8      AtomicU64
0x28    join_waiters        8      AtomicU64
0x30    start_time_ns       8      AtomicU64
0x38    wake_time_ns        8      AtomicU64   ← For sleep queue

0x40    voluntary_regs      64     VoluntarySavedRegs (rsp,rip,rbx,rbp,r12-r15)
0x80    forced_regs         256    ForcedSavedRegs (all GPRs + flags)
```

### Ready Queue (Go-like)

```
┌─────────────────────────────────────────────────────────┐
│  Worker 0      Worker 1      Worker 2      Worker N    │
│  ┌───────┐    ┌───────┐    ┌───────┐    ┌───────┐     │
│  │ Local │    │ Local │    │ Local │    │ Local │     │
│  │ Queue │    │ Queue │    │ Queue │    │ Queue │     │
│  └───────┘    └───────┘    └───────┘    └───────┘     │
│       └────────────┴────────────┴───────────┘          │
│                         │                              │
│               ┌─────────────────┐                      │
│               │  Global Queue   │                      │
│               └─────────────────┘                      │
└─────────────────────────────────────────────────────────┘

Pop order: Local → Global → Steal from others
Push: spawn() → Global, yield() with hint → Local
```

### Sleep Queue

```rust
struct SleepEntry {
    wake_time_ns: u64,
    gvthread_id: u32,
    generation: u32,  // Prevents stale wakes
}
// BinaryHeap min-heap ordered by wake_time_ns
// Protected by SpinLock (no syscalls from GVThread stack)
```

---

## Current Implementation Status

### ✅ DONE
- Core types (GVThreadId, State, Priority, Errors)
- GVThreadMetadata with stable repr(C) layout
- Context switch assembly (voluntary ~20ns, forced ~200ns)
- Memory region (mmap, activate/deactivate slots)
- Scheduler with spawn, yield, block, wake
- Worker pool with parking/wake
- **Ready queue (Go-like local + global)**
- **Timer module with sleep queue + preemption monitoring**
- Sleep API (sleep, sleep_ms, sleep_us)
- Channels (basic MPMC)
- All cmd/ examples working

### 🔲 TODO (Future)
- Blocking channel integration with scheduler
- SchedMutex blocking integration
- Work stealing optimization
- aarch64 support
- Timing wheel backend for timer
- io_uring integration

---

## Build & Run

```bash
# On Linux VM (Rust 1.88+)
cd ~/src/gvthread
cargo build --release
cargo test
cargo run -p gvthread-basic --release
cargo run -p gvthread-stress --release -- 100000
cargo run -p gvthread-benchmark --release
```

---

## How to Continue Development

1. **Start chat with:** "Continue gvthread work, repo: https://github.com/GssMahadevan/gvthread"
2. **I can fetch files directly** from the public repo
3. **State what to work on**, e.g.:
   - "Let's implement work stealing"
   - "Let's add timing wheel backend"
   - "Let's optimize channel blocking"

---

## Design Principles

1. **16MB fixed slots** - Simple addressing, no fragmentation
2. **LIFO slot allocator** - Cache-friendly reuse
3. **Queue-based scheduling** - Go-like, better cache locality than bitmaps
4. **Worker affinity** - Preserve cache locality on yield/sleep
5. **Contiguous WorkerStates** - Timer scans single 4KB page
6. **SIGURG for preemption** - Per-thread, doesn't badly interrupt syscalls
7. **Two-phase preemption** - Cooperative flag first, forced after grace period
8. **repr(C) metadata** - Stable offsets for hand-written assembly
9. **Single sleep queue** - All timers in one heap, simple and correct