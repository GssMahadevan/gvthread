//! GVThread state and priority types

use core::fmt;

/// State of a GVThread
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GVThreadState {
    /// Just created, not yet started
    Created = 0,
    
    /// Ready to run, in the ready queue
    Ready = 1,
    
    /// Currently executing on a worker
    Running = 2,
    
    /// Blocked waiting for I/O, lock, channel, etc.
    Blocked = 3,
    
    /// Preempted by SIGURG (forced), needs full register restore
    Preempted = 4,
    
    /// Finished execution, awaiting cleanup
    Finished = 5,
    
    /// Cancelled
    Cancelled = 6,
}

impl GVThreadState {
    /// Check if this state allows the GVThread to be scheduled
    #[inline]
    pub const fn is_runnable(&self) -> bool {
        matches!(self, GVThreadState::Ready)
    }
    
    /// Check if this GVThread has terminated (finished or cancelled)
    #[inline]
    pub const fn is_terminated(&self) -> bool {
        matches!(self, GVThreadState::Finished | GVThreadState::Cancelled)
    }
    
    /// Check if this GVThread needs full register restore (was force-preempted)
    #[inline]
    pub const fn needs_full_restore(&self) -> bool {
        matches!(self, GVThreadState::Preempted)
    }
}

impl From<u8> for GVThreadState {
    fn from(v: u8) -> Self {
        match v {
            0 => GVThreadState::Created,
            1 => GVThreadState::Ready,
            2 => GVThreadState::Running,
            3 => GVThreadState::Blocked,
            4 => GVThreadState::Preempted,
            5 => GVThreadState::Finished,
            6 => GVThreadState::Cancelled,
            _ => GVThreadState::Created, // Default for invalid values
        }
    }
}

impl From<GVThreadState> for u8 {
    fn from(state: GVThreadState) -> u8 {
        state as u8
    }
}

/// Priority level for GVThreads
///
/// Higher priority GVThreads are scheduled before lower priority ones.
/// The scheduler uses separate bitmaps for each priority level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Priority {
    /// Critical priority - system tasks, interrupt handlers
    /// Always scheduled first, dedicated worker(s)
    Critical = 0,
    
    /// High priority - latency-sensitive tasks
    High = 1,
    
    /// Normal priority - default for user tasks
    Normal = 2,
    
    /// Low priority - background tasks, cleanup
    /// May be starved if higher priority work available
    Low = 3,
}

impl Priority {
    /// Number of priority levels
    pub const COUNT: usize = 4;
    
    /// Get priority as index (0 = Critical, 3 = Low)
    #[inline]
    pub const fn as_index(&self) -> usize {
        *self as usize
    }
    
    /// Get priority from index
    #[inline]
    pub const fn from_index(idx: usize) -> Option<Priority> {
        match idx {
            0 => Some(Priority::Critical),
            1 => Some(Priority::High),
            2 => Some(Priority::Normal),
            3 => Some(Priority::Low),
            _ => None,
        }
    }
    
    /// Iterator over all priorities (highest to lowest)
    pub fn iter() -> impl Iterator<Item = Priority> {
        [Priority::Critical, Priority::High, Priority::Normal, Priority::Low].into_iter()
    }
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

impl From<u8> for Priority {
    fn from(v: u8) -> Self {
        match v {
            0 => Priority::Critical,
            1 => Priority::High,
            2 => Priority::Normal,
            3 => Priority::Low,
            _ => Priority::Normal, // Default for invalid
        }
    }
}

impl From<Priority> for u8 {
    fn from(p: Priority) -> u8 {
        p as u8
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Priority::Critical => write!(f, "CRITICAL"),
            Priority::High => write!(f, "HIGH"),
            Priority::Normal => write!(f, "NORMAL"),
            Priority::Low => write!(f, "LOW"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_state_transitions() {
        assert!(GVThreadState::Ready.is_runnable());
        assert!(!GVThreadState::Running.is_runnable());
        assert!(!GVThreadState::Blocked.is_runnable());
        
        assert!(GVThreadState::Finished.is_terminated());
        assert!(GVThreadState::Cancelled.is_terminated());
        assert!(!GVThreadState::Running.is_terminated());
        
        assert!(GVThreadState::Preempted.needs_full_restore());
        assert!(!GVThreadState::Ready.needs_full_restore());
    }
    
    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Critical < Priority::High);
        assert!(Priority::High < Priority::Normal);
        assert!(Priority::Normal < Priority::Low);
    }
    
    #[test]
    fn test_priority_iter() {
        let priorities: Vec<_> = Priority::iter().collect();
        assert_eq!(priorities, vec![
            Priority::Critical,
            Priority::High,
            Priority::Normal,
            Priority::Low,
        ]);
    }
}
