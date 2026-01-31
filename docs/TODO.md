# GVThread TODO

## Legend
- ✅ Done
- 🔲 Not started
- 🚧 In progress
- ⚠️ Needs review/testing

---

## Core Types (gvthread-core)

### Completed ✅
- [x] `GVThreadId` - u32 wrapper with NONE sentinel
- [x] `GVThreadState` - Created/Ready/Running/Blocked/Preempted/Finished/Cancelled
- [x] `Priority` - Critical/High/Normal/Low
- [x] `SchedError`, `MemoryError`, `WorkerError` - Error types
- [x] `GVThreadMetadata` - repr(C), 64-byte aligned, stable ASM offsets
- [x] `VoluntarySavedRegs` - 64 bytes, callee-saved registers
- [x] `ForcedSavedRegs` - 256 bytes, all registers
- [x] `WorkerState` - 64 bytes, cache-line aligned
- [x] `ReadyBitmaps` - Atomic u64 blocks, O(1) scheduling
- [x] `SlotAllocator` - LIFO free stack
- [x] `SpinLock<T>` - Internal use only
- [x] `CancellationToken` - Parent-child hierarchy
- [x] `Channel` / `Sender` / `Receiver` - Bounded MPMC (basic)
- [x] `SchedMutex<T>` - Scheduler-aware mutex (basic)
- [x] Platform/Arch traits

### TODO 🔲
- [ ] `Channel` - Integrate blocking send/recv with scheduler yield
- [ ] `SchedMutex` - Integrate lock contention with scheduler yield
- [ ] `Condvar` - Condition variable for SchedMutex
- [ ] `Barrier` - Synchronization barrier
- [ ] `RwLock` - Reader-writer lock
- [ ] `OnceCell` / `Lazy` - One-time initialization

---

## Runtime (gvthread-runtime)

### Completed ✅
- [x] `SchedulerConfig` - Builder pattern configuration
- [x] `Scheduler` - Basic structure with slot/bitmap management
- [x] `WorkerPool` - OS thread pool management
- [x] `WorkerStates` - Global 4KB array (64 workers)
- [x] `TimerThread` - Scans workers, detects stuck GVThreads
- [x] `MemoryRegion` - mmap reservation, activate/deactivate slots
- [x] TLS - worker_id, current_gvthread_id, gvthread_base
- [x] x86_64 `init_context` - Stack setup for new GVThread
- [x] x86_64 `context_switch_voluntary` - naked_asm! callee-saved switch
- [x] x86_64 `context_restore_forced` - naked_asm! full register restore
- [x] x86_64 `gvthread_entry_trampoline` - Entry point for new GVThreads

### High Priority 🔲
- [ ] **Scheduler.run_gvthread()** - Actually perform context switch (currently placeholder)
- [ ] **Scheduler.yield_now()** - Save context, mark Ready, switch to next
- [ ] **SIGURG handler** - Save registers to forced_regs, yield to scheduler
- [ ] **GVThread cleanup** - On finish: release slot, wake joiners, return result
- [ ] **safepoint!() macro** - Bump activity_counter, check preempt_flag
- [ ] **Worker main loop** - Proper shutdown handling

### Medium Priority 🔲
- [ ] **Join support** - `GVThreadHandle::join()` blocks until finished
- [ ] **Sleep support** - `sleep(Duration)` yields for specified time
- [ ] **Timer wheel** - Efficient sleep/timeout management
- [ ] **Steal scheduling** - Workers steal from other workers' local queues
- [ ] **NUMA awareness** - Pin workers to NUMA nodes
- [ ] **io_uring integration** - Async I/O without blocking workers

### Low Priority 🔲
- [ ] aarch64 context switch assembly
- [ ] macOS signal handling differences
- [ ] Windows fiber-based implementation
- [ ] Statistics/metrics collection
- [ ] Tracing/debugging support

---

## Public API (gvthread facade)

### Completed ✅
- [x] `Runtime` - Lifecycle management
- [x] `spawn()` / `spawn_with_priority()` - Basic spawning
- [x] `yield_now()` - Yields (currently just std::thread::yield_now)
- [x] `current_id()` - Get current GVThread ID
- [x] `is_in_gvthread()` - Check if in GVThread context
- [x] `safepoint!` macro - Stub implementation
- [x] Re-exports from core

### TODO 🔲
- [ ] `spawn_blocking()` - Run closure on dedicated thread pool
- [ ] `GVThreadHandle` - Handle for join/cancel
- [ ] `sleep()` / `sleep_until()` - Scheduler-aware sleep
- [ ] `timeout()` - Wrap operation with timeout
- [ ] `select!` macro - Wait on multiple channels
- [ ] `scope()` - Scoped spawning (borrows allowed)

---

## Testing

