//! KSVC HTTP/1.1 Server
//!
//! Single-threaded HTTP server driven entirely by io_uring via KSVC Tier 1.
//! Supports HTTP/1.1 keep-alive for maximum wrk throughput.
//!
//! Modes:
//!   Default: returns "Hello from KSVC!" for every request (throughput bench)
//!   --dir <path>: serves static files from directory (full opcode exercise)
//!
//! Usage:
//!     ./target/release/ksvc-httpd [--port 8080] [--dir ./www]
//!
//! Benchmark:
//!     wrk -t4 -c100 -d10s http://127.0.0.1:8080/
//!     ab -n 100000 -c 100 -k http://127.0.0.1:8080/

use ksvc_core::entry::{CorrId, SubmitEntry};
use ksvc_core::io_backend::{IoBackend, IoCompletion};
use ksvc_core::router::SyscallRouter;

use ksvc_module::basic_iouring::{BasicIoUring, BasicIoUringConfig};
use ksvc_module::probe_router::ProbeRouter;

use std::env;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};

// ── Syscall numbers (x86_64) ──
const NR_READ: u32 = 0;
const NR_CLOSE: u32 = 3;
const NR_OPENAT: u32 = 257;
const NR_ACCEPT4: u32 = 288;
const NR_SENDTO: u32 = 44;
const NR_RECVFROM: u32 = 45;

// ── Op + state encoded in corr_id ──
// Layout: [op:8][state:8][file_idx:16][conn_idx:32]
const OP_ACCEPT: u64 = 1 << 56;
const OP_RECV: u64 = 2 << 56;
const OP_SEND: u64 = 3 << 56;
const OP_CLOSE: u64 = 4 << 56;
const OP_FILE_OPEN: u64 = 5 << 56;
const OP_FILE_READ: u64 = 6 << 56;
const OP_FILE_CLOSE: u64 = 7 << 56;
const OP_MASK: u64 = 0xFF << 56;
const IDX_MASK: u64 = 0xFFFFFFFF;

fn make_id(op: u64, idx: usize) -> CorrId { CorrId(op | idx as u64) }
fn decode_op(id: CorrId) -> u64 { id.0 & OP_MASK }
fn decode_idx(id: CorrId) -> usize { (id.0 & IDX_MASK) as usize }

// ── Buffers ──
const RECV_BUF: usize = 4096;
const SEND_BUF: usize = 8192;
const FILE_BUF: usize = 65536;

// ── Static response (hello world mode) ──
const HELLO_BODY: &[u8] = b"Hello from KSVC!\n";

fn make_hello_response() -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: keep-alive\r\n\
         Server: ksvc-httpd\r\n\
         \r\n",
        HELLO_BODY.len()
    ).into_bytes()
    .into_iter()
    .chain(HELLO_BODY.iter().copied())
    .collect()
}

fn make_404_response() -> Vec<u8> {
    let body = b"404 Not Found\n";
    format!(
        "HTTP/1.1 404 Not Found\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: keep-alive\r\n\
         \r\n",
        body.len()
    ).into_bytes()
    .into_iter()
    .chain(body.iter().copied())
    .collect()
}

fn make_file_response(content_type: &str, body: &[u8]) -> Vec<u8> {
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Connection: keep-alive\r\n\
         Server: ksvc-httpd\r\n\
         \r\n",
        content_type,
        body.len()
    );
    let mut resp = header.into_bytes();
    resp.extend_from_slice(body);
    resp
}

fn guess_content_type(path: &str) -> &'static str {
    if path.ends_with(".html") || path.ends_with(".htm") { "text/html" }
    else if path.ends_with(".css") { "text/css" }
    else if path.ends_with(".js") { "application/javascript" }
    else if path.ends_with(".json") { "application/json" }
    else if path.ends_with(".txt") { "text/plain" }
    else if path.ends_with(".png") { "image/png" }
    else if path.ends_with(".jpg") || path.ends_with(".jpeg") { "image/jpeg" }
    else if path.ends_with(".gif") { "image/gif" }
    else if path.ends_with(".svg") { "image/svg+xml" }
    else { "application/octet-stream" }
}

// ── Connection state ──

