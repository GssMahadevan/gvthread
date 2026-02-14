# makefiles/itest.mk — Integration test & benchmark targets
# Included by root Makefile.
# Target naming: itest-{action}-{test_type}-{server}

PYTHON       ?= python3
TEST_RUNNER  := $(PYTHON) itests/test-runner.py
PORT_BASE    ?= 9100
WRK_THREADS  ?= 2
THRESHOLD    ?= 5.0

# ─── Discovery ──────────────────────────────────────────────

itest-list: ## List all discovered test plugins and servers
	@$(TEST_RUNNER) --list

itest-check-deps: ## Verify benchmark dependencies (wrk, Go, cargo, python)
	@echo "Checking dependencies..."
	@command -v wrk  >/dev/null 2>&1 && echo "  wrk:    OK" || echo "  wrk:    MISSING (install for HTTP benchmarks)"
	@command -v go   >/dev/null 2>&1 && echo "  go:     OK ($$(go version 2>/dev/null | head -c 30))" || echo "  go:     MISSING"
	@command -v cargo >/dev/null 2>&1 && echo "  cargo:  OK ($$(cargo --version 2>/dev/null))" || echo "  cargo:  MISSING"
	@$(PYTHON) --version >/dev/null 2>&1 && echo "  python: OK ($$($(PYTHON) --version 2>/dev/null))" || echo "  python: MISSING"

# ─── Smoke Tests ────────────────────────────────────────────

itest-smoke: ## Run KSVC smoke tests (33 correctness tests)
	@$(TEST_RUNNER) smoke

# ─── Echo Benchmarks ────────────────────────────────────────

itest-echo-all: ## Run echo benchmarks for all servers (Go, Tokio, KSVC)
	@$(TEST_RUNNER) echo --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

itest-echo-ksvc: ## Run echo benchmark for KSVC only
	@$(TEST_RUNNER) echo --servers ksvc --port-base $(PORT_BASE)

itest-echo-tokio: ## Run echo benchmark for Tokio only
	@$(TEST_RUNNER) echo --servers tokio --port-base $(PORT_BASE)

itest-echo-go: ## Run echo benchmark for Go only
	@$(TEST_RUNNER) echo --servers go --port-base $(PORT_BASE)

itest-echo-rust: ## Run echo benchmark for Rust servers (KSVC + Tokio)
	@$(TEST_RUNNER) echo --servers ksvc tokio --port-base $(PORT_BASE)

# ─── HTTP Benchmarks ────────────────────────────────────────

itest-httpd-all: ## Run HTTP benchmarks for all servers (requires wrk)
	@$(TEST_RUNNER) httpd --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

itest-httpd-ksvc: ## Run HTTP benchmark for KSVC only
	@$(TEST_RUNNER) httpd --servers ksvc --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

itest-httpd-tokio: ## Run HTTP benchmark for Tokio only
	@$(TEST_RUNNER) httpd --servers tokio --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

itest-httpd-go: ## Run HTTP benchmark for Go only
	@$(TEST_RUNNER) httpd --servers go --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

itest-httpd-rust: ## Run HTTP benchmarks for Rust servers (KSVC + Tokio)
	@$(TEST_RUNNER) httpd --servers ksvc tokio --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

# ─── Run All ────────────────────────────────────────────────

itest-all: ## Run all integration tests and benchmarks
	@$(TEST_RUNNER) --all --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

# ─── Baselines ──────────────────────────────────────────────

itest-baseline-save-echo: ## Save current echo results as baseline
	@$(TEST_RUNNER) echo --save-baseline --port-base $(PORT_BASE)

itest-baseline-save-httpd: ## Save current httpd results as baseline (requires wrk)
	@$(TEST_RUNNER) httpd --save-baseline --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

itest-baseline-check-echo: ## Compare echo results vs saved baseline (fail on regression)
	@$(TEST_RUNNER) echo --baseline --threshold $(THRESHOLD) --port-base $(PORT_BASE)

itest-baseline-check-httpd: ## Compare httpd results vs saved baseline (fail on regression)
	@$(TEST_RUNNER) httpd --baseline --threshold $(THRESHOLD) --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

itest-baseline-check-all: ## Compare all test types vs their saved baselines
	@$(TEST_RUNNER) --all --baseline --threshold $(THRESHOLD) --port-base $(PORT_BASE) --wrk-threads $(WRK_THREADS)

# ─── Result Management ─────────────────────────────────────

itest-results-clean: ## Remove saved results (keeps baselines)
	@find results/ -name "*.json" ! -name "baseline.json" -delete 2>/dev/null; \
	echo "Cleaned results/ (baselines kept)"

itest-results-clean-all: ## Remove all saved results including baselines
	@rm -rf results/
	@echo "Cleaned all results/"

.PHONY: itest-list itest-check-deps itest-smoke \
        itest-echo-all itest-echo-ksvc itest-echo-tokio itest-echo-go itest-echo-rust \
        itest-httpd-all itest-httpd-ksvc itest-httpd-tokio itest-httpd-go itest-httpd-rust \
        itest-all \
        itest-baseline-save-echo itest-baseline-save-httpd \
        itest-baseline-check-echo itest-baseline-check-httpd itest-baseline-check-all \
        itest-results-clean itest-results-clean-all