//! # GVThread HTTP/1.1 Server
//!
//! One GVThread per connection. Blocking-style code. io_uring underneath.
//!
//! This is the Go-like programming model:
//! - The accept loop spawns a GVThread for each connection
//! - Each GVThread reads, parses, responds using blocking-style calls
//! - Under the hood, I/O is multiplexed onto io_uring by the reactor thread
//! - Worker OS threads are never blocked — they run other GVThreads
//!
//! ## Comparison
//!
//! | Server              | Model                  | I/O Backend  |
//! |---------------------|------------------------|--------------|
//! | ksvc-httpd          | single-thread callback | io_uring     |
//! | tokio-httpd         | async/await            | epoll/mio    |
//! | **gvthread-httpd**  | **green thread (this)**| **io_uring** |
//! | Go net/http         | goroutine              | epoll        |
//!
//! ## Usage
//!
//!     cargo run -p gvthread-httpd --release -- [--port 8080] [--workers 4]
//!
//! ## Benchmark
//!
//!     wrk -t4 -c100 -d10s http://127.0.0.1:8080/

use gvthread::{Runtime, SchedulerConfig, spawn, Priority};
use ksvc_gvthread::{WorkerReactorPool, GvtListener, GvtStream};

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

// ── Configuration ──

const RECV_BUF_SIZE: usize = 4096;

static RUNNING: AtomicBool = AtomicBool::new(true);
static TOTAL_REQUESTS: AtomicU64 = AtomicU64::new(0);
static TOTAL_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
static ACTIVE_CONNECTIONS: AtomicU64 = AtomicU64::new(0);

// ── HTTP response ──

const HELLO_BODY: &[u8] = b"Hello from GVThread!\n";

fn make_hello_response() -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: keep-alive\r\n\
         Server: gvthread-httpd\r\n\
         \r\n",
        HELLO_BODY.len()
    )
    .into_bytes()
    .into_iter()
    .chain(HELLO_BODY.iter().copied())
    .collect()
}

// ── HTTP parsing ──

/// Find \r\n\r\n in buffer. Returns true if a complete request is found.
fn has_complete_request(buf: &[u8], len: usize) -> bool {
    if len < 4 {
        return false;
    }
    buf[..len]
        .windows(4)
        .any(|w| w == b"\r\n\r\n")
}

// ── Per-connection handler ──

/// Handle a single connection. Runs as a GVThread.
///
/// This is the beauty of the green thread model: straightforward
/// sequential code, no callbacks, no async/await, no state machines.
fn handle_connection(stream: GvtStream) {
    let response = make_hello_response();
    let mut buf = [0u8; RECV_BUF_SIZE];
    let mut recv_len: usize = 0;

    ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed);

    // Keep-alive loop
    loop {
        if !RUNNING.load(Ordering::Relaxed) {
            break;
        }

        // Read data
        let n = stream.read(&mut buf[recv_len..]);
        if n <= 0 {
            // EOF or error — client disconnected
            break;
        }
        recv_len += n as usize;

        // Check for complete HTTP request
        if !has_complete_request(&buf, recv_len) {
            if recv_len >= RECV_BUF_SIZE {
                // Buffer full, no complete request — drop
                break;
            }
            continue;
        }

        // Got a complete request — send response
        TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed);

        let sent = stream.write_all(&response);
        if sent < 0 {
            break;
        }

        // Reset for next request (keep-alive)
        recv_len = 0;
    }

    ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
    // GvtStream::drop() closes the fd
}

// ── Accept loop ──