### Unit Tests 🔲
- [ ] `GVThreadId` - Creation, comparison, NONE handling
- [ ] `GVThreadState` - Transitions
- [ ] `SlotAllocator` - Alloc/release/exhaustion
- [ ] `ReadyBitmaps` - Set/clear/find_and_claim
- [ ] `Channel` - Send/recv, full/empty conditions
- [ ] `CancellationToken` - Cancel propagation
- [ ] `SchedulerConfig` - Validation

### Integration Tests 🔲
- [ ] Basic spawn and finish
- [ ] Multiple GVThreads yielding
- [ ] Channel communication between GVThreads
- [ ] Priority scheduling (Critical runs first)
- [ ] Cancellation mid-execution
- [ ] Memory activation/deactivation

### Stress Tests 🔲
- [ ] 100K concurrent GVThreads
- [ ] Rapid spawn/finish cycles
- [ ] Channel throughput under contention
- [ ] Preemption under CPU-bound load
- [ ] Memory pressure (activate/deactivate churn)

### Preemption Tests 🔲
- [ ] Cooperative preemption via safepoints
- [ ] Forced preemption via SIGURG
- [ ] Mixed workload (cooperative + forced)
- [ ] Grace period behavior

---

## cmd/ Examples

### Completed ✅
- [x] `basic/` - Simple spawn + yield demo (compiles)
- [x] `channel/` - Channel communication (compiles)
- [x] `preemption/` - Preemption test (compiles)
- [x] `stress/` - Scale test (compiles)
- [x] `benchmark/` - Performance benchmarks (compiles)
- [x] `playground/` - Quick experiments (compiles)

### TODO 🔲
- [ ] Make examples actually work (need scheduler completion)
- [ ] Add more realistic examples (web server simulation, etc.)

---

## Documentation

### Completed ✅
- [x] README.md - Project overview
- [x] ARCHITECTURE.md - Full architecture reference
- [x] TODO.md - This file
- [x] Rustdoc comments on public API

### TODO 🔲
- [ ] CONTRIBUTING.md - How to contribute
- [ ] BENCHMARKS.md - Performance results
- [ ] Examples in rustdoc
- [ ] Architecture diagrams (draw.io/mermaid)

---

## Build & CI

### Completed ✅
- [x] Workspace Cargo.toml with all crates
- [x] scripts/setup-ubuntu.sh - VM setup
- [x] scripts/verify-env.sh - Environment check
- [x] scripts/dev.sh - Build helper
- [x] scripts/sync-to-vm.sh - Mac → VM sync

### TODO 🔲
- [ ] GitHub Actions CI
  - [ ] Build on Linux x86_64
  - [ ] Build on macOS x86_64/aarch64
  - [ ] Run tests
  - [ ] Clippy lints
  - [ ] Rustfmt check
- [ ] Benchmarks in CI (track regressions)
- [ ] Code coverage
- [ ] Miri for unsafe code validation

---

## Next Implementation Steps (Recommended Order)

### Phase 1: Working Context Switch
1. [ ] Implement actual context switch in `run_gvthread()`
2. [ ] Implement `yield_now()` with context save/restore
3. [ ] Test basic spawn → yield → finish cycle
4. [ ] Implement `gvthread_finished()` cleanup

### Phase 2: Preemption
5. [ ] Implement full SIGURG handler (save forced_regs)
6. [ ] Wire timer thread to send SIGURG
7. [ ] Implement `safepoint!()` macro properly
8. [ ] Test cooperative + forced preemption

### Phase 3: Synchronization
9. [ ] Wire `Channel` blocking to scheduler yield
10. [ ] Wire `SchedMutex` blocking to scheduler yield
11. [ ] Implement join waiters

### Phase 4: Polish
12. [ ] Sleep/timer support
13. [ ] Statistics collection
14. [ ] Error handling improvements
15. [ ] Documentation polish

---

## Known Issues / Tech Debt

- [ ] `worker.rs:136` - Unused doc comment warning on thread_local macro
- [ ] Timer thread uses busy loop when no workers active
- [ ] No graceful shutdown (workers loop forever)
- [ ] `gvthread_finished()` just spins (needs scheduler integration)
- [ ] Forced preemption doesn't save FPU/SSE state properly yet
- [ ] No ASLR for memory region base address

---

## Performance Optimization Ideas

- [ ] Per-worker local run queues (reduce contention on global bitmaps)
- [ ] Work stealing between workers
- [ ] Batch wakeups for channel operations
- [ ] Lazy FPU state save (only if used)
- [ ] Huge pages for stack memory
- [ ] CPU affinity for workers
- [ ] NUMA-local slot allocation

---

## Future Features (Post-MVP)

- [ ] Async/await integration
- [ ] io_uring integration for async I/O
- [ ] Structured concurrency (scopes, nurseries)
- [ ] Distributed scheduling across processes
- [ ] GPU offload coordination
- [ ] WebAssembly target support