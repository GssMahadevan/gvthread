# GVThread Project — Continuation Context
## Date: 2026-02-15 (session spanning ~05:00 UTC)

---

## PROJECT OVERVIEW

GVThread is a personal open-source Rust project — a green threading runtime with kernel syscall virtualization. The system provides goroutine-style blocking code on top of io_uring, targeting Linux x86-64.

### Repository: `/home/gssm/src/gvthread`

### Crate structure:
```
crates/
  gvthread-runtime/     — scheduler, worker pool, ready queues, context switching
  ksvc-module/          — io_uring wrapper (BasicIoUring), probe router
  ksvc-gvthread/        — bridge: reactor, syscall wrappers, networking (GvtListener/GvtStream)

cmd/
  httpd/rust/ksvc/      — ksvc-httpd: event-loop + io_uring
  httpd/rust/tokio/     — tokio-httpd: async/await + epoll
  httpd/rust/gvthread/  — gvthread-httpd: green threads + io_uring (old shared reactor = gvthread0)
                          Also builds gvthread1-httpd (per-worker reactor, Phase 1)
  httpd/go/             — go-httpd: multi-variant (naive/mux/fiber via gvt_app_variant env)
  wrkr/rust/wrkr/       — wrkr: benchmark load generator (hyper/reqwest strategies)

benches/
  bench-runner.py       — benchmark orchestrator (YAML manifest, wrk/wrkr support)
  httpd/manifest.yml    — benchmark matrix definition
```

---

## WHAT WE ACCOMPLISHED (THIS SESSION + PRIOR)

### 1. Benchmark Framework (bench-runner.py, ~1240 lines)
- YAML manifest: common profiles × app configs matrix
- Per-app ports (avoids TIME_WAIT conflicts)
- `--build debug|release`, `--common`, `--app`, `--config` filters
- `--wrkr PATH`, `--use-wrk` flags for load generator selection
- Auto-detects wrkr in `target/<build>/wrkr`, falls back to wrk
- Crash diagnostics: signal names, server stderr dump, exit code reporting
- **Critical fix**: `_dump_server_output()` now kills server before reading pipes (prevents blocking forever on alive process)
- Env passthrough: `gvt_app_http=hyper|reqwest` sent to wrkr

### 2. Stack Alignment SIGSEGV Fix (bug-2.md)
- gvthread-httpd crashed on `movaps` (SSE alignment) in `memcpy`
- Root cause: double subtraction in `arch/x86_64/mod.rs` line 28
- Stack was 8-byte aligned instead of 16-byte aligned

### 3. Performance Analysis & Per-Worker Reactor Architecture
- gvthread0-httpd: 7K req/s (40× slower than ksvc-httpd due to cross-thread coordination)
- Root cause: single shared reactor thread, MPSC queue, mutex-protected ready queue
- Designed 4-phase optimization roadmap

### 4. Phase 1: Per-Worker Reactor (COMPLETED — 48× speedup)

**Files created/modified:**

| File | What |
|------|------|
| `crates/ksvc-gvthread/src/worker_reactor.rs` (NEW, ~324 lines) | WorkerReactorPool: N io_uring instances, one per worker |
| `crates/gvthread-runtime/src/scheduler.rs` (MODIFIED) | Hook functions for worker I/O polling, flush_and_wait integration |
| `crates/ksvc-gvthread/src/syscall.rs` (MODIFIED) | Worker-local submit path (wr_* functions), fixed worker_id lookup |
| `crates/ksvc-gvthread/src/net.rs` (MODIFIED) | Dual-path: bind_local() for per-worker, bind(shared) for legacy |
| `crates/ksvc-gvthread/src/lib.rs` (MODIFIED) | pub mod worker_reactor |
| `cmd/httpd/rust/gvthread/src/main.rs` (MODIFIED) | gvthread1-httpd uses WorkerReactorPool::init_global() |

**Critical bug fix**: `syscall.rs` line 271-272 used wrong thread-local (`tls::try_current_worker_id()` returns None on workers). Fixed to use `worker::current_worker_id()`.

