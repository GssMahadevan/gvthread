//! Basic GVThread example
//!
//! Demonstrates spawning GVThreads and yielding.

use gvthread::{Runtime, spawn, yield_now, Priority, SchedulerConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    println!("=== GVThread Basic Example ===\n");
    
    // Use single worker initially to debug context switching
    let config = SchedulerConfig::default()
        .num_workers(1)
        .num_low_priority_workers(0)
        .debug_logging(true);
    
    let mut runtime = Runtime::new(config);
    
    // Counter to track completed GVThreads
    let completed = Arc::new(AtomicUsize::new(0));
    
    runtime.block_on(|| {
        println!("Spawning GVThreads...\n");
        
        // Spawn a simple GVThread
        let c1 = completed.clone();
        let id1 = spawn(move |_token| {
            println!("[GVThread 1] Started!");
            
            for i in 0..3 {
                println!("[GVThread 1] Iteration {}", i);
                yield_now();
            }
            
            println!("[GVThread 1] Finished!");
            c1.fetch_add(1, Ordering::SeqCst);
        });
        println!("Spawned GVThread: {}", id1);
        
        // Wait for first one to complete before spawning more
        println!("\nWaiting for GVThread 1 to complete...");
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5);
        
        while completed.load(Ordering::SeqCst) < 1 {
            if start.elapsed() > timeout {
                println!("WARNING: Timeout waiting for GVThread 1!");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        
        let count = completed.load(Ordering::SeqCst);
        println!("\n{} GVThread(s) completed!", count);
    });
    std::thread::sleep(std::time::Duration::from_secs(10));

    println!("\n=== Example Complete ===");
    
}