#[derive(Clone, Copy, PartialEq)]
enum ConnState {
    Receiving,      // waiting for HTTP request
    SendingReply,   // sending response (hello mode)
    FileOpening,    // opening file (file mode)
    FileReading,    // reading file content
    FileSending,    // sending file response
    FileClosing,    // closing file fd
    Closing,        // connection closing
}

struct Conn {
    fd: i32,
    state: ConnState,
    recv_buf: Box<[u8; RECV_BUF]>,
    recv_len: usize,
    send_buf: Vec<u8>,
    send_off: usize,
    // File serving state
    file_fd: i32,
    file_buf: Box<[u8; FILE_BUF]>,
    file_path: CString,
}

impl Conn {
    fn reset_for_next_request(&mut self) {
        self.recv_len = 0;
        self.send_buf.clear();
        self.send_off = 0;
        self.state = ConnState::Receiving;
        self.file_fd = -1;
    }
}

struct ConnSlab {
    slots: Vec<Option<Conn>>,
    free: Vec<usize>,
}

impl ConnSlab {
    fn new(max: usize) -> Self {
        let mut free = Vec::with_capacity(max);
        for i in (0..max).rev() { free.push(i); }
        Self {
            slots: (0..max).map(|_| None).collect(),
            free,
        }
    }

    fn alloc(&mut self, fd: i32) -> Option<usize> {
        let idx = self.free.pop()?;
        self.slots[idx] = Some(Conn {
            fd,
            state: ConnState::Receiving,
            recv_buf: Box::new([0u8; RECV_BUF]),
            recv_len: 0,
            send_buf: Vec::with_capacity(SEND_BUF),
            send_off: 0,
            file_fd: -1,
            file_buf: Box::new([0u8; FILE_BUF]),
            file_path: CString::default(),
        });
        Some(idx)
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

// ── HTTP parsing (minimal, fast) ──

/// Find \r\n\r\n in buffer, return index past it + extracted path.
/// Only parses GET requests. Returns None if request is incomplete.
fn parse_request(buf: &[u8], len: usize) -> Option<(usize, &[u8])> {
    // Find end of headers
    let data = &buf[..len];
    let end = data.windows(4)
        .position(|w| w == b"\r\n\r\n")?;
    let header_end = end + 4;

    // Extract path from "GET /path HTTP/1.1\r\n"
    if data.len() < 14 || &data[..4] != b"GET " {
        return Some((header_end, b"/"));
    }

    let path_start = 4;
    let path_end = data[path_start..].iter()
        .position(|&b| b == b' ')
        .map(|p| path_start + p)
        .unwrap_or(path_start + 1);

    Some((header_end, &data[path_start..path_end]))
}

// ── Stats ──
struct Stats {
    accepts: u64,
    requests: u64,
    responses: u64,
    bytes_in: u64,
    bytes_out: u64,
    errors: u64,
    file_opens: u64,
}

impl Stats {
    fn new() -> Self {
        Self { accepts: 0, requests: 0, responses: 0, bytes_in: 0, bytes_out: 0, errors: 0, file_opens: 0 }
    }

    fn print(&self, conns: &ConnSlab, elapsed: f64) {
        let rps = if elapsed > 0.0 { self.responses as f64 / elapsed } else { 0.0 };
        eprintln!(
            "[{:.1}s] conns={} req={} resp={} rps={:.0} bytes_in={} bytes_out={} err={} files={}",
            elapsed, conns.active(), self.requests, self.responses, rps,
            self.bytes_in, self.bytes_out, self.errors, self.file_opens,
        );
    }
}

// ── Setup ──

fn setup_listener(port: u16) -> i32 {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        assert!(fd >= 0, "socket() failed");

        let opt: i32 = 1;
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR,
            &opt as *const _ as *const _, 4);
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT,
            &opt as *const _ as *const _, 4);

        // Disable Nagle for lower latency
        libc::setsockopt(fd, libc::IPPROTO_TCP, libc::TCP_NODELAY,
            &opt as *const _ as *const _, 4);

        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_family = libc::AF_INET as u16;
        addr.sin_addr.s_addr = 0;
        addr.sin_port = port.to_be();