**Architecture flow (new):**
```
GVThread → submit_and_park_worker() → inline SQE to worker's io_uring
         → block_current() → context switch to worker scheduler
Worker   → poll() → flush SQEs → drain CQEs → wake_gvthread()
         → LocalQueue.push() → SpinLock (same core)
         → run GVThread → read result slab
```
Zero cross-thread hops. 2 SpinLock ops per I/O (vs 6 mutex ops before).

### 5. Go HTTP Server (multi-variant, single binary)
- `cmd/httpd/go/main.go` — reads `gvt_app_variant` env var
- **naive**: raw `net.Listener`, manual HTTP parse, goroutine-per-conn
- **mux**: stdlib `net/http` + `ServeMux` (idiomatic Go)
- **fiber**: gofiber/fiber v2 (fasthttp, worker-pool model)
- Reads `gvt_app_port`, `gvt_parallelism` → `runtime.GOMAXPROCS()`
- go.mod has `gofiber/fiber/v2` dependency

### 6. wrkr Load Generator (cmd/wrkr/rust/wrkr/, 586 lines)
- **Two HTTP strategies** selected via `gvt_app_http` env var:
  - `hyper` (default) — bare HTTP engine, zero-copy, minimal alloc
  - `reqwest` — application-grade (TLS, cookies, redirects)
- **Echo strategy** — tokio TcpStream for raw TCP benchmarks
- JSON output on stdout, progress on stderr
- CLI: `-c` connections, `-d` duration, `--warmup`, `--no-keepalive`, `--payload`
- Env overrides: `WRKR_CONNECTIONS`, `WRKR_DURATION`, `WRKR_WARMUP`
- HTTPS auto-switches to reqwest
- Deps: tokio, hyper, hyper-util, http-body-util, reqwest (rustls-tls)

---

## BENCHMARK RESULTS (LATEST — with wrk client, light profile)

```
Profile: light — parallelism=2, connections=50, 5s measure

App              Config    Model          IO        req/s    rps/core   p50μs  p99μs  RSS MB
gvthread1-httpd  default   green-thread   io_uring  355,115  177,557   120    239    3.6  ◀ WINNER
tokio-httpd      default   async-await    epoll     320,676  160,338    90    270    3.4
ksvc-httpd       big-ring  event-loop     io_uring  266,794  133,397   195    270    5.4
ksvc-httpd       default   event-loop     io_uring  247,865  123,932   198    291    5.9
gvthread1-httpd  big-ring  green-thread   io_uring  236,960  118,480   218    371    4.5
go-httpd         fiber     goroutine      epoll     112,166   56,083   416  1,640   13.6
go-httpd         mux       goroutine      epoll     110,415   55,208   429  1,610   13.5
go-httpd         naive     goroutine      epoll     110,312   55,156   430  1,620   13.5
gvthread0-httpd  big-ring  green-thread   io_uring   56,143   28,071   264    367    5.3
gvthread0-httpd  default   green-thread   io_uring   13,319    6,659   229    380    5.3
```

**Key findings:**
- GVThread Phase 1: 48× speedup (7K → 355K), now fastest overall
- GVThread 3× faster than Go with same programming model (blocking green threads)
- Tokio has better p50 (90μs vs 120μs) — async callbacks avoid context switch
- Go variants within 4% of each other — bottleneck is Go runtime, not framework
- wrkr with reqwest: only 146K (client-side bottleneck), hyper not yet tested

---

## KNOWN ISSUES / TECHNICAL DEBT

### Immediate
1. **wrkr hyper strategy not yet benchmarked** — first run used reqwest (146K). Need to verify hyper matches wrk numbers (~355K)
2. **gvthread1 big-ring regression** — 236K vs default's 355K. Unexpected. May be related to SQ ring size interaction with per-worker architecture. Needs investigation.
3. **rust-analyzer CPU issue** — "Propagating panic for cycle head" in salsa, burns 165% CPU. Workaround: `linkedProjects` in VS Code settings to limit analysis scope

### Phase 2 TODO (Thread-Local Ready Queue)
- `scheduler::wake_gvthread()` uses `tls::try_current_worker_id()` which returns None on worker threads
- Woken GVThreads land in GlobalQueue instead of LocalQueue, defeating locality
- Fix: either set `tls::WORKER_ID` in `worker_main_loop()` or pass `worker_id` explicitly through wake path
- Expected improvement: 355K → maybe 400-500K (eliminate remaining mutex contention)

