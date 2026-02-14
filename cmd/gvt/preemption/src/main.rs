//! Preemption test example
//!
//! Tests that CPU-bound GVThreads are preempted correctly.
//! This demonstrates both cooperative (safepoint) and forced (SIGURG) preemption.

use gvthread::{Runtime, spawn, yield_now, safepoint, SchedulerConfig};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() {
    println!("=== GVThread Preemption Test ===\n");
    
    let config = SchedulerConfig::default()
        .num_workers(2)
        .time_slice(Duration::from_millis(10))
        .enable_forced_preempt(true)
        .debug_logging(true);
    
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        println!("Test 1: Cooperative preemption (with safepoints)");
        println!("{}", "─".repeat(50));
        
        let counter1 = Arc::new(AtomicU64::new(0));
        let counter1_clone = counter1.clone();
        
        // GVThread with safepoints - should yield cooperatively
        spawn(move |_token| {
            println!("[Cooperative] Starting CPU-bound loop with safepoints...");
            let start = Instant::now();
            
            loop {
                counter1_clone.fetch_add(1, Ordering::Relaxed);
                
                // Safepoint: allows cooperative preemption
                safepoint!();
                
                if start.elapsed() > Duration::from_millis(50) {
                    break;
                }
            }
            
            println!("[Cooperative] Finished! Iterations: {}", 
                counter1_clone.load(Ordering::Relaxed));
        });
        
        println!("\nTest 2: Forced preemption (no safepoints)");
        println!("{}", "─".repeat(50));
        
        let counter2 = Arc::new(AtomicU64::new(0));
        let counter2_clone = counter2.clone();
        
        // GVThread WITHOUT safepoints - should be preempted via SIGURG
        spawn(move |_token| {
            println!("[Forced] Starting CPU-bound loop WITHOUT safepoints...");
            println!("[Forced] This should be forcibly preempted after time slice!");
            let start = Instant::now();
            
            loop {
                counter2_clone.fetch_add(1, Ordering::Relaxed);
                
                // NO safepoint! This is a "bad citizen" loop
                // Timer thread should detect this and send SIGURG
                
                if start.elapsed() > Duration::from_millis(50) {
                    break;
                }
            }
            
            println!("[Forced] Finished! Iterations: {}", 
                counter2_clone.load(Ordering::Relaxed));
        });
        
        println!("\nTest 3: Mixed workload");
        println!("{}", "─".repeat(50));
        
        // Well-behaved I/O-like GVThread
        spawn(|_token| {
            for i in 0..5 {
                println!("[I/O-like] Step {} - yielding...", i);
                yield_now();
            }
            println!("[I/O-like] Done!");
        });
        
        // Give time for all GVThreads to run
        std::thread::sleep(Duration::from_millis(200));
        
        println!("\nFinal counters:");
        println!("  Cooperative: {}", counter1.load(Ordering::Relaxed));
        println!("  Forced:      {}", counter2.load(Ordering::Relaxed));
    });
    
    println!("\n=== Preemption Test Complete ===");
}
