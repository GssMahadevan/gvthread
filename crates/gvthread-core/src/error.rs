//! Error types for the GVThread scheduler

use core::fmt;

/// Result type for scheduler operations
pub type SchedResult<T> = Result<T, SchedError>;

/// Errors that can occur in scheduler operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedError {
    /// Operation was cancelled via CancellationToken
    Cancelled,
    
    /// Operation timed out
    Timeout,
    
    /// Channel was closed
    ChannelClosed,
    
    /// Channel is full (for try_send)
    ChannelFull,
    
    /// Channel is empty (for try_recv)
    ChannelEmpty,
    
    /// No GVThread slots available
    NoSlotsAvailable,
    
    /// GVThread not found
    GVThreadNotFound,
    
    /// Invalid GVThread state for operation
    InvalidState,
    
    /// Scheduler not initialized
    NotInitialized,
    
    /// Scheduler already initialized
    AlreadyInitialized,
    
    /// Memory allocation/mapping failed
    MemoryError(MemoryError),
    
    /// Worker thread error
    WorkerError(WorkerError),
    
    /// Platform-specific error
    PlatformError(i32),
}

impl fmt::Display for SchedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchedError::Cancelled => write!(f, "operation cancelled"),
            SchedError::Timeout => write!(f, "operation timed out"),
            SchedError::ChannelClosed => write!(f, "channel closed"),
            SchedError::ChannelFull => write!(f, "channel full"),
            SchedError::ChannelEmpty => write!(f, "channel empty"),
            SchedError::NoSlotsAvailable => write!(f, "no GVThread slots available"),
            SchedError::GVThreadNotFound => write!(f, "GVThread not found"),
            SchedError::InvalidState => write!(f, "invalid GVThread state"),
            SchedError::NotInitialized => write!(f, "scheduler not initialized"),
            SchedError::AlreadyInitialized => write!(f, "scheduler already initialized"),
            SchedError::MemoryError(e) => write!(f, "memory error: {}", e),
            SchedError::WorkerError(e) => write!(f, "worker error: {}", e),
            SchedError::PlatformError(code) => write!(f, "platform error: {}", code),
        }
    }
}

impl std::error::Error for SchedError {}

/// Memory-related errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryError {
    /// mmap or VirtualAlloc failed
    AllocationFailed,
    
    /// mprotect or VirtualProtect failed
    ProtectionFailed,
    
    /// madvise failed
    AdviseFailed,
    
    /// Region already initialized
    AlreadyInitialized,
    
    /// Too many slots requested
    TooManySlots,
    
    /// Invalid slot ID
    InvalidSlot,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryError::AllocationFailed => write!(f, "memory allocation failed"),
            MemoryError::ProtectionFailed => write!(f, "memory protection change failed"),
            MemoryError::AdviseFailed => write!(f, "memory advise failed"),
            MemoryError::AlreadyInitialized => write!(f, "memory region already initialized"),
            MemoryError::TooManySlots => write!(f, "too many slots requested"),
            MemoryError::InvalidSlot => write!(f, "invalid slot ID"),
        }
    }
}

impl From<MemoryError> for SchedError {
    fn from(e: MemoryError) -> Self {
        SchedError::MemoryError(e)
    }
}

/// Worker thread related errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerError {
    /// Failed to spawn worker thread
    SpawnFailed,
    
    /// Worker thread panicked
    Panicked,
    
    /// Failed to set thread affinity
    AffinityFailed,
    
    /// Signal setup failed
    SignalSetupFailed,
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerError::SpawnFailed => write!(f, "failed to spawn worker thread"),
            WorkerError::Panicked => write!(f, "worker thread panicked"),
            WorkerError::AffinityFailed => write!(f, "failed to set thread affinity"),
            WorkerError::SignalSetupFailed => write!(f, "signal setup failed"),
        }
    }
}

impl From<WorkerError> for SchedError {
    fn from(e: WorkerError) -> Self {
        SchedError::WorkerError(e)
    }
}

/// Error returned when trying to send on a full channel
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrySendError<T>(pub T);

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "channel full")
    }
}

/// Error returned when trying to receive from an empty channel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TryRecvError;

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "channel empty")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_display() {
        let e = SchedError::Cancelled;
        assert_eq!(format!("{}", e), "operation cancelled");
        
        let e = SchedError::MemoryError(MemoryError::AllocationFailed);
        assert_eq!(format!("{}", e), "memory error: memory allocation failed");
    }
    
    #[test]
    fn test_error_conversion() {
        let mem_err = MemoryError::TooManySlots;
        let sched_err: SchedError = mem_err.into();
        assert!(matches!(sched_err, SchedError::MemoryError(MemoryError::TooManySlots)));
    }
}
