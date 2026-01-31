# Ready Queue Refactoring - Bitmap → SimpleQueue (Go-like)

## Overview

Replaced the bitmap-based ready queue with a Go-like queue system:
- Per-worker local queues (256 slots each, SpinLock)
- Global queue (VecDeque, Mutex + Condvar for parking)
- Work stealing from random victims

## Architecture

```
                    ┌─────────────────────┐
                    │    Global Queue     │
                    │  (Mutex + Condvar)  │
                    │                     │
                    │  [id][id][id]...    │
                    └──────────┬──────────┘
                               │
           ┌───────────────────┼───────────────────┐
           │                   │                   │
    ┌──────▼──────┐     ┌──────▼──────┐     ┌──────▼──────┐
    │  Worker 0   │     │  Worker 1   │     │  Worker N   │
    │ Local Queue │     │ Local Queue │     │ Local Queue │
    │ [id][id]... │     │ [id][id]... │     │ [id][id]... │
    └─────────────┘     └─────────────┘     └─────────────┘
```

## Trait Definition

```rust
pub trait ReadyQueue: Send + Sync {
    fn push(&self, id: GVThreadId, priority: Priority, hint_worker: Option<usize>);
    fn pop(&self, worker_id: usize) -> Option<(GVThreadId, Priority)>;
    fn park(&self, worker_id: usize, timeout_ms: u64);
    fn wake_one(&self);
    fn wake_all(&self);
    fn len(&self) -> usize;
}
```

## SimpleQueue (MVP)

### Push Strategy
1. If `hint_worker` provided → try that worker's local queue
2. If local full or no hint → push to global queue
3. Wake a parked worker

### Pop Strategy (per worker)
1. Every 61st pop → check global first (starvation prevention)
2. Try local queue
3. Try global queue (+ grab batch for local)
4. Try work stealing from random victim
5. Return None → worker should park

### Parking
- Workers park on GlobalQueue's Condvar
- `push()` wakes one parked worker
- `wake_all()` for shutdown

## Files Changed

| File | Change |
|------|--------|
| `ready_queue/mod.rs` | New - trait definition |
| `ready_queue/simple.rs` | New - Go-like implementation |
| `scheduler.rs` | Use trait instead of bitmap |
| `tls.rs` | Added `try_current_worker_id()` |
| `lib.rs` | Export ready_queue module |

## Priority Handling (MVP)

For MVP, all priorities treated as `Normal`. Multi-priority support 
disabled via the trait abstraction - can be added later by implementing
a different `ReadyQueue`.

```rust
// In SimpleQueue
fn push(&self, id: GVThreadId, _priority: Priority, ...) {
    // Priority ignored for MVP
}
```

## Future Implementations

The trait allows for different strategies:

```rust
// Bitmap-based (original)
impl ReadyQueue for BitmapQueue { ... }

// Priority queues (5 levels)
impl ReadyQueue for PriorityQueue { ... }

// Lock-free (crossbeam)  
impl ReadyQueue for LockFreeQueue { ... }
```

## Expected Benefits

1. **Lower CPU** - Workers sleep on Condvar, not futex polling
2. **Better locality** - `hint_worker` keeps GVThreads on same worker
3. **Simpler code** - Queue push/pop vs bitmap scanning
4. **Go-proven** - Same strategy as Go's runtime

## Testing

```bash
GVT_WORKERS=4 GVT_GVTHREADS=2000 GVT_YIELDS=300 GVT_SLEEP_MS=100 \
  ./target/debug/playground

# Compare CPU usage with previous bitmap implementation
```