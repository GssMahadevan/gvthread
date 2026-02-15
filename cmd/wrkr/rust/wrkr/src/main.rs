//! # wrkr — benchmark load generator
//!
//! Pluggable strategies using battle-tested libraries.
//! We write measurement + orchestration only.
//!
//! ## HTTP strategies (selected via gvt_app_http env var)
//!
//!   hyper   — bare HTTP engine, zero-copy parsing, minimal alloc (default)
//!   reqwest — application-grade client (TLS, cookies, redirects)
//!
//! ## Protocol strategies (auto-detected from URL scheme)
//!
//!   http://   — HTTP/1.1 GET
//!   https://  — HTTPS GET (reqwest only for now)
//!   echo://   — raw TCP send/recv via tokio
//!
//! ## Usage
//!
//!   wrkr http://127.0.0.1:8080/ -c50 -d5
//!   gvt_app_http=reqwest wrkr https://example.com/ -c20 -d10
//!   wrkr echo://127.0.0.1:9000/ -c50 -d5 --payload 64

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// hyper imports
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

// ═══════════════════════════════════════════════════════════════════
// HTTP strategy: hyper — bare engine, minimal overhead
// ═══════════════════════════════════════════════════════════════════
//
// hyper handles:  HTTP/1.1 parsing, keep-alive, pipelining
// hyper does NOT: TLS, cookies, redirects, compression
//
// For benchmarking http:// on localhost this is ideal —
// zero per-request allocation beyond what HTTP requires.

fn build_hyper_client(keepalive: bool) -> HyperClient<HttpConnector, Empty<Bytes>> {
    let mut connector = HttpConnector::new();
    connector.set_nodelay(true);

    let builder = HyperClient::builder(TokioExecutor::new())
        .pool_idle_timeout(if keepalive { Duration::from_secs(90) } else { Duration::ZERO });

    builder.build(connector)
}

async fn run_hyper_phase(
    client: &HyperClient<HttpConnector, Empty<Bytes>>,
    url: &str,
    connections: usize,
    duration: Duration,
    counter: Arc<AtomicU64>,
    collect: bool,
) -> PhaseResult {
    let stop = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    let stop_t = stop.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(duration).await;
        stop_t.store(true, Ordering::Release);
    });

    let uri: hyper::Uri = url.parse().expect("invalid URL for hyper");

    let mut handles = Vec::with_capacity(connections);
    for _ in 0..connections {
        let stop = stop.clone();
        let client = client.clone();
        let uri = uri.clone();
        let counter = counter.clone();

        handles.push(tokio::spawn(async move {
            let mut r = ConnResult {
                requests: 0, errors: 0,
                latencies_ns: Vec::with_capacity(if collect { 200_000 } else { 0 }),
            };

            while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                match client.get(uri.clone()).await {
                    Ok(resp) => {
                        // Consume body to allow connection reuse
                        match resp.into_body().collect().await {
                            Ok(_) => {
                                let lat = t.elapsed().as_nanos() as u64;
                                r.requests += 1;
                                if collect { r.latencies_ns.push(lat); }
                                counter.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => r.errors += 1,
                        }
                    }
                    Err(_) => r.errors += 1,
                }
            }
            r
        }));
    }

    timer.await.ok();
    merge_results(start.elapsed().as_secs_f64(), handles, collect).await
}

// ═══════════════════════════════════════════════════════════════════
// HTTP strategy: reqwest — application-grade client
// ═══════════════════════════════════════════════════════════════════
//
// reqwest handles: TLS (rustls), connection pool, keep-alive,
//                  redirects, cookies, compression, header maps
//
// Higher per-request overhead but needed for HTTPS and real-world
// protocol features.

fn build_reqwest_client(keepalive: bool, connections: usize) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .pool_max_idle_per_host(if keepalive { connections } else { 0 })
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .tcp_nodelay(true);

    if !keepalive {
        builder = builder.connection_verbose(false);
    }

    builder.build().expect("failed to build reqwest client")
}

async fn run_reqwest_phase(
    client: &reqwest::Client,
    url: &str,
    connections: usize,
    duration: Duration,
    counter: Arc<AtomicU64>,
    collect: bool,
) -> PhaseResult {
    let stop = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    let stop_t = stop.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(duration).await;
        stop_t.store(true, Ordering::Release);
    });

    let mut handles = Vec::with_capacity(connections);
    for _ in 0..connections {
        let stop = stop.clone();
        let client = client.clone();
        let url = url.to_string();
        let counter = counter.clone();

        handles.push(tokio::spawn(async move {
            let mut r = ConnResult {
                requests: 0, errors: 0,
                latencies_ns: Vec::with_capacity(if collect { 200_000 } else { 0 }),
            };

            while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                match client.get(&url).send().await {
                    Ok(resp) => match resp.bytes().await {
                        Ok(_) => {
                            let lat = t.elapsed().as_nanos() as u64;
                            r.requests += 1;
                            if collect { r.latencies_ns.push(lat); }
                            counter.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => r.errors += 1,
                    },
                    Err(_) => r.errors += 1,
                }
            }
            r
        }));
    }

    timer.await.ok();
    merge_results(start.elapsed().as_secs_f64(), handles, collect).await
}

