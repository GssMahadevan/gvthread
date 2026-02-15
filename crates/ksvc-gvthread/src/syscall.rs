//! # Blocking syscall wrappers for GVThreads
//!
//! Each function submits an I/O request to the reactor, blocks the
//! calling GVThread, and returns the result when the reactor wakes it.
//!
//! From the GVThread's perspective, these look like regular blocking calls.
//! Under the hood, the worker OS thread is freed to run other GVThreads.
//!
//! ```ignore
//! // Inside a GVThread — looks like blocking, but is async under the hood:
//! let n = ksvc_read(fd, &mut buf);
//! let n = ksvc_write(fd, &buf);
//! let client_fd = ksvc_accept4(listener, flags);
//! ```

use ksvc_core::entry::CorrId;

use gvthread_core::id::GVThreadId;
use gvthread_core::state::Priority;
use gvthread_runtime::scheduler;

use crate::reactor::{IoRequest, ReactorShared};

use std::sync::Arc;

// ── Linux x86_64 syscall numbers ──

const NR_READ: u32 = 0;
const NR_WRITE: u32 = 1;
const NR_CLOSE: u32 = 3;
const NR_SENDTO: u32 = 44;
const NR_RECVFROM: u32 = 45;
const NR_CONNECT: u32 = 42;
const NR_ACCEPT4: u32 = 288;
const NR_OPENAT: u32 = 257;
const NR_SOCKET: u32 = 41;
const NR_SHUTDOWN: u32 = 48;

/// Submit a syscall to the reactor and block until completion.
///
/// This is the core primitive. All typed wrappers call this.
///
/// # Returns
/// The syscall return value (>= 0 on success, negative errno on error).
///
/// # Panics
/// Panics if called outside a GVThread context.
#[inline]
fn submit_and_park(
    shared: &ReactorShared,
    syscall_nr: u32,
    args: [u64; 6],
) -> i64 {
    let gvt_id = gvthread_runtime::tls::current_gvthread_id();
    assert!(!gvt_id.is_none(), "ksvc_syscall called outside GVThread");
    let slot = gvt_id.as_u32();

    let req = IoRequest {
        corr_id: CorrId::from_gvthread_id(slot),
        syscall_nr,
        args,
        priority: Priority::Normal,
    };

    // Push to reactor queue
    // If queue is full, spin-yield until space. In practice the queue
    // should be sized large enough that this is rare.
    let mut pending = req;
    loop {
        match shared.request_queue.push(pending) {
            Ok(()) => break,
            Err(returned) => {
                // Queue full — yield and retry
                std::thread::yield_now();
                pending = returned;
            }
        }
    }

    // Block this GVThread — the worker thread is now free to run others
    scheduler::block_current();

    // We're back! The reactor wrote our result to the slab.
    shared.read_result(slot)
}

// ── Typed syscall wrappers ──

/// Read from a file descriptor. Returns bytes read or negative errno.
///
/// Equivalent to `read(fd, buf, len)` but non-blocking to the worker thread.
#[inline]
pub fn ksvc_read(shared: &ReactorShared, fd: i32, buf: &mut [u8]) -> i64 {
    submit_and_park(shared, NR_READ, [
        fd as u64,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
        0, 0, 0,
    ])
}

/// Write to a file descriptor. Returns bytes written or negative errno.
#[inline]
pub fn ksvc_write(shared: &ReactorShared, fd: i32, buf: &[u8]) -> i64 {
    submit_and_park(shared, NR_WRITE, [
        fd as u64,
        buf.as_ptr() as u64,
        buf.len() as u64,
        0, 0, 0,
    ])
}

/// Close a file descriptor.
#[inline]
pub fn ksvc_close(shared: &ReactorShared, fd: i32) -> i64 {
    submit_and_park(shared, NR_CLOSE, [
        fd as u64,
        0, 0, 0, 0, 0,
    ])
}

/// Accept a connection. Returns new fd or negative errno.
///
/// `flags` is typically `SOCK_CLOEXEC` or `SOCK_NONBLOCK | SOCK_CLOEXEC`.
#[inline]
pub fn ksvc_accept4(
    shared: &ReactorShared,
    listener_fd: i32,
    addr: *mut libc::sockaddr,
    addrlen: *mut libc::socklen_t,
    flags: i32,
) -> i64 {
    submit_and_park(shared, NR_ACCEPT4, [
        listener_fd as u64,
        addr as u64,
        addrlen as u64,
        flags as u64,
        0, 0,
    ])
}

/// Receive from a socket. Returns bytes received or negative errno.
#[inline]
pub fn ksvc_recv(shared: &ReactorShared, fd: i32, buf: &mut [u8], flags: i32) -> i64 {
    submit_and_park(shared, NR_RECVFROM, [
        fd as u64,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
        flags as u64,
        0, // NULL addr
        0, // NULL addrlen
    ])
}

/// Send to a socket. Returns bytes sent or negative errno.
#[inline]
pub fn ksvc_send(shared: &ReactorShared, fd: i32, buf: &[u8], flags: i32) -> i64 {
    submit_and_park(shared, NR_SENDTO, [
        fd as u64,
        buf.as_ptr() as u64,
        buf.len() as u64,
        flags as u64,
        0, // NULL dest_addr
        0, // 0 addrlen
    ])
}

/// Connect a socket. Returns 0 on success or negative errno.
#[inline]
pub fn ksvc_connect(
    shared: &ReactorShared,
    fd: i32,
    addr: *const libc::sockaddr,
    addrlen: libc::socklen_t,
) -> i64 {
    submit_and_park(shared, NR_CONNECT, [
        fd as u64,
        addr as u64,
        addrlen as u64,
        0, 0, 0,
    ])
}

