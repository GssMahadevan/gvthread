# makefiles/perf.mk - Performance profiling targets
#
# Tools for tracking CPU usage, profiling, and flame graphs
#
# Requirements:
#   - perf (linux-tools-generic)
#   - flamegraph (cargo install flamegraph)
#   - pidstat (sysstat package)

# Default parameters (inherit from bench.mk if included)
N         ?= 20000
YIELDS    ?= 300
SLEEP_MS  ?= 100
WORKERS   ?= 4
DURATION  ?= 10

# Paths
RUST_BENCH := target/release/gvthread-benchmark
GO_BENCH   := other/go/playground1/main
PERF_DATA  := /tmp/gvthread-perf
FLAME_DIR  := /tmp/gvthread-flames

.PHONY: perf-setup perf-rust perf-go perf-compare

perf-setup: ## Install performance tools
	@echo "Installing performance tools..."
	@which perf > /dev/null || (echo "Install: sudo apt install linux-tools-generic" && exit 1)
	@which pidstat > /dev/null || (echo "Install: sudo apt install sysstat" && exit 1)
	@which flamegraph > /dev/null || cargo install flamegraph
	@mkdir -p $(PERF_DATA) $(FLAME_DIR)
	@echo "Done. Tools ready."

# CPU usage tracking with pidstat
perf-cpu-rust: build-release ## Track Rust CPU usage with pidstat
	@echo "=== Rust CPU Usage (pidstat) ==="
	@mkdir -p $(PERF_DATA)
	@GVT_NUM_WORKERS=$(WORKERS) $(RUST_BENCH) --threads=$(N) --yields=$(YIELDS) --sleep=$(SLEEP_MS) --duration=$(DURATION) & \
		PID=$$!; \
		echo "PID: $$PID"; \
		pidstat -p $$PID 1 $(DURATION) | tee $(PERF_DATA)/rust_cpu.txt; \
		wait $$PID 2>/dev/null || true
	@echo ""
	@echo "Results: $(PERF_DATA)/rust_cpu.txt"

perf-cpu-go: build-go ## Track Go CPU usage with pidstat
	@echo "=== Go CPU Usage (pidstat) ==="
	@mkdir -p $(PERF_DATA)
	@GOMAXPROCS=$(WORKERS) $(GO_BENCH) -goroutines=$(N) -yields=$(YIELDS) -sleep=$(SLEEP_MS) -duration=$(DURATION) & \
		PID=$$!; \
		echo "PID: $$PID"; \
		pidstat -p $$PID 1 $(DURATION) | tee $(PERF_DATA)/go_cpu.txt; \
		wait $$PID 2>/dev/null || true
	@echo ""
	@echo "Results: $(PERF_DATA)/go_cpu.txt"

perf-cpu-compare: perf-cpu-go perf-cpu-rust ## Compare CPU usage between Go and Rust
	@echo ""
	@echo "=============================================="
	@echo "           CPU Usage Comparison"
	@echo "=============================================="
	@echo ""
	@echo "Go average CPU:"
	@awk '/Average:/ && /%CPU/ {getline; print "  " $$0}' $(PERF_DATA)/go_cpu.txt || echo "  (parse error)"
	@echo ""
	@echo "Rust average CPU:"
	@awk '/Average:/ && /%CPU/ {getline; print "  " $$0}' $(PERF_DATA)/rust_cpu.txt || echo "  (parse error)"

# Simple CPU monitor using top
perf-top-rust: build-release ## Monitor Rust with top (interactive)
	@echo "Starting Rust benchmark in background..."
	@GVT_NUM_WORKERS=$(WORKERS) $(RUST_BENCH) --threads=$(N) --yields=$(YIELDS) --sleep=$(SLEEP_MS) --duration=60 & \
		PID=$$!; \
		echo "PID: $$PID"; \
		echo "Press 'q' to quit top, benchmark will continue for 60s"; \
		sleep 1; \
		top -p $$PID; \
		kill $$PID 2>/dev/null || true

perf-top-go: build-go ## Monitor Go with top (interactive)
	@echo "Starting Go benchmark in background..."
	@GOMAXPROCS=$(WORKERS) $(GO_BENCH) -goroutines=$(N) -yields=$(YIELDS) -sleep=$(SLEEP_MS) -duration=60 & \
		PID=$$!; \
		echo "PID: $$PID"; \
		echo "Press 'q' to quit top, benchmark will continue for 60s"; \
		sleep 1; \
		top -p $$PID; \
		kill $$PID 2>/dev/null || true