/// The accept loop runs as a GVThread. It blocks on accept() (via io_uring)
/// and spawns a new GVThread for each incoming connection.
fn accept_loop(listener: GvtListener) {
    eprintln!("gvthread-httpd: accept loop running (GVThread)");

    loop {
        if !RUNNING.load(Ordering::Relaxed) {
            break;
        }

        match listener.accept() {
            Ok(stream) => {
                TOTAL_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
                // Spawn a GVThread for this connection — just like Go!
                spawn(move |_token| {
                    handle_connection(stream);
                });
            }
            Err(e) => {
                if e == -(libc::EAGAIN as i64) || e == -(libc::EINTR as i64) {
                    gvthread::yield_now();
                    continue;
                }
                eprintln!("gvthread-httpd: accept error: {}", e);
                if !RUNNING.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
    }
}

// ── Stats printer ──

fn stats_loop() {
    let start = std::time::Instant::now();
    let mut last_reqs: u64 = 0;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if !RUNNING.load(Ordering::Relaxed) {
            break;
        }

        let elapsed = start.elapsed().as_secs_f64();
        let total_reqs = TOTAL_REQUESTS.load(Ordering::Relaxed);
        let delta = total_reqs - last_reqs;
        let rps = delta as f64 / 5.0;
        let active = ACTIVE_CONNECTIONS.load(Ordering::Relaxed);
        let total_conns = TOTAL_CONNECTIONS.load(Ordering::Relaxed);

        eprintln!(
            "[{:.1}s] active={} total_conns={} reqs={} rps={:.0}",
            elapsed, active, total_conns, total_reqs, rps,
        );
        last_reqs = total_reqs;
    }
}

// ── Main ──

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Defaults
    let mut port: u16 = 8080;
    let mut num_workers: usize = 4;
    let mut max_gvthreads: usize = 100_000;
    let mut sq_entries: u32 = 1024;

    // Phase 1: Read gvt_* env vars (bench-runner sets these)
    if let Ok(v) = std::env::var("gvt_app_port") {
        if let Ok(p) = v.parse::<u16>() { port = p; }
    }
    if let Ok(v) = std::env::var("gvt_parallelism") {
        if let Ok(w) = v.parse::<usize>() { num_workers = w; }
    }
    if let Ok(v) = std::env::var("gvt_app_max_gvthreads") {
        if let Ok(m) = v.parse::<usize>() { max_gvthreads = m; }
    }
    if let Ok(v) = std::env::var("gvt_app_sq_entries") {
        if let Ok(s) = v.parse::<u32>() { sq_entries = s; }
    }

    // Phase 2: CLI flags override env vars
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                i += 1;
                if let Some(p) = args.get(i).and_then(|s| s.parse().ok()) { port = p; }
            }
            "--workers" | "-w" => {
                i += 1;
                if let Some(w) = args.get(i).and_then(|s| s.parse().ok()) { num_workers = w; }
            }
            "--max-gvthreads" => {
                i += 1;
                if let Some(m) = args.get(i).and_then(|s| s.parse().ok()) { max_gvthreads = m; }
            }
            "--sq" => {
                i += 1;
                if let Some(s) = args.get(i).and_then(|s| s.parse().ok()) { sq_entries = s; }
            }
            s if s.parse::<u16>().is_ok() => {
                port = s.parse().unwrap();
            }
            _ => {}
        }
        i += 1;
    }

    unsafe {
        libc::signal(libc::SIGINT, handle_sigint as usize);
        libc::signal(libc::SIGTERM, handle_sigint as usize);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    eprintln!("gvthread-httpd: port={} workers={} max_gvt={} sq={}",
        port, num_workers, max_gvthreads, sq_entries);
    eprintln!("gvthread-httpd: model = one GVThread per connection + per-worker io_uring");

    // ── 1. Start GVThread runtime ──
    let config = SchedulerConfig::default()
        .num_workers(num_workers)
        .max_gvthreads(max_gvthreads);
    let mut runtime = Runtime::new(config);

    // ── 2. Per-worker io_uring pool (replaces shared reactor) ──
    //
    // Each worker thread gets its own io_uring instance.  GVThreads
    // submit I/O inline to their worker's ring — no MPSC queue, no
    // cross-thread hop, no lock contention.  Hooks are auto-installed.
    let pool = WorkerReactorPool::init_global(num_workers, sq_entries, max_gvthreads);

    // ── 3. Stats thread (OS thread, not GVThread) ──
    let _stats = std::thread::Builder::new()
        .name("stats".into())
        .spawn(|| stats_loop())
        .unwrap();

    // ── 4. Run accept loop as GVThread ──
    eprintln!("gvthread-httpd: listening on http://0.0.0.0:{}/", port);

    runtime.block_on(|| {
        let listener = GvtListener::bind_local(port)
            .expect("failed to bind listener");

        // The accept loop itself is a GVThread
        spawn(move |_token| {
            accept_loop(listener);
        });

        // Main GVThread just waits for shutdown
        while RUNNING.load(Ordering::Relaxed) {
            gvthread::sleep_ms(100);
        }
    });

    // ── 5. Cleanup ──
    pool.shutdown();

    let total = TOTAL_REQUESTS.load(Ordering::Relaxed);
    let conns = TOTAL_CONNECTIONS.load(Ordering::Relaxed);
    eprintln!("\ngvthread-httpd: shutdown — {} requests, {} connections", total, conns);
}

extern "C" fn handle_sigint(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::Relaxed);
}
