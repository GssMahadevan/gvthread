//! Stress test - many GVThreads
//!
//! Tests spawning and running large numbers of GVThreads.

use gvthread::{Runtime, spawn, yield_now, SchedulerConfig};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    println!("=== GVThread Stress Test ===\n");
    
    let num_gvthreads: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    
    println!("Spawning {} GVThreads...", num_gvthreads);
    
    let config = SchedulerConfig::default()
        .num_workers(8)
        .max_gvthreads(num_gvthreads + 1000);
    
    let mut runtime = Runtime::new(config);
    
    let completed = Arc::new(AtomicU64::new(0));
    
    runtime.block_on(|| {
        let start = Instant::now();
        
        // Spawn many GVThreads
        for i in 0..num_gvthreads {
            let completed = completed.clone();
            
            spawn(move |_token| {
                // Do a little work
                for _ in 0..10 {
                    yield_now();
                }
                
                completed.fetch_add(1, Ordering::Relaxed);
            });
            
            // Progress indicator
            if (i + 1) % 1000 == 0 {
                print!("\rSpawned: {}/{}", i + 1, num_gvthreads);
            }
        }
        
        let spawn_time = start.elapsed();
        println!("\n\nSpawn time: {:?}", spawn_time);
        println!("Spawn rate: {:.0} GVThreads/sec", 
            num_gvthreads as f64 / spawn_time.as_secs_f64());
        
        // Wait for completion
        println!("\nWaiting for completion...");
        let run_start = Instant::now();
        
        loop {
            let done = completed.load(Ordering::Relaxed) as usize;
            if done >= num_gvthreads {
                break;
            }
            
            if run_start.elapsed().as_secs() > 30 {
                println!("Timeout! Only {}/{} completed", done, num_gvthreads);
                break;
            }
            
            print!("\rCompleted: {}/{}", done, num_gvthreads);
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        
        let total_time = start.elapsed();
        let run_time = run_start.elapsed();
        
        println!("\n\n=== Results ===");
        println!("Total GVThreads: {}", num_gvthreads);
        println!("Completed:       {}", completed.load(Ordering::Relaxed));
        println!("Spawn time:      {:?}", spawn_time);
        println!("Run time:        {:?}", run_time);
        println!("Total time:      {:?}", total_time);
        println!("Throughput:      {:.0} GVThreads/sec", 
            num_gvthreads as f64 / total_time.as_secs_f64());
    });
    
    println!("\n=== Stress Test Complete ===");
}
