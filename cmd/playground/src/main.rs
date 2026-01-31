//! Context switch test with configurable parameters
//!
//! # Environment Variables
//!
//! - `GVT_WORKERS=<n>` - Number of worker threads (default: 4)
//! - `GVT_LOW_WORKERS=<n>` - Low-priority workers (default: 1)
//! - `GVT_GVTHREADS=<n>` - Number of GVThreads (default: 10)
//! - `GVT_YIELDS=<n>` - Yields per GVThread (default: 5)
//! - `GVT_SLEEP_MS=<n>` - Sleep time in ms (0 = no sleep, default: 0)
//! - `GVT_DEBUG=1` - Enable debug logging
//! - `GVT_LOG_LEVEL=debug` - Set log level
//! - `GVT_KPRINT_TIME=1` - Show timestamps

use gvthread::{Runtime, spawn, yield_now, sleep_ms, SchedulerConfig, GVThreadId};
use gvthread::{env_get, env_get_bool, kinfo, kerror};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    println!("=== Context Switch Test ===\n");
    
    // Read configuration from environment
    let num_workers: usize = env_get("GVT_WORKERS", 4);
    let num_low_workers: usize = env_get("GVT_LOW_WORKERS", 1);
    let total_gvthreads: usize = env_get("GVT_GVTHREADS", 10);
    let yields_per_gvthread: usize = env_get("GVT_YIELDS", 5);
    let sleep_time_ms: u64 = env_get("GVT_SLEEP_MS", 0);
    let debug_logging: bool = env_get_bool("GVT_DEBUG", false);
    
    println!("Configuration:");
    println!("  Workers: {} (low-priority: {})", num_workers, num_low_workers);
    println!("  GVThreads: {}, yields/thread: {}", total_gvthreads, yields_per_gvthread);
    if sleep_time_ms > 0 {
        println!("  Sleep per yield: {}ms (cooperative)", sleep_time_ms);
    }
    println!("  Debug: {}", debug_logging);
    println!();
    
    let config = SchedulerConfig::default()
        .num_workers(num_workers)
        .num_low_priority_workers(num_low_workers)
        .debug_logging(debug_logging);
    
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        let spawned = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let total_yields = Arc::new(AtomicUsize::new(0));
        
        kinfo!("Spawning {} GVThreads, each yielding {} times", 
               total_gvthreads, yields_per_gvthread);
        
        let mut spawn_ids: Vec<GVThreadId> = Vec::with_capacity(total_gvthreads);
        
        for _i in 0..total_gvthreads {
            let st = started.clone();
            let c = completed.clone();
            let ty = total_yields.clone();
            let yields = yields_per_gvthread;
            let sleep_duration_ms = sleep_time_ms;
            
            let id = spawn(move |_token| {
                st.fetch_add(1, Ordering::SeqCst);
                for _j in 0..yields {
                    ty.fetch_add(1, Ordering::SeqCst);
                    if sleep_duration_ms > 0 {
                        // Cooperative sleep - yields GVThread, doesn't block worker
                        sleep_ms(sleep_duration_ms);
                    } else {
                        yield_now();
                    }
                }
                c.fetch_add(1, Ordering::SeqCst);
            });
            
            spawn_ids.push(id);
            spawned.fetch_add(1, Ordering::SeqCst);
        }
        
        let spawned_count = spawned.load(Ordering::SeqCst);
        kinfo!("Spawned {} GVThreads (IDs 0..{})", spawned_count, 
               spawn_ids.last().map(|id| id.as_u32()).unwrap_or(0));
        
        // Wait for completion
        let start = std::time::Instant::now();
        let timeout_secs = if sleep_time_ms > 0 {
            // Allow more time when sleeping
            60 + (total_gvthreads as u64 * yields_per_gvthread as u64 * sleep_time_ms / 1000)
        } else {
            30
        };
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let mut last_completed = 0;
        let mut stall_count = 0;
        
        while completed.load(Ordering::SeqCst) < total_gvthreads {
            if start.elapsed() > timeout {
                kerror!("TIMEOUT after {}s!", timeout_secs);
                break;
            }
            
            std::thread::sleep(std::time::Duration::from_millis(100));
            
            let current_completed = completed.load(Ordering::SeqCst);
            if current_completed == last_completed {
                stall_count += 1;
                // Allow longer stalls when using sleep
                let stall_limit = if sleep_time_ms > 0 { 50 } else { 20 };
                if stall_count >= stall_limit {
                    kerror!("STALL detected - no progress for {}s", stall_limit / 10);
                    kerror!("  spawned={}, started={}, completed={}, yields={}",
                           spawned.load(Ordering::SeqCst),
                           started.load(Ordering::SeqCst),
                           current_completed,
                           total_yields.load(Ordering::SeqCst));
                    break;
                }
            } else {
                stall_count = 0;
                last_completed = current_completed;
            }
        }
        
        let sp = spawned.load(Ordering::SeqCst);
        let st = started.load(Ordering::SeqCst);
        let c = completed.load(Ordering::SeqCst);
        let y = total_yields.load(Ordering::SeqCst);
        let expected_yields = total_gvthreads * yields_per_gvthread;
        
        println!("\n=== Results ===");
        println!("Spawned:   {}/{}", sp, total_gvthreads);
        println!("Started:   {}/{}", st, total_gvthreads);
        println!("Completed: {}/{}", c, total_gvthreads);
        println!("Yields:    {} (expected: {})", y, expected_yields);
        println!("Time:      {:?}", start.elapsed());
        
        if c == total_gvthreads && y == expected_yields {
            println!("\n*** SUCCESS ***");
        } else {
            println!("\n*** FAILURE ***");
            if st < sp {
                println!("  -> {} GVThreads never started!", sp - st);
            }
            if c < st {
                println!("  -> {} GVThreads started but didn't complete!", st - c);
            }
        }
    });
    
    println!("\n=== Done ===");
}