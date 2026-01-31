//! Basic GVThread example
//!
//! Demonstrates spawning multiple GVThreads with multiple workers.
//!
//! # Environment Variables
//!
//! - `GVT_FLUSH_EPRINT=1` - Flush debug output immediately (useful for crash debugging)
//! - `GVT_LOG_LEVEL=debug` - Set log level (off, error, warn, info, debug, trace)

use gvthread::{Runtime, spawn, spawn_with_priority, yield_now, Priority, SchedulerConfig};
use gvthread::{kinfo, kdebug, set_log_level, LogLevel};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
// GVT_LOG_LEVEL=debug GVT_FLUSH_EPRINT=1 cargo run -p gvthread-basic
fn main() {
    println!("=== GVThread Basic Example ===\n");
    
    // Initialize logging (reads GVT_FLUSH_EPRINT and GVT_LOG_LEVEL env vars)
    // Or set programmatically:
    // set_log_level(LogLevel::Debug);
    
    // Use 4 workers (3 normal + 1 low priority)
    let config = SchedulerConfig::default()
        .num_workers(4)
        .num_low_priority_workers(1)
        .debug_logging(true);
    
    let mut runtime = Runtime::new(config);
    
    // Counter to track completed GVThreads
    let completed = Arc::new(AtomicUsize::new(0));
    
    runtime.block_on(|| {
        kinfo!("Spawning GVThreads...");
        
        // Spawn 3 normal priority GVThreads
        for i in 1..=3 {
            let c = completed.clone();
            let id = spawn(move |_token| {
                kdebug!("[GVThread {}] Started", i);
                
                for j in 0..3 {
                    kdebug!("[GVThread {}] Iteration {}", i, j);
                    yield_now();
                }
                
                kdebug!("[GVThread {}] Finished", i);
                c.fetch_add(1, Ordering::SeqCst);
            });
            println!("Spawned normal GVThread {} (ID={})", i, id);
        }
        
        // Spawn a HIGH priority GVThread
        let c = completed.clone();
        let high_id = spawn_with_priority(move |_token| {
            kdebug!("[HIGH] Started");
            yield_now();
            kdebug!("[HIGH] Finished");
            c.fetch_add(1, Ordering::SeqCst);
        }, Priority::High);
        println!("Spawned HIGH priority GVThread (ID={})", high_id);
        
        // Wait for all to complete
        println!("\nWaiting for {} GVThreads to complete...\n", 4);
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10);
        
        while completed.load(Ordering::SeqCst) < 4 {
            if start.elapsed() > timeout {
                println!("WARNING: Timeout!");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        
        let count = completed.load(Ordering::SeqCst);
        kinfo!("{} GVThread(s) completed", count);
    });
    
    println!("\n=== Example Complete ===");
}