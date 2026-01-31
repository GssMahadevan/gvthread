//! Basic GVThread example
//!
//! Demonstrates spawning multiple GVThreads with multiple workers.
//!
//! # Environment Variables
//!
//! Configuration:
//! - `GVT_WORKERS=<n>` - Number of worker threads (default: 4)
//! - `GVT_LOW_WORKERS=<n>` - Number of low-priority workers (default: 1)
//! - `GVT_GVTHREADS=<n>` - Number of GVThreads to spawn (default: 3)
//! - `GVT_YIELDS=<n>` - Number of yields per GVThread (default: 3)
//!
//! Logging:
//! - `GVT_LOG_LEVEL=<level>` - Log level: off, error, warn, info, debug, trace (default: info)
//! - `GVT_KPRINT_TIME=1` - Include nanosecond timestamps
//! - `GVT_FLUSH_EPRINT=1` - Flush output immediately (for crash debugging)
//! - `GVT_DEBUG=1` - Enable scheduler debug logging

use gvthread::{Runtime, spawn, spawn_with_priority, yield_now, Priority, SchedulerConfig};
use gvthread::{kinfo, kdebug, env_get, env_get_bool};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    println!("=== GVThread Basic Example ===\n");
    
    // Read configuration from environment
    let num_workers: usize = env_get("GVT_WORKERS", 4);
    let num_low_workers: usize = env_get("GVT_LOW_WORKERS", 1);
    let num_gvthreads: usize = env_get("GVT_GVTHREADS", 3);
    let num_yields: usize = env_get("GVT_YIELDS", 3);
    let debug_logging: bool = env_get_bool("GVT_DEBUG", true);
    
    println!("Configuration:");
    println!("  Workers: {} (low-priority: {})", num_workers, num_low_workers);
    println!("  GVThreads: {}, yields per thread: {}", num_gvthreads, num_yields);
    println!("  Debug logging: {}", debug_logging);
    println!();
    
    let config = SchedulerConfig::default()
        .num_workers(num_workers)
        .num_low_priority_workers(num_low_workers)
        .debug_logging(debug_logging);
    
    let mut runtime = Runtime::new(config);
    
    // Counter to track completed GVThreads
    let total_expected = num_gvthreads + 1; // +1 for HIGH priority
    let completed = Arc::new(AtomicUsize::new(0));
    
    runtime.block_on(|| {
        kinfo!("Spawning {} normal + 1 HIGH priority GVThreads", num_gvthreads);
        
        // Spawn normal priority GVThreads
        for i in 1..=num_gvthreads {
            let c = completed.clone();
            let yields = num_yields;
            let id = spawn(move |_token| {
                kdebug!("GVThread {} started", i);
                
                for j in 0..yields {
                    kdebug!("GVThread {} iteration {}", i, j);
                    yield_now();
                }
                
                kdebug!("GVThread {} finished", i);
                c.fetch_add(1, Ordering::SeqCst);
            });
            println!("Spawned normal GVThread {} (ID={})", i, id);
        }
        
        // Spawn a HIGH priority GVThread
        let c = completed.clone();
        let high_id = spawn_with_priority(move |_token| {
            kdebug!("HIGH priority started");
            yield_now();
            kdebug!("HIGH priority finished");
            c.fetch_add(1, Ordering::SeqCst);
        }, Priority::High);
        println!("Spawned HIGH priority GVThread (ID={})", high_id);
        
        // Wait for all to complete
        println!("\nWaiting for {} GVThreads to complete...\n", total_expected);
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10);
        
        while completed.load(Ordering::SeqCst) < total_expected {
            if start.elapsed() > timeout {
                println!("WARNING: Timeout!");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        
        let count = completed.load(Ordering::SeqCst);
        kinfo!("{}/{} GVThread(s) completed", count, total_expected);
    });
    
    println!("\n=== Example Complete ===");
}