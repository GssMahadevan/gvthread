//! `BasicIoUring` — default `IoBackend` implementation.
//!
//! Uses `io_uring_enter()` for submission, polls CQ for completions.
//! No SQPOLL, no fixed files, no fixed buffers.
//! Safe, correct, works on any kernel with io_uring (5.1+).

use ksvc_core::entry::{CorrId, SubmitEntry};
use ksvc_core::error::{KsvcError, Result};
use ksvc_core::io_backend::{IoBackend, IoCompletion};

use std::os::unix::io::{AsRawFd, RawFd};

/// Configuration for BasicIoUring.
pub struct BasicIoUringConfig {
    /// Number of SQ entries. Must be power of 2.
    pub sq_entries: u32,
    /// Number of CQ entries. Defaults to 2 * sq_entries.
    pub cq_entries: Option<u32>,
}

impl Default for BasicIoUringConfig {
    fn default() -> Self {
        Self {
            sq_entries: 256,
            cq_entries: None,
        }
    }
}

/// Default io_uring backend.
///
/// This wraps the `io-uring` crate's safe API. The dispatcher calls:
/// 1. `submit()` for each Tier 1 entry (queues SQE)
/// 2. `flush()` once per batch (calls io_uring_enter)
/// 3. `poll_completions()` to drain CQEs
///
/// NEVER blocks on the dispatch path. io-wq workers handle blocking.
pub struct BasicIoUring {
    ring: io_uring::IoUring,
    inflight: usize,
    pending_submit: u32,
}

impl BasicIoUring {
    pub fn new(config: BasicIoUringConfig) -> Result<Self> {
        let ring = io_uring::IoUring::builder()
            .build(config.sq_entries)
            .map_err(|e| KsvcError::IoUringSetup(e.raw_os_error().unwrap_or(-1)))?;

        Ok(Self {
            ring,
            inflight: 0,
            pending_submit: 0,
        })
    }

    /// Get the io_uring fd for passing to the kernel module.
    pub fn fd(&self) -> RawFd {
        self.ring.as_raw_fd()
    }

    /// Probe supported opcodes via IORING_REGISTER_PROBE.
    pub fn probe_opcodes_static(&self) -> Vec<u8> {
        // Use the io-uring crate's probe API
        match io_uring::Probe::new() {
            probe => {
                let mut opcodes = Vec::new();
                for opc in 0..=80u8 {
                    if probe.is_supported(opc) {
                        opcodes.push(opc);
                    }
                }
                opcodes
            }
        }
    }

    /// Translate a KSVC SubmitEntry to an io_uring SQE and push it.
    ///
    /// This is the core translation layer. The SubmitEntry's syscall_nr
    /// has already been routed to Tier 1, so we know the opcode.
    /// The `args` array contains the syscall arguments in standard order.
    fn translate_and_push(&mut self, entry: &SubmitEntry, opcode: u8) -> Result<()> {
        let user_data = entry.corr_id.0;

        // Safety: we're building SQEs from trusted dispatch data.
        // The io-uring crate provides safe wrappers for each opcode.
        // We use the opaque_entry path for a generic translation.
        unsafe {
            let sq = self.ring.submission_shared();
            if sq.is_full() {
                return Err(KsvcError::RingFull);
            }
        }

        // Build the SQE based on opcode.
        // This is a dispatcher — it translates KSVC entry args to io_uring SQE fields.
        //
        // For each opcode, the args[] mapping follows the syscall ABI:
        //   args[0] = fd,  args[1] = buf/addr,  args[2] = len,
        //   args[3] = offset/flags, args[4] = ..., args[5] = ...
        //
        // We construct the typed opcode entry from the io-uring crate.
        let sqe = self.build_sqe(entry, opcode)?;

        // Push the SQE
        unsafe {
            self.ring.submission()
                .push(&sqe)
                .map_err(|_| KsvcError::RingFull)?;
        }
        self.pending_submit += 1;
        Ok(())
    }

