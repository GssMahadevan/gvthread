//! Playground for quick experiments
//!
//! Use this for testing new features or debugging.

use gvthread::{Runtime, spawn, yield_now, SchedulerConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    println!("=== GVThread Playground ===\n");
    
    // Simple test: just ONE gvthread with ONE worker
    let config = SchedulerConfig::default()
        .num_workers(1)  // Single worker for simpler debugging
        .debug_logging(true);
    
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        println!("Spawning single GVThread...\n");
        
        // Track how many times we enter the closure
        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        
        spawn(move |_token| {
            let count = cc.fetch_add(1, Ordering::SeqCst);
            println!(">>> CLOSURE ENTRY (call #{}) <<<", count + 1);
            
            if count > 0 {
                panic!("ERROR: Closure was called more than once!");
            }
            
            println!("Before first yield");
            yield_now();
            println!("After first yield");
            
            yield_now();
            println!("After second yield");
            
            yield_now();
            println!("After third yield - DONE!");
        });
        
        // Wait for completion
        println!("Waiting for GVThread to complete...");
        std::thread::sleep(std::time::Duration::from_secs(2));
        
        println!("call_count = {}", call_count.load(Ordering::SeqCst));
    });
    
    println!("\n=== Done ===");
}
