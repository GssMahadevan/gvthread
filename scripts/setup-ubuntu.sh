#!/bin/bash
#
# gvthread Development Environment Setup
# Target: Ubuntu 22.04 LTS (Proxmox VM)
#
# Usage:
#   chmod +x setup-ubuntu.sh
#   ./setup-ubuntu.sh
#
# This script installs:
#   - Build essentials (gcc, make, etc.)
#   - Rust toolchain (stable + nightly for naked_functions)
#   - Development tools (gdb, perf, strace)
#   - Useful utilities
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if running on Ubuntu
if ! grep -q "Ubuntu" /etc/os-release 2>/dev/null; then
    log_warn "This script is designed for Ubuntu. Proceed with caution."
fi

log_info "Starting gvthread development environment setup..."

# Update system
log_info "Updating system packages..."
sudo apt update
sudo apt upgrade -y

# Install build essentials
log_info "Installing build essentials..."
sudo apt install -y \
    build-essential \
    pkg-config \
    cmake \
    git \
    curl \
    wget

# Install development libraries
log_info "Installing development libraries..."
sudo apt install -y \
    libssl-dev \
    libclang-dev \
    llvm-dev

# Install debugging and profiling tools
log_info "Installing debugging and profiling tools..."
sudo apt install -y \
    gdb \
    strace \
    ltrace \
    valgrind \
    binutils

# Install perf (kernel-specific)
log_info "Installing perf tools..."
KERNEL_VERSION=$(uname -r)
sudo apt install -y linux-tools-common linux-tools-generic || true
sudo apt install -y "linux-tools-${KERNEL_VERSION}" || {
    log_warn "Could not install kernel-specific perf tools."
    log_warn "You may need to install them manually for your kernel version."
}

# Install kernel headers (for perf and potential kernel module work)
log_info "Installing kernel headers..."
sudo apt install -y "linux-headers-${KERNEL_VERSION}" || {
    log_warn "Could not install kernel headers for ${KERNEL_VERSION}"
}

# Install useful utilities
log_info "Installing useful utilities..."
sudo apt install -y \
    htop \
    tree \
    ripgrep \
    fd-find \
    jq \
    tmux \
    vim

# Install Rust
log_info "Installing Rust toolchain..."
if command -v rustup &> /dev/null; then
    log_info "Rust is already installed, updating..."
    rustup update
else
    log_info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    
    # Source cargo environment
    source "$HOME/.cargo/env"
fi

# Ensure cargo is in PATH for this script
export PATH="$HOME/.cargo/bin:$PATH"

# Install Rust components
log_info "Installing Rust components..."

# Stable toolchain (default)
rustup default stable

# Install nightly (needed for naked_functions feature)
log_info "Installing nightly toolchain (required for naked_functions)..."
rustup toolchain install nightly

# Install rust-src (required for some advanced features)
rustup component add rust-src
rustup component add rust-src --toolchain nightly

# Install useful Rust tools
log_info "Installing Rust development tools..."
rustup component add rustfmt
rustup component add clippy
rustup component add rust-analyzer

# Install cargo tools
log_info "Installing cargo tools..."
cargo install cargo-watch || log_warn "cargo-watch install failed"
cargo install cargo-expand || log_warn "cargo-expand install failed"
cargo install cargo-asm || log_warn "cargo-asm install failed"

# Verify installations
log_info "Verifying installations..."

echo ""
echo "============================================"
echo "Installation Summary"
echo "============================================"

# Check Rust
if command -v rustc &> /dev/null; then
    echo -e "${GREEN}✓${NC} Rust: $(rustc --version)"
else
    echo -e "${RED}✗${NC} Rust: NOT INSTALLED"
fi

# Check Cargo
if command -v cargo &> /dev/null; then
    echo -e "${GREEN}✓${NC} Cargo: $(cargo --version)"
else
    echo -e "${RED}✗${NC} Cargo: NOT INSTALLED"
fi

# Check nightly
if rustup run nightly rustc --version &> /dev/null; then
    echo -e "${GREEN}✓${NC} Nightly: $(rustup run nightly rustc --version)"
else
    echo -e "${RED}✗${NC} Nightly: NOT INSTALLED"
fi

# Check GCC
if command -v gcc &> /dev/null; then
    echo -e "${GREEN}✓${NC} GCC: $(gcc --version | head -1)"
else
    echo -e "${RED}✗${NC} GCC: NOT INSTALLED"
fi

# Check GDB
if command -v gdb &> /dev/null; then
    echo -e "${GREEN}✓${NC} GDB: $(gdb --version | head -1)"
else
    echo -e "${RED}✗${NC} GDB: NOT INSTALLED"
fi

# Check perf
if command -v perf &> /dev/null; then
    echo -e "${GREEN}✓${NC} Perf: $(perf --version 2>/dev/null || echo 'installed')"
else
    echo -e "${YELLOW}!${NC} Perf: NOT INSTALLED (optional)"
fi

echo "============================================"
echo ""

# Create shell profile additions
log_info "Setting up shell environment..."

PROFILE_ADDITIONS='
# Rust environment
. "$HOME/.cargo/env"

# gvthread development aliases
alias cb="cargo build"
alias ct="cargo test"
alias cr="cargo run"
alias cw="cargo watch -x check"
alias clippy="cargo clippy -- -W clippy::all"

# Useful aliases
alias ll="ls -la"
alias gs="git status"
alias gd="git diff"
'

# Add to .bashrc if not already present
if ! grep -q "gvthread development aliases" "$HOME/.bashrc" 2>/dev/null; then
    echo "$PROFILE_ADDITIONS" >> "$HOME/.bashrc"
    log_info "Added aliases to .bashrc"
fi

# Print next steps
echo ""
echo "============================================"
echo "Setup Complete!"
echo "============================================"
echo ""
echo "Next steps:"
echo ""
echo "  1. Reload shell or run:"
echo "     source ~/.bashrc"
echo ""
echo "  2. Clone/copy the gvthread project:"
echo "     git clone <your-repo> ~/gvthread"
echo "     # or rsync from Mac:"
echo "     # rsync -avz --exclude target/ user@mac:~/gvthread/ ~/gvthread/"
echo ""
echo "  3. Build the project:"
echo "     cd ~/gvthread"
echo "     cargo build"
echo ""
echo "  4. Run tests:"
echo "     cargo test"
echo ""
echo "  5. For naked_functions (asm), use nightly:"
echo "     cargo +nightly build"
echo ""
echo "============================================"