// ═══════════════════════════════════════════════════════════════════
// Echo strategy — tokio async TCP
// ═══════════════════════════════════════════════════════════════════

struct EchoConn {
    stream: TcpStream,
    send_buf: Vec<u8>,
    recv_buf: Vec<u8>,
}

impl EchoConn {
    async fn connect(host: &str, port: u16, payload_size: usize) -> std::io::Result<Self> {
        let stream = TcpStream::connect((host, port)).await?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            send_buf: vec![0x42u8; payload_size],
            recv_buf: vec![0u8; payload_size],
        })
    }

    async fn roundtrip(&mut self) -> std::io::Result<u64> {
        let start = Instant::now();
        self.stream.write_all(&self.send_buf).await?;
        let mut total = 0;
        while total < self.send_buf.len() {
            let n = self.stream.read(&mut self.recv_buf[total..]).await?;
            if n == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof"));
            }
            total += n;
        }
        Ok(start.elapsed().as_nanos() as u64)
    }
}

async fn run_echo_phase(
    host: &str,
    port: u16,
    payload_size: usize,
    connections: usize,
    duration: Duration,
    counter: Arc<AtomicU64>,
    collect: bool,
) -> PhaseResult {
    let stop = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    let stop_t = stop.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(duration).await;
        stop_t.store(true, Ordering::Release);
    });

    let mut handles = Vec::with_capacity(connections);
    for _ in 0..connections {
        let stop = stop.clone();
        let host = host.to_string();
        let counter = counter.clone();

        handles.push(tokio::spawn(async move {
            let mut r = ConnResult {
                requests: 0, errors: 0,
                latencies_ns: Vec::with_capacity(if collect { 200_000 } else { 0 }),
            };

            let mut conn = match EchoConn::connect(&host, port, payload_size).await {
                Ok(c) => c,
                Err(_) => { r.errors = 1; return r; }
            };

            while !stop.load(Ordering::Relaxed) {
                match conn.roundtrip().await {
                    Ok(lat) => {
                        r.requests += 1;
                        if collect { r.latencies_ns.push(lat); }
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        r.errors += 1;
                        match EchoConn::connect(&host, port, payload_size).await {
                            Ok(c) => conn = c,
                            Err(_) => break,
                        }
                    }
                }
            }
            r
        }));
    }

    timer.await.ok();
    merge_results(start.elapsed().as_secs_f64(), handles, collect).await
}

// ═══════════════════════════════════════════════════════════════════
// Shared: ConnResult, PhaseResult, merge
// ═══════════════════════════════════════════════════════════════════

struct ConnResult {
    requests: u64,
    errors: u64,
    latencies_ns: Vec<u64>,
}

struct PhaseResult {
    duration_sec: f64,
    total_requests: u64,
    total_errors: u64,
    latencies_ns: Vec<u64>,
}

async fn merge_results(
    elapsed: f64,
    handles: Vec<tokio::task::JoinHandle<ConnResult>>,
    collect: bool,
) -> PhaseResult {
    let mut total_req = 0u64;
    let mut total_err = 0u64;
    let mut all_lat = Vec::new();
    for h in handles {
        if let Ok(r) = h.await {
            total_req += r.requests;
            total_err += r.errors;
            if collect { all_lat.extend(r.latencies_ns); }
        }
    }
    PhaseResult {
        duration_sec: elapsed,
        total_requests: total_req,
        total_errors: total_err,
        latencies_ns: all_lat,
    }
}

// ═══════════════════════════════════════════════════════════════════
// Progress reporter
// ═══════════════════════════════════════════════════════════════════

fn spawn_progress(counter: Arc<AtomicU64>) -> (Arc<AtomicBool>, tokio::task::JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_c = stop.clone();
    let handle = tokio::spawn(async move {
        let start = Instant::now();
        let mut last = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if stop_c.load(Ordering::Relaxed) { break; }
            let now = counter.load(Ordering::Relaxed);
            eprintln!("wrkr: [{:.0}s] {} req/s (total: {})",
                start.elapsed().as_secs_f64(), now - last, now);
            last = now;
        }
    });
    (stop, handle)
}

