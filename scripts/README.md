# gvthread Development Scripts

Scripts for setting up and working with the gvthread development environment.

## VM Setup (Ubuntu 22.04)

### Initial Setup

SSH into your Proxmox VM and run:

```bash
# Download the setup script
curl -O https://raw.githubusercontent.com/gssmahadevan/gvthread/main/scripts/setup-ubuntu.sh
chmod +x setup-ubuntu.sh
./setup-ubuntu.sh

# Or if you've already cloned the repo:
cd gvthread/scripts
./setup-ubuntu.sh
```

### Verify Environment

After setup, verify everything is working:

```bash
./verify-env.sh
```

## Development Workflow

### Option 1: VS Code Remote SSH (Recommended)

1. Install "Remote - SSH" extension in VS Code
2. Connect to your VM: `Cmd+Shift+P` → "Remote-SSH: Connect to Host"
3. Open `/home/user/gvthread` folder
4. Use integrated terminal to build/test

### Option 2: Edit on Mac, Sync to VM

On your Mac:

```bash
# Set your VM address
export GVTHREAD_VM=user@192.168.1.100

# Sync and build
./scripts/sync-to-vm.sh             # Just sync
./scripts/sync-to-vm.sh build       # Sync and build
./scripts/sync-to-vm.sh test        # Sync and test
```

### Option 3: Direct Development on VM

SSH to VM and use the dev helper:

```bash
./scripts/dev.sh build      # Build all
./scripts/dev.sh test       # Run tests
./scripts/dev.sh check      # Quick check
./scripts/dev.sh run basic  # Run cmd/basic
./scripts/dev.sh watch      # Watch mode
./scripts/dev.sh clippy     # Lint
./scripts/dev.sh fmt        # Format
```

## Scripts Reference

| Script | Description |
|--------|-------------|
| `setup-ubuntu.sh` | One-time setup for Ubuntu 22.04 VM |
| `verify-env.sh` | Verify all tools and features are available |
| `dev.sh` | Development helper (build, test, run, etc.) |
| `sync-to-vm.sh` | Sync code from Mac to Linux VM |

## VM Requirements

- **OS**: Ubuntu 22.04 LTS
- **CPU**: 4+ cores (for multi-worker testing)
- **RAM**: 8-16 GB
- **Disk**: 50 GB
- **CPU Type**: "host" in Proxmox (for accurate perf)

## Troubleshooting

### perf not working

```bash
# Check if perf is installed for your kernel
sudo apt install linux-tools-$(uname -r)

# If permission denied
sudo sysctl kernel.perf_event_paranoid=-1
```

### max_map_count too low

If you're creating many GVThreads:

```bash
sudo sysctl vm.max_map_count=262144

# Make permanent
echo 'vm.max_map_count=262144' | sudo tee -a /etc/sysctl.conf
```

### naked_functions require nightly

The context switch assembly uses `#[naked]` which requires nightly:

```bash
# Build with nightly
cargo +nightly build

# Or set as default for this project
rustup override set nightly
```
