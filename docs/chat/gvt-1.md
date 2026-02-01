# GVThread Session Summary - Ready Queue Refactor

**Date:** January 31, 2026
**Result:** CPU performance now matches Go (~0% idle with sleep)

## What Was Accomplished

### 1. Replaced Bitmap Scheduler with Queue-Based (Go-like)

**Before:** O(n) bitmap scanning to find ready GVThreads
**After:** O(1) queue pop with per-worker local queues + global queue

Key files changed:
- `crates/gvthread-runtime/src/ready_queue/mod.rs` - ReadyQueue trait
- `crates/gvthread-runtime/src/ready_queue/simple.rs` - Go-like implementation
- `crates/gvthread-runtime/src/scheduler.rs` - Uses trait instead of bitmap

### 2. Fixed Sleep Queue

**Design:**
- BinaryHeap (min-heap) for O(1) next wake time peek
- SpinLock protection (safe from GVThread stack - no syscalls)
- Pre-allocated capacity to avoid allocation during push

**Key insight:** Only running GVThreads call sleep(), so their slots are already activated (no SEGV risk).

### 3. Critical Bug Fixes

**Bug 1: Workers not woken after timer wake**
```rust
// simple.rs - global.push() wasn't waking workers
self.global.push(gid);
// ADDED: Wake a parked worker
if self.global.parked_count() > 0 {
    self.global.wake_one();
}
```

**Bug 2: Type mismatches in timer.rs**
- `last_counter` changed from u64 to u32
- `thread_id()` changed to `thread_id.load()`
- `send_preempt_signal` changed to `send_sigurg`

## Architecture Overview

```
                    ┌─────────────────────┐
                    │    Global Queue     │
                    │  (Mutex + Condvar)  │
                    └──────────┬──────────┘
                               │
           ┌───────────────────┼───────────────────┐
           │                   │                   │
    ┌──────▼──────┐     ┌──────▼──────┐     ┌──────▼──────┐
    │  Worker 0   │     │  Worker 1   │     │  Worker N   │
    │ Local Queue │     │ Local Queue │     │ Local Queue │
    │ (SpinLock)  │     │ (SpinLock)  │     │ (SpinLock)  │
    └─────────────┘     └─────────────┘     └─────────────┘

Sleep Flow:
  GVThread.sleep() → BinaryHeap.push() → block
  Timer.tick() → BinaryHeap.pop() → wake_gvthread() → global.push() → wake_one()
```

## Key Files (in ready_queue_files/)

| File | Description |
|------|-------------|
| `mod.rs` | ReadyQueue trait definition |
| `simple.rs` | Go-like queue implementation |
| `scheduler.rs` | Updated to use ReadyQueue trait |
| `timer.rs` | BinaryHeap sleep queue |
| `tls.rs` | Added try_current_worker_id() |
| `runtime_lib.rs` | Exports ready_queue module |
| `copy_ready_queue.sh` | Install script |

## Test Commands

```bash
# Build
cargo build --release -p gvthread-playground

# Test with sleep
GVT_WORKERS=4 GVT_GVTHREADS=20000 GVT_YIELDS=300 GVT_SLEEP_MS=100 \
  ./target/release/playground

# Compare with Go
cd other/go/playground1
GOMAXPROCS=4 ./main -goroutines=20000 -yields=300 -sleep=100
```

## Source Location

Project: `~/src/gvthread`

Key crates:
- `gvthread-core` - Core types, metadata, SpinLock
- `gvthread-runtime` - Scheduler, timer, memory, ready_queue
- `gvthread-playground` - Test binary

## Next Steps / Future Work

1. **timerfd integration** - Match Go's zero-CPU timer sleeping
2. **Lock-free local queues** - Use crossbeam-deque
3. **Priority queue variant** - Multi-priority via trait
4. **Work-stealing improvements** - More sophisticated algorithm
5. **Benchmark suite** - Automated perf comparison with Go

## Memory Layout (unchanged)

- 16MB per GVThread slot (virtual)
- Metadata at slot base (4KB)
- Stack grows down from slot top
- Guard page at end

## Context Switch (unchanged)

- Voluntary: `context_switch_voluntary()` - saves/restores callee-saved regs
- x86_64 assembly in `crates/gvthread-runtime/src/arch/x86_64.rs`