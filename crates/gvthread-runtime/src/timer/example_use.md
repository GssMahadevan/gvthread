## Usage
```rust
//! Example: Integrating timer with ready queue
//!
//! This shows how the timer system connects to your ready queue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

// Import timer module
mod timer;

use timer::{
    impls::HeapTimerBackend, spawn_timer_thread, ExpiredTimer, TimerBackendType,
    TimerRegistry, TimerThreadConfig, TimerType,
    worker::TimerWakeCallback,
};

/// Your ReadyQueue implementation
pub struct ReadyQueue {
    // Per-worker queues for affinity
    // worker_queues: Vec<SegQueue<GvtContext>>,
    // Global queue for non-affine GVTs
    // global_queue: SegQueue<GvtContext>,
}

impl ReadyQueue {
    pub fn new(_num_workers: usize) -> Self {
        Self {
            // worker_queues: (0..num_workers).map(|_| SegQueue::new()).collect(),
            // global_queue: SegQueue::new(),
        }
    }

    /// Wake a GVT by pushing it to appropriate queue
    pub fn wake(&self, gvt_id: u32, affinity: Option<u8>, timer_type: TimerType) {
        // Look up GVT context from registry
        // let gvt = gvt_registry.get(gvt_id);

        match affinity {
            Some(worker_id) => {
                // Push to specific worker's queue
                println!(
                    "  -> Waking GVT {} on worker {} (type: {:?})",
                    gvt_id, worker_id, timer_type
                );
                // self.worker_queues[worker_id as usize].push(gvt);
            }
            None => {
                // Push to global queue (any worker can take it)
                println!(
                    "  -> Waking GVT {} on any worker (type: {:?})",
                    gvt_id, timer_type
                );
                // self.global_queue.push(gvt);
            }
        }
    }
}

/// Implement TimerWakeCallback for ReadyQueue
impl TimerWakeCallback for ReadyQueue {
    fn on_timer_expired(&self, expired: ExpiredTimer) {
        self.wake(expired.gvt_id, expired.worker_affinity, expired.timer_type);
    }
}

fn main() {
    println!("=== GVThread Timer Integration Example ===\n");

    // 1. Create timer backend (using factory for configurability)
    let backend = Arc::new(HeapTimerBackend::new());
    println!("Created timer backend: {}", backend.name());

    // 2. Create timer registry (high-level API)
    let timer_registry = TimerRegistry::new(backend.clone());
    println!("Timer registry ready\n");

    // 3. Create ready queue
    let ready_queue = Arc::new(ReadyQueue::new(4));

    // 4. Create shutdown signal
    let shutdown = Arc::new(AtomicBool::new(false));

    // 5. Spawn timer thread
    let timer_handle = spawn_timer_thread(
        backend,
        ready_queue,
        shutdown.clone(),
        TimerThreadConfig::default(),
    );
    println!("Timer thread spawned\n");

    // === Example Usage ===

    println!("Scheduling timers...\n");

    // Preemption timer (with worker affinity)
    let preempt_handle = timer_registry.schedule_preempt(
        1,    // gvt_id
        0,    // worker_id (affinity)
        Duration::from_millis(50),
    );
    println!("Scheduled preempt for GVT 1 (worker 0, 50ms)");

    // Sleep timer (no affinity)
    let _sleep_handle = timer_registry.schedule_sleep(
        2,    // gvt_id
        Duration::from_millis(30),
        None, // no affinity
    );
    println!("Scheduled sleep for GVT 2 (any worker, 30ms)");

    // Sleep with affinity (maintain cache locality)
    let _affine_sleep = timer_registry.schedule_sleep(
        3,
        Duration::from_millis(40),
        Some(2), // prefer worker 2
    );
    println!("Scheduled sleep for GVT 3 (worker 2, 40ms)");

    // Timeout timer
    let timeout_handle = timer_registry.schedule_timeout(
        4,
        Duration::from_millis(100),
        None,
    );
    println!("Scheduled timeout for GVT 4 (100ms)");

    println!("\nActive timers: {}", timer_registry.active_timers());

    // Simulate GVT 1 yielding early - cancel its preemption
    std::thread::sleep(Duration::from_millis(20));
    if timer_registry.cancel(preempt_handle) {
        println!("\nGVT 1 yielded early, cancelled preemption timer");
    }

    // Simulate operation completing before timeout
    std::thread::sleep(Duration::from_millis(20));
    if timer_registry.cancel(timeout_handle) {
        println!("GVT 4 operation completed, cancelled timeout");
    }

    println!("\nActive timers after cancellations: {}", timer_registry.active_timers());

    // Wait for remaining timers to fire
    println!("\nWaiting for timers to fire...\n");
    std::thread::sleep(Duration::from_millis(100));

    // Shutdown
    println!("\nShutting down timer thread...");
    let stats = timer_handle.shutdown();

    println!("\n=== Timer Thread Stats ===");
    println!("Poll count: {}", stats.poll_count);
    println!("Timers fired: {}", stats.timers_fired);
    println!("Max batch size: {}", stats.max_batch_size);
    println!("Total poll time: {:?}", stats.poll_time);
}

```