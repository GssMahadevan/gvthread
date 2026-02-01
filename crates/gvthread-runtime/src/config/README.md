# GVThread Configuration Subsystem

## Overview

GVThread uses a layered configuration system inspired by FreeRTOS's `FreeRTOSConfig.h` and Linux's Kconfig:

```
┌─────────────────────────────────────────────────────────────┐
│                 Configuration Priority                       │
│                 (highest wins)                               │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  3. Environment Variables     ◄── Runtime override          │
│     GVT_NUM_WORKERS=16           (always available)         │
│                                                             │
│  2. User Config File          ◄── Compile-time override     │
│     gvt_config.rs                (feature = "custom-config")│
│                                                             │
│  1. Library Defaults          ◄── Built into gvthread       │
│     (always present)                                        │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```
## How it works
```txt
User's gvt_config.rs          Library defaults (build.rs)
(partial, only overrides)            (all values)
         │                              │
         └──────────┬───────────────────┘
                    ▼
              build.rs merges
                    │
                    ▼
          OUT_DIR/gvt_merged_config.rs
                    │
                    ▼
           defaults.rs includes it
                    │
                    ▼
        SchedulerConfig::from_env()
                    │
                    ▼
           Environment variables
            (runtime override)
```
## Quick Start

### Using Defaults Only

```rust
use gvthread::SchedulerConfig;

let config = SchedulerConfig::from_env();  // Defaults + env overrides
```

### Runtime Override (No Recompile)

```bash
GVT_NUM_WORKERS=16 GVT_TIME_SLICE_MS=5 ./my-app
```

### Compile-Time Custom Config

1. Create `gvt_config.rs` in your project:

```rust
// my-project/gvt_config.rs
// Only specify values you want to change!

pub const NUM_WORKERS: usize = 8;
pub const TIME_SLICE_MS: u64 = 5;
pub const MAX_GVTHREADS: usize = 500_000;
```

2. Build with the feature and env var:

```bash
GVT_CONFIG_RS=./gvt_config.rs cargo build --features custom-config
```

3. Values not specified in your file use library defaults.

## Configuration Parameters

| Parameter | Type | Default | Env Var | Description |
|-----------|------|---------|---------|-------------|
| `NUM_WORKERS` | usize | 4 | `GVT_NUM_WORKERS` | Worker threads for running GVThreads |
| `NUM_LOW_PRIORITY_WORKERS` | usize | 1 | `GVT_NUM_LOW_PRIORITY_WORKERS` | Workers dedicated to low priority |
| `MAX_GVTHREADS` | usize | 1048576 | `GVT_MAX_GVTHREADS` | Maximum concurrent GVThreads |
| `TIME_SLICE_MS` | u64 | 10 | `GVT_TIME_SLICE_MS` | Time slice before preemption hint |
| `GRACE_PERIOD_MS` | u64 | 1 | `GVT_GRACE_PERIOD_MS` | Grace period before forced preemption |
| `TIMER_INTERVAL_MS` | u64 | 1 | `GVT_TIMER_INTERVAL_MS` | Timer thread check interval |
| `TIMER_MAX_SLEEP_MS` | u64 | 10 | `GVT_TIMER_MAX_MS` | Max timer thread sleep |
| `ENABLE_FORCED_PREEMPT` | bool | true | `GVT_ENABLE_FORCED_PREEMPT` | Enable SIGURG preemption |
| `DEBUG_LOGGING` | bool | false | `GVT_DEBUG` | Enable debug output |
| `STACK_SIZE` | usize | 16777216 | `GVT_STACK_SIZE` | Virtual stack size (16MB) |
| `LOCAL_QUEUE_CAPACITY` | usize | 256 | `GVT_LOCAL_QUEUE_CAPACITY` | Per-worker queue size |
| `GLOBAL_QUEUE_CAPACITY` | usize | 65536 | `GVT_GLOBAL_QUEUE_CAPACITY` | Global queue size |
| `IDLE_SPINS` | u32 | 10 | `GVT_IDLE_SPINS` | Spins before parking |
| `PARK_TIMEOUT_MS` | u64 | 100 | `GVT_PARK_TIMEOUT_MS` | Worker park timeout |

