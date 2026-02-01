# GVThread Makefile System - Installation Guide

## Overview

This package contains:
1. **Makefile system** - Modular make targets with help documentation
2. **Config subsystem** - Layered configuration (compile-time + runtime)
3. **Benchmark/Perf tools** - Go vs Rust CPU comparison targets

## Files to Copy

```
makefile-system/
├── Makefile                          → Makefile
├── makefiles/
│   ├── README.md                     → makefiles/README.md
│   ├── build.mk                      → makefiles/build.mk
│   ├── test.mk                       → makefiles/test.mk
│   ├── bench.mk                      → makefiles/bench.mk
│   ├── perf.mk                       → makefiles/perf.mk
│   └── clean.mk                      → makefiles/clean.mk
├── scripts/
│   ├── makehelper.py            → scripts/makehelper.py
│   └── make_completion.bash          → scripts/make_completion.bash
├── crates/gvthread-runtime/
│   ├── build.rs                      → crates/gvthread-runtime/build.rs
│   └── src/config/
│       ├── README.md                 → crates/gvthread-runtime/src/config/README.md
│       ├── mod.rs                    → crates/gvthread-runtime/src/config/mod.rs
│       └── defaults.rs               → crates/gvthread-runtime/src/config/defaults.rs
└── cmd/playground/
    └── gvt_config.rs                 → cmd/playground/gvt_config.rs (example)
```

## Quick Install

```bash
cd ~/src/gvthread

# Copy everything
cp -r /path/to/makefile-system/* .

# Make scripts executable
chmod +x scripts/*.py scripts/*.bash

# Test
make help
```

## Update lib.rs for Config Module

In `crates/gvthread-runtime/src/lib.rs`, add:

```rust
pub mod config;
pub use config::SchedulerConfig;
```

Remove old `config.rs` if it exists:
```bash
rm crates/gvthread-runtime/src/config.rs 2>/dev/null
```

## Update Cargo.toml

In `crates/gvthread-runtime/Cargo.toml`, add feature:

```toml
[features]
default = []
custom-config = []
```

## Usage

### Makefile Help

```bash
make              # Show all targets
make help         # Same as above

# Filtering
MF=bench make     # Show targets containing 'bench'
MCAT=perf make    # Show Performance category
MA=1 make         # Flat list
```

### Benchmarks

```bash
# Compare Go vs Rust
make bench-compare

# With parameters
make bench-compare N=50000 WORKERS=8

# Quick CPU check
make perf-quick-compare
```

### Configuration

```bash
# Use defaults + env override
GVT_NUM_WORKERS=16 cargo run -p gvthread-basic

# Custom config file
echo 'pub const NUM_WORKERS: usize = 8;' > my_config.rs
GVT_CONFIG_RS=./my_config.rs cargo build
```

### Bash Completion

```bash
# Enable
source scripts/make_completion.bash

# Use
make bench-<TAB>
make perf-<TAB>
```

## Tracking CPU (Your Original Question)

To compare Go 5% vs Rust 10% CPU:

```bash
# Quick comparison
make perf-quick-compare N=20000 YIELDS=300 WORKERS=4

# With pidstat (more accurate)
make perf-cpu-compare N=20000 YIELDS=300 WORKERS=4

# Flame graph (find hotspots)
make perf-flame-rust N=20000 YIELDS=300
```