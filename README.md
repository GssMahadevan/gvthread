# gvthread

**Green Virtual Thread Scheduler for Rust**

*Named in memory of Gorti Viswanadham*

A high-performance userspace threading library that provides lightweight green threads (GVThreads) with preemptive scheduling.

## Features

- **Lightweight**: 16MB virtual address space per GVThread, physical memory on-demand
- **Fast Context Switch**: ~20ns voluntary yield via hand-written assembly  
- **Preemption**: Cooperative (safepoints) + Forced (SIGURG) for CPU-bound code
- **Priority Scheduling**: Critical, High, Normal, Low with bitmap-based O(1) lookup
- **Synchronization**: Channels, Mutex, Sleep primitives
- **Cancellation**: Result-based cancellation with token propagation

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        User Code                                │
│                spawn(), yield_now(), channel                    │
└─────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Scheduler                                │
│          Bitmap scan, priority, worker coordination             │
└─────────────────────────────────────────────────────────────────┘
                             │
         ┌───────────────────┼───────────────────┐
         ▼                   ▼                   ▼
   ┌───────────┐      ┌───────────┐      ┌───────────┐
   │  Worker   │      │  Worker   │      │   Timer   │
   │  Thread   │      │  Thread   │      │   Thread  │
   └───────────┘      └───────────┘      └───────────┘
         │                   │                   │
         └───────────────────┼───────────────────┘
                             ▼
   ┌─────────────────────────────────────────────────────────────┐
   │                    Memory Region                            │
   │      16MB slots × N GVThreads, guard pages, mmap            │
   └─────────────────────────────────────────────────────────────┘
```

## Quick Start

```rust
use gvthread::{Runtime, spawn, yield_now, channel};

fn main() {
    let mut runtime = Runtime::new(Default::default());
    
    runtime.block_on(|| {
        // Spawn GVThreads
        spawn(|token| {
            println!("Hello from GVThread!");
            yield_now();
            println!("Back again!");
        });
        
        // Channel communication
        let (tx, rx) = channel(10);
        
        spawn(move |_| {
            for i in 0..5 {
                tx.try_send(i).unwrap();
            }
        });
        
        spawn(move |_| {
            while let Ok(val) = rx.try_recv() {
                println!("Received: {}", val);
            }
        });
    });
}
```

## Workspace Structure

```
gvthread/
├── crates/
│   ├── gvthread-core/      # Platform-agnostic core types
│   ├── gvthread-runtime/   # Platform-specific runtime
│   └── gvthread/           # Main facade crate
├── cmd/                    # Example binaries (Go-style)
│   ├── basic/              # Basic spawn/yield demo
│   ├── channel/            # Channel communication
│   ├── preemption/         # Preemption tests
│   ├── stress/             # Scale testing
│   ├── benchmark/          # Performance benchmarks
│   └── playground/         # Quick experiments
└── tests/                  # Integration tests
```

## Building

```bash
# Build all crates
cargo build --workspace

# Run an example
cargo run -p gvthread-basic

# Run stress test with 100k GVThreads
cargo run -p gvthread-stress --release -- 100000

# Run benchmarks
cargo run -p gvthread-benchmark --release
```

## Platform Support

| Platform | Architecture | Status |
|----------|--------------|--------|
| Linux    | x86_64       | ✅ Primary |
| Linux    | aarch64      | 🚧 Planned |
| macOS    | x86_64       | 🚧 Planned |
| macOS    | aarch64      | 🚧 Planned |
| Windows  | x86_64       | 🚧 Planned |

## Preemption

GVThreads can be preempted in two ways:

1. **Cooperative (Safepoints)**: Insert `safepoint!()` in loops. The scheduler sets a flag that safepoints check.

2. **Forced (SIGURG)**: For CPU-bound code without safepoints, the timer thread sends SIGURG after the time slice expires. The signal handler saves all registers and redirects execution to the scheduler.

```rust
// Good citizen - uses safepoints
spawn(|_| {
    loop {
        safepoint!();  // Allows cooperative preemption
        do_work();
    }
});

// Bad citizen - no safepoints, will be SIGURG'd
spawn(|_| {
    loop {
        do_cpu_intensive_work();  // Will be forcibly preempted
    }
});
```

## Memory Layout

Each GVThread gets a 16MB virtual address slot:

```
┌────────────────────────────────────────┐ ← Slot base
│ Metadata (4KB)                         │
│   - Flags, state, priority             │
│   - Saved registers                    │
├────────────────────────────────────────┤
│                                        │
│ Stack (grows down)                     │
│                                        │
│                                        │
├────────────────────────────────────────┤
│ Guard Page (4KB) - PROT_NONE           │
└────────────────────────────────────────┘ ← Slot end (16MB)
```

Physical memory is only allocated on demand via page faults.

## License

MIT

## Acknowledgments

Named in memory of **Gorti Viswanadham** - the "GV" in GVThread.
