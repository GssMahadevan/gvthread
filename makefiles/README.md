# GVThread Makefile System

## Overview

Modular Makefile system with subsystem-specific targets and help documentation.

## Structure

```
gvthread/
├── Makefile              # Main entry point
├── makefiles/
│   ├── build.mk          # build-* targets
│   ├── test.mk           # test-* targets
│   ├── bench.mk          # bench-* targets (Go vs Rust comparison)
│   ├── perf.mk           # perf-* targets (CPU profiling)
│   └── clean.mk          # clean-* targets
└── scripts/
    ├── makehelper.py    # Help generator
    └── make_completion.bash  # Bash completion
```

## Quick Start

```bash
# Show all available targets
make

# Build everything
make build-all

# Run benchmarks
make bench-compare

# Check CPU usage
make perf-quick-compare
```

## Target Documentation

Each target has a `## description` comment:

```makefile
bench-go: $(GO_BENCH) ## Run Go goroutine benchmark
```

The `##` comment is parsed by `scripts/makehelper.py` to generate help.

## Filtering Help

```bash
# Filter by text
MF=bench make          # Show targets containing 'bench'

# Filter by category
MCAT=perf make         # Show Performance category only

# Flat list
MA=1 make              # Show all targets in flat sorted list

# Combine filters
MF=cpu MCAT=perf make  # CPU targets in Performance category
```

## Parameters

Benchmarks accept parameters via environment or make variables:

```bash
# Using make variables
make bench-compare N=50000 YIELDS=500 WORKERS=8

# Using environment
GVT_NUM_WORKERS=8 make bench-rust
GOMAXPROCS=8 make bench-go
```

### Default Parameters

| Variable | Default | Description |
|----------|---------|-------------|
| N | 20000 | Number of threads/goroutines |
| YIELDS | 300 | Yields per thread |
| SLEEP_MS | 100 | Sleep duration (ms) |
| WORKERS | 4 | Worker threads / GOMAXPROCS |
| DURATION | 10 | Duration for timed benchmarks |

## Bash Completion

```bash
# Enable completion
source scripts/make_completion.bash

# Then use tab completion
make bench-<TAB>
make perf-<TAB>
```

## Adding New Targets

1. Create/edit `makefiles/<subsystem>.mk`
2. Add target with `## description`:

```makefile
mysubsystem-newtarget: deps ## My new target description
	@echo "Doing something"
```

3. If new subsystem, add include to main `Makefile`:

```makefile
include $(MAKEFILES_DIR)/mysubsystem.mk
```

## Categories

Targets are auto-categorized by prefix:

| Prefix | Category |
|--------|----------|
| build-, compile- | Build |
| test-, check- | Test |
| bench- | Benchmark |
| perf-, profile-, flame- | Performance |
| config-, cfg- | Config |
| docker-, container- | Docker |
| clean-, distclean- | Clean |
| help- | Help |
| (other) | Other |