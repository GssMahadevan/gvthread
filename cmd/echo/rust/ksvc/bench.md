```text
gssm@gvthread2:~/src/gvthread$ python3 cmd/ksvc-echo/bench_compare.py --build
Building servers...
  Building Go... OK
  Building Tokio... OK
  Building KSVC... OK

==========================================================================================
  KSVC vs Tokio vs Go — Echo Server Benchmark
==========================================================================================

──────────────────────────────────────────────────────────────────────────────────────────
  Test 0: light:  4 conn × 1K msgs × 64B
  Total: 4,000 messages, 250 KB
──────────────────────────────────────────────────────────────────────────────────────────
  Starting Go     on :9997... OK  Running... PASS      35,103 msg/s  p50=0.09ms  p99=0.35ms
  Starting Tokio  on :9998... OK  Running... PASS      33,494 msg/s  p50=0.09ms  p99=0.34ms
  Starting KSVC   on :9999... OK  Running... PASS      40,944 msg/s  p50=0.07ms  p99=0.32ms

──────────────────────────────────────────────────────────────────────────────────────────
  Test 1: medium: 50 conn × 1K msgs × 64B
  Total: 50,000 messages, 3125 KB
──────────────────────────────────────────────────────────────────────────────────────────
  Starting Go     on :9997... OK  Running... PASS      30,790 msg/s  p50=1.29ms  p99=5.43ms
  Starting Tokio  on :9998... OK  Running... PASS      31,851 msg/s  p50=1.26ms  p99=5.21ms
  Starting KSVC   on :9999... OK  Running... PASS      31,942 msg/s  p50=1.26ms  p99=5.15ms

──────────────────────────────────────────────────────────────────────────────────────────
  Test 2: heavy:  100 conn × 2K msgs × 128B
  Total: 200,000 messages, 25000 KB
──────────────────────────────────────────────────────────────────────────────────────────
  Starting Go     on :9997... OK  Running... PASS      28,335 msg/s  p50=2.83ms  p99=11.84ms
  Starting Tokio  on :9998... OK  Running... PASS      29,508 msg/s  p50=2.70ms  p99=11.32ms
  Starting KSVC   on :9999... OK  Running... PASS      29,716 msg/s  p50=2.71ms  p99=11.31ms

──────────────────────────────────────────────────────────────────────────────────────────
  Test 3: bulk:   20 conn × 500 msgs × 4KB
  Total: 10,000 messages, 39062 KB
──────────────────────────────────────────────────────────────────────────────────────────
  Starting Go     on :9997... OK  Running... PASS      28,564 msg/s  p50=0.55ms  p99=2.24ms
  Starting Tokio  on :9998... OK  Running... PASS      28,489 msg/s  p50=0.56ms  p99=2.22ms
  Starting KSVC   on :9999... OK  Running... PASS      27,798 msg/s  p50=0.58ms  p99=2.33ms

──────────────────────────────────────────────────────────────────────────────────────────
  Test 4: storm:  200 conn × 1K msgs × 64B
  Total: 200,000 messages, 12500 KB
──────────────────────────────────────────────────────────────────────────────────────────
  Starting Go     on :9997... OK  Running... PASS      27,615 msg/s  p50=5.76ms  p99=24.35ms
  Starting Tokio  on :9998... OK  Running... PASS      29,149 msg/s  p50=5.44ms  p99=22.94ms
  Starting KSVC   on :9999... OK  Running... PASS      29,283 msg/s  p50=5.43ms  p99=22.79ms

==========================================================================================
  THROUGHPUT (msg/s) — higher is better
==========================================================================================

  Test                             Go        Tokio         KSVC    Best
  ────────────────────── ──────────── ──────────── ────────────  ──────
  light                        35,103       33,494       40,944    KSVC
  medium                       30,790       31,851       31,942    KSVC
  heavy                        28,335       29,508       29,716    KSVC
  bulk                         28,564       28,489       27,798      Go
  storm                        27,615       29,149       29,283    KSVC

==========================================================================================
  LATENCY p99 (ms) — lower is better
==========================================================================================

  Test                             Go        Tokio         KSVC    Best
  ────────────────────── ──────────── ──────────── ────────────  ──────
  light                         0.35ms        0.34ms        0.32ms    KSVC
  medium                        5.43ms        5.21ms        5.15ms    KSVC
  heavy                        11.84ms       11.32ms       11.31ms    KSVC
  bulk                          2.24ms        2.22ms        2.33ms   Tokio
  storm                        24.35ms       22.94ms       22.79ms    KSVC

==========================================================================================
  LATENCY p50 (ms) — lower is better
==========================================================================================

  Test                             Go        Tokio         KSVC    Best
  ────────────────────── ──────────── ──────────── ────────────  ──────
  light                         0.09ms        0.09ms        0.07ms    KSVC
  medium                        1.29ms        1.26ms        1.26ms   Tokio
  heavy                         2.83ms        2.70ms        2.71ms   Tokio
  bulk                          0.55ms        0.56ms        0.58ms      Go
  storm                         5.76ms        5.44ms        5.43ms    KSVC

──────────────────────────────────────────────────────────────────────────────────────────
  ARCHITECTURE:
  Go:    goroutine-per-conn, netpoller (epoll), M:N scheduling
  Tokio: task-per-conn, multi-threaded, epoll (mio)
  KSVC:  single-threaded, io_uring, event loop (flush_and_wait)

  SYSCALLS PER ECHO (steady state):
  Go:    epoll_wait + read + write per msg  (3 syscalls)
  Tokio: epoll_ctl + epoll_wait + read + write  (3-4 syscalls)
  KSVC:  io_uring_enter per BATCH  (1 syscall for N msgs)
──────────────────────────────────────────────────────────────────────────────────────────

```