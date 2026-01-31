//! Minimal context switch test
//!
//! Tests context switching directly without the full scheduler.

use gvthread::{Runtime, spawn, yield_now, SchedulerConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    println!("=== Minimal Context Switch Test ===\n");
    
    // Use single worker to eliminate race conditions
    let config = SchedulerConfig::default()
        .num_workers(1)
        .num_low_priority_workers(0)
        .debug_logging(true);
    
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        // Counter to detect if closure restarts
        let entry_count = Arc::new(AtomicUsize::new(0));
        let yield_count = Arc::new(AtomicUsize::new(0));
        
        let ec = entry_count.clone();
        let yc = yield_count.clone();
        
        println!("Spawning test GVThread...");
        
        spawn(move |_token| {
            // Track closure entry
            let entries = ec.fetch_add(1, Ordering::SeqCst) + 1;
            println!("==> CLOSURE ENTERED (entry #{})", entries);
            
            if entries > 1 {
                println!("!!! ERROR: Closure entered {} times - context not saved properly!", entries);
                return;
            }
            
            // Do 3 yields
            for i in 1..=3 {
                println!("  Before yield #{}", i);
                yield_now();
                let yields = yc.fetch_add(1, Ordering::SeqCst) + 1;
                println!("  After yield #{} (total yields: {})", i, yields);
            }
            
            println!("==> CLOSURE FINISHED");
        });
        
        // Wait for completion
        println!("\nWaiting for test to complete...");
        for i in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            
            let entries = entry_count.load(Ordering::SeqCst);
            let yields = yield_count.load(Ordering::SeqCst);
            
            if entries > 1 {
                println!("\n!!! FAILURE: Multiple closure entries detected!");
                break;
            }
            
            if yields >= 3 {
                println!("\n*** SUCCESS: All 3 yields completed! ***");
                break;
            }
            
            if i % 10 == 9 {
                println!("  (waiting... entries={}, yields={})", entries, yields);
            }
        }
        
        println!("\nFinal: entry_count={}, yield_count={}", 
                 entry_count.load(Ordering::SeqCst),
                 yield_count.load(Ordering::SeqCst));
    });
    std::thread::sleep(std::time::Duration::from_secs(10));

    println!("\n=== Test Complete ===");
}