// ═══════════════════════════════════════════════════════════════════
// Latency statistics
// ═══════════════════════════════════════════════════════════════════

struct Stats {
    min_us: f64, max_us: f64, avg_us: f64,
    p50_us: f64, p75_us: f64, p90_us: f64, p99_us: f64, p99_9_us: f64,
}

fn compute_stats(lat: &mut Vec<u64>) -> Stats {
    if lat.is_empty() {
        return Stats { min_us:0.0, max_us:0.0, avg_us:0.0,
            p50_us:0.0, p75_us:0.0, p90_us:0.0, p99_us:0.0, p99_9_us:0.0 };
    }
    lat.sort_unstable();
    let n = lat.len();
    let sum: u64 = lat.iter().sum();
    let pct = |p: f64| -> f64 {
        let idx = ((p / 100.0) * (n as f64 - 1.0)).ceil() as usize;
        lat[idx.min(n - 1)] as f64 / 1000.0
    };
    Stats {
        min_us: lat[0] as f64 / 1000.0,
        max_us: lat[n-1] as f64 / 1000.0,
        avg_us: (sum as f64 / n as f64) / 1000.0,
        p50_us: pct(50.0), p75_us: pct(75.0), p90_us: pct(90.0),
        p99_us: pct(99.0), p99_9_us: pct(99.9),
    }
}

// ═══════════════════════════════════════════════════════════════════
// JSON output (no serde)
// ═══════════════════════════════════════════════════════════════════

fn emit_json(strategy: &str, target: &str, connections: usize,
             dur: f64, reqs: u64, errs: u64, s: &Stats) {
    let rps = reqs as f64 / dur;
    let e = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    println!(r#"{{
  "strategy": "{}",
  "target": "{}",
  "connections": {},
  "duration_sec": {:.3},
  "total_requests": {},
  "requests_per_sec": {:.2},
  "total_errors": {},
  "latency_us": {{
    "min": {:.1},
    "avg": {:.1},
    "p50": {:.1},
    "p75": {:.1},
    "p90": {:.1},
    "p99": {:.1},
    "p99.9": {:.1},
    "max": {:.1}
  }}
}}"#, e(strategy), e(target), connections, dur, reqs, rps, errs,
        s.min_us, s.avg_us, s.p50_us, s.p75_us, s.p90_us,
        s.p99_us, s.p99_9_us, s.max_us);
}

// ═══════════════════════════════════════════════════════════════════
// CLI
// ═══════════════════════════════════════════════════════════════════

#[derive(Clone, Copy, PartialEq)]
enum HttpImpl {
    Hyper,
    Reqwest,
}

struct Cfg {
    url: String,
    connections: usize,
    duration_sec: u64,
    warmup_sec: u64,
    keepalive: bool,
    payload_size: usize,
    http_impl: HttpImpl,
}

fn parse_args() -> Cfg {
    let args: Vec<String> = std::env::args().collect();
    let mut c = Cfg {
        url: String::new(), connections: 50, duration_sec: 10,
        warmup_sec: 0, keepalive: true, payload_size: 64,
        http_impl: HttpImpl::Hyper,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-c"|"--connections" => { i+=1; c.connections = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(50); }
            "-d"|"--duration" => { i+=1; c.duration_sec = args.get(i).and_then(|s| s.trim_end_matches('s').parse().ok()).unwrap_or(10); }
            "--warmup" => { i+=1; c.warmup_sec = args.get(i).and_then(|s| s.trim_end_matches('s').parse().ok()).unwrap_or(0); }
            "--no-keepalive" => { c.keepalive = false; }
            "-H"|"--header" => { i+=1; if args.get(i).map_or(false, |h| h.to_lowercase().starts_with("connection: close")) { c.keepalive = false; } }
            "--payload" => { i+=1; c.payload_size = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(64); }
            "-h"|"--help" => { eprint_usage(); std::process::exit(0); }
            s if !s.starts_with('-') && c.url.is_empty() => { c.url = s.to_string(); }
            other => { eprintln!("wrkr: unknown: {}", other); std::process::exit(1); }
        }
        i += 1;
    }

    // Env overrides
    if let Ok(v) = std::env::var("WRKR_CONNECTIONS") { if let Ok(n) = v.parse() { c.connections = n; } }
    if let Ok(v) = std::env::var("WRKR_DURATION") { if let Ok(n) = v.parse() { c.duration_sec = n; } }
    if let Ok(v) = std::env::var("WRKR_WARMUP") { if let Ok(n) = v.parse() { c.warmup_sec = n; } }

    // HTTP implementation: gvt_app_http=hyper|reqwest
    if let Ok(v) = std::env::var("gvt_app_http") {
        match v.as_str() {
            "reqwest" => c.http_impl = HttpImpl::Reqwest,
            "hyper" => c.http_impl = HttpImpl::Hyper,
            other => {
                eprintln!("wrkr: unknown gvt_app_http={} (use hyper|reqwest)", other);
                std::process::exit(1);
            }
        }
    }

    // HTTPS requires reqwest (hyper needs separate TLS setup)
    if c.url.starts_with("https://") && c.http_impl == HttpImpl::Hyper {
        eprintln!("wrkr: https:// requires reqwest, switching gvt_app_http=reqwest");
        c.http_impl = HttpImpl::Reqwest;
    }

    if c.url.is_empty() { eprintln!("wrkr: missing URL"); eprint_usage(); std::process::exit(1); }
    c
}

