//! Minimal context switch test
//!
//! Tests context switching with multiple workers.

use gvthread::{Runtime, spawn, yield_now, SchedulerConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
// GVT_LOG_LEVEL=debug GVT_FLUSH_EPRINT=1 cargo run -p gvthread-basic
fn main() {
    println!("=== Context Switch Test (Multi-Worker) ===\n");
    
    // Test with 4 workers
    let config = SchedulerConfig::default()
        .num_workers(4)
        .num_low_priority_workers(1)
        .debug_logging(true);
    
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        let total_gvthreads = 10;
        let yields_per_gvthread = 5;
        
        let completed = Arc::new(AtomicUsize::new(0));
        let total_yields = Arc::new(AtomicUsize::new(0));
        
        println!("Spawning {} GVThreads, each yielding {} times...\n", 
                 total_gvthreads, yields_per_gvthread);
        
        for i in 0..total_gvthreads {
            let c = completed.clone();
            let ty = total_yields.clone();
            
            spawn(move |_token| {
                for j in 0..yields_per_gvthread {
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
        
        println!("\n=== Results ===");
        println!("Completed: {}/{}", c, total_gvthreads);
        println!("Total yields: {} (expected: {})", y, total_gvthreads * yields_per_gvthread);
        
        if c == total_gvthreads && y == total_gvthreads * yields_per_gvthread {
            println!("\n*** SUCCESS ***");
        } else {
            println!("\n*** FAILURE ***");
        }
    });
    
    println!("\n=== Done ===");
}