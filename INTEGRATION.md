# KSVC Integration — Step by Step

## Extract this archive into your gvthread repo root

```bash
cd ~/gvthread
tar xzf ksvc-patch.tar.gz
```

This creates:
```
crates/ksvc-core/          ← NEW (11 .rs files)
crates/ksvc-module/        ← NEW (10 .rs files)
crates/ksvc-executor/      ← NEW (1 .rs file)
kmod/                      ← NEW (C kernel module)
docs/TRAIT_MAP.md          ← NEW
```

## Apply Cargo.toml changes

Edit your existing `Cargo.toml`:

### 1. Add KSVC to workspace members

```toml
members = [
    # ... existing members ...

    # KSVC crates
    "crates/ksvc-core",
    "crates/ksvc-module",
    "crates/ksvc-executor",
]
```

### 2. Add workspace dependencies

```toml
[workspace.dependencies]
# ... existing deps ...

# KSVC internal
ksvc-core = { path = "crates/ksvc-core" }
ksvc-module = { path = "crates/ksvc-module" }
ksvc-executor = { path = "crates/ksvc-executor" }

# KSVC external (add these if not present)
io-uring = "0.7"
crossbeam-queue = "0.3"
```

### 3. Bump nix version

Change `nix` from `0.27` to `0.29` and add features:

```toml
nix = { version = "0.29", features = ["signal", "pthread", "mman", "fs", "event", "ioctl"] }
```

**Note:** nix 0.27 → 0.29 may require minor changes in gvthread-runtime
(signal handling API). Check `cargo build` output.

### 4. Add Makefile targets

Append contents of `Makefile.ksvc` to your existing `Makefile`:

```bash
cat Makefile.ksvc >> Makefile
```

## Verify

```bash
# Build Rust workspace
cargo build --workspace

# Build kernel module (needs kernel headers)
make kmod

# Test kernel module (needs Ubuntu 24.04 / kernel 6.8+)
make kmod-install
make kmod-test
make kmod-uninstall
```

## Git commit sequence

```bash
git add crates/ksvc-core crates/ksvc-module crates/ksvc-executor kmod docs
git add Cargo.toml Makefile
git commit -m "feat: add KSVC — kernel syscall virtualization channel

Phase 0: rings + shared page (kernel module)
Rust trait architecture: 7 traits, default-safe implementations
Tier 0 (shared page), Tier 1 (io_uring), Tier 2 (worker pool), Tier 3 (legacy)

Kernel module: /dev/ksvc miscdevice for Ubuntu 24.04 / Linux 6.8+
- Submit ring + completion ring (mmap'd, lock-free SPSC)
- Shared page (Tier 0: pid/uid/uname in ~4ns)
- 14-test suite

Rust crates:
- ksvc-core: trait definitions (SyscallRouter, IoBackend, WorkerPool, etc.)
- ksvc-module: default impls (ProbeRouter, BasicIoUring, FixedPool, etc.)
- ksvc-executor: generic dispatcher loop
"
```
