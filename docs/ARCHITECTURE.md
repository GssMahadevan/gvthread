
# GVThread Architecture

> **Green Virtual Thread** - A high-performance userspace threading library for Rust  
> Named in memory of Gorti Viswanadham (GV)

## Overview

GVThread provides lightweight cooperative/preemptive green threads with:
- **16MB virtual address space** per GVThread (physical memory on-demand)
- **~20ns voluntary context switch** via hand-written assembly
- **Hybrid preemption**: cooperative (safepoints) + forced (SIGURG)
- **O(1) scheduling** via atomic bitmaps
- **2M+ concurrent GVThreads** supported

```
┌─────────────────────────────────────────────────────────────────┐
│                         User Code                                │
│                spawn(), yield_now(), channel                     │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                         Scheduler                                │
│              Bitmap scan, priority, worker coordination          │
└─────────────────────────────────────────────────────────────────┘
                               │
           ┌───────────────────┼───────────────────┐
           ▼                   ▼                   ▼
     ┌───────────┐      ┌───────────┐      ┌───────────┐
     │  Worker   │      │  Worker   │      │   Timer   │
     │  Thread   │      │  Thread   │      │   Thread  │
     └───────────┘      └───────────┘      └───────────┘
           │                   │                   │
           └───────────────────┼───────────────────┘
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                       Memory Region                              │
│          16MB slots × N GVThreads, guard pages, mmap             │
└─────────────────────────────────────────────────────────────────┘
```

---

## Crate Structure

```
gvthread/
├── crates/
│   ├── gvthread-core/       # Platform-agnostic types (0 dependencies)
│   ├── gvthread-runtime/    # Platform + arch specific implementation
│   └── gvthread/            # Public facade API
├── cmd/                     # Example binaries (Go-style)
│   ├── basic/
│   ├── benchmark/
│   ├── channel/
│   ├── preemption/
│   ├── stress/
│   └── playground/
└── docs/
```

### gvthread-core

Zero platform dependencies. Contains:

| Module | Purpose |
|--------|---------|
| `id.rs` | `GVThreadId` (u32 wrapper, NONE = u32::MAX) |
| `state.rs` | `GVThreadState` enum, `Priority` enum |
| `error.rs` | `SchedError`, `MemoryError`, `WorkerError` |
| `metadata.rs` | `GVThreadMetadata`, `VoluntarySavedRegs`, `ForcedSavedRegs`, `WorkerState` |
| `bitmap.rs` | `ReadyBitmaps` - O(1) scheduling with atomic u64 blocks |
| `slot.rs` | `SlotAllocator` - LIFO free stack for cache locality |
| `channel.rs` | Bounded MPMC channel |
| `mutex.rs` | `SchedMutex<T>` - scheduler-aware mutex |
| `cancel.rs` | `CancellationToken` with parent-child hierarchy |
| `spinlock.rs` | Internal `SpinLock<T>` (NOT for GVThread use) |
| `traits.rs` | Platform/Arch abstraction traits |

### gvthread-runtime

Platform-specific implementation:

| Module | Purpose |
|--------|---------|
| `config.rs` | `SchedulerConfig` with builder pattern |
| `scheduler.rs` | Main `Scheduler` struct, spawn/yield/schedule |
| `worker.rs` | `WorkerPool`, `WorkerStates` array (4KB, 64 entries) |
| `timer.rs` | `TimerThread` - scans workers, detects stuck GVThreads |
| `tls.rs` | Thread-local: worker_id, current_gthread_id, gvthread_base |
| `memory/` | mmap-based slot allocation |
| `signal/` | SIGURG handler for forced preemption |
| `arch/x86_64/` | Context switch assembly (naked_asm!) |
| `arch/aarch64/` | ARM64 stubs (TODO) |

### gvthread (facade)

Public API re-exports:
- `Runtime`, `SchedulerConfig`
- `spawn`, `spawn_with_priority`, `yield_now`
- `channel`, `Sender`, `Receiver`
- `SchedMutex`, `CancellationToken`
- `safepoint!` macro

---

## Memory Layout

### Virtual Address Space

```
Total reservation: 16MB × max_gvthreads (e.g., 32TB for 2M threads)

Each 16MB Slot:
┌────────────────────────────────────────┐ ← slot_base + 16MB
│                                        │
│              Stack Space               │  ~16MB - 8KB
│           (grows downward)             │
│                                        │
├────────────────────────────────────────┤ ← stack_top
│            Guard Page                  │  4KB (PROT_NONE)
├────────────────────────────────────────┤
│            Metadata                    │  4KB
│         (GVThreadMetadata)             │
└────────────────────────────────────────┘ ← slot_base
```