    /// Build a raw io_uring SQE from a KSVC submit entry.
    ///
    /// Uses the opaque entry type for maximum flexibility.
    /// Each opcode's argument mapping is documented inline.
    fn build_sqe(
        &self,
        entry: &SubmitEntry,
        opcode: u8,
    ) -> Result<io_uring::squeue::Entry> {
        use io_uring::opcode;
        use io_uring::types;

        let a = &entry.args;
        let user_data = entry.corr_id.0;

        // Helper closures for common patterns
        let fd = types::Fd(a[0] as i32);

        let sqe = match opcode {
            // ── File I/O ──
            // read(fd, buf, count) → READ(fd, buf, len, offset=-1)
            super::probe_router::op::READ => {
                opcode::Read::new(fd, a[1] as *mut u8, a[2] as u32)
                    .offset(u64::MAX) // -1 = use current file position
                    .build()
            }
            // write(fd, buf, count) → WRITE(fd, buf, len, offset=-1)
            super::probe_router::op::WRITE => {
                opcode::Write::new(fd, a[1] as *const u8, a[2] as u32)
                    .offset(u64::MAX)
                    .build()
            }
            // readv(fd, iov, iovcnt) → READV(fd, iov, iovcnt)
            super::probe_router::op::READV => {
                let iov = a[1] as *const libc::iovec;
                let iovcnt = a[2] as u32;
                opcode::Readv::new(fd, iov, iovcnt)
                    .offset(u64::MAX)
                    .build()
            }
            // writev(fd, iov, iovcnt) → WRITEV(fd, iov, iovcnt)
            super::probe_router::op::WRITEV => {
                let iov = a[1] as *const libc::iovec;
                let iovcnt = a[2] as u32;
                opcode::Writev::new(fd, iov, iovcnt)
                    .offset(u64::MAX)
                    .build()
            }

            // ── File lifecycle ──
            // openat(dirfd, pathname, flags, mode)
            super::probe_router::op::OPENAT => {
                let dirfd = types::Fd(a[0] as i32);
                let path = a[1] as *const libc::c_char;
                opcode::OpenAt::new(dirfd, path)
                    .flags(a[2] as i32)
                    .mode(a[3] as u32)
                    .build()
            }
            // close(fd)
            super::probe_router::op::CLOSE => {
                opcode::Close::new(fd)
                    .build()
            }
            // statx(dirfd, pathname, flags, mask, statxbuf)
            super::probe_router::op::STATX => {
                let dirfd = types::Fd(a[0] as i32);
                let path = a[1] as *const libc::c_char;
                let statxbuf = a[4] as *mut io_uring::types::statx;
                opcode::Statx::new(dirfd, path, statxbuf)
                    .flags(a[2] as i32)
                    .mask(a[3] as u32)
                    .build()
            }
            // fallocate(fd, mode, offset, len)
            super::probe_router::op::FALLOCATE => {
                opcode::Fallocate::new(fd, a[2] as u64)
                    .offset(a[1] as u64) // NB: fallocate arg order differs
                    .mode(a[1] as i32)
                    .build()
            }

            // ── Sync ──
            // fsync(fd) or fdatasync(fd)
            super::probe_router::op::FSYNC => {
                let mut builder = opcode::Fsync::new(fd);
                // If original syscall was fdatasync, set DATASYNC flag
                if entry.syscall_nr == 75 { // __NR_fdatasync
                    builder = builder.flags(io_uring::types::FsyncFlags::DATASYNC);
                }
                builder.build()
            }

            // ── Network ──
            // accept4(sockfd, addr, addrlen, flags)
            super::probe_router::op::ACCEPT => {
                opcode::Accept::new(fd, a[1] as *mut libc::sockaddr, a[2] as *mut libc::socklen_t)
                    .flags(a[3] as i32)
                    .build()
            }
            // connect(sockfd, addr, addrlen)
            super::probe_router::op::CONNECT => {
                opcode::Connect::new(fd, a[1] as *const libc::sockaddr, a[2] as u32)
                    .build()
            }
            // send(sockfd, buf, len, flags)
            super::probe_router::op::SEND => {
                opcode::Send::new(fd, a[1] as *const u8, a[2] as u32)
                    .flags(a[3] as i32)
                    .build()
            }
            // recv(sockfd, buf, len, flags)
            super::probe_router::op::RECV => {
                opcode::Recv::new(fd, a[1] as *mut u8, a[2] as u32)
                    .flags(a[3] as i32)
                    .build()
            }
            // sendmsg(sockfd, msg, flags)
            super::probe_router::op::SENDMSG => {
                opcode::SendMsg::new(fd, a[1] as *const libc::msghdr)
                    .flags(a[2] as u32)
                    .build()
            }
            // recvmsg(sockfd, msg, flags)
            super::probe_router::op::RECVMSG => {
                opcode::RecvMsg::new(fd, a[1] as *mut libc::msghdr)
                    .flags(a[2] as u32)
                    .build()
            }
            // shutdown(sockfd, how)
            super::probe_router::op::SHUTDOWN => {
                opcode::Shutdown::new(fd, a[1] as i32)
                    .build()
            }
            // socket(domain, type, protocol)
            super::probe_router::op::SOCKET => {
                opcode::Socket::new(a[0] as i32, a[1] as i32, a[2] as i32)
                    .build()
            }

            // ── Metadata ops ──
            super::probe_router::op::RENAMEAT => {
                let olddirfd = types::Fd(a[0] as i32);
                let oldpath = a[1] as *const libc::c_char;
                let newdirfd = types::Fd(a[2] as i32);
                let newpath = a[3] as *const libc::c_char;
                opcode::RenameAt::new(olddirfd, oldpath, newdirfd, newpath)
                    .flags(a[4] as u32)
                    .build()
            }
            super::probe_router::op::UNLINKAT => {
                let dirfd = types::Fd(a[0] as i32);
                let path = a[1] as *const libc::c_char;
                opcode::UnlinkAt::new(dirfd, path)
                    .flags(a[2] as i32)
                    .build()
            }
            super::probe_router::op::MKDIRAT => {
                let dirfd = types::Fd(a[0] as i32);
                let path = a[1] as *const libc::c_char;
                opcode::MkDirAt::new(dirfd, path)
                    .mode(a[2] as u32)
                    .build()
            }
            super::probe_router::op::SYMLINKAT => {
                let target = a[0] as *const libc::c_char;
                let newdirfd = types::Fd(a[1] as i32);
                let linkpath = a[2] as *const libc::c_char;
                opcode::SymlinkAt::new(newdirfd, target, linkpath)
                    .build()
            }
            super::probe_router::op::LINKAT => {
                let olddirfd = types::Fd(a[0] as i32);
                let oldpath = a[1] as *const libc::c_char;
                let newdirfd = types::Fd(a[2] as i32);
                let newpath = a[3] as *const libc::c_char;
                opcode::LinkAt::new(olddirfd, oldpath, newdirfd, newpath)
                    .flags(a[4] as i32)
                    .build()
            }

            // ── Splice/tee ──
            super::probe_router::op::SPLICE => {
                let fd_in = types::Fd(a[0] as i32);
                let off_in = a[1] as i64;
                let fd_out = types::Fd(a[2] as i32);
                let off_out = a[3] as i64;
                let len = a[4] as u32;
                opcode::Splice::new(fd_in, off_in, fd_out, off_out, len)
                    .flags(a[5] as u32)
                    .build()
            }

            // ── Catch-all for opcodes we haven't mapped yet ──
            // This should not happen if the router is correct, but
            // return ENOSYS rather than panic.
            _ => {
                return Err(KsvcError::Unsupported(entry.syscall_nr));
            }
        };

        // Stamp the user_data for correlation
        let sqe = sqe.user_data(user_data);
        Ok(sqe)
    }
}

