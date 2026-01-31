#!/bin/bash
#
# Verify gvthread development environment
#
# Checks that all required tools and features are available.
#

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

ERRORS=0
WARNINGS=0

check_command() {
    local cmd=$1
    local required=$2
    
    if command -v "$cmd" &> /dev/null; then
        echo -e "${GREEN}✓${NC} $cmd: $(command -v $cmd)"
        return 0
    else
        if [ "$required" = "required" ]; then
            echo -e "${RED}✗${NC} $cmd: NOT FOUND (required)"
            ((ERRORS++))
        else
            echo -e "${YELLOW}!${NC} $cmd: NOT FOUND (optional)"
            ((WARNINGS++))
        fi
        return 1
    fi
}

check_rust_feature() {
    local feature=$1
    local toolchain=${2:-stable}
    
    if rustup run "$toolchain" rustc --print cfg 2>/dev/null | grep -q "$feature"; then
        echo -e "${GREEN}✓${NC} Rust feature '$feature' ($toolchain)"
        return 0
    else
        echo -e "${YELLOW}!${NC} Rust feature '$feature' not detected ($toolchain)"
        return 1
    fi
}

echo "============================================"
echo "gvthread Environment Verification"
echo "============================================"
echo ""

echo "--- Required Tools ---"
check_command rustc required
check_command cargo required
check_command gcc required
check_command git required

echo ""
echo "--- Rust Toolchains ---"
if command -v rustup &> /dev/null; then
    echo -e "${GREEN}✓${NC} rustup installed"
    
    echo "  Installed toolchains:"
    rustup toolchain list | while read -r line; do
        echo "    - $line"
    done
    
    # Check nightly for naked_functions
    if rustup run nightly rustc --version &> /dev/null; then
        echo -e "${GREEN}✓${NC} nightly toolchain available"
    else
        echo -e "${RED}✗${NC} nightly toolchain NOT installed"
        echo "    Run: rustup toolchain install nightly"
        ((ERRORS++))
    fi
else
    echo -e "${RED}✗${NC} rustup NOT installed"
    ((ERRORS++))
fi

echo ""
echo "--- Debugging Tools ---"
check_command gdb optional
check_command strace optional
check_command perf optional
check_command valgrind optional

echo ""
echo "--- Architecture Check ---"
ARCH=$(uname -m)
echo "  Architecture: $ARCH"
if [ "$ARCH" = "x86_64" ]; then
    echo -e "${GREEN}✓${NC} x86_64 supported"
elif [ "$ARCH" = "aarch64" ]; then
    echo -e "${YELLOW}!${NC} aarch64 - assembly not yet implemented"
    ((WARNINGS++))
else
    echo -e "${RED}✗${NC} Unsupported architecture: $ARCH"
    ((ERRORS++))
fi

echo ""
echo "--- Kernel Features ---"
KERNEL=$(uname -r)
echo "  Kernel: $KERNEL"

# Check if we can use signals
if [ -f /proc/sys/kernel/pid_max ]; then
    echo -e "${GREEN}✓${NC} /proc filesystem available"
else
    echo -e "${YELLOW}!${NC} /proc not fully available (container?)"
    ((WARNINGS++))
fi

# Check signal delivery
echo "  Testing SIGURG delivery..."
if timeout 1 bash -c 'kill -URG $$ 2>/dev/null' 2>/dev/null; then
    echo -e "${GREEN}✓${NC} SIGURG can be sent"
else
    echo -e "${YELLOW}!${NC} SIGURG test inconclusive"
fi

echo ""
echo "--- Memory Features ---"
# Check if we can create large mappings
if [ -f /proc/sys/vm/max_map_count ]; then
    MAX_MAP=$(cat /proc/sys/vm/max_map_count)
    echo "  vm.max_map_count: $MAX_MAP"
    if [ "$MAX_MAP" -lt 65530 ]; then
        echo -e "${YELLOW}!${NC} max_map_count may be too low for many GVThreads"
        echo "    Consider: sudo sysctl vm.max_map_count=262144"
        ((WARNINGS++))
    else
        echo -e "${GREEN}✓${NC} max_map_count sufficient"
    fi
fi

# Check overcommit settings
if [ -f /proc/sys/vm/overcommit_memory ]; then
    OVERCOMMIT=$(cat /proc/sys/vm/overcommit_memory)
    echo "  vm.overcommit_memory: $OVERCOMMIT"
    echo -e "${GREEN}✓${NC} Memory overcommit available"
fi

echo ""
echo "============================================"
echo "Summary"
echo "============================================"

if [ $ERRORS -eq 0 ] && [ $WARNINGS -eq 0 ]; then
    echo -e "${GREEN}All checks passed!${NC}"
    exit 0
elif [ $ERRORS -eq 0 ]; then
    echo -e "${YELLOW}$WARNINGS warning(s), but environment is usable.${NC}"
    exit 0
else
    echo -e "${RED}$ERRORS error(s), $WARNINGS warning(s).${NC}"
    echo "Please fix the errors above before building gvthread."
    exit 1
fi