### Memory Strategy

| Phase | Action |
|-------|--------|
| Init | `mmap(PROT_NONE)` entire region - reserves virtual addresses only |
| Activate | `mprotect(PROT_READ\|PROT_WRITE)` on slot - triggers physical allocation |
| Deactivate | `madvise(MADV_DONTNEED)` - releases physical pages, keeps virtual |

### Constants

```rust
SLOT_SIZE      = 16MB      // 16 * 1024 * 1024
METADATA_SIZE  = 4KB       // 4096
GUARD_SIZE     = 4KB       // 4096
STACK_SIZE     = SLOT_SIZE - METADATA_SIZE - GUARD_SIZE
MAX_GVTHREADS  = 2_097_152 // 2M default
MAX_WORKERS    = 64
```

---

## GVThreadMetadata Layout

64-byte aligned, `repr(C)` for stable ASM offsets:

```
Offset  Size  Field
──────  ────  ─────────────────────────
0x00    1     preempt_flag (AtomicU8)     ← Timer sets this
0x01    1     cancelled (AtomicU8)
0x02    1     state (AtomicU8)
0x03    1     priority (AtomicU8)
0x04    4     gvthread_id (AtomicU32)
0x08    4     parent_id (AtomicU32)
0x0C    4     worker_id (AtomicU32)
0x10    8     entry_fn (AtomicU64)
0x18    8     entry_arg (AtomicU64)
0x20    8     result_ptr (AtomicU64)
0x28    8     join_waiters (AtomicU64)
0x30    8     start_time_ns (AtomicU64)
0x38    8     _padding

0x40    64    voluntary_regs (VoluntarySavedRegs)
              ├─ rsp, rip, rbx, rbp, r12, r13, r14, r15
              
0x80    256   forced_regs (ForcedSavedRegs)
              ├─ All GPRs + rflags + fpu_state_ptr

Total: 384 bytes (fits in 4KB metadata page)
```

---

## WorkerState Layout

64-byte cache-line aligned:

```
Offset  Size  Field
──────  ────  ─────────────────────────
0x00    4     current_gthread (AtomicU32)
0x04    4     activity_counter (AtomicU32)  ← Bumped at safepoints
0x08    8     start_time_ns (AtomicU64)
0x10    8     thread_id (AtomicU64)         ← pthread_t for SIGURG
0x18    1     is_parked (AtomicBool)
0x19    1     is_low_priority (AtomicBool)
0x1A-0x3F     _padding to 64 bytes
```

Global array: `WorkerStates` - 64 × 64 = 4KB total (single page, no TLB misses)

---

## Scheduling

### Priority Levels

```rust
enum Priority {
    Critical = 0,  // Real-time, always runs first
    High     = 1,  // Interactive
    Normal   = 2,  // Default
    Low      = 3,  // Background
}
```

### Ready Bitmaps

```
Per-priority bitmap: 2M bits = 32,768 × u64 blocks

┌─────────────────────────────────────────────────────┐
│ Block 0  │ Block 1  │ Block 2  │ ... │ Block 32767 │
│ bits 0-63│ bits 64+ │          │     │             │
└─────────────────────────────────────────────────────┘

Operations:
- set_ready(id):    atomic OR on block[id/64]
- clear_ready(id):  atomic AND on block[id/64]
- find_and_claim(): scan blocks, atomic clear first set bit
```

### Fairness

- Random starting block per `find_and_claim()` call
- Prevents starvation of high-numbered GVThreads
- O(1) amortized with uniform distribution

---

## Context Switching

### Voluntary (yield_now, channel block, mutex block)

Saves only callee-saved registers (64 bytes):
```
rsp, rip, rbx, rbp, r12, r13, r14, r15
```

Assembly (`naked_asm!`):
```asm
; Save to old_regs (rdi)
mov [rdi + 0x00], rsp
lea rax, [rip + return_label]
mov [rdi + 0x08], rax
mov [rdi + 0x10], rbx
; ... r12-r15

; Load from new_regs (rsi)
mov rsp, [rsi + 0x00]
mov rax, [rsi + 0x08]
mov rbx, [rsi + 0x10]
; ... r12-r15
jmp rax
```

**Cost: ~20ns**

### Forced (SIGURG preemption)

Saves ALL registers (256 bytes):
```
rax, rbx, rcx, rdx, rsi, rdi, rbp, rsp,
r8-r15, rip, rflags, cs, ss, fpu_state_ptr
```

