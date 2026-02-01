# makefiles/bench.mk - Benchmark targets
#
# Benchmark commands for comparing Go vs Rust performance
#
# Usage:
#   make bench-all              - Run both Go and Rust benchmarks
#   make bench-compare          - Run and compare side-by-side
#   make bench-rust N=50000     - Run Rust with 50k gvthreads
#   make bench-go GOMAXPROCS=8  - Run Go with 8 procs

# Default benchmark parameters
N         ?= 20000    # Number of goroutines/gvthreads
YIELDS    ?= 300      # Yields per thread
SLEEP_MS  ?= 100      # Sleep duration in ms
WORKERS   ?= 4        # Worker threads (Rust) / GOMAXPROCS (Go)
DURATION  ?= 10       # Duration for timed benchmarks (seconds)

# Paths
GO_BENCH     := other/go/playground1/main
RUST_BENCH   := target/release/gvthread-benchmark
RUST_STRESS  := target/release/gvthread-stress
COMPARE_SCRIPT := $(SCRIPTS_DIR)/bench_compare.py

# Ensure binaries are built
$(GO_BENCH): build-go
$(RUST_BENCH): build-release
$(RUST_STRESS): build-release

.PHONY: bench-all bench-go bench-rust bench-compare bench-stress

bench-all: bench-go bench-rust ## Run both Go and Rust benchmarks

bench-go: $(GO_BENCH) ## Run Go goroutine benchmark
	@echo "=== Go Benchmark ==="
	@echo "GOMAXPROCS=$(WORKERS) goroutines=$(N) yields=$(YIELDS) sleep=$(SLEEP_MS)ms"
	@echo ""
	GOMAXPROCS=$(WORKERS) $(GO_BENCH) -goroutines=$(N) -yields=$(YIELDS) -sleep=$(SLEEP_MS)

bench-rust: $(RUST_BENCH) ## Run Rust GVThread benchmark
	@echo "=== Rust Benchmark ==="
	@echo "workers=$(WORKERS) gvthreads=$(N) yields=$(YIELDS) sleep=$(SLEEP_MS)ms"
	@echo ""
	GVT_NUM_WORKERS=$(WORKERS) $(RUST_BENCH) --threads=$(N) --yields=$(YIELDS) --sleep=$(SLEEP_MS)

bench-stress: $(RUST_STRESS) ## Run Rust stress test with N gvthreads
	@echo "=== Rust Stress Test ==="
	@echo "gvthreads=$(N)"
	GVT_NUM_WORKERS=$(WORKERS) $(RUST_STRESS) $(N)

bench-compare: build-release build-go ## Run Go and Rust benchmarks and compare
	@echo "=============================================="
	@echo "        GVThread vs Goroutine Comparison"
	@echo "=============================================="
	@echo ""
	@echo "Parameters:"
	@echo "  Threads/Goroutines: $(N)"
	@echo "  Yields per thread:  $(YIELDS)"
	@echo "  Sleep duration:     $(SLEEP_MS)ms"
	@echo "  Workers/GOMAXPROCS: $(WORKERS)"
	@echo ""
	@echo "----------------------------------------------"
	@echo "Go:"
	@echo "----------------------------------------------"
	@GOMAXPROCS=$(WORKERS) $(GO_BENCH) -goroutines=$(N) -yields=$(YIELDS) -sleep=$(SLEEP_MS) 2>&1 | tee /tmp/bench_go.txt
	@echo ""
	@echo "----------------------------------------------"
	@echo "Rust:"
	@echo "----------------------------------------------"
	@GVT_NUM_WORKERS=$(WORKERS) $(RUST_BENCH) --threads=$(N) --yields=$(YIELDS) --sleep=$(SLEEP_MS) 2>&1 | tee /tmp/bench_rust.txt
	@echo ""
	@echo "=============================================="
	@echo "Results saved to /tmp/bench_go.txt and /tmp/bench_rust.txt"

# CPU monitoring benchmarks
bench-go-cpu: $(GO_BENCH) ## Run Go benchmark with CPU monitoring
	@echo "=== Go Benchmark with CPU Monitoring ==="
	@echo "Run 'top -p <pid>' or 'htop' in another terminal"
	@echo "Press Ctrl+C to stop"
	@echo ""
	GOMAXPROCS=$(WORKERS) $(GO_BENCH) -goroutines=$(N) -yields=$(YIELDS) -sleep=$(SLEEP_MS) -duration=$(DURATION)

bench-rust-cpu: $(RUST_BENCH) ## Run Rust benchmark with CPU monitoring
	@echo "=== Rust Benchmark with CPU Monitoring ==="
	@echo "Run 'top -p <pid>' or 'htop' in another terminal"
	@echo "Press Ctrl+C to stop"
	@echo ""
	GVT_NUM_WORKERS=$(WORKERS) $(RUST_BENCH) --threads=$(N) --yields=$(YIELDS) --sleep=$(SLEEP_MS) --duration=$(DURATION)

# Quick benchmarks with different scales
bench-quick: ## Quick benchmark: 1k threads, 100 yields
	@$(MAKE) bench-compare N=1000 YIELDS=100 SLEEP_MS=50

bench-medium: ## Medium benchmark: 10k threads, 200 yields
	@$(MAKE) bench-compare N=10000 YIELDS=200 SLEEP_MS=100

bench-large: ## Large benchmark: 50k threads, 300 yields
	@$(MAKE) bench-compare N=50000 YIELDS=300 SLEEP_MS=100

bench-huge: ## Huge benchmark: 100k threads, 500 yields
	@$(MAKE) bench-compare N=100000 YIELDS=500 SLEEP_MS=100

# Parameter sweep
bench-sweep-workers: ## Sweep worker count (1,2,4,8,16)
	@for w in 1 2 4 8 16; do \
		echo ""; \
		echo "========== Workers: $$w =========="; \
		$(MAKE) -s bench-compare WORKERS=$$w N=$(N) YIELDS=$(YIELDS); \
	done

bench-sweep-threads: ## Sweep thread count (1k,5k,10k,20k,50k)
	@for n in 1000 5000 10000 20000 50000; do \
		echo ""; \
		echo "========== Threads: $$n =========="; \
		$(MAKE) -s bench-compare N=$$n WORKERS=$(WORKERS) YIELDS=$(YIELDS); \
	done