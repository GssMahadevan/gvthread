//! Timer worker thread
//!
//! Single thread that polls the timer backend and wakes expired GVTs
//! by pushing them to the ready queue.
//!
//! # Design
//!
//! The timer thread:
//! 1. Polls the backend for expired timers
//! 2. For each expired timer, calls `ready_queue.wake(gvt_id, affinity)`
//! 3. Sleeps until the next deadline (or a max poll interval)
//!
//! This design keeps the timer subsystem decoupled from worker topology -
//! the ready queue handles routing based on affinity.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::timer::{ExpiredTimer, TimerBackend};

/// Configuration for the timer thread
#[derive(Debug, Clone)]
pub struct TimerThreadConfig {
    /// Maximum time between polls (even if no timers are due)
    /// Default: 1ms
    pub max_poll_interval: Duration,

    /// Minimum sleep time (prevents busy-spinning)
    /// Default: 50µs
    pub min_sleep: Duration,

    /// Thread name
    /// Default: "gvt-timer"
    pub thread_name: String,

    /// Stack size for timer thread (None = system default)
    pub stack_size: Option<usize>,
}

impl Default for TimerThreadConfig {
    fn default() -> Self {
        Self {
            max_poll_interval: Duration::from_millis(1),
            min_sleep: Duration::from_micros(50),
            thread_name: "gvt-timer".into(),
            stack_size: None,
        }
    }
}

impl TimerThreadConfig {
    /// Create config optimized for low latency
    pub fn low_latency() -> Self {
        Self {
            max_poll_interval: Duration::from_micros(100),
            min_sleep: Duration::from_micros(10),
            thread_name: "gvt-timer".into(),
            stack_size: None,
        }
    }

    /// Create config optimized for low CPU usage
    pub fn low_cpu() -> Self {
        Self {
            max_poll_interval: Duration::from_millis(10),
            min_sleep: Duration::from_micros(500),
            thread_name: "gvt-timer".into(),
            stack_size: None,
        }
    }
}

/// Callback trait for waking GVTs
///
/// The ready queue implements this to receive timer expirations.
pub trait TimerWakeCallback: Send + Sync {
    /// Called when a timer expires
    ///
    /// # Arguments
    ///
    /// * `expired` - Information about the expired timer
    ///
    /// Implementation should push the GVT to the appropriate worker queue
    /// based on `expired.worker_affinity`.
    fn on_timer_expired(&self, expired: ExpiredTimer);
}

/// Handle to a running timer thread
pub struct TimerThreadHandle {
    handle: Option<JoinHandle<TimerStats>>,
    shutdown: Arc<AtomicBool>,
}

impl TimerThreadHandle {
    /// Request shutdown and wait for the timer thread to exit
    pub fn shutdown(mut self) -> TimerStats {
        self.shutdown.store(true, Ordering::Release);
        self.handle
            .take()
            .expect("handle already taken")
            .join()
            .expect("timer thread panicked")
    }

