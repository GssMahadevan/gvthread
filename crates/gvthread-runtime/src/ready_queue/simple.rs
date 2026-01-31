//! Simple Go-like ready queue (MVP)
//!
//! Design:
//! - Per-worker local queue (VecDeque, SpinLock)
//! - Global queue (VecDeque, Mutex + Condvar)
//! - Work stealing from random victim
//! - Single priority (all treated as Normal)

use super::ReadyQueue;
use gvthread_core::id::GVThreadId;
use gvthread_core::state::Priority;
use gvthread_core::SpinLock;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::sync::{Mutex, Condvar};
use std::time::Duration;

/// Local queue capacity per worker
const LOCAL_CAPACITY: usize = 256;

/// Check global every N pops (Go uses 61)
const GLOBAL_CHECK_INTERVAL: u32 = 61;

/// Per-worker local queue
struct LocalQueue {
    queue: SpinLock<VecDeque<u32>>,
    len: AtomicUsize,
}

impl LocalQueue {
    fn new() -> Self {
        Self {
            queue: SpinLock::new(VecDeque::with_capacity(LOCAL_CAPACITY)),
            len: AtomicUsize::new(0),
        }
    }
    
    /// Push to back. Returns false if full.
    fn push(&self, id: u32) -> bool {
        let mut q = self.queue.lock();
        if q.len() >= LOCAL_CAPACITY {
            return false;
        }
        q.push_back(id);
        self.len.store(q.len(), Ordering::Release);
        true
    }
    
    /// Pop from front
    fn pop(&self) -> Option<u32> {
        if self.len.load(Ordering::Acquire) == 0 {
            return None;
        }
        let mut q = self.queue.lock();
        let item = q.pop_front();
        self.len.store(q.len(), Ordering::Release);
        item
    }
    
    /// Steal half from front (for work stealing)
    fn steal_half(&self) -> Vec<u32> {
        if self.len.load(Ordering::Acquire) == 0 {
            return Vec::new();
        }
        let mut q = self.queue.lock();
        let n = q.len() / 2;
        if n == 0 {
            return Vec::new();
        }
        let mut stolen = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(id) = q.pop_front() {
                stolen.push(id);
            }
        }
        self.len.store(q.len(), Ordering::Release);
        stolen
    }
    
    fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }
}

/// Global queue with parking
struct GlobalQueue {
    queue: Mutex<VecDeque<u32>>,
    cond: Condvar,
    len: AtomicUsize,
    parked: AtomicUsize,
}

impl GlobalQueue {
    fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            cond: Condvar::new(),
            len: AtomicUsize::new(0),
            parked: AtomicUsize::new(0),
        }
    }
    
    fn push(&self, id: u32) {
        {
            let mut q = self.queue.lock().unwrap();
            q.push_back(id);
            self.len.store(q.len(), Ordering::Release);
        }
        // Wake one parked worker
        if self.parked.load(Ordering::Acquire) > 0 {
            self.cond.notify_one();
        }
    }
    
    fn pop(&self) -> Option<u32> {
        if self.len.load(Ordering::Acquire) == 0 {
            return None;
        }
        let mut q = self.queue.lock().unwrap();
        let item = q.pop_front();
        self.len.store(q.len(), Ordering::Release);
        item
    }
    
    /// Pop batch for better throughput
    fn pop_batch(&self, max: usize) -> Vec<u32> {
        if self.len.load(Ordering::Acquire) == 0 {
            return Vec::new();
        }
        let mut q = self.queue.lock().unwrap();
        let n = q.len().min(max);
        let mut batch = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(id) = q.pop_front() {
                batch.push(id);
            }
        }
        self.len.store(q.len(), Ordering::Release);
        batch
    }
    
    fn park(&self, timeout_ms: u64) {
        self.parked.fetch_add(1, Ordering::AcqRel);
        let guard = self.queue.lock().unwrap();
        if guard.is_empty() {
            let _ = self.cond.wait_timeout(guard, Duration::from_millis(timeout_ms));
        }
        self.parked.fetch_sub(1, Ordering::AcqRel);
    }
    
    fn wake_one(&self) {
        self.cond.notify_one();
    }
    
    fn wake_all(&self) {
        self.cond.notify_all();
    }
    
    fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }
    
    fn parked_count(&self) -> usize {
        self.parked.load(Ordering::Acquire)
    }
}

/// Simple Go-like scheduler (MVP)
pub struct SimpleQueue {
    local: Vec<LocalQueue>,
    global: GlobalQueue,
    num_workers: AtomicUsize,
    /// Per-worker counter for periodic global check
    counters: Vec<AtomicUsize>,
    /// Per-worker RNG for stealing
    rng: Vec<AtomicUsize>,
    /// Initialized flag
    initialized: AtomicBool,
}

impl SimpleQueue {
    pub fn new() -> Self {
        Self {
            local: Vec::new(),
            global: GlobalQueue::new(65536),
            num_workers: AtomicUsize::new(0),
            counters: Vec::new(),
            rng: Vec::new(),
            initialized: AtomicBool::new(false),
        }
    }
    
    /// Initialize with worker count (called once at startup)
    pub fn init(&mut self, num_workers: usize) {
        if self.initialized.swap(true, Ordering::SeqCst) {
            return; // Already initialized
        }
        
        self.local = (0..num_workers).map(|_| LocalQueue::new()).collect();
        self.counters = (0..num_workers).map(|_| AtomicUsize::new(0)).collect();
        self.rng = (0..num_workers)
            .map(|i| AtomicUsize::new(i.wrapping_mul(2654435761) + 1))
            .collect();
        self.num_workers.store(num_workers, Ordering::Release);
    }
    