fn eprint_usage() {
    eprintln!(
"Usage: wrkr [OPTIONS] <URL>

URL schemes:
  http://    HTTP/1.1 GET
  https://   HTTPS GET (forces reqwest)
  echo://    Raw TCP echo via tokio

Env vars:
  gvt_app_http=hyper|reqwest   HTTP strategy (default: hyper)
  WRKR_CONNECTIONS=N           Override -c
  WRKR_DURATION=N              Override -d
  WRKR_WARMUP=N                Override --warmup

Options:
  -c  --connections <N>   Concurrent connections (default: 50)
  -d  --duration <SEC>    Measurement duration (default: 10)
      --warmup <SEC>      Warmup duration (default: 0)
      --no-keepalive      Close after each request
      --payload <BYTES>   Echo payload size (default: 64)

Output: JSON on stdout, progress on stderr.");
}

// ═══════════════════════════════════════════════════════════════════
// Main
// ═══════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() {
    let cfg = parse_args();
    let is_echo = cfg.url.starts_with("echo://");

    let strategy_name = if is_echo {
        "echo".to_string()
    } else {
        let proto = if cfg.url.starts_with("https") { "https" } else { "http" };
        let impl_name = match cfg.http_impl { HttpImpl::Hyper => "hyper", HttpImpl::Reqwest => "reqwest" };
        format!("{}+{}", proto, impl_name)
    };

    eprintln!("wrkr: {} conns, {}s, strategy={}, keepalive={}, target={}",
        cfg.connections, cfg.duration_sec, strategy_name, cfg.keepalive, cfg.url);

    let counter = Arc::new(AtomicU64::new(0));

    // Dispatch once — build the right client, run phases
    let run = |dur: Duration, collect: bool, counter: Arc<AtomicU64>| {
        let url = cfg.url.clone();
        let conns = cfg.connections;
        let ka = cfg.keepalive;
        let ps = cfg.payload_size;
        let http_impl = cfg.http_impl;
        async move {
            if is_echo {
                let (host, port) = parse_echo_addr(&url);
                run_echo_phase(&host, port, ps, conns, dur, counter, collect).await
            } else {
                match http_impl {
                    HttpImpl::Hyper => {
                        let client = build_hyper_client(ka);
                        run_hyper_phase(&client, &url, conns, dur, counter, collect).await
                    }
                    HttpImpl::Reqwest => {
                        let client = build_reqwest_client(ka, conns);
                        run_reqwest_phase(&client, &url, conns, dur, counter, collect).await
                    }
                }
            }
        }
    };

    // Warmup
    if cfg.warmup_sec > 0 {
        eprintln!("wrkr: warmup ({}s) ...", cfg.warmup_sec);
        run(Duration::from_secs(cfg.warmup_sec), false, counter.clone()).await;
        counter.store(0, Ordering::Release);
        eprintln!("wrkr: warmup done");
    }

    // Measure
    eprintln!("wrkr: measuring ({}s) ...", cfg.duration_sec);
    let (prog_stop, prog_handle) = spawn_progress(counter.clone());

    let mut result = run(Duration::from_secs(cfg.duration_sec), true, counter.clone()).await;

    prog_stop.store(true, Ordering::Release);
    prog_handle.await.ok();

    let stats = compute_stats(&mut result.latencies_ns);
    eprintln!("wrkr: done — {} reqs in {:.2}s ({:.0} req/s), {} errors",
        result.total_requests, result.duration_sec,
        result.total_requests as f64 / result.duration_sec, result.total_errors);

    emit_json(&strategy_name, &cfg.url, cfg.connections,
        result.duration_sec, result.total_requests, result.total_errors, &stats);
}

fn parse_echo_addr(url: &str) -> (String, u16) {
    let rest = url.strip_prefix("echo://").unwrap_or(url).trim_end_matches('/');
    match rest.rfind(':') {
        Some(i) => (rest[..i].to_string(), rest[i+1..].parse().unwrap_or(9000)),
        None => (rest.to_string(), 9000),
    }
}