        let ret = libc::bind(fd, &addr as *const _ as *const _, std::mem::size_of_val(&addr) as u32);
        assert!(ret == 0, "bind() failed: {}", std::io::Error::last_os_error());

        libc::listen(fd, 4096);
        fd
    }
}

// ── Submit helpers ──

fn submit_accept(io: &mut BasicIoUring, r: &ProbeRouter, listener: i32,
    addr: &mut libc::sockaddr_in, addr_len: &mut libc::socklen_t) {
    let route = r.route(NR_ACCEPT4);
    let _ = io.submit_with_opcode(&SubmitEntry {
        corr_id: make_id(OP_ACCEPT, 0), syscall_nr: NR_ACCEPT4, flags: 0,
        args: [listener as u64, addr as *mut _ as u64, addr_len as *mut _ as u64,
               libc::SOCK_CLOEXEC as u64, 0, 0],
    }, route.iouring_opcode);
}

fn submit_recv(io: &mut BasicIoUring, r: &ProbeRouter, conn: &mut Conn, idx: usize) {
    let route = r.route(NR_RECVFROM);
    let offset = conn.recv_len;
    let _ = io.submit_with_opcode(&SubmitEntry {
        corr_id: make_id(OP_RECV, idx), syscall_nr: NR_RECVFROM, flags: 0,
        args: [conn.fd as u64,
               conn.recv_buf[offset..].as_mut_ptr() as u64,
               (RECV_BUF - offset) as u64,
               0, 0, 0],
    }, route.iouring_opcode);
}

fn submit_send(io: &mut BasicIoUring, r: &ProbeRouter, conn: &Conn, idx: usize) {
    let route = r.route(NR_SENDTO);
    let remaining = conn.send_buf.len() - conn.send_off;
    let _ = io.submit_with_opcode(&SubmitEntry {
        corr_id: make_id(OP_SEND, idx), syscall_nr: NR_SENDTO, flags: 0,
        args: [conn.fd as u64,
               conn.send_buf[conn.send_off..].as_ptr() as u64,
               remaining as u64,
               0, 0, 0],
    }, route.iouring_opcode);
}

fn submit_close(io: &mut BasicIoUring, r: &ProbeRouter, fd: i32, idx: usize) {
    let route = r.route(NR_CLOSE);
    let _ = io.submit_with_opcode(&SubmitEntry {
        corr_id: make_id(OP_CLOSE, idx), syscall_nr: NR_CLOSE, flags: 0,
        args: [fd as u64, 0, 0, 0, 0, 0],
    }, route.iouring_opcode);
}

fn submit_file_open(io: &mut BasicIoUring, r: &ProbeRouter, conn: &Conn, idx: usize) {
    let route = r.route(NR_OPENAT);
    let _ = io.submit_with_opcode(&SubmitEntry {
        corr_id: make_id(OP_FILE_OPEN, idx), syscall_nr: NR_OPENAT, flags: 0,
        args: [libc::AT_FDCWD as u64,
               conn.file_path.as_ptr() as u64,
               libc::O_RDONLY as u64,
               0, 0, 0],
    }, route.iouring_opcode);
}

fn submit_file_read(io: &mut BasicIoUring, r: &ProbeRouter, conn: &mut Conn, idx: usize) {
    let route = r.route(NR_READ);
    let _ = io.submit_with_opcode(&SubmitEntry {
        corr_id: make_id(OP_FILE_READ, idx), syscall_nr: NR_READ, flags: 0,
        args: [conn.file_fd as u64,
               conn.file_buf.as_mut_ptr() as u64,
               FILE_BUF as u64,
               0, 0, 0],
    }, route.iouring_opcode);
}

fn submit_file_close(io: &mut BasicIoUring, r: &ProbeRouter, fd: i32, idx: usize) {
    let route = r.route(NR_CLOSE);
    let _ = io.submit_with_opcode(&SubmitEntry {
        corr_id: make_id(OP_FILE_CLOSE, idx), syscall_nr: NR_CLOSE, flags: 0,
        args: [fd as u64, 0, 0, 0, 0, 0],
    }, route.iouring_opcode);
}

// ── Main ──

