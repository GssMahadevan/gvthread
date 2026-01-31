# GVThread Project Context

> Copy this entire file into a new Claude chat to continue development.
> Last updated: 2025-01-31

---

## Project Summary

**gvthread** is a high-performance userspace green thread library for Rust, named in memory of Gorti Viswanadham (GV). It provides:

- 16MB virtual address slots per GVThread (physical memory on-demand via mmap)
- ~20ns voluntary context switch (hand-written x86_64 assembly)
- Hybrid preemption: cooperative (safepoints) + forced (SIGURG signal)
- O(1) scheduling via atomic bitmaps
- 2M+ concurrent GVThreads supported

**Developer:** GssMahadevan  
**Environment:** macOS → SSH → Ubuntu Linux VM (8 cores, 16GB RAM, Rust 1.88+)  
**Workflow:** Claude.ai browser chat for design/code generation → download/copy to VM → build/test

---

## Repository Structure

```
gvthread/
├── Cargo.toml                    # Workspace root, rust-version = "1.88"
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
│   │       ├── bitmap.rs         # ReadyBitmaps (atomic u64 blocks)
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
│   │       ├── timer.rs          # TimerThread (preemption monitor)
│   │       ├── tls.rs            # Thread-local storage
│   │       ├── memory/
│   │       │   ├── mod.rs
│   │       │   └── unix.rs       # mmap-based MemoryRegion
│   │       ├── signal/
│   │       │   ├── mod.rs
│   │       │   └── unix.rs       # SIGURG handler stubs
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
    ├── setup-ubuntu.sh
    ├── verify-env.sh
    ├── dev.sh
    └── sync-to-vm.sh
```

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
0x38    _padding            8

0x40    voluntary_regs      64     VoluntarySavedRegs (rsp,rip,rbx,rbp,r12-r15)
0x80    forced_regs         256    ForcedSavedRegs (all GPRs + flags)
```

### Memory Layout (16MB per slot)

```
slot_base + 16MB  ┌────────────────────┐
                  │   Stack Space      │  ~16MB - 8KB (grows down)
                  │                    │
stack_top         ├────────────────────┤
                  │   Guard Page       │  4KB (PROT_NONE)
                  ├────────────────────┤
                  │   Metadata         │  4KB (GVThreadMetadata)
slot_base         └────────────────────┘
```

### WorkerState (64 bytes, cache-line aligned)

```
Offset  Field              Type
0x00    current_gthread    AtomicU32
0x04    activity_counter   AtomicU32   ← Bumped at safepoints
0x08    start_time_ns      AtomicU64
0x10    thread_id          AtomicU64   ← pthread_t for SIGURG
0x18    is_parked          AtomicBool
0x19    is_low_priority    AtomicBool
```

Global: `WorkerStates` array = 64 workers × 64 bytes = 4KB (single page)

---

## Context Switching

### Voluntary (yield_now) - ~20ns
Saves callee-saved only: rsp, rip, rbx, rbp, r12-r15

```rust
// In arch/x86_64/mod.rs
#[unsafe(naked)]
pub extern "C" fn context_switch_voluntary(
    _old_regs: *mut VoluntarySavedRegs,
    _new_regs: *const VoluntarySavedRegs,
) {
    naked_asm!(
        "mov [rdi + 0x00], rsp",
        "lea rax, [rip + 1f]",
        "mov [rdi + 0x08], rax",
        // ... save rbx, rbp, r12-r15
        "mov rsp, [rsi + 0x00]",
        "mov rax, [rsi + 0x08]",
        // ... restore rbx, rbp, r12-r15
        "jmp rax",
        "1:", "ret",
    );
}
```

### Forced (SIGURG preemption) - ~200ns
Saves ALL registers to forced_regs, handled by signal handler.

---

## Preemption Flow

```
Timer Thread (1ms interval):
  for each worker:
    if activity_counter unchanged for > time_slice:
      set metadata.preempt_flag = 1
      if still unchanged after grace_period:
        pthread_kill(worker.thread_id, SIGURG)

