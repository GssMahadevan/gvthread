//! Context switch test with configurable parameters
//!
//! # Environment Variables
//!
//! - `GVT_WORKERS=<n>` - Number of worker threads (default: 4)
//! - `GVT_LOW_WORKERS=<n>` - Low-priority workers (default: 1)
//! - `GVT_GVTHREADS=<n>` - Number of GVThreads (default: 10)
//! - `GVT_YIELDS=<n>` - Yields per GVThread (default: 5)
//! - `GVT_DEBUG=1` - Enable debug logging
//! - `GVT_LOG_LEVEL=debug` - Set log level
//! - `GVT_KPRINT_TIME=1` - Show timestamps

use gvthread::{Runtime, spawn, yield_now, SchedulerConfig};
use gvthread::{env_get, env_get_bool, kinfo};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    println!("=== Context Switch Test ===\n");
    
    // Read configuration from environment
    let num_workers: usize = env_get("GVT_WORKERS", 4);
    let num_low_workers: usize = env_get("GVT_LOW_WORKERS", 1);
    let total_gvthreads: usize = env_get("GVT_GVTHREADS", 10);
    let yields_per_gvthread: usize = env_get("GVT_YIELDS", 5);
    let debug_logging: bool = env_get_bool("GVT_DEBUG", true);
    
    println!("Configuration:");
    println!("  Workers: {} (low-priority: {})", num_workers, num_low_workers);
    println!("  GVThreads: {}, yields/thread: {}", total_gvthreads, yields_per_gvthread);
    println!("  Debug: {}", debug_logging);
    println!();
    
    let config = SchedulerConfig::default()
        .num_workers(num_workers)
        .num_low_priority_workers(num_low_workers)
        .debug_logging(debug_logging);
    
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        let completed = Arc::new(AtomicUsize::new(0));
        let total_yields = Arc::new(AtomicUsize::new(0));
        
        kinfo!("Spawning {} GVThreads, each yielding {} times", 
               total_gvthreads, yields_per_gvthread);
        
        for _i in 0..total_gvthreads {
            let c = completed.clone();
            let ty = total_yields.clone();
            let yields = yields_per_gvthread;
            
            spawn(move |_token| {
                for _j in 0..yields {
                    ty.fetch_add(1, Ordering::SeqCst);
                    yield_now();
                }
                c.fetch_add(1, Ordering::SeqCst);
            });
        }
        
        // Wait for completion
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10);
        
        while completed.load(Ordering::SeqCst) < total_gvthreads {
            if start.elapsed() > timeout {
                println!("TIMEOUT!");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        
        let c = completed.load(Ordering::SeqCst);
        let y = total_yields.load(Ordering::SeqCst);
        let expected_yields = total_gvthreads * yields_per_gvthread;
        
        println!("\n=== Results ===");
        println!("Completed: {}/{}", c, total_gvthreads);
        println!("Total yields: {} (expected: {})", y, expected_yields);
        
        if c == total_gvthreads && y == expected_yields {
            println!("\n*** SUCCESS ***");
        } else {
            println!("\n*** FAILURE ***");
        }
    });
    
    println!("\n=== Done ===");
}