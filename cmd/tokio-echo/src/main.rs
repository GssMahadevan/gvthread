//! Tokio Echo Server — the gold standard
//!
//! Idiomatic async Tokio echo for comparison with ksvc-echo.
//! Uses epoll under the hood (tokio's mio runtime).
//! cargo build --release -p ksvc-echo -p tokio-echo
//! Usage:
//!     ./target/release/tokio-echo [port]
//!
//! Test with same client:
//!     python3 cmd/ksvc-echo/test_echo.py --port 9998

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

struct Stats {
    accepts: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    active: AtomicU64,
    errors: AtomicU64,
}

impl Stats {
    fn new() -> Self {
        Self {
            accepts: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            active: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }
}

async fn handle_client(
    mut stream: tokio::net::TcpStream,
    stats: Arc<Stats>,
) {
    stats.active.fetch_add(1, Ordering::Relaxed);

    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break, // EOF
            Ok(n) => {
                stats.bytes_in.fetch_add(n as u64, Ordering::Relaxed);
                match stream.write_all(&buf[..n]).await {
                    Ok(()) => {
                        stats.bytes_out.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    Err(_) => {
                        stats.errors.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                }
            }
            Err(_) => {
                stats.errors.fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
    }

    stats.active.fetch_sub(1, Ordering::Relaxed);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(9998);

    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .expect("bind failed");

    let stats = Arc::new(Stats::new());
    let start = std::time::Instant::now();

    eprintln!("tokio-echo: listening on 0.0.0.0:{}", port);
    eprintln!("tokio-echo: runtime = multi-thread (epoll)");

    // Stats printer
    let stats_clone = Arc::clone(&stats);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let elapsed = start.elapsed().as_secs_f64();
            eprintln!(
                "[{:.1}s] active={} accepts={} bytes_in={} bytes_out={} err={}",
                elapsed,
                stats_clone.active.load(Ordering::Relaxed),
                stats_clone.accepts.load(Ordering::Relaxed),
                stats_clone.bytes_in.load(Ordering::Relaxed),
                stats_clone.bytes_out.load(Ordering::Relaxed),
                stats_clone.errors.load(Ordering::Relaxed),
            );
        }
    });

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                stats.accepts.fetch_add(1, Ordering::Relaxed);
                let s = Arc::clone(&stats);
                tokio::spawn(handle_client(stream, s));
            }
            Err(e) => {
                eprintln!("tokio-echo: accept error: {}", e);
                stats.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
