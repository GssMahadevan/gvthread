//! `EventFdNotifier` — default `Notifier` implementation.
//!
//! Writes to an eventfd to wake the userspace completion handler.
//! Coalescing: multiple calls to `notify()` before the consumer
//! reads the eventfd result in a single wakeup (eventfd counter semantics).

use ksvc_core::error::{KsvcError, Result};
use ksvc_core::notifier::Notifier;

use std::os::unix::io::RawFd;

pub struct EventFdNotifier {
    fd: RawFd,
    owned: bool,  // true if we created the fd (must close on drop)
}

impl EventFdNotifier {
    /// Create a notifier wrapping an existing eventfd.
    ///
    /// The eventfd should be created with `EFD_NONBLOCK | EFD_CLOEXEC`.
    /// Ownership of the fd remains with the caller — the notifier
    /// does NOT close it on drop.
    pub fn new(eventfd: RawFd) -> Self {
        Self { fd: eventfd, owned: false }
    }

    /// Create a new eventfd and wrap it.
    ///
    /// The notifier OWNS the fd and closes it on Drop.
    pub fn create() -> Result<Self> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(KsvcError::Os(unsafe { *libc::__errno_location() }));
        }
        Ok(Self { fd, owned: true })
    }

    /// Get the raw eventfd descriptor (for passing to the kernel module
    /// or for the completion handler to poll/read).
    pub fn fd(&self) -> RawFd {
        self.fd
    }
}

impl Notifier for EventFdNotifier {
    fn notify(&self) -> Result<()> {
        let val: u64 = 1;
        let ret = unsafe {
            libc::write(
                self.fd,
                &val as *const u64 as *const libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if ret < 0 {
            let errno = unsafe { *libc::__errno_location() };
            // EAGAIN is OK — means the counter would overflow,
            // which implies a signal is already pending. That's fine.
            if errno == libc::EAGAIN {
                return Ok(());
            }
            return Err(KsvcError::Os(errno));
        }
        Ok(())
    }
}

impl Drop for EventFdNotifier {
    fn drop(&mut self) {
        if self.owned && self.fd >= 0 {
            unsafe { libc::close(self.fd); }
            self.fd = -1;
        }
    }
}