/// Open a file. Returns fd or negative errno.
#[inline]
pub fn ksvc_openat(
    shared: &ReactorShared,
    dirfd: i32,
    pathname: *const libc::c_char,
    flags: i32,
    mode: u32,
) -> i64 {
    submit_and_park(shared, NR_OPENAT, [
        dirfd as u64,
        pathname as u64,
        flags as u64,
        mode as u64,
        0, 0,
    ])
}

/// Create a socket. Returns fd or negative errno.
#[inline]
pub fn ksvc_socket(
    shared: &ReactorShared,
    domain: i32,
    sock_type: i32,
    protocol: i32,
) -> i64 {
    submit_and_park(shared, NR_SOCKET, [
        domain as u64,
        sock_type as u64,
        protocol as u64,
        0, 0, 0,
    ])
}

/// Shutdown a socket.
#[inline]
pub fn ksvc_shutdown(shared: &ReactorShared, fd: i32, how: i32) -> i64 {
    submit_and_park(shared, NR_SHUTDOWN, [
        fd as u64,
        how as u64,
        0, 0, 0, 0,
    ])
}

// ── Convenience wrappers for common patterns ──

/// Send all bytes, retrying on partial writes.
pub fn ksvc_send_all(shared: &ReactorShared, fd: i32, mut buf: &[u8]) -> i64 {
    let mut total: usize = 0;
    while !buf.is_empty() {
        let n = ksvc_send(shared, fd, buf, 0);
        if n < 0 {
            if n == -(libc::EAGAIN as i64) || n == -(libc::EINTR as i64) {
                gvthread::yield_now();
                continue;
            }
            return n;
        }
        let n = n as usize;
        total += n;
        buf = &buf[n..];
    }
    total as i64
}

// ══════════════════════════════════════════════════════════════════════
// Worker-local path — submit directly to this worker's io_uring
// ══════════════════════════════════════════════════════════════════════

use crate::worker_reactor;

/// Submit a syscall to this worker's io_uring and block until completion.
///
/// This is the per-worker equivalent of `submit_and_park`.  Instead of
/// pushing to an MPSC queue (cross-thread hop), we submit the SQE inline
/// to the worker-local io_uring ring.  The worker loop polls CQEs and
/// wakes us on the same core — zero lock contention, zero cache bouncing.
///
/// # Panics
/// Panics if called outside a GVThread context or if the worker reactor
/// pool has not been initialized.
#[inline]
fn submit_and_park_worker(syscall_nr: u32, args: [u64; 6]) -> i64 {
    let gvt_id = gvthread_runtime::tls::current_gvthread_id();
    assert!(!gvt_id.is_none(), "ksvc_syscall called outside GVThread");
    let slot = gvt_id.as_u32();

    let worker_id = gvthread_runtime::worker::current_worker_id();
    assert!(worker_id != usize::MAX, "ksvc_syscall: not on a worker thread");

    let pool = worker_reactor::global_pool();

    // Submit SQE directly to this worker's io_uring (inline, no MPSC!)
    pool.submit(worker_id, slot, syscall_nr, &args);

    // Block this GVThread — worker is free to run others + poll CQEs
    scheduler::block_current();

    // Woken! Result was written to the slab by this worker's poll loop.
    pool.read_result(slot)
}

// ── Worker-local typed wrappers ──

/// Worker-local read.  Same semantics as `ksvc_read` but uses per-worker io_uring.
#[inline]
pub fn wr_read(fd: i32, buf: &mut [u8]) -> i64 {
    submit_and_park_worker(NR_READ, [
        fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64,
        0, 0, 0,
    ])
}

/// Worker-local write.
#[inline]
pub fn wr_write(fd: i32, buf: &[u8]) -> i64 {
    submit_and_park_worker(NR_WRITE, [
        fd as u64, buf.as_ptr() as u64, buf.len() as u64,
        0, 0, 0,
    ])
}

/// Worker-local close.
#[inline]
pub fn wr_close(fd: i32) -> i64 {
    submit_and_park_worker(NR_CLOSE, [fd as u64, 0, 0, 0, 0, 0])
}

/// Worker-local accept4.
#[inline]
pub fn wr_accept4(
    listener_fd: i32,
    addr: *mut libc::sockaddr,
    addrlen: *mut libc::socklen_t,
    flags: i32,
) -> i64 {
    submit_and_park_worker(NR_ACCEPT4, [
        listener_fd as u64, addr as u64, addrlen as u64,
        flags as u64, 0, 0,
    ])
}

/// Worker-local recv.
#[inline]
pub fn wr_recv(fd: i32, buf: &mut [u8], flags: i32) -> i64 {
    submit_and_park_worker(NR_RECVFROM, [
        fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64,
        flags as u64, 0, 0,
    ])
}

/// Worker-local send.
#[inline]
pub fn wr_send(fd: i32, buf: &[u8], flags: i32) -> i64 {
    submit_and_park_worker(NR_SENDTO, [
        fd as u64, buf.as_ptr() as u64, buf.len() as u64,
        flags as u64, 0, 0,
    ])
}

/// Worker-local send_all (retries partial writes).
pub fn wr_send_all(fd: i32, mut buf: &[u8]) -> i64 {
    let mut total: usize = 0;
    while !buf.is_empty() {
        let n = wr_send(fd, buf, 0);
        if n < 0 {
            if n == -(libc::EAGAIN as i64) || n == -(libc::EINTR as i64) {
                gvthread::yield_now();
                continue;
            }
            return n;
        }
        let n = n as usize;
        total += n;
        buf = &buf[n..];
    }
    total as i64
}