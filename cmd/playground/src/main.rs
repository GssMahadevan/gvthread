//! Playground for quick experiments
//!
//! Use this for testing new features or debugging.

use gvthread::{Runtime, spawn, yield_now, SchedulerConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn main() {
    println!("=== GVThread Playground ===\n");
    
    let config = SchedulerConfig::default()
        .num_workers(2)
        .debug_logging(true);
    
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        println!("Playground is ready!");
        println!("Add your experimental code here.\n");
        
        // Track completion
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = done.clone();
        
        // Example: spawn a simple GVThread
        spawn(move |_token| {
            println!("Hello from GVThread!");
            yield_now();
            println!("Back from yield!");
            yield_now();
            println!("Goodbye!");
            done_clone.store(true, Ordering::SeqCst);
        });
        
        // Wait for completion
        let start = std::time::Instant::now();
        while !done.load(Ordering::SeqCst) {
            if start.elapsed() > std::time::Duration::from_secs(5) {
                println!("Timeout!");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        
        println!("GVThread completed: {}", done.load(Ordering::SeqCst));
    });
    
    println!("\n=== Done ===");
}
