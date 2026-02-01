#!/bin/bash
#
# gvthread development helper script
#
# Usage:
#   ./dev.sh build        - Build all crates
#   ./dev.sh test         - Run all tests
#   ./dev.sh check        - Quick check (no codegen)
#   ./dev.sh clippy       - Run clippy lints
#   ./dev.sh fmt          - Format code
#   ./dev.sh run <cmd>    - Run example from cmd/
#   ./dev.sh watch        - Watch and check on changes
#   ./dev.sh asm <fn>     - Show assembly for function
#   ./dev.sh clean        - Clean build artifacts
#   ./dev.sh nightly      - Build with nightly (for naked fns)
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_cmd() {
    echo -e "${BLUE}>>> $@${NC}"
}

case "${1:-help}" in
    build)
        log_cmd "cargo build --workspace"
        cargo build --workspace
        ;;
        
    build-release)
        log_cmd "cargo build --workspace --release"
        cargo build --workspace --release
        ;;
        
    test)
        log_cmd "cargo test --workspace"
        cargo test --workspace
        ;;
        
    test-verbose)
        log_cmd "cargo test --workspace -- --nocapture"
        cargo test --workspace -- --nocapture
        ;;
        
    check)
        log_cmd "cargo check --workspace"
        cargo check --workspace
        ;;
        
    clippy)
        log_cmd "cargo clippy --workspace -- -W clippy::all"
        cargo clippy --workspace -- -W clippy::all
        ;;
        
    fmt)
        log_cmd "cargo fmt --all"
        cargo fmt --all
        ;;
        
    fmt-check)
        log_cmd "cargo fmt --all -- --check"
        cargo fmt --all -- --check
        ;;
        
    run)
        if [ -z "$2" ]; then
            echo "Usage: $0 run <cmd-name>"
            echo ""
            echo "Available commands:"
            ls -1 cmd/ | while read -r cmd; do
                echo "  - $cmd"
            done
            exit 1
        fi
        PKG_NAME="gvthread-$2"
        log_cmd "cargo run -p $PKG_NAME"
        cargo run -p "$PKG_NAME"
        ;;
        
    run-release)
        if [ -z "$2" ]; then
            echo "Usage: $0 run-release <cmd-name>"
            exit 1
        fi
        PKG_NAME="gvthread-$2"
        log_cmd "cargo run -p $PKG_NAME --release"
        cargo run -p "$PKG_NAME" --release
        ;;
        
    watch)
        if ! command -v cargo-watch &> /dev/null; then
            echo "cargo-watch not installed. Run: cargo install cargo-watch"
            exit 1
        fi
        log_cmd "cargo watch -x check"
        cargo watch -x check
        ;;
        
    asm)
        if [ -z "$2" ]; then
            echo "Usage: $0 asm <function-name>"
            exit 1
        fi
        if ! command -v cargo-asm &> /dev/null; then
            echo "cargo-asm not installed. Run: cargo install cargo-asm"
            exit 1
        fi
        log_cmd "cargo asm --lib gvthread-runtime $2"
        cargo asm --lib gvthread-runtime "$2" || {
            echo ""
            echo "Try with full path like: gvthread_runtime::arch::x86_64::context_switch_voluntary"
        }
        ;;
        
    expand)
        if ! command -v cargo-expand &> /dev/null; then
            echo "cargo-expand not installed. Run: cargo install cargo-expand"
            exit 1
        fi
        CRATE="${2:-gvthread-core}"
        log_cmd "cargo expand -p $CRATE"
        cargo expand -p "$CRATE"
        ;;
        
    nightly)
        log_cmd "cargo +nightly build --workspace"
        cargo +nightly build --workspace
        ;;
        
    nightly-test)
        log_cmd "cargo +nightly test --workspace"
        cargo +nightly test --workspace
        ;;
        
    clean)
        log_cmd "cargo clean"
        cargo clean
        ;;
        
    doc)
        log_cmd "cargo doc --workspace --no-deps --open"
        cargo doc --workspace --no-deps --open
        ;;
        
    tree)
        log_cmd "cargo tree -p gvthread"
        cargo tree -p gvthread
        ;;
        
    outdated)
        if ! command -v cargo-outdated &> /dev/null; then
            echo "cargo-outdated not installed. Run: cargo install cargo-outdated"
            exit 1
        fi
        log_cmd "cargo outdated"
        cargo outdated
        ;;
        
    help|--help|-h|*)
        echo "gvthread development helper"
        echo ""
        echo "Usage: $0 <command> [args]"
        echo ""
        echo "Commands:"
        echo "  build         Build all crates (debug)"
        echo "  build-release Build all crates (release)"
        echo "  test          Run all tests"
        echo "  test-verbose  Run tests with output"
        echo "  check         Quick syntax/type check"
        echo "  clippy        Run clippy lints"
        echo "  fmt           Format all code"
        echo "  fmt-check     Check formatting"
        echo "  run <cmd>     Run command from cmd/"
        echo "  run-release   Run command in release mode"
        echo "  watch         Watch for changes and check"
        echo "  asm <fn>      Show assembly for function"
        echo "  expand [crate] Expand macros"
        echo "  nightly       Build with nightly toolchain"
        echo "  nightly-test  Test with nightly toolchain"
        echo "  clean         Clean build artifacts"
        echo "  doc           Build and open documentation"
        echo "  tree          Show dependency tree"
        echo "  outdated      Check for outdated dependencies"
        echo ""
        ;;
esac
