//! Bounded MPMC channel for GVThread communication
//!
//! This channel is designed to work with the GVThread scheduler.
//! When a send or receive would block, the calling GVThread yields
//! to the scheduler instead of blocking the OS thread.

use std::collections::VecDeque;
use std::sync::Arc;
use crate::id::GVThreadId;
use crate::spinlock::SpinLock;
use crate::error::{SchedError, SchedResult, TrySendError, TryRecvError};

/// Create a new bounded channel with the specified capacity
pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let inner = Arc::new(ChannelInner {
        buffer: SpinLock::new(VecDeque::with_capacity(capacity)),
        capacity,
        send_waiters: SpinLock::new(VecDeque::new()),
        recv_waiters: SpinLock::new(VecDeque::new()),
        closed: SpinLock::new(false),
        sender_count: SpinLock::new(1),
        receiver_count: SpinLock::new(1),
    });
    
    (
        Sender { inner: Arc::clone(&inner) },
        Receiver { inner },
    )
}

/// Sending half of a channel
pub struct Sender<T> {
    inner: Arc<ChannelInner<T>>,
}

/// Receiving half of a channel
pub struct Receiver<T> {
    inner: Arc<ChannelInner<T>>,
}

/// Internal channel state
struct ChannelInner<T> {
    /// Ring buffer of messages
    buffer: SpinLock<VecDeque<T>>,
    
    /// Maximum buffer size
    capacity: usize,
    
    /// GVThreads waiting to send (buffer full)
    send_waiters: SpinLock<VecDeque<GVThreadId>>,
    
    /// GVThreads waiting to receive (buffer empty)
    recv_waiters: SpinLock<VecDeque<GVThreadId>>,
    
    /// Channel closed flag
    closed: SpinLock<bool>,
    
    /// Number of senders
    sender_count: SpinLock<usize>,
    
    /// Number of receivers
    receiver_count: SpinLock<usize>,
}

impl<T> Sender<T> {
    /// Send a value, blocking (yielding) if the channel is full
    ///
    /// Returns `Err(Cancelled)` if the token is cancelled while waiting.
    /// Returns `Err(ChannelClosed)` if all receivers have been dropped.
    pub fn send(&self, value: T) -> SchedResult<()> {
        loop {
            // Check if channel is closed
            if *self.inner.closed.lock() {
                return Err(SchedError::ChannelClosed);
            }
            
            // Try to send without blocking
            match self.try_send_inner(value) {
                Ok(()) => {
                    // Wake a waiting receiver if any
                    self.wake_receiver();
                    return Ok(());
                }
                Err(TrySendError(returned_value)) => {
                    // Buffer full, need to wait
                    // In real implementation, this would:
                    // 1. Add current GVThread to send_waiters
                    // 2. Yield to scheduler
                    // 3. When woken, retry
                    
                    // For now, just spin (will be fixed when integrated with scheduler)
                    std::thread::yield_now();
                    
                    // Try again with the returned value
                    match self.try_send_inner(returned_value) {
                        Ok(()) => {
                            self.wake_receiver();
                            return Ok(());
                        }
                        Err(TrySendError(v)) => {
                            // Still full, continue loop
                            // This is a placeholder - real impl yields to scheduler
                            std::hint::spin_loop();
                            return self.send(v); // Recursive retry (will be proper yield)
                        }
                    }
                }
            }
        }
    }
    
    /// Try to send without blocking
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        if *self.inner.closed.lock() {
            return Err(TrySendError(value));
        }
        
        let result = self.try_send_inner(value);
        if result.is_ok() {
            self.wake_receiver();
        }
        result
    }
    
    fn try_send_inner(&self, value: T) -> Result<(), TrySendError<T>> {
        let mut buffer = self.inner.buffer.lock();
        if buffer.len() >= self.inner.capacity {
            Err(TrySendError(value))
        } else {
            buffer.push_back(value);
            Ok(())
        }
    }
    
    fn wake_receiver(&self) {
        let mut waiters = self.inner.recv_waiters.lock();
        if let Some(_waiter) = waiters.pop_front() {
            // TODO: Mark waiter GVThread as ready via scheduler
            // scheduler.mark_ready(waiter);
        }
    }
    
    /// Check if the channel is closed
    pub fn is_closed(&self) -> bool {
        *self.inner.closed.lock()
    }
    
    /// Get current number of items in the buffer
    pub fn len(&self) -> usize {
        self.inner.buffer.lock().len()
    }
    
    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.inner.buffer.lock().is_empty()
    }
    
    /// Get channel capacity
    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }
}

