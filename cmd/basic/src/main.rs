//! Basic GVThread example
//!
//! Demonstrates spawning GVThreads and yielding.

use gvthread::{Runtime, spawn, yield_now, Priority, SchedulerConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    println!("=== GVThread Basic Example ===\n");
    
    // Create runtime with custom config
    let config = SchedulerConfig::default()
        .num_workers(4)
        .debug_logging(true);
    
    let mut runtime = Runtime::new(config);
    
    // Counter to track completed GVThreads
    let completed = Arc::new(AtomicUsize::new(0));
    let total_gvthreads = 3;
    
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
        
        // Spawn another GVThread
        let c2 = completed.clone();
        let id2 = spawn(move |_token| {
            println!("[GVThread 2] Started!");
            
            for i in 0..3 {
                println!("[GVThread 2] Iteration {}", i);
                yield_now();
            }
            
            println!("[GVThread 2] Finished!");
            c2.fetch_add(1, Ordering::SeqCst);
        });
        println!("Spawned GVThread: {}", id2);
        
        // Spawn a high-priority GVThread
        let c3 = completed.clone();
        let id3 = gvthread::spawn_with_priority(move |_token| {
            println!("[GVThread 3 - HIGH] Started!");
            println!("[GVThread 3 - HIGH] Finished!");
            c3.fetch_add(1, Ordering::SeqCst);
        }, Priority::High);
        println!("Spawned HIGH priority GVThread: {}", id3);
        
        // Wait for all GVThreads to complete (with timeout)
        println!("\nWaiting for GVThreads to complete...");
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5);
        
        while completed.load(Ordering::SeqCst) < total_gvthreads {
            if start.elapsed() > timeout {
                println!("WARNING: Timeout waiting for GVThreads!");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        
        let count = completed.load(Ordering::SeqCst);
        println!("\n{}/{} GVThreads completed!", count, total_gvthreads);
    });
    
    println!("\n=== Example Complete ===");
}