    /// Check if shutdown has been requested
    pub fn is_shutdown_requested(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// Request shutdown without waiting
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

/// Statistics from timer thread execution
#[derive(Debug, Clone, Default)]
pub struct TimerStats {
    /// Total poll iterations
    pub poll_count: u64,

    /// Total timers fired
    pub timers_fired: u64,

    /// Total time spent in poll_expired()
    pub poll_time: Duration,

    /// Maximum batch size (timers fired in single poll)
    pub max_batch_size: usize,
}

/// Spawn the timer thread
///
/// # Arguments
///
/// * `backend` - Timer backend to poll
/// * `wake_callback` - Called for each expired timer (typically ready queue)
/// * `shutdown` - Set to true to request shutdown
/// * `config` - Thread configuration
///
/// # Returns
///
/// Handle to the spawned thread
pub fn spawn_timer_thread<W>(
    backend: Arc<dyn TimerBackend>,
    wake_callback: Arc<W>,
    shutdown: Arc<AtomicBool>,
    config: TimerThreadConfig,
) -> TimerThreadHandle
where
    W: TimerWakeCallback + 'static,
{
    let shutdown_clone = shutdown.clone();

    let mut builder = thread::Builder::new().name(config.thread_name.clone());

    if let Some(stack_size) = config.stack_size {
        builder = builder.stack_size(stack_size);
    }

    let handle = builder
        .spawn(move || timer_loop(backend, wake_callback, shutdown_clone, config))
        .expect("failed to spawn timer thread");

    TimerThreadHandle {
        handle: Some(handle),
        shutdown,
    }
}

/// Main timer loop
fn timer_loop<W>(
    backend: Arc<dyn TimerBackend>,
    wake_callback: Arc<W>,
    shutdown: Arc<AtomicBool>,
    config: TimerThreadConfig,
) -> TimerStats
where
    W: TimerWakeCallback,
{
    let mut stats = TimerStats::default();

    while !shutdown.load(Ordering::Relaxed) {
        let poll_start = Instant::now();

        // Poll for expired timers
        let expired = backend.poll_expired(poll_start);
        let batch_size = expired.len();

        stats.poll_count += 1;
        stats.timers_fired += batch_size as u64;
        stats.max_batch_size = stats.max_batch_size.max(batch_size);

        // Wake all expired GVTs
        for timer in expired {
            wake_callback.on_timer_expired(timer);
        }

        stats.poll_time += poll_start.elapsed();

        // Smart sleep: until next deadline or max interval
        let sleep_duration = calculate_sleep(&backend, &config);
        
        if sleep_duration > Duration::ZERO {
            thread::sleep(sleep_duration);
        }
    }

    stats
}

/// Calculate how long to sleep before next poll
#[inline]
fn calculate_sleep(backend: &Arc<dyn TimerBackend>, config: &TimerThreadConfig) -> Duration {
    match backend.next_deadline() {
        Some(deadline) => {
            let now = Instant::now();
            if deadline <= now {
                // Timer already due, don't sleep
                Duration::ZERO
            } else {
                let until_deadline = deadline - now;
                // Sleep until deadline, but not longer than max_poll_interval
                // and not shorter than min_sleep
                until_deadline
                    .min(config.max_poll_interval)
                    .max(config.min_sleep)
            }
        }
        None => {
            // No timers scheduled, sleep for max interval
            config.max_poll_interval
        }
    }
}

// ============================================================================
// Convenience: Simple function-based callback
// ============================================================================

/// Simple callback wrapper for closures
pub struct FnWakeCallback<F>(pub F);

impl<F> TimerWakeCallback for FnWakeCallback<F>
where
    F: Fn(ExpiredTimer) + Send + Sync,
{
    fn on_timer_expired(&self, expired: ExpiredTimer) {
        (self.0)(expired)
    }
}

/// Spawn timer thread with a closure callback
pub fn spawn_timer_thread_fn<F>(
    backend: Arc<dyn TimerBackend>,
    wake_fn: F,
    shutdown: Arc<AtomicBool>,
    config: TimerThreadConfig,
) -> TimerThreadHandle
where
    F: Fn(ExpiredTimer) + Send + Sync + 'static,
{
    spawn_timer_thread(backend, Arc::new(FnWakeCallback(wake_fn)), shutdown, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::impls::HeapTimerBackend;
    use crate::timer::TimerEntry;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    struct TestCallback {
        woken: Mutex<Vec<u32>>,
        count: AtomicUsize,
    }

    impl TestCallback {
        fn new() -> Self {
            Self {
                woken: Mutex::new(Vec::new()),
                count: AtomicUsize::new(0),
            }
        }

        fn woken_gvts(&self) -> Vec<u32> {
            self.woken.lock().unwrap().clone()
        }
    }

    impl TimerWakeCallback for TestCallback {
        fn on_timer_expired(&self, expired: ExpiredTimer) {
            self.woken.lock().unwrap().push(expired.gvt_id);
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_timer_thread_basic() {
        let backend = Arc::new(HeapTimerBackend::new());
        let callback = Arc::new(TestCallback::new());
        let shutdown = Arc::new(AtomicBool::new(false));

        // Insert a timer that fires quickly
        backend.insert(TimerEntry::sleep(42, Duration::from_millis(10), None));

        let handle = spawn_timer_thread(
            backend,
            callback.clone(),
            shutdown,
            TimerThreadConfig::default(),
        );

        // Wait for timer to fire
        thread::sleep(Duration::from_millis(50));

        let stats = handle.shutdown();

        assert!(stats.timers_fired >= 1);
        assert!(callback.woken_gvts().contains(&42));
    }

    #[test]
    fn test_timer_thread_worker_affinity() {
        let backend = Arc::new(HeapTimerBackend::new());
        let callback = Arc::new(TestCallback::new());
        let shutdown = Arc::new(AtomicBool::new(false));

        // Insert preemption timer with affinity
        backend.insert(TimerEntry::preempt(42, 3, Duration::from_millis(5)));

        let handle = spawn_timer_thread(
            backend,
            callback.clone(),
            shutdown,
            TimerThreadConfig::default(),
        );

        thread::sleep(Duration::from_millis(50));

        handle.shutdown();

        assert!(callback.woken_gvts().contains(&42));
    }

    #[test]
    fn test_config_presets() {
        let low_latency = TimerThreadConfig::low_latency();
        let low_cpu = TimerThreadConfig::low_cpu();

        assert!(low_latency.max_poll_interval < low_cpu.max_poll_interval);
        assert!(low_latency.min_sleep < low_cpu.min_sleep);
    }

    #[test]
    fn test_fn_callback() {
        let backend = Arc::new(HeapTimerBackend::new());
        let woken = Arc::new(Mutex::new(Vec::new()));
        let woken_clone = woken.clone();
        let shutdown = Arc::new(AtomicBool::new(false));

        backend.insert(TimerEntry::sleep(42, Duration::from_millis(5), None));

        let handle = spawn_timer_thread_fn(
            backend,
            move |expired| {
                woken_clone.lock().unwrap().push(expired.gvt_id);
            },
            shutdown,
            TimerThreadConfig::default(),
        );

        thread::sleep(Duration::from_millis(50));
        handle.shutdown();

        assert!(woken.lock().unwrap().contains(&42));
    }
}