    /// Simple LCG random for victim selection
    fn random_victim(&self, worker_id: usize) -> usize {
        let num = self.num_workers.load(Ordering::Relaxed);
        if num <= 1 {
            return 0;
        }
        let rng = &self.rng[worker_id];
        let old = rng.load(Ordering::Relaxed);
        let new = old.wrapping_mul(1103515245).wrapping_add(12345);
        rng.store(new, Ordering::Relaxed);
        new % num
    }
    
    /// Try to steal from another worker
    fn try_steal(&self, worker_id: usize) -> Option<u32> {
        let num = self.num_workers.load(Ordering::Relaxed);
        
        // Try a few random victims
        for _ in 0..num.min(4) {
            let victim = self.random_victim(worker_id);
            if victim == worker_id {
                continue;
            }
            
            let stolen = self.local[victim].steal_half();
            if !stolen.is_empty() {
                let mut iter = stolen.into_iter();
                let first = iter.next();
                
                // Push rest to our local queue
                for id in iter {
                    if !self.local[worker_id].push(id) {
                        self.global.push(id);
                    }
                }
                return first;
            }
        }
        None
    }
}

impl Default for SimpleQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadyQueue for SimpleQueue {
    fn push(&self, id: GVThreadId, _priority: Priority, hint_worker: Option<usize>) {
        let gid = id.as_u32();
        
        // Try local queue if hint provided
        if let Some(w) = hint_worker {
            let num = self.num_workers.load(Ordering::Relaxed);
            if w < num && self.local[w].push(gid) {
                // Wake a worker since we added work
                if self.global.parked_count() > 0 {
                    self.global.wake_one();
                }
                return;
            }
        }
        
        // Fall back to global
        self.global.push(gid);
    }
    
    fn pop(&self, worker_id: usize) -> Option<(GVThreadId, Priority)> {
        let num = self.num_workers.load(Ordering::Relaxed);
        if worker_id >= num {
            return None;
        }
        
        // Increment counter, check global every N pops
        let cnt = self.counters[worker_id].fetch_add(1, Ordering::Relaxed) as u32;
        
        if cnt % GLOBAL_CHECK_INTERVAL == 0 {
            // Check global first (prevents starvation)
            if let Some(id) = self.global.pop() {
                return Some((GVThreadId::new(id), Priority::Normal));
            }
        }
        
        // 1. Try local
        if let Some(id) = self.local[worker_id].pop() {
            return Some((GVThreadId::new(id), Priority::Normal));
        }
        
        // 2. Try global + batch
        if let Some(id) = self.global.pop() {
            // Grab a batch for local
            let batch = self.global.pop_batch(LOCAL_CAPACITY / 2);
            for bid in batch {
                let _ = self.local[worker_id].push(bid);
            }
            return Some((GVThreadId::new(id), Priority::Normal));
        }
        
        // 3. Try steal
        if let Some(id) = self.try_steal(worker_id) {
            return Some((GVThreadId::new(id), Priority::Normal));
        }
        
        None
    }
    
    fn park(&self, _worker_id: usize, timeout_ms: u64) {
        self.global.park(timeout_ms);
    }
    
    fn wake_one(&self) {
        self.global.wake_one();
    }
    
    fn wake_all(&self) {
        self.global.wake_all();
    }
    
    fn len(&self) -> usize {
        let mut total = self.global.len();
        for lq in &self.local {
            total += lq.len();
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_local_queue() {
        let lq = LocalQueue::new();
        assert_eq!(lq.len(), 0);
        
        assert!(lq.push(1));
        assert!(lq.push(2));
        assert_eq!(lq.len(), 2);
        
        assert_eq!(lq.pop(), Some(1));
        assert_eq!(lq.pop(), Some(2));
        assert_eq!(lq.pop(), None);
    }
    
    #[test]
    fn test_simple_queue() {
        let mut sq = SimpleQueue::new();
        sq.init(2);
        
        // Push to global
        sq.push(GVThreadId::new(1), Priority::Normal, None);
        sq.push(GVThreadId::new(2), Priority::Normal, None);
        
        // Pop from worker 0
        assert!(sq.pop(0).is_some());
        assert!(sq.pop(0).is_some());
        assert!(sq.pop(0).is_none());
    }
    
    #[test]
    fn test_local_hint() {
        let mut sq = SimpleQueue::new();
        sq.init(2);
        
        sq.push(GVThreadId::new(10), Priority::Normal, Some(0));
        sq.push(GVThreadId::new(20), Priority::Normal, Some(1));
        
        // Worker 0 gets 10
        let r0 = sq.pop(0);
        assert_eq!(r0.map(|(id, _)| id.as_u32()), Some(10));
        
        // Worker 1 gets 20
        let r1 = sq.pop(1);
        assert_eq!(r1.map(|(id, _)| id.as_u32()), Some(20));
    }
    
    #[test]
    fn test_work_stealing() {
        let mut sq = SimpleQueue::new();
        sq.init(2);
        
        // Fill worker 0's local
        for i in 0..10 {
            sq.push(GVThreadId::new(i), Priority::Normal, Some(0));
        }
        
        // Worker 1 should steal
        let r = sq.pop(1);
        assert!(r.is_some());
    }
}