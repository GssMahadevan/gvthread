# makefiles/build.mk - Build targets
#
# Build commands for GVThread project

.PHONY: build-all build-debug build-release build-go

build-all: build-release build-go ## Build everything (Rust release + Go)

build-debug: ## Build Rust in debug mode
	$(CARGO) build --workspace

build-release: ## Build Rust in release mode
	$(CARGO) build --workspace --release

build-go: ## Build Go playground
	cd other/go/playground1 && $(GO) build -o main .

build-check: ## Check compilation without building
	$(CARGO) check --workspace

build-clippy: ## Run clippy lints
	$(CARGO) clippy --workspace -- -D warnings