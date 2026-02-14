//! # High-level networking for GVThreads
//!
//! Provides `GvtListener` and `GvtStream` that wrap raw fds with
//! the ksvc_* syscall wrappers. These give a Go-like programming model:
//!
//! ```ignore
//! let listener = GvtListener::bind(reactor, 8080)?;
//! loop {
//!     let stream = listener.accept()?;
//!     gvthread::spawn(move |_| {
//!         handle_connection(stream);
//!     });
//! }
//! ```

use crate::reactor::ReactorShared;
use crate::syscall::*;

use std::sync::Arc;

/// A TCP listener bound to a port, using KSVC io_uring for accept().
pub struct GvtListener {
    fd: i32,
    shared: Arc<ReactorShared>,
}

impl GvtListener {
    /// Create a listener from an existing fd + reactor shared state.
    pub fn from_raw(fd: i32, shared: Arc<ReactorShared>) -> Self {
        Self { fd, shared }
    }

    /// Bind and listen on a port. Uses traditional syscalls for setup
    /// (bind/listen are one-shot, no need for io_uring).
    pub fn bind(shared: Arc<ReactorShared>, port: u16) -> Result<Self, i32> {
        let fd = unsafe {
            libc::socket(
                libc::AF_INET,
                libc::SOCK_STREAM | libc::SOCK_CLOEXEC,
                0,
            )
        };
        if fd < 0 {
            return Err(unsafe { *libc::__errno_location() });
        }

        // SO_REUSEADDR + SO_REUSEPORT
        unsafe {
            let opt: i32 = 1;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEADDR,
                &opt as *const _ as *const _,
                4,
            );
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                &opt as *const _ as *const _,
                4,
            );
            // TCP_NODELAY
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_NODELAY,
                &opt as *const _ as *const _,
                4,
            );
        }

        let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        addr.sin_family = libc::AF_INET as u16;
        addr.sin_addr.s_addr = 0; // INADDR_ANY
        addr.sin_port = port.to_be();

        let ret = unsafe {
            libc::bind(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of_val(&addr) as u32,
            )
        };
        if ret != 0 {
            unsafe { libc::close(fd); }
            return Err(unsafe { *libc::__errno_location() });
        }

        unsafe { libc::listen(fd, 4096); }

        Ok(Self { fd, shared })
    }

    /// Accept a connection. Blocks the calling GVThread until a client connects.
    ///
    /// Returns a `GvtStream` for the new connection.
    pub fn accept(&self) -> Result<GvtStream, i64> {
        let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        let mut addr_len: libc::socklen_t =
            std::mem::size_of::<libc::sockaddr_in>() as u32;

        let client_fd = ksvc_accept4(
            &self.shared,
            self.fd,
            &mut addr as *mut _ as *mut libc::sockaddr,
            &mut addr_len,
            libc::SOCK_CLOEXEC,
        );

        if client_fd < 0 {
            return Err(client_fd);
        }

        // TCP_NODELAY on accepted socket
        unsafe {
            let opt: i32 = 1;
            libc::setsockopt(
                client_fd as i32,
                libc::IPPROTO_TCP,
                libc::TCP_NODELAY,
                &opt as *const _ as *const _,
                4,
            );
        }

        Ok(GvtStream {
            fd: client_fd as i32,
            shared: self.shared.clone(),
        })
    }

    /// Get the raw fd.
    pub fn fd(&self) -> i32 {
        self.fd
    }
}

impl Drop for GvtListener {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd); }
    }
}

/// A TCP stream (connection), using KSVC io_uring for read/write.
pub struct GvtStream {
    fd: i32,
    shared: Arc<ReactorShared>,
}

impl GvtStream {
    /// Create a stream from a raw fd.
    pub fn from_raw(fd: i32, shared: Arc<ReactorShared>) -> Self {
        Self { fd, shared }
    }

    /// Read into buffer. Blocks the GVThread until data is available.
    /// Returns bytes read, 0 for EOF, or negative errno.
    pub fn read(&self, buf: &mut [u8]) -> i64 {
        ksvc_recv(&self.shared, self.fd, buf, 0)
    }

    /// Write buffer. Blocks until all bytes are sent.
    /// Returns total bytes written or negative errno.
    pub fn write_all(&self, buf: &[u8]) -> i64 {
        ksvc_send_all(&self.shared, self.fd, buf)
    }

    /// Write buffer (single send). Returns bytes sent or negative errno.
    pub fn write(&self, buf: &[u8]) -> i64 {
        ksvc_send(&self.shared, self.fd, buf, 0)
    }

    /// Close the connection via io_uring.
    pub fn close_uring(&self) -> i64 {
        ksvc_close(&self.shared, self.fd)
    }

    /// Get the raw fd.
    pub fn fd(&self) -> i32 {
        self.fd
    }

    /// Get the reactor shared state (for passing to sub-operations).
    pub fn shared(&self) -> &Arc<ReactorShared> {
        &self.shared
    }
}

impl Drop for GvtStream {
    fn drop(&mut self) {
        // Use synchronous close in drop (simpler, always works)
        unsafe { libc::close(self.fd); }
    }
}

// Safety: GvtStream can be sent to other GVThreads.
// The fd is valid until close, and the shared Arc is thread-safe.
unsafe impl Send for GvtStream {}
unsafe impl Sync for GvtStream {}
unsafe impl Send for GvtListener {}
unsafe impl Sync for GvtListener {}
