# makefiles/clean.mk - Clean targets
#
# Cleanup commands for GVThread project

.PHONY: clean clean-rust clean-go clean-perf clean-all

clean: clean-rust ## Clean Rust build artifacts

clean-rust: ## Clean Rust target directory
	$(CARGO) clean

clean-go: ## Clean Go build artifacts
	rm -f other/go/playground1/main

clean-perf: ## Clean performance data files
	rm -rf /tmp/gvthread-perf /tmp/gvthread-flames
	rm -f /tmp/bench_*.txt

clean-all: clean-rust clean-go clean-perf ## Clean everything
	@echo "All clean."