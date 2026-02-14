//! KSVC Echo Server
//!
//! Single-threaded TCP echo server driven entirely by io_uring
//! through KSVC's Tier 1 translation layer.
//!
//! Proves: KSVC's SQE translation handles real concurrent I/O under load.
//!
//! Usage:
//!     cargo build --release -p ksvc-echo
//!     ./target/release/ksvc-echo [port] [max_conns]
//!
//! Test with:
//!     # Correctness:
//!     echo "hello" | nc localhost 9999
//!
//!     # Load:
//!     wrk -t4 -c100 -d10s tcp://localhost:9999  (won't work, wrk is HTTP)
//!     # Better: use a TCP flood tool or multiple nc instances
//!
//!     # Quick benchmark (from another terminal):
//!     for i in $(seq 1 100); do echo "ping $i" | nc -q0 localhost 9999 & done

use ksvc_core::entry::{CorrId, SubmitEntry};
use ksvc_core::io_backend::{IoBackend, IoCompletion};
use ksvc_core::router::SyscallRouter;

use ksvc_module::basic_iouring::{BasicIoUring, BasicIoUringConfig};
use ksvc_module::probe_router::ProbeRouter;

use std::sync::atomic::{AtomicBool, Ordering};

// ── Syscall numbers (x86_64) ──
const NR_CLOSE: u32 = 3;
const NR_ACCEPT4: u32 = 288;
const NR_SENDTO: u32 = 44;
const NR_RECVFROM: u32 = 45;

// ── Op types encoded in corr_id high bits ──
const OP_ACCEPT: u64 = 1 << 56;
const OP_RECV: u64 = 2 << 56;
const OP_SEND: u64 = 3 << 56;
const OP_CLOSE: u64 = 4 << 56;
const OP_MASK: u64 = 0xFF << 56;
const IDX_MASK: u64 = (1 << 56) - 1;

fn make_id(op: u64, idx: usize) -> CorrId {
    CorrId(op | idx as u64)
}
fn decode_op(id: CorrId) -> u64 {
    id.0 & OP_MASK
}
fn decode_idx(id: CorrId) -> usize {
    (id.0 & IDX_MASK) as usize
}

// ── Per-connection state ──
const BUF_SIZE: usize = 4096;

struct Conn {
    fd: i32,
    buf: Box<[u8; BUF_SIZE]>,
    /// How many bytes are pending to send (set after recv)
    pending: usize,
}

struct ConnSlab {
    slots: Vec<Option<Conn>>,
    free: Vec<usize>,
}

impl ConnSlab {
    fn new(max: usize) -> Self {
        let mut free = Vec::with_capacity(max);
        for i in (0..max).rev() {
            free.push(i);
        }
        Self {
            slots: (0..max).map(|_| None).collect(),
            free,
        }
    }

    fn alloc(&mut self, fd: i32) -> Option<usize> {
        let idx = self.free.pop()?;
        self.slots[idx] = Some(Conn {
            fd,
            buf: Box::new([0u8; BUF_SIZE]),
            pending: 0,
        });
        Some(idx)
    }

    fn get(&self, idx: usize) -> Option<&Conn> {
        self.slots.get(idx)?.as_ref()
    }

    fn get_mut(&mut self, idx: usize) -> Option<&mut Conn> {
        self.slots.get_mut(idx)?.as_mut()
    }

    fn free(&mut self, idx: usize) {
        self.slots[idx] = None;
        self.free.push(idx);
    }