SIGURG Handler:
  1. Save all registers to metadata.forced_regs
  2. Set state = Preempted
  3. Switch to scheduler context

Safepoint (cooperative):
  safepoint!() macro:
    worker.activity_counter.fetch_add(1)
    if metadata.preempt_flag { yield_now() }
```

---

## Current Implementation Status

### ✅ DONE
- All core types (GVThreadId, State, Priority, Errors)
- GVThreadMetadata with stable repr(C) layout
- VoluntarySavedRegs / ForcedSavedRegs
- WorkerState (64-byte aligned)
- ReadyBitmaps (atomic, O(1) scheduling)
- SlotAllocator (LIFO)
- Channel (bounded MPMC, basic)
- SchedMutex (basic)
- CancellationToken (parent-child)
- SchedulerConfig (builder pattern)
- Scheduler struct (spawn, get_next, mark_ready/blocked/finished)
- WorkerPool (thread management)
- WorkerStates global array (4KB, lazy init)
- TimerThread (scans workers, detects stuck GVThreads)
- MemoryRegion (mmap PROT_NONE, activate/deactivate)
- TLS (worker_id, current_gthread_id)
- x86_64 init_context (stack setup)
- x86_64 naked_asm! functions (context_switch_voluntary, context_restore_forced, trampoline)
- All cmd/ examples (compile but don't fully work yet)
- docs/ARCHITECTURE.md, docs/TODO.md

### 🔲 TODO (Next Steps)

**Phase 1: Working Context Switch**
1. `run_gvthread()` - Actually call context_switch_voluntary
2. `yield_now()` - Save context, mark Ready, switch to scheduler
3. `gvthread_finished()` - Release slot, wake joiners
4. Test: spawn → yield → finish cycle

**Phase 2: Preemption**
5. SIGURG handler - Save forced_regs, yield
6. Wire timer to send SIGURG
7. `safepoint!()` macro - Proper implementation
8. Test: cooperative + forced preemption

**Phase 3: Synchronization**
9. Channel blocking → scheduler yield
10. SchedMutex blocking → scheduler yield
11. Join waiters

---

## Build & Run

```bash
# On Linux VM (Rust 1.88+)
cd ~/src/gvthread
cargo build              # Builds all crates
cargo test               # Runs tests (39 pass)
cargo run -p gvthread-basic  # Run example (won't fully work until Phase 1 done)
```

---

## Key Files to Reference

When implementing, these are the critical files:

1. **scheduler.rs** - `run_gvthread()` needs actual context switch
2. **arch/x86_64/mod.rs** - Assembly functions (already done)
3. **worker.rs** - Worker main loop, needs proper context switch call
4. **signal/unix.rs** - SIGURG handler (stub, needs implementation)
5. **tls.rs** - Thread-local current GVThread tracking
6. **metadata.rs** - Register save areas

---

## Constants

```rust
SLOT_SIZE      = 16 * 1024 * 1024  // 16MB
METADATA_SIZE  = 4096              // 4KB
GUARD_SIZE     = 4096              // 4KB
MAX_GVTHREADS  = 2_097_152         // 2M
MAX_WORKERS    = 64
GVTHREAD_NONE  = u32::MAX
```

---

## How to Continue Development

1. **Copy this CONTEXT.md into new chat**
2. **State what you want to work on**, e.g.:
   - "Let's implement run_gvthread() to actually perform context switches"
   - "Let's implement the SIGURG handler"
   - "Let's fix the safepoint! macro"
3. **I'll generate the code**, you download/copy to VM
4. **Build & test**, report back with errors if any

---

## Design Principles

1. **16MB fixed slots** - Simple addressing, no fragmentation
2. **LIFO slot allocator** - Cache-friendly reuse
3. **Bitmap scheduling** - O(1) with random start for fairness
4. **Contiguous WorkerStates** - Timer scans single 4KB page
5. **SIGURG for preemption** - Per-thread, doesn't badly interrupt syscalls
6. **Two-phase preemption** - Cooperative flag first, forced after grace period
7. **repr(C) metadata** - Stable offsets for hand-written assembly