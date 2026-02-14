//! Tokio HTTP/1.1 Server — comparison baseline
//!
//! Idiomatic async Tokio HTTP server with keep-alive support.
//! Same functionality as ksvc-httpd for fair comparison.
//!
//! Usage:
//!     ./target/release/tokio-httpd [--port 8081] [--dir ./www]
//!
//! Benchmark:
//!     wrk -t4 -c100 -d10s http://127.0.0.1:8081/

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

struct Stats {
    accepts: AtomicU64,
    requests: AtomicU64,
    responses: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    errors: AtomicU64,
}

impl Stats {
    fn new() -> Self {
        Self {
            accepts: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            responses: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }
}

const HELLO_BODY: &[u8] = b"Hello from Tokio!\n";

fn make_hello_response() -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: keep-alive\r\n\
         Server: tokio-httpd\r\n\
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
         Server: tokio-httpd\r\n\
         \r\n",
        content_type, body.len()
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
    else { "application/octet-stream" }
}

/// Extract path from "GET /path HTTP/1.1\r\n..."
fn parse_path(buf: &[u8]) -> Option<&str> {
    if buf.len() < 14 || &buf[..4] != b"GET " {
        return Some("/");
    }
    let start = 4;
    let end = buf[start..].iter().position(|&b| b == b' ')
        .map(|p| start + p)
        .unwrap_or(start + 1);
    std::str::from_utf8(&buf[start..end]).ok()
}

async fn handle_client(
    mut stream: tokio::net::TcpStream,
    stats: Arc<Stats>,
    hello_response: Arc<Vec<u8>>,
    not_found_response: Arc<Vec<u8>>,
    serve_dir: Option<Arc<String>>,
) {
    // TCP_NODELAY
    let _ = stream.set_nodelay(true);

    let mut buf = [0u8; 4096];
    let mut pos = 0usize;

    loop {
        // Read request
        let n = match stream.read(&mut buf[pos..]).await {
            Ok(0) => return,  // EOF
            Ok(n) => n,
            Err(_) => {
                stats.errors.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        stats.bytes_in.fetch_add(n as u64, Ordering::Relaxed);
        pos += n;

        // Check for \r\n\r\n
        let header_end = match buf[..pos].windows(4).position(|w| w == b"\r\n\r\n") {
            Some(p) => p + 4,
            None => {
                if pos >= buf.len() {
                    return; // buffer full, no request
                }
                continue;
            }
        };

        stats.requests.fetch_add(1, Ordering::Relaxed);

        let response = match &serve_dir {
            None => {
                // Hello mode
                hello_response.as_ref().clone()
            }
            Some(base) => {
                let path = parse_path(&buf[..header_end]).unwrap_or("/");
                let path = if path == "/" { "/index.html" } else { path };

                if path.contains("..") {
                    not_found_response.as_ref().clone()
                } else {
                    let full_path = format!("{}{}", base, path);
                    match tokio::fs::read(&full_path).await {
                        Ok(content) => {
                            let ctype = guess_content_type(&full_path);
                            make_file_response(ctype, &content)
                        }
                        Err(_) => not_found_response.as_ref().clone(),
                    }
                }
            }
        };

        // Send response
        let resp_len = response.len();
        match stream.write_all(&response).await {
            Ok(()) => {
                stats.bytes_out.fetch_add(resp_len as u64, Ordering::Relaxed);
                stats.responses.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                stats.errors.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }

        // Keep-alive: reset for next request
        pos = 0;
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut port: u16 = env::var("gvt_app_port").ok()
         .and_then(|v| v.parse().ok())
      .unwrap_or(8080);
    let mut serve_dir: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => { i += 1; port = args[i].parse().unwrap_or(8081); }
            "--dir" | "-d" => { i += 1; serve_dir = Some(args[i].clone()); }
            s if s.parse::<u16>().is_ok() => { port = s.parse().unwrap(); }
            _ => {}
        }
        i += 1;
    }

    let file_mode = serve_dir.is_some();
    eprintln!("tokio-httpd: port={} mode={}",
        port, if file_mode { format!("file({})", serve_dir.as_ref().unwrap()) } else { "hello".into() });

    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
        .await.expect("bind failed");

    // Set SO_REUSEPORT on the listener
    let stats = Arc::new(Stats::new());
    let hello_response = Arc::new(make_hello_response());
    let not_found_response = Arc::new(make_404_response());
    let serve_dir = serve_dir.map(|s| Arc::new(s));

    let start = std::time::Instant::now();
    let stats_clone = Arc::clone(&stats);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let elapsed = start.elapsed().as_secs_f64();
            let resp = stats_clone.responses.load(Ordering::Relaxed);
            let rps = if elapsed > 0.0 { resp as f64 / elapsed } else { 0.0 };
            eprintln!("[{:.1}s] req={} resp={} rps={:.0} bytes_in={} bytes_out={} err={}",
                elapsed,
                stats_clone.requests.load(Ordering::Relaxed),
                resp, rps,
                stats_clone.bytes_in.load(Ordering::Relaxed),
                stats_clone.bytes_out.load(Ordering::Relaxed),
                stats_clone.errors.load(Ordering::Relaxed),
            );
        }
    });

    eprintln!("tokio-httpd: listening on http://0.0.0.0:{}/", port);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                stats.accepts.fetch_add(1, Ordering::Relaxed);
                let s = Arc::clone(&stats);
                let hr = Arc::clone(&hello_response);
                let nr = Arc::clone(&not_found_response);
                let sd = serve_dir.clone();
                tokio::spawn(handle_client(stream, s, hr, nr, sd));
            }
            Err(e) => {
                eprintln!("tokio-httpd: accept error: {}", e);
            }
        }
    }
}