### Phase 3 TODO (Work Stealing)
- Already partially implemented in `ready_queue/simple.rs`
- Needs integration with per-worker reactor polling

### Phase 4 TODO (Batched io_uring Submit)
- Submit multiple SQEs per `io_uring_enter()` call
- Amortize syscall overhead across batch

### Stack Size
- Currently 16MB virtual per GVThread (only ~8KB physically touched for HTTP)
- User concerned about page faults at scale (10K+ connections)
- `gvt_config.rs` can reduce to 2MB — matches Linux thread default
- Hugepages (2MB THP) worth exploring later

### Backward Compatibility
- `reactor.rs` untouched — shared Reactor still works
- `ksvc_*` functions still work with `Arc<ReactorShared>`
- `GvtListener::bind(shared, port)` still supported alongside `bind_local(port)`
- gvthread0-httpd and gvthread1-httpd coexist for A/B testing

---

## FILE LOCATIONS (on gvthread2 VM)

```
~/src/gvthread/                          — repo root
~/src/gvthread/benches/bench-runner.py   — orchestrator
~/src/gvthread/benches/httpd/manifest.yml — benchmark matrix
~/src/gvthread/cmd/wrkr/rust/wrkr/      — wrkr load generator
~/src/gvthread/cmd/httpd/go/main.go     — Go multi-variant server
~/src/gvthread/docs/bugs/bug-2.md       — stack alignment bug writeup
~/src/gvthread/docs/                     — architecture docs
```

---

## BUILD COMMANDS

```bash
# All Rust servers + wrkr
cargo build --release

# Go server
cd cmd/httpd/go && go mod tidy && go build -o httpd-server . && cd -

# Run benchmarks (auto-detects wrkr)
python3 benches/bench-runner.py benches/httpd/manifest.yml --common light

# Force wrk
python3 benches/bench-runner.py benches/httpd/manifest.yml --common light --use-wrk

# Single app
python3 benches/bench-runner.py benches/httpd/manifest.yml --common light --app gvthread1-httpd

# wrkr standalone
target/release/wrkr http://127.0.0.1:8080/ -c50 -d5
gvt_app_http=reqwest target/release/wrkr http://127.0.0.1:8080/ -c50 -d5
```

---

## MANIFEST APPS (current)

| App | Binary | Port | Model |
|-----|--------|------|-------|
| ksvc-httpd | target/release/ksvc-httpd | 8080 | event-loop + io_uring |
| gvthread0-httpd | target/release/gvthread-httpd | 8081 | green-thread + shared reactor |
| gvthread1-httpd | target/release/gvthread1-httpd | 8081 | green-thread + per-worker reactor |
| tokio-httpd | target/release/tokio-httpd | 8082 | async-await + epoll |
| go-httpd | cmd/httpd/go/httpd-server | 8083 | goroutine + epoll (naive/mux/fiber) |

Note: gvthread0 and gvthread1 share port 8081 in manifest — user may have modified this. Check manifest.yml for current state.

---

## CONVERSATION TRANSCRIPT HISTORY

All prior session transcripts are in `/mnt/transcripts/`:
1. `2026-02-14-15-59-42-ksvc-gvthread-bridge-implementation.txt` — bridge crate creation
2. `2026-02-14-19-17-01-bench-runner-implementation.txt` — bench-runner creation
3. `2026-02-15-02-39-57-gvthread-stack-alignment-segfault-fix.txt` — SIGSEGV fix
4. `2026-02-15-03-12-59-gvthread-perf-analysis-architecture.txt` — 40× slowdown analysis
5. `2026-02-15-03-50-28-gvthread-phase1-worker-reactor-prep.txt` — codebase analysis for Phase 1
6. `2026-02-15-04-22-58-phase1-worker-reactor-implementation.txt` — Phase 1 implementation + bug fixes

Current session (not yet in transcript): Phase 1 benchmarks, Go multi-variant server, wrkr load generator with hyper/reqwest strategies, bench-runner wrkr integration, multiple bug fixes.