impl IoBackend for BasicIoUring {
    fn submit(&mut self, entry: &SubmitEntry) -> Result<()> {
        // The opcode must be looked up from the router, but it's passed
        // via the entry's route. We'll need the opcode from the caller.
        // For now, we store it... actually the dispatcher knows the opcode
        // from the route_table lookup. We need it passed in.
        //
        // Design decision: the IoBackend::submit takes a SubmitEntry
        // which has syscall_nr but not the opcode. The dispatcher
        // translates syscall_nr → opcode via the router, and we need
        // that opcode here.
        //
        // Solution: store opcode in a reserved field of SubmitEntry,
        // or have a separate method. For now, we'll look it up from
        // a helper that the Instance provides.
        //
        // TODO: clean this up in the trait design
        Err(KsvcError::Unsupported(entry.syscall_nr))
    }

    fn flush(&mut self) -> Result<usize> {
        if self.pending_submit == 0 {
            return Ok(0);
        }
        let submitted = self.ring.submit()
            .map_err(|e| KsvcError::IoUringSubmit(e.raw_os_error().unwrap_or(-1)))?;
        self.inflight += submitted;
        self.pending_submit = 0;
        Ok(submitted)
    }

    fn poll_completions(&mut self, buf: &mut [IoCompletion], max: usize) -> usize {
        let cq = self.ring.completion();
        let mut count = 0;
        for cqe in cq {
            if count >= max || count >= buf.len() {
                break;
            }
            buf[count] = IoCompletion {
                corr_id: CorrId(cqe.user_data()),
                result: cqe.result() as i64,
                flags: cqe.flags(),
            };
            count += 1;
            self.inflight = self.inflight.saturating_sub(1);
        }
        count
    }

    fn cancel(&mut self, corr_id: CorrId) -> Result<()> {
        use io_uring::opcode;
        let sqe = opcode::AsyncCancel::new(corr_id.0)
            .build()
            .user_data(u64::MAX - 1); // sentinel for cancel completions
        unsafe {
            self.ring.submission()
                .push(&sqe)
                .map_err(|_| KsvcError::RingFull)?;
        }
        self.pending_submit += 1;
        Ok(())
    }

    fn inflight(&self) -> usize {
        self.inflight
    }

    fn capacity(&self) -> usize {
        self.ring.params().sq_entries() as usize
    }

    fn probe_opcodes(&self) -> Vec<u8> {
        self.probe_opcodes_static()
    }

    fn shutdown(&mut self) {
        // Drain remaining CQEs so io_uring can release resources cleanly.
        // The io-uring crate's Drop impl closes the fd and unmaps rings,
        // but we want to drain completions first to avoid leaking inflight ops.
        let mut buf = [IoCompletion { corr_id: CorrId(0), result: 0, flags: 0 }; 64];
        loop {
            let n = self.poll_completions(&mut buf, 64);
            if n == 0 {
                break;
            }
        }
        self.inflight = 0;
        self.pending_submit = 0;
        // io_uring::IoUring::drop() handles fd close + munmap
    }
}

// Note: BasicIoUring does NOT need a manual Drop impl.
// The inner `io_uring::IoUring` already implements Drop
// (closes fd, unmaps SQ/CQ rings). We call shutdown()
// explicitly from KsvcInstance::drop() for orderly drain.

/// Extended submit method that takes the pre-resolved opcode.
/// This is what the dispatcher actually calls.
impl BasicIoUring {
    pub fn submit_with_opcode(&mut self, entry: &SubmitEntry, opcode: u8) -> Result<()> {
        self.translate_and_push(entry, opcode)
    }
}