## How It Works

### Build Process

```
┌──────────────────────────────────────────────────────────────┐
│                    Cargo Build                               │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  1. gvthread-runtime/build.rs runs                           │
│                                                              │
│  2. Reads GVT_CONFIG_RS env var (if set)                     │
│     └── Parses user's gvt_config.rs                          │
│                                                              │
│  3. Merges with library defaults                             │
│     └── User values override defaults                        │
│     └── Missing values keep defaults                         │
│                                                              │
│  4. Generates OUT_DIR/merged_config.rs                       │
│     └── All parameters defined                               │
│                                                              │
│  5. src/config/defaults.rs includes generated file           │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

### Runtime Flow

```rust
// SchedulerConfig::from_env() does:
Self {
    num_workers: env_get("GVT_NUM_WORKERS", defaults::NUM_WORKERS),
    time_slice: Duration::from_millis(
        env_get("GVT_TIME_SLICE_MS", defaults::TIME_SLICE_MS)
    ),
    // ... etc
}
```

## User's gvt_config.rs Format

Simple Rust constants. Only include what you want to override:

```rust
// Minimal example - just change workers
pub const NUM_WORKERS: usize = 16;
```

```rust
// Full example - all available parameters
pub const NUM_WORKERS: usize = 8;
pub const NUM_LOW_PRIORITY_WORKERS: usize = 2;
pub const MAX_GVTHREADS: usize = 500_000;
pub const TIME_SLICE_MS: u64 = 5;
pub const GRACE_PERIOD_MS: u64 = 1;
pub const TIMER_INTERVAL_MS: u64 = 1;
pub const TIMER_MAX_SLEEP_MS: u64 = 10;
pub const ENABLE_FORCED_PREEMPT: bool = true;
pub const DEBUG_LOGGING: bool = false;
pub const STACK_SIZE: usize = 16 * 1024 * 1024;
pub const LOCAL_QUEUE_CAPACITY: usize = 256;
pub const GLOBAL_QUEUE_CAPACITY: usize = 65536;
pub const IDLE_SPINS: u32 = 10;
pub const PARK_TIMEOUT_MS: u64 = 100;
```

## Important Notes

1. **No build.rs needed** in user's project - gvthread's build.rs handles everything

2. **Partial configs OK** - only specify what you need to change

3. **Env vars always work** - override any value at runtime without recompile

4. **Feature gating** - `custom-config` feature only needed for compile-time config file

5. **Type safety** - invalid values caught at compile time (config file) or runtime (env vars)

## Files

```
crates/gvthread-runtime/src/config/
├── README.md       # This file
├── mod.rs          # SchedulerConfig struct and from_env()
└── defaults.rs     # includes!(OUT_DIR/merged_config.rs)

crates/gvthread-runtime/build.rs  # Merge logic
```

## Examples

### High-Throughput Server

```rust
// gvt_config.rs
pub const NUM_WORKERS: usize = 32;
pub const MAX_GVTHREADS: usize = 2_000_000;
pub const LOCAL_QUEUE_CAPACITY: usize = 1024;
pub const TIME_SLICE_MS: u64 = 1;  // Quick preemption
```

### Low-Latency Application

```rust
// gvt_config.rs
pub const NUM_WORKERS: usize = 4;
pub const TIME_SLICE_MS: u64 = 50;  // Less preemption overhead
pub const ENABLE_FORCED_PREEMPT: bool = false;  // Cooperative only
pub const IDLE_SPINS: u32 = 100;  // More spinning, less parking
```

### Development/Debug

```rust
// gvt_config.rs
pub const NUM_WORKERS: usize = 2;
pub const DEBUG_LOGGING: bool = true;
pub const MAX_GVTHREADS: usize = 1000;  // Catch leaks early
```