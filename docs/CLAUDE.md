# CLAUDE.md - Quick Reference for Claude AI

> Read this first when starting a gvthread development session.

## Project

**gvthread** - High-performance userspace Generic threads for Rust  
**Repo:** https://github.com/GssMahadevan/gvthread  
**Dev:** GssMahadevan

## Key Facts

- 16MB virtual slots per GVThread, physical memory on-demand
- ~20ns voluntary context switch (x86_64 assembly)
- Go-like scheduling: per-worker local queues + global queue
- CPU performance matches Go's goroutines ✓

## Structure

```
crates/
├── gvthread-core/      # Types (no deps): id, state, metadata, channel, mutex
├── gvthread-runtime/   # Implementation: scheduler, worker, timer/, memory/, arch/
└── gvthread/           # Public facade
cmd/                    # Examples: basic, benchmark, stress, etc.
docs/                   # ARCHITECTURE.md, CONTEXT.md, TODO.md
```

## Critical Files

| File | Purpose |
|------|---------|
| `scheduler.rs` | Main scheduler, spawn/yield/wake |
| `ready_queue.rs` | Go-like local + global queues |
| `timer/mod.rs` | Sleep queue, preemption monitoring |
| `worker.rs` | Worker pool, worker states |
| `arch/x86_64/mod.rs` | Context switch assembly |
| `metadata.rs` | GVThreadMetadata (repr(C)) |

## Timer Module

```
timer/
├── mod.rs      # Sleep queue (BinaryHeap), TimerThread, preemption
├── entry.rs    # TimerEntry, TimerHandle (for future)
├── registry.rs # TimerRegistry API (for future)
└── impls/      # Backend implementations (for future)
```

## Recent Work (2025-02)

1. Ready queue refactor: bitmap → Go-like queues
2. Timer refactor: single file → modular directory
3. CPU perf now matches Go

## Fetch Files

I can fetch directly from the public repo:
```
https://raw.githubusercontent.com/GssMahadevan/gvthread/master/path/to/file.rs
```

## Docs

- `docs/ARCHITECTURE.md` - Full technical reference
- `docs/CONTEXT.md` - Detailed context for continuation
- `docs/TODO.md` - Task checklist