impl<T> Receiver<T> {
    /// Receive a value, blocking (yielding) if the channel is empty
    ///
    /// Returns `Err(Cancelled)` if the token is cancelled while waiting.
    /// Returns `Err(ChannelClosed)` if all senders have been dropped and buffer is empty.
    pub fn recv(&self) -> SchedResult<T> {
        loop {
            // Try to receive without blocking
            match self.try_recv_inner() {
                Ok(value) => {
                    // Wake a waiting sender if any
                    self.wake_sender();
                    return Ok(value);
                }
                Err(TryRecvError) => {
                    // Buffer empty
                    // Check if all senders are gone
                    if *self.inner.sender_count.lock() == 0 {
                        return Err(SchedError::ChannelClosed);
                    }
                    
                    // In real implementation, this would:
                    // 1. Add current GVThread to recv_waiters
                    // 2. Yield to scheduler
                    // 3. When woken, retry
                    
                    // For now, just spin (will be fixed when integrated with scheduler)
                    std::thread::yield_now();
                    std::hint::spin_loop();
                }
            }
        }
    }
    
    /// Try to receive without blocking
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let result = self.try_recv_inner();
        if result.is_ok() {
            self.wake_sender();
        }
        result
    }
    
    fn try_recv_inner(&self) -> Result<T, TryRecvError> {
        let mut buffer = self.inner.buffer.lock();
        buffer.pop_front().ok_or(TryRecvError)
    }
    
    fn wake_sender(&self) {
        let mut waiters = self.inner.send_waiters.lock();
        if let Some(_waiter) = waiters.pop_front() {
            // TODO: Mark waiter GVThread as ready via scheduler
            // scheduler.mark_ready(waiter);
        }
    }
    
    /// Check if the channel is closed
    pub fn is_closed(&self) -> bool {
        *self.inner.closed.lock()
    }
    
    /// Get current number of items in the buffer
    pub fn len(&self) -> usize {
        self.inner.buffer.lock().len()
    }
    
    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.inner.buffer.lock().is_empty()
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        *self.inner.sender_count.lock() += 1;
        Sender { inner: Arc::clone(&self.inner) }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut count = self.inner.sender_count.lock();
        *count -= 1;
        if *count == 0 {
            // Last sender dropped, close channel
            *self.inner.closed.lock() = true;
            // Wake all waiting receivers
            let mut waiters = self.inner.recv_waiters.lock();
            waiters.clear();
            // TODO: Mark all waiters as ready with ChannelClosed error
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        *self.inner.receiver_count.lock() += 1;
        Receiver { inner: Arc::clone(&self.inner) }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut count = self.inner.receiver_count.lock();
        *count -= 1;
        if *count == 0 {
            // Last receiver dropped, close channel
            *self.inner.closed.lock() = true;
            // Wake all waiting senders
            let mut waiters = self.inner.send_waiters.lock();
            waiters.clear();
            // TODO: Mark all waiters as ready with ChannelClosed error
        }
    }
}

// Safety: Channel is safe to share between threads
unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}
unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Sync for Receiver<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_send_recv() {
        let (tx, rx) = channel(10);
        
        tx.try_send(42).unwrap();
        assert_eq!(rx.try_recv().unwrap(), 42);
    }
    
    #[test]
    fn test_multiple_values() {
        let (tx, rx) = channel(10);
        
        for i in 0..5 {
            tx.try_send(i).unwrap();
        }
        
        for i in 0..5 {
            assert_eq!(rx.try_recv().unwrap(), i);
        }
    }
    
    #[test]
    fn test_buffer_full() {
        let (tx, rx) = channel(2);
        
        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        
        // Buffer full
        let result = tx.try_send(3);
        assert!(result.is_err());
        
        // Make room
        rx.try_recv().unwrap();
        
        // Now should succeed
        tx.try_send(3).unwrap();
    }
    
    #[test]
    fn test_empty_recv() {
        let (_tx, rx) = channel::<i32>(10);
        
        let result = rx.try_recv();
        assert!(result.is_err());
    }
    
    #[test]
    fn test_sender_drop_closes() {
        let (tx, rx) = channel::<i32>(10);
        
        tx.try_send(1).unwrap();
        drop(tx);
        
        // Can still receive buffered value
        assert_eq!(rx.try_recv().unwrap(), 1);
        
        // Now should indicate closed
        assert!(rx.is_closed());
    }
    
    #[test]
    fn test_clone_sender() {
        let (tx1, rx) = channel(10);
        let tx2 = tx1.clone();
        
        tx1.try_send(1).unwrap();
        tx2.try_send(2).unwrap();
        
        assert_eq!(rx.len(), 2);
    }
}
