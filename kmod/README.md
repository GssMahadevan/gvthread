# KSVC Kernel Module — Phase 0

**Kernel SysCall Virtualization Channel**

Target: Ubuntu 24.04 LTS / Linux 6.8+

## What it provides

1. **`/dev/ksvc` device** — miscdevice, `O_RDWR` access
2. **Submit ring** — mmap'd, userspace writes entries, dispatcher reads
3. **Completion ring** — mmap'd, dispatcher writes completions, userspace reads
4. **Shared page** — mmap'd read-only, kernel-populated process metadata (Tier 0)

## What it does NOT provide (Phase 0)

- No kthread dispatcher (userspace Rust dispatcher handles Tier 1 + Tier 2)
- No direct syscall execution (Tier 2 is userspace thread pool via `libc::syscall`)

Phase 1 will add kthread workers for Tier 2 (eliminates privilege transition).

## Tier 0 — Shared Page

Userspace reads these fields with a memory load (~4ns) instead of syscall (~200ns):

| Field            | Syscall replaced | Savings        |
|-----------------|------------------|----------------|
| `pid`           | `getpid()`       | ~200ns → ~4ns  |
| `uid/gid`       | `getuid/getgid`  | ~200ns → ~4ns  |
| `euid/egid`     | `geteuid/getegid`| ~200ns → ~4ns  |
| `ppid`          | `getppid()`      | ~200ns → ~4ns  |
| `utsname_*`     | `uname()`        | ~500ns → ~4ns  |
| `rlimit_nofile` | `getrlimit()`    | ~200ns → ~4ns  |

## Build

```bash
make                    # build kernel module
make test               # build test binary
sudo insmod ksvc.ko     # load
sudo ./test/test_basic  # run tests
sudo rmmod ksvc         # unload
```

## DKMS Install (survives kernel upgrades)

```bash
make dkms-install
```

## API

```c
#include "ksvc_uapi.h"

int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);

// Create instance
struct ksvc_create_params params = {
    .submit_ring_entries = 256,
    .complete_ring_entries = 256,
    .eventfd = evfd,
};
ioctl(fd, KSVC_IOC_CREATE, &params);

// mmap rings
size_t submit_sz = ring_pages(256, sizeof(struct ksvc_entry)) * PAGE_SIZE;
void *submit = mmap(NULL, submit_sz, PROT_READ|PROT_WRITE, MAP_SHARED,
                    fd, KSVC_OFF_SUBMIT_RING);

size_t compl_sz = ring_pages(256, sizeof(struct ksvc_completion)) * PAGE_SIZE;
void *compl = mmap(NULL, compl_sz, PROT_READ|PROT_WRITE, MAP_SHARED,
                   fd, KSVC_OFF_COMPLETE_RING);

// mmap shared page (read-only)
void *shared = mmap(NULL, 4096, PROT_READ, MAP_SHARED,
                    fd, KSVC_OFF_SHARED_PAGE);

// Tier 0: read pid without syscall
struct ksvc_shared_page *sp = shared;
pid_t pid = sp->pid;  // ~4ns, no syscall
```

## 6.8 API Notes

- `eventfd_signal(ctx)` — 1 arg (6.7+ removed the `n` parameter)
- `vm_flags_set()/vm_flags_clear()` — available (6.3+)
- `vm_insert_page()` — used instead of `remap_pfn_range()` for proper refcounting
- `strscpy()` — used instead of deprecated `strlcpy()`