    fn active(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

// ── Stats ──
struct Stats {
    accepts: u64,
    recvs: u64,
    sends: u64,
    closes: u64,
    bytes_in: u64,
    bytes_out: u64,
    errors: u64,
}

impl Stats {
    fn new() -> Self {
        Self { accepts: 0, recvs: 0, sends: 0, closes: 0, bytes_in: 0, bytes_out: 0, errors: 0 }
    }

    fn print(&self, conns: &ConnSlab, elapsed_secs: f64) {
        eprintln!(
            "[{:.1}s] conns={} accepts={} recv={} send={} close={} bytes_in={} bytes_out={} err={}",
            elapsed_secs,
            conns.active(),
            self.accepts, self.recvs, self.sends, self.closes,
            self.bytes_in, self.bytes_out, self.errors,
        );
    }
}

// ── Setup listener (normal syscalls — startup only) ──

fn setup_listener(port: u16) -> i32 {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        assert!(fd >= 0, "socket() failed");

        let opt: i32 = 1;
        libc::setsockopt(
            fd, libc::SOL_SOCKET, libc::SO_REUSEADDR,
            &opt as *const _ as *const _, std::mem::size_of::<i32>() as u32,
        );
        libc::setsockopt(
            fd, libc::SOL_SOCKET, libc::SO_REUSEPORT,
            &opt as *const _ as *const _, std::mem::size_of::<i32>() as u32,
        );

        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_family = libc::AF_INET as u16;
        addr.sin_addr.s_addr = 0; // INADDR_ANY
        addr.sin_port = port.to_be();

        let ret = libc::bind(fd, &addr as *const _ as *const _, std::mem::size_of_val(&addr) as u32);
        assert!(ret == 0, "bind() failed: {}", std::io::Error::last_os_error());

        libc::listen(fd, 4096);
        fd
    }
}

// ── Submit helpers ──

fn submit_accept(
    io: &mut BasicIoUring,
    router: &ProbeRouter,
    listener: i32,
    addr: &mut libc::sockaddr_in,
    addr_len: &mut libc::socklen_t,
) {
    let route = router.route(NR_ACCEPT4);
    let entry = SubmitEntry {
        corr_id: make_id(OP_ACCEPT, 0),
        syscall_nr: NR_ACCEPT4,
        flags: 0,
        args: [
            listener as u64,
            addr as *mut _ as u64,
            addr_len as *mut _ as u64,
            libc::SOCK_CLOEXEC as u64,
            0, 0,
        ],
    };
    let _ = io.submit_with_opcode(&entry, route.iouring_opcode);
}

fn submit_recv(io: &mut BasicIoUring, router: &ProbeRouter, conn: &mut Conn, idx: usize) {
    let route = router.route(NR_RECVFROM);
    let entry = SubmitEntry {
        corr_id: make_id(OP_RECV, idx),
        syscall_nr: NR_RECVFROM,
        flags: 0,
        args: [
            conn.fd as u64,
            conn.buf.as_mut_ptr() as u64,
            BUF_SIZE as u64,
            0, 0, 0,
        ],
    };
    let _ = io.submit_with_opcode(&entry, route.iouring_opcode);
}

fn submit_send(io: &mut BasicIoUring, router: &ProbeRouter, conn: &Conn, idx: usize) {
    let route = router.route(NR_SENDTO);
    let entry = SubmitEntry {
        corr_id: make_id(OP_SEND, idx),
        syscall_nr: NR_SENDTO,
        flags: 0,
        args: [
            conn.fd as u64,
            conn.buf.as_ptr() as u64,
            conn.pending as u64,
            0, 0, 0,
        ],
    };
    let _ = io.submit_with_opcode(&entry, route.iouring_opcode);
}

fn submit_close(io: &mut BasicIoUring, router: &ProbeRouter, fd: i32, idx: usize) {
    let route = router.route(NR_CLOSE);
    let entry = SubmitEntry {
        corr_id: make_id(OP_CLOSE, idx),
        syscall_nr: NR_CLOSE,
        flags: 0,
        args: [fd as u64, 0, 0, 0, 0, 0],
    };
    let _ = io.submit_with_opcode(&entry, route.iouring_opcode);
}

// ── Main event loop ──

static RUNNING: AtomicBool = AtomicBool::new(true);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(9999);
    let max_conns: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1024);

    // SIGINT handler for clean shutdown
    unsafe {
        libc::signal(libc::SIGINT, handle_sigint as usize);
        libc::signal(libc::SIGTERM, handle_sigint as usize);
    }

    eprintln!("ksvc-echo: starting on port {} (max {} connections)", port, max_conns);

    // Setup
    let listener = setup_listener(port);
    let mut io = BasicIoUring::new(BasicIoUringConfig {
        sq_entries: 256,
        ..Default::default()
    }).expect("io_uring setup failed");

    let supported = io.probe_opcodes_static();
    let router = ProbeRouter::new(&supported);
    let counts = router.tier_counts();
    eprintln!("ksvc-echo: routing T0={} T1={} T2={} T3={}", counts.tier0, counts.tier1, counts.tier2, counts.tier3);

    let mut conns = ConnSlab::new(max_conns);
    let mut stats = Stats::new();

    // Accept address storage (reused, kernel fills it each time)
    let mut accept_addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut accept_addr_len: libc::socklen_t = std::mem::size_of::<libc::sockaddr_in>() as u32;

    // Seed: submit first accept
    submit_accept(&mut io, &router, listener, &mut accept_addr, &mut accept_addr_len);
    let _ = io.flush();

    let start = std::time::Instant::now();
    let mut last_stats = start;
    let mut comp_buf = [IoCompletion { corr_id: CorrId(0), result: 0, flags: 0 }; 64];

    eprintln!("ksvc-echo: listening on 0.0.0.0:{} (io_uring, blocking wait)", port);

    // ── Event loop ──
    while RUNNING.load(Ordering::Relaxed) {
        // Submit pending SQEs AND block until at least 1 CQE is ready.
        // This is ONE io_uring_enter() call that does both submit + wait.
        // The kernel wakes us the instant a completion arrives — zero idle waste.
        let _ = io.flush_and_wait(1);

        // Drain all available completions
        let n = io.poll_completions(&mut comp_buf, 64);

        if n == 0 {
            // Can happen if interrupted by signal
            continue;
        }

        for i in 0..n {
            let cqe = &comp_buf[i];
            let op = decode_op(cqe.corr_id);
            let idx = decode_idx(cqe.corr_id);
            let result = cqe.result;

            match op {
                OP_ACCEPT => {
                    if result < 0 {
                        stats.errors += 1;
                        // Re-submit accept unless shutting down
                        if RUNNING.load(Ordering::Relaxed) {
                            submit_accept(&mut io, &router, listener, &mut accept_addr, &mut accept_addr_len);
                        }
                        continue;
                    }

                    let new_fd = result as i32;
                    stats.accepts += 1;

                    // Allocate connection slot
                    match conns.alloc(new_fd) {
                        Some(cidx) => {
                            // Submit recv on new connection
                            if let Some(conn) = conns.get_mut(cidx) {
                                submit_recv(&mut io, &router, conn, cidx);
                            }
                        }
                        None => {
                            // At capacity — close immediately
                            unsafe { libc::close(new_fd); }
                            stats.errors += 1;
                        }
                    }

                    // Always re-submit accept for next connection
                    accept_addr_len = std::mem::size_of::<libc::sockaddr_in>() as u32;
                    submit_accept(&mut io, &router, listener, &mut accept_addr, &mut accept_addr_len);
                }

                OP_RECV => {
                    if result <= 0 {
                        // Connection closed or error → close fd
                        if let Some(conn) = conns.get(idx) {
                            submit_close(&mut io, &router, conn.fd, idx);
                        }
                        stats.closes += 1;
                        continue;
                    }

                    stats.recvs += 1;
                    stats.bytes_in += result as u64;

                    // Echo: send back what we received
                    if let Some(conn) = conns.get_mut(idx) {
                        conn.pending = result as usize;
                        submit_send(&mut io, &router, conn, idx);
                    }
                }

                OP_SEND => {
                    if result <= 0 {
                        // Send failed → close
                        if let Some(conn) = conns.get(idx) {
                            submit_close(&mut io, &router, conn.fd, idx);
                        }
                        stats.errors += 1;
                        continue;
                    }

                    stats.sends += 1;
                    stats.bytes_out += result as u64;

                    // Send done → ready for next recv
                    if let Some(conn) = conns.get_mut(idx) {
                        submit_recv(&mut io, &router, conn, idx);
                    }
                }

                OP_CLOSE => {
                    // Connection fully closed — free the slot
                    conns.free(idx);
                }

                _ => {
                    eprintln!("ksvc-echo: unknown op 0x{:x}", op);
                }
            }
        }

        // Print stats every 5 seconds
        let now = std::time::Instant::now();
        if now.duration_since(last_stats).as_secs() >= 5 {
            stats.print(&conns, now.duration_since(start).as_secs_f64());
            last_stats = now;
        }
    }

    // Shutdown
    eprintln!("\nksvc-echo: shutting down...");
    stats.print(&conns, start.elapsed().as_secs_f64());
    unsafe { libc::close(listener); }
    eprintln!("ksvc-echo: done.");
}

extern "C" fn handle_sigint(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::Relaxed);
}