# Flame graphs
perf-flame-rust: build-release ## Generate Rust flame graph
	@echo "=== Generating Rust Flame Graph ==="
	@mkdir -p $(FLAME_DIR)
	@echo "This requires sudo for perf access"
	sudo flamegraph -o $(FLAME_DIR)/rust.svg -- \
		$(RUST_BENCH) --threads=$(N) --yields=$(YIELDS) --sleep=$(SLEEP_MS)
	@echo ""
	@echo "Flame graph: $(FLAME_DIR)/rust.svg"
	@echo "Open in browser: firefox $(FLAME_DIR)/rust.svg"

perf-flame-go: build-go ## Generate Go flame graph (requires go tool pprof)
	@echo "=== Generating Go Flame Graph ==="
	@echo "Note: Go flame graphs require pprof integration in the Go binary"
	@echo "Add -cpuprofile flag support to the Go benchmark for full profiling"
	@mkdir -p $(FLAME_DIR)
	GOMAXPROCS=$(WORKERS) $(GO_BENCH) -goroutines=$(N) -yields=$(YIELDS) -sleep=$(SLEEP_MS) \
		-cpuprofile=$(FLAME_DIR)/go.prof 2>/dev/null || \
		echo "Go binary doesn't support -cpuprofile yet"

# Linux perf profiling
perf-record-rust: build-release ## Record Rust with Linux perf
	@echo "=== Recording Rust with perf ==="
	@mkdir -p $(PERF_DATA)
	sudo perf record -g -o $(PERF_DATA)/rust.perf.data -- \
		$(RUST_BENCH) --threads=$(N) --yields=$(YIELDS) --sleep=$(SLEEP_MS)
	@echo ""
	@echo "View with: sudo perf report -i $(PERF_DATA)/rust.perf.data"

perf-record-go: build-go ## Record Go with Linux perf
	@echo "=== Recording Go with perf ==="
	@mkdir -p $(PERF_DATA)
	sudo perf record -g -o $(PERF_DATA)/go.perf.data -- \
		env GOMAXPROCS=$(WORKERS) $(GO_BENCH) -goroutines=$(N) -yields=$(YIELDS) -sleep=$(SLEEP_MS)
	@echo ""
	@echo "View with: sudo perf report -i $(PERF_DATA)/go.perf.data"

perf-stat-rust: build-release ## Show Rust perf stats (cache, cycles, etc.)
	@echo "=== Rust Performance Statistics ==="
	sudo perf stat -d -- \
		$(RUST_BENCH) --threads=$(N) --yields=$(YIELDS) --sleep=$(SLEEP_MS)

perf-stat-go: build-go ## Show Go perf stats (cache, cycles, etc.)
	@echo "=== Go Performance Statistics ==="
	sudo perf stat -d -- \
		env GOMAXPROCS=$(WORKERS) $(GO_BENCH) -goroutines=$(N) -yields=$(YIELDS) -sleep=$(SLEEP_MS)

# Quick CPU check (no tools needed)
perf-quick-rust: build-release ## Quick Rust CPU check (5 samples)
	@echo "=== Quick Rust CPU Check ==="
	@GVT_NUM_WORKERS=$(WORKERS) $(RUST_BENCH) --threads=$(N) --yields=$(YIELDS) --sleep=$(SLEEP_MS) --duration=10 & \
		PID=$$!; \
		sleep 2; \
		for i in 1 2 3 4 5; do \
			ps -p $$PID -o %cpu,rss,vsz --no-headers 2>/dev/null && sleep 1; \
		done; \
		kill $$PID 2>/dev/null || true
	@echo "(columns: %CPU, RSS KB, VSZ KB)"

perf-quick-go: build-go ## Quick Go CPU check (5 samples)
	@echo "=== Quick Go CPU Check ==="
	@GOMAXPROCS=$(WORKERS) $(GO_BENCH) -goroutines=$(N) -yields=$(YIELDS) -sleep=$(SLEEP_MS) -duration=10 & \
		PID=$$!; \
		sleep 2; \
		for i in 1 2 3 4 5; do \
			ps -p $$PID -o %cpu,rss,vsz --no-headers 2>/dev/null && sleep 1; \
		done; \
		kill $$PID 2>/dev/null || true
	@echo "(columns: %CPU, RSS KB, VSZ KB)"

perf-quick-compare: ## Quick CPU comparison (Go vs Rust, 5 samples each)
	@echo "=============================================="
	@echo "         Quick CPU Comparison"
	@echo "=============================================="
	@echo "Parameters: N=$(N) YIELDS=$(YIELDS) WORKERS=$(WORKERS)"
	@echo ""
	@$(MAKE) -s perf-quick-go
	@echo ""
	@$(MAKE) -s perf-quick-rust