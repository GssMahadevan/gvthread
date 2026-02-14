//! Benchmark suite for GVThread
//!
//! Measures various performance metrics.

use gvthread::{Runtime, spawn, yield_now, channel, SchedulerConfig};
use std::time::Instant;

fn main() {
    println!("=== GVThread Benchmarks ===\n");
    
    let config = SchedulerConfig::default()
        .num_workers(4);
    
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        bench_spawn();
        bench_yield();
        bench_channel();
    });
    
    println!("\n=== Benchmarks Complete ===");
}

fn bench_spawn() {
    println!("Benchmark: Spawn");
    println!("{}", "─".repeat(40));
    
    let iterations = 10_000;
    
    let start = Instant::now();
    for _ in 0..iterations {
        spawn(|_| {});
    }
    let elapsed = start.elapsed();
    
    let per_spawn = elapsed.as_nanos() as f64 / iterations as f64;
    println!("  Iterations:  {}", iterations);
    println!("  Total time:  {:?}", elapsed);
    println!("  Per spawn:   {:.1} ns", per_spawn);
    println!("  Rate:        {:.0}/sec\n", iterations as f64 / elapsed.as_secs_f64());
}

fn bench_yield() {
    println!("Benchmark: Yield");
    println!("{}", "─".repeat(40));
    
    let iterations = 100_000;
    
    let start = Instant::now();
    for _ in 0..iterations {
        yield_now();
    }
    let elapsed = start.elapsed();
    
    let per_yield = elapsed.as_nanos() as f64 / iterations as f64;
    println!("  Iterations:  {}", iterations);
    println!("  Total time:  {:?}", elapsed);
    println!("  Per yield:   {:.1} ns", per_yield);
    println!("  Rate:        {:.0}/sec\n", iterations as f64 / elapsed.as_secs_f64());
}

fn bench_channel() {
    println!("Benchmark: Channel (try_send/try_recv)");
    println!("{}", "─".repeat(40));
    
    let (tx, rx) = channel::<u64>(1024);
    let iterations = 100_000;
    
    let start = Instant::now();
    for i in 0..iterations {
        let _ = tx.try_send(i);
        let _ = rx.try_recv();
    }
    let elapsed = start.elapsed();
    
    let per_op = elapsed.as_nanos() as f64 / (iterations * 2) as f64;
    println!("  Iterations:  {} (send+recv pairs)", iterations);
    println!("  Total time:  {:?}", elapsed);
    println!("  Per op:      {:.1} ns", per_op);
    println!("  Rate:        {:.0} ops/sec\n", (iterations * 2) as f64 / elapsed.as_secs_f64());
}