static RUNNING: AtomicBool = AtomicBool::new(true);

/// Per-thread worker: own listener (SO_REUSEPORT), own io_uring ring, own ConnSlab.
/// Completely independent — zero cross-thread synchronization.
fn worker_loop(wid: usize, num_workers: usize, port: u16, max_conns: usize,
               file_mode: bool, base_dir: &str)
{
    let hello_response = make_hello_response();
    let not_found_response = make_404_response();
    let base_dir = base_dir.to_string();

    let listener = setup_listener(port);
    let mut io = BasicIoUring::new(BasicIoUringConfig {
        sq_entries: 512,
        ..Default::default()
    }).expect("io_uring setup failed");

    let supported = io.probe_opcodes_static();
    let router = ProbeRouter::new(&supported);

    if wid == 0 {
        let counts = router.tier_counts();
        eprintln!("ksvc-httpd: routing T0={} T1={} T2={} T3={}",
            counts.tier0, counts.tier1, counts.tier2, counts.tier3);
    }

    let mut conns = ConnSlab::new(max_conns);
    let mut stats = Stats::new();

    let mut accept_addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut accept_addr_len: libc::socklen_t = std::mem::size_of::<libc::sockaddr_in>() as u32;

    submit_accept(&mut io, &router, listener, &mut accept_addr, &mut accept_addr_len);
    let _ = io.flush();

    let start = std::time::Instant::now();
    let mut last_stats = start;
    let mut comp_buf = [IoCompletion { corr_id: CorrId(0), result: 0, flags: 0 }; 128];

    let w_tag = if num_workers > 1 { format!("[w{}] ", wid) } else { String::new() };

    eprintln!("{}ksvc-httpd: listening on http://0.0.0.0:{}/ (ring {}/{})",
        w_tag, port, wid + 1, num_workers);

    // ── Event loop ──
    while RUNNING.load(Ordering::Relaxed) {
        let _ = io.flush_and_wait(1);
        let n = io.poll_completions(&mut comp_buf, 128);
        if n == 0 { continue; }

        for ci in 0..n {
            let cqe = comp_buf[ci];
            let op = decode_op(cqe.corr_id);
            let idx = decode_idx(cqe.corr_id);
            let result = cqe.result;

            match op {
                // ─── ACCEPT ───
                OP_ACCEPT => {
                    if result >= 0 {
                        let new_fd = result as i32;
                        stats.accepts += 1;

                        // TCP_NODELAY on accepted socket
                        unsafe {
                            let opt: i32 = 1;
                            libc::setsockopt(new_fd, libc::IPPROTO_TCP, libc::TCP_NODELAY,
                                &opt as *const _ as *const _, 4);
                        }

                        match conns.alloc(new_fd) {
                            Some(cidx) => {
                                if let Some(conn) = conns.get_mut(cidx) {
                                    submit_recv(&mut io, &router, conn, cidx);
                                }
                            }
                            None => {
                                unsafe { libc::close(new_fd); }
                                stats.errors += 1;
                            }
                        }
                    } else {
                        stats.errors += 1;
                    }
                    // Re-submit accept
                    accept_addr_len = std::mem::size_of::<libc::sockaddr_in>() as u32;
                    submit_accept(&mut io, &router, listener, &mut accept_addr, &mut accept_addr_len);
                }

                // ─── RECV ───
                OP_RECV => {
                    if result <= 0 {
                        // EOF or error → close
                        if let Some(conn) = conns.get_mut(idx) {
                            conn.state = ConnState::Closing;
                            submit_close(&mut io, &router, conn.fd, idx);
                        }
                        continue;
                    }

                    let nbytes = result as usize;
                    stats.bytes_in += nbytes as u64;

                    let conn = match conns.get_mut(idx) {
                        Some(c) => c,
                        None => continue,
                    };
                    conn.recv_len += nbytes;

                    // Try to parse HTTP request
                    match parse_request(conn.recv_buf.as_mut_slice(), conn.recv_len) {
                        None => {
                            // Incomplete — recv more
                            if conn.recv_len >= RECV_BUF {
                                // Buffer full, no complete request → drop
                                conn.state = ConnState::Closing;
                                submit_close(&mut io, &router, conn.fd, idx);
                            } else {
                                submit_recv(&mut io, &router, conn, idx);
                            }
                        }
                        Some((_consumed, path)) => {
                            stats.requests += 1;

                            if !file_mode {
                                // Hello world mode — immediate response
                                conn.send_buf.clear();
                                conn.send_buf.extend_from_slice(&hello_response);
                                conn.send_off = 0;
                                conn.state = ConnState::SendingReply;
                                submit_send(&mut io, &router, conn, idx);
                            } else {
                                // File mode — build path, open file
                                let path_str = std::str::from_utf8(path).unwrap_or("/");
                                let path_str = if path_str == "/" { "/index.html" } else { path_str };

                                // Sanitize: no ".." traversal
                                if path_str.contains("..") {
                                    conn.send_buf.clear();
                                    conn.send_buf.extend_from_slice(&not_found_response);
                                    conn.send_off = 0;
                                    conn.state = ConnState::SendingReply;
                                    submit_send(&mut io, &router, conn, idx);
                                    continue;
                                }

                                let full_path = format!("{}{}", base_dir, path_str);
                                match CString::new(full_path) {
                                    Ok(cpath) => {
                                        conn.file_path = cpath;
                                        conn.state = ConnState::FileOpening;
                                        submit_file_open(&mut io, &router, conn, idx);
                                        stats.file_opens += 1;
                                    }
                                    Err(_) => {
                                        conn.send_buf.clear();
                                        conn.send_buf.extend_from_slice(&not_found_response);
                                        conn.send_off = 0;
                                        conn.state = ConnState::SendingReply;
                                        submit_send(&mut io, &router, conn, idx);
                                    }
                                }
                            }
                        }
                    }
                }

                // ─── SEND ───
                OP_SEND => {
                    if result <= 0 {
                        if let Some(conn) = conns.get_mut(idx) {
                            conn.state = ConnState::Closing;
                            submit_close(&mut io, &router, conn.fd, idx);
                        }
                        stats.errors += 1;
                        continue;
                    }

                    let nbytes = result as usize;
                    stats.bytes_out += nbytes as u64;

                    let conn = match conns.get_mut(idx) {
                        Some(c) => c,
                        None => continue,
                    };

                    conn.send_off += nbytes;

                    if conn.send_off < conn.send_buf.len() {
                        // Partial send — continue
                        submit_send(&mut io, &router, conn, idx);
                    } else {
                        // Send complete
                        stats.responses += 1;

                        match conn.state {
                            ConnState::FileSending => {
                                // Close the file fd, then back to recv
                                let ffd = conn.file_fd;
                                conn.file_fd = -1;
                                conn.state = ConnState::FileClosing;
                                submit_file_close(&mut io, &router, ffd, idx);
                            }
                            _ => {
                                // Keep-alive: ready for next request
                                conn.reset_for_next_request();
                                submit_recv(&mut io, &router, conn, idx);
                            }
                        }
                    }
                }

                // ─── FILE OPEN ───
                OP_FILE_OPEN => {
                    let conn = match conns.get_mut(idx) {
                        Some(c) => c,
                        None => continue,
                    };

                    if result < 0 {
                        // File not found → 404
                        conn.send_buf.clear();
                        conn.send_buf.extend_from_slice(&not_found_response);
                        conn.send_off = 0;
                        conn.state = ConnState::SendingReply;
                        submit_send(&mut io, &router, conn, idx);
                    } else {
                        conn.file_fd = result as i32;
                        conn.state = ConnState::FileReading;
                        submit_file_read(&mut io, &router, conn, idx);
                    }
                }

                // ─── FILE READ ───
                OP_FILE_READ => {
                    let conn = match conns.get_mut(idx) {
                        Some(c) => c,
                        None => continue,
                    };

                    if result <= 0 {
                        // Empty file or error → send empty 200
                        let resp = make_file_response("text/plain", b"");
                        conn.send_buf.clear();
                        conn.send_buf.extend_from_slice(&resp);
                        conn.send_off = 0;
                        conn.state = ConnState::FileSending;
                        submit_send(&mut io, &router, conn, idx);
                    } else {
                        let nbytes = result as usize;
                        let path_str = conn.file_path.to_str().unwrap_or("");
                        let ctype = guess_content_type(path_str);
                        let resp = make_file_response(ctype, &conn.file_buf[..nbytes]);
                        conn.send_buf.clear();
                        conn.send_buf.extend_from_slice(&resp);
                        conn.send_off = 0;
                        conn.state = ConnState::FileSending;
                        submit_send(&mut io, &router, conn, idx);
                    }
                }

                // ─── FILE CLOSE ───
                OP_FILE_CLOSE => {
                    if let Some(conn) = conns.get_mut(idx) {
                        // File closed → keep-alive, back to recv
                        conn.reset_for_next_request();
                        submit_recv(&mut io, &router, conn, idx);
                    }
                }

                // ─── CONN CLOSE ───
                OP_CLOSE => {
                    conns.free(idx);
                }

                _ => {}
            }
        }

        // Stats every 5s
        let now = std::time::Instant::now();
        if now.duration_since(last_stats).as_secs() >= 5 {
            eprint!("{}", w_tag);
            stats.print(&conns, now.duration_since(start).as_secs_f64());
            last_stats = now;
        }
    }

    eprint!("{}", w_tag);
    stats.print(&conns, start.elapsed().as_secs_f64());
    unsafe { libc::close(listener); }
}


fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut port: u16 = env::var("gvt_app_port").ok()
         .and_then(|v| v.parse().ok())
      .unwrap_or(8080);;
    let mut serve_dir: Option<String> = None;
    let mut max_conns: usize = 4096;
    let mut num_threads: usize = 1;

    // Parse --threads from CLI
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => { i += 1; port = args[i].parse().unwrap_or(8080); }
            "--dir" | "-d" => { i += 1; serve_dir = Some(args[i].clone()); }
            "--max-conns" => { i += 1; max_conns = args[i].parse().unwrap_or(4096); }
            "--threads" | "-t" => { i += 1; num_threads = args[i].parse().unwrap_or(1); }
            s if s.parse::<u16>().is_ok() => { port = s.parse().unwrap(); }
            _ => {}
        }
        i += 1;
    }

    // KSVC_THREADS env overrides CLI (allows: KSVC_THREADS=4 make itest-httpd-ksvc)
    if let Ok(t) = std::env::var("KSVC_THREADS") {
        if let Ok(n) = t.parse::<usize>() {
            if n >= 1 { num_threads = n; }
        }
    }

    num_threads = num_threads.max(1);

    unsafe {
        libc::signal(libc::SIGINT, handle_sigint as usize);
        libc::signal(libc::SIGTERM, handle_sigint as usize);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let file_mode = serve_dir.is_some();
    let base_dir = serve_dir.unwrap_or_default();

    // Divide connection slots among workers
    let conns_per_worker = max_conns / num_threads;

    eprintln!("ksvc-httpd: port={} threads={} max_conns={}({}/worker) mode={}",
        port, num_threads, max_conns, conns_per_worker,
        if file_mode { format!("file({})", base_dir) } else { "hello".into() });

    if num_threads == 1 {
        // Single-thread: run directly on main thread (zero overhead)
        worker_loop(0, 1, port, conns_per_worker, file_mode, &base_dir);
    } else {
        // Multi-ring: each thread gets its own listener + io_uring ring
        let base_dir = std::sync::Arc::new(base_dir);
        let mut handles = Vec::with_capacity(num_threads - 1);

        for wid in 1..num_threads {
            let bd = base_dir.clone();
            handles.push(std::thread::Builder::new()
                .name(format!("ksvc-w{}", wid))
                .spawn(move || {
                    worker_loop(wid, num_threads, port, conns_per_worker, file_mode, &bd);
                })
                .expect("failed to spawn worker thread"));
        }

        // Worker 0 runs on main thread
        worker_loop(0, num_threads, port, conns_per_worker, file_mode, &base_dir);

        // Wait for all workers
        for h in handles {
            let _ = h.join();
        }
    }

    eprintln!("\nksvc-httpd: shutdown complete");
}

extern "C" fn handle_sigint(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::Relaxed);
}