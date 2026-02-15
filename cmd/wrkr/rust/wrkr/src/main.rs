//! # wrkr — benchmark load generator
//!
//! Pluggable strategies using battle-tested libraries.
//! We write measurement + orchestration only.
//!
//! ## Strategies (auto-detected from URL scheme)
//!
//!   http://   — reqwest (HTTP/1.1, keep-alive, connection pool)
//!   https://  — reqwest (same, TLS via rustls — zero config)
//!   echo://   — tokio::net::TcpStream (raw TCP send/recv)
//!
//! Future: grpc:// (tonic), ws:// (tokio-tungstenite), udp:// (tokio UdpSocket)
//!
//! ## Usage
//!
//!   wrkr http://127.0.0.1:8080/ -c50 -d5
//!   wrkr https://example.com/ -c20 -d10
//!   wrkr echo://127.0.0.1:9000/ -c50 -d5 --payload 64

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ═══════════════════════════════════════════════════════════════════
// HTTP/HTTPS — reqwest handles TLS, pooling, keepalive, parsing
// ═══════════════════════════════════════════════════════════════════

fn build_http_client(keepalive: bool, connections: usize) -> reqwest::Client {
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

// ═══════════════════════════════════════════════════════════════════
// Echo — tokio async TCP
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

// ═══════════════════════════════════════════════════════════════════
// Per-connection result
// ═══════════════════════════════════════════════════════════════════

struct ConnResult {
    requests: u64,
    errors: u64,
    latencies_ns: Vec<u64>,
}

// ═══════════════════════════════════════════════════════════════════
// Phase runner — shared across strategies
// ═══════════════════════════════════════════════════════════════════

struct PhaseResult {
    duration_sec: f64,
    total_requests: u64,
    total_errors: u64,
    latencies_ns: Vec<u64>,
}

async fn run_http_phase(
    client: &reqwest::Client,
    url: &str,
    connections: usize,
    duration: Duration,
    counter: Arc<AtomicU64>,
    collect: bool,
) -> PhaseResult {
    let stop = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    // Timer
    let stop_t = stop.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(duration).await;
        stop_t.store(true, Ordering::Release);
    });

    // Spawn one task per connection
    let mut handles = Vec::with_capacity(connections);
    for _ in 0..connections {
        let stop = stop.clone();
        let client = client.clone(); // reqwest::Client is Arc internally
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
    let elapsed = start.elapsed().as_secs_f64();

    // Merge
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
    let elapsed = start.elapsed().as_secs_f64();

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
    let pct = |p: f64| lat[((p/100.0)*(n as f64 -1.0)).ceil() as usize].min(lat[n-1]) as f64 / 1000.0;

    Stats {
        min_us: lat[0] as f64 / 1000.0,
        max_us: lat[n-1] as f64 / 1000.0,
        avg_us: (sum as f64 / n as f64) / 1000.0,
        p50_us: pct(50.0), p75_us: pct(75.0), p90_us: pct(90.0),
        p99_us: pct(99.0), p99_9_us: pct(99.9),
    }
}

// ═══════════════════════════════════════════════════════════════════
// JSON output (no serde — fewer deps to compile)
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

struct Cfg {
    url: String,
    connections: usize,
    duration_sec: u64,
    warmup_sec: u64,
    keepalive: bool,
    payload_size: usize,
}

fn parse_args() -> Cfg {
    let args: Vec<String> = std::env::args().collect();
    let mut c = Cfg {
        url: String::new(), connections: 50, duration_sec: 10,
        warmup_sec: 0, keepalive: true, payload_size: 64,
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

    // Env overrides (bench-runner integration)
    if let Ok(v) = std::env::var("WRKR_CONNECTIONS") { if let Ok(n) = v.parse() { c.connections = n; } }
    if let Ok(v) = std::env::var("WRKR_DURATION") { if let Ok(n) = v.parse() { c.duration_sec = n; } }
    if let Ok(v) = std::env::var("WRKR_WARMUP") { if let Ok(n) = v.parse() { c.warmup_sec = n; } }

    if c.url.is_empty() { eprintln!("wrkr: missing URL"); eprint_usage(); std::process::exit(1); }
    c
}

fn eprint_usage() {
    eprintln!(
"Usage: wrkr [OPTIONS] <URL>

URL schemes (auto-detected):
  http://    HTTP/1.1 GET via reqwest
  https://   HTTPS GET via reqwest + rustls
  echo://    Raw TCP echo via tokio

Options:
  -c  --connections <N>   Concurrent connections (default: 50)
  -d  --duration <SEC>    Measurement duration (default: 10)
      --warmup <SEC>      Warmup duration (default: 0)
      --no-keepalive      Close connection after each request
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
    let strategy = if is_echo { "echo" }
                   else if cfg.url.starts_with("https") { "https" }
                   else { "http" };

    eprintln!("wrkr: {} conns, {}s, strategy={}, keepalive={}, target={}",
        cfg.connections, cfg.duration_sec, strategy, cfg.keepalive, cfg.url);

    let counter = Arc::new(AtomicU64::new(0));

    let phase = |dur, collect| {
        let counter = counter.clone();
        let url = cfg.url.clone();
        let conns = cfg.connections;
        let ka = cfg.keepalive;
        let ps = cfg.payload_size;
        async move {
            if is_echo {
                let (host, port) = parse_echo_addr(&url);
                run_echo_phase(&host, port, ps, conns, dur, counter, collect).await
            } else {
                let client = build_http_client(ka, conns);
                run_http_phase(&client, &url, conns, dur, counter, collect).await
            }
        }
    };

    // Warmup
    if cfg.warmup_sec > 0 {
        eprintln!("wrkr: warmup ({}s) ...", cfg.warmup_sec);
        phase(Duration::from_secs(cfg.warmup_sec), false).await;
        counter.store(0, Ordering::Release);
        eprintln!("wrkr: warmup done");
    }

    // Measure
    eprintln!("wrkr: measuring ({}s) ...", cfg.duration_sec);
    let (prog_stop, prog_handle) = spawn_progress(counter.clone());

    let mut result = phase(Duration::from_secs(cfg.duration_sec), true).await;

    prog_stop.store(true, Ordering::Release);
    prog_handle.await.ok();

    let stats = compute_stats(&mut result.latencies_ns);
    eprintln!("wrkr: done — {} reqs in {:.2}s ({:.0} req/s), {} errors",
        result.total_requests, result.duration_sec,
        result.total_requests as f64 / result.duration_sec, result.total_errors);

    emit_json(strategy, &cfg.url, cfg.connections,
        result.duration_sec, result.total_requests, result.total_errors, &stats);
}

fn parse_echo_addr(url: &str) -> (String, u16) {
    let rest = url.strip_prefix("echo://").unwrap_or(url).trim_end_matches('/');
    match rest.rfind(':') {
        Some(i) => (rest[..i].to_string(), rest[i+1..].parse().unwrap_or(9000)),
        None => (rest.to_string(), 9000),
    }
}