**Cost: ~200ns** (signal overhead + full save/restore)

---

## Preemption

### Cooperative (Safepoints)

User inserts `safepoint!()` in loops:
```rust
for i in 0..1_000_000 {
    safepoint!();  // Check preempt_flag, bump activity_counter
    // work...
}
```

Safepoint expands to:
1. `worker.activity_counter.fetch_add(1)`
2. Check `metadata.preempt_flag`
3. If set → `yield_now()`

### Forced (SIGURG)

Timer thread detects stuck GVThreads:

```
Timer Loop (every 1ms):
┌─────────────────────────────────────────────────────────┐
│ for worker in workers:                                  │
│   if worker.current_gthread != NONE:                    │
│     if worker.activity_counter == last_seen[worker]:    │
│       if first_stall_time.elapsed() > time_slice:       │
│         metadata.preempt_flag = 1  // Cooperative hint  │
│         if elapsed() > time_slice + grace_period:       │
│           pthread_kill(worker.thread_id, SIGURG)        │
│     else:                                               │
│       last_seen[worker] = activity_counter              │
│       first_stall_time = None                           │
└─────────────────────────────────────────────────────────┘
```

SIGURG Handler:
1. Save all registers to `forced_regs`
2. Set `state = Preempted`
3. Switch to scheduler context
4. Resume different GVThread

---

## Synchronization Primitives

### Channel (Bounded MPMC)

```rust
let (tx, rx) = channel::<T>(capacity);

// Non-blocking
tx.try_send(value) -> Result<(), SendError>
rx.try_recv() -> Result<T, RecvError>

// Blocking (yields to scheduler)
tx.send(value)   // TODO: integrate with scheduler
rx.recv() -> T   // TODO: integrate with scheduler
```

Implementation: Ring buffer with atomic head/tail, `Arc<ChannelInner<T>>`

### SchedMutex

```rust
let mutex = SchedMutex::new(data);
let guard = mutex.lock();  // Yields if contended
```

Implementation: Atomic state + waiter queue (TODO: full scheduler integration)

### CancellationToken

```rust
let token = CancellationToken::new();
let child = token.child();

spawn(move |_| {
    while !token.is_cancelled() {
        // work...
    }
});

token.cancel();  // Propagates to children
```

---

## Configuration

```rust
let config = SchedulerConfig::default()
    .num_workers(8)              // OS threads running GVThreads
    .num_low_priority_workers(2) // Dedicated to Priority::Low
    .max_gvthreads(1_000_000)    // Max concurrent GVThreads
    .time_slice(Duration::from_millis(10))
    .grace_period(Duration::from_millis(1))
    .timer_interval(Duration::from_millis(1))
    .enable_forced_preempt(true)
    .debug_logging(false);
```

---

## Typical Usage

```rust
use gvthread::{Runtime, spawn, yield_now, channel, SchedulerConfig};

fn main() {
    let config = SchedulerConfig::default().num_workers(4);
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        // Spawn GVThreads
        let (tx, rx) = channel(100);
        
        spawn(move |token| {
            for i in 0..1000 {
                if token.is_cancelled() { break; }
                tx.try_send(i).ok();
                yield_now();
            }
        });
        
        spawn(move |_| {
            while let Ok(val) = rx.try_recv() {
                println!("Got: {}", val);
            }
        });
    });
}
```

---

## Platform Support

| Platform | Status |
|----------|--------|
| Linux x86_64 | ✅ Primary target |
| Linux aarch64 | 🔲 Stubs only |
| macOS x86_64 | 🔲 Should work (untested) |
| macOS aarch64 | 🔲 Needs implementation |
| Windows | 🔲 Not started |

---

## Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Voluntary context switch | <30ns | Callee-saved only |
| Forced context switch | <500ns | Full register save via signal |
| Spawn overhead | <1μs | Slot alloc + metadata init |
| Channel send/recv | <100ns | Lock-free ring buffer |
| Memory per GVThread | 4KB idle | 16MB virtual, physical on-demand |

---

## Key Design Decisions

1. **16MB fixed slots** - Simplifies addressing, avoids fragmentation
2. **LIFO slot allocator** - Cache-friendly reuse of recently freed slots
3. **Bitmap scheduling** - O(1) find_and_claim with random start
4. **Contiguous WorkerStates** - Timer scans 4KB array, no pointer chasing
5. **SIGURG for preemption** - Per-thread signal, doesn't interrupt syscalls badly
6. **Two-phase preemption** - Cooperative flag first, forced signal after grace period
7. **repr(C) metadata** - Stable offsets for hand-written assembly