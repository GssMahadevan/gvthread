//! Channel communication example
//!
//! Demonstrates inter-GVThread communication using channels.

use gvthread::{Runtime, spawn, channel, SchedulerConfig};

fn main() {
    println!("=== GVThread Channel Example ===\n");
    
    let config = SchedulerConfig::default()
        .num_workers(4)
        .debug_logging(true);
    
    let mut runtime = Runtime::new(config);
    
    runtime.block_on(|| {
        // Create a bounded channel
        let (tx, rx) = channel::<i32>(10);
        
        println!("Created channel with capacity 10\n");
        
        // Producer GVThread
        let tx_clone = tx.clone();
        spawn(move |_token| {
            println!("[Producer] Starting...");
            
            for i in 1..=5 {
                match tx_clone.try_send(i) {
                    Ok(()) => println!("[Producer] Sent: {}", i),
                    Err(e) => println!("[Producer] Failed to send {}: {:?}", i, e),
                }
            }
            
            println!("[Producer] Done!");
        });
        
        // Consumer GVThread
        spawn(move |_token| {
            println!("[Consumer] Starting...");
            
            // Give producer time to send
            std::thread::sleep(std::time::Duration::from_millis(10));
            
            loop {
                match rx.try_recv() {
                    Ok(val) => println!("[Consumer] Received: {}", val),
                    Err(_) => {
                        println!("[Consumer] Channel empty, done!");
                        break;
                    }
                }
            }
        });
        
        // Give time for GVThreads to run
        std::thread::sleep(std::time::Duration::from_millis(100));
    });
    
    println!("\n=== Example Complete ===");
}
