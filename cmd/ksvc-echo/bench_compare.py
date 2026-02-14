#!/usr/bin/env python3
"""
KSVC vs Tokio vs Go Echo Server Benchmark

Starts each server, runs identical load tests, prints side-by-side comparison.

Usage:
    python3 cmd/ksvc-echo/bench_compare.py [--build]

Requires:
    ./target/release/ksvc-echo
    ./target/release/tokio-echo
    ./cmd/go-echo/go-echo  (or pass --build to auto-build)
"""

import subprocess
import time
import sys
import os
import signal
import socket
import argparse

KSVC_PORT = 9999
TOKIO_PORT = 9998
GO_PORT = 9997

# Test configurations: (threads, messages, size, label)
TESTS = [
    (4,   1000,   64,  "light:  4 conn × 1K msgs × 64B"),
    (50,  1000,   64,  "medium: 50 conn × 1K msgs × 64B"),
    (100, 2000,  128,  "heavy:  100 conn × 2K msgs × 128B"),
    (20,   500, 4000,  "bulk:   20 conn × 500 msgs × 4KB"),
    (200, 1000,   64,  "storm:  200 conn × 1K msgs × 64B"),
]

# Server definitions: (name, binary, port, build_cmd)
SERVERS = [
    ("Go",    "./cmd/go-echo/go-echo",       GO_PORT,
     ["go", "build", "-o", "./cmd/go-echo/go-echo", "./cmd/go-echo/main.go"]),
    ("Tokio", "./target/release/tokio-echo",  TOKIO_PORT,
     ["cargo", "build", "--release", "-p", "tokio-echo"]),
    ("KSVC",  "./target/release/ksvc-echo",   KSVC_PORT,
     ["cargo", "build", "--release", "-p", "ksvc-echo"]),
]


def build_all():
    for name, binary, _, cmd in SERVERS:
        print(f"  Building {name}...", end=" ", flush=True)
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            print(f"FAILED\n{r.stderr}")
            sys.exit(1)
        print("OK")
    print()


def wait_for_port(port, timeout=5.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            s.settimeout(0.5)
            s.connect(("127.0.0.1", port))
            s.close()
            return True
        except (ConnectionRefusedError, OSError):
            time.sleep(0.05)
    return False


def start_server(binary, port):
    proc = subprocess.Popen(
        [binary, str(port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        preexec_fn=os.setsid,
    )
    if not wait_for_port(port):
        proc.kill()
        raise RuntimeError(f"Server {binary} failed to start on port {port}")
    return proc


def stop_server(proc):
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        proc.wait(timeout=2)


def run_test(port, threads, messages, size):
    script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "test_echo.py")
    result = subprocess.run(
        [
            sys.executable, script,
            "--port", str(port),
            "--threads", str(threads),
            "--messages", str(messages),
            "--size", str(size),
            "--quiet",
        ],
        capture_output=True, text=True,
        timeout=120,
    )

    lines = result.stdout.strip().split("\n")
    metrics = {}
    for line in lines:
        line = line.strip()
        if "Throughput:" in line:
            val = line.split(":")[1].strip().replace(",", "").split()[0]
            metrics["msg_per_sec"] = float(val)
        elif "Latency avg:" in line:
            val = line.split(":")[1].strip().split("ms")[0]
            metrics["lat_avg"] = float(val)
        elif "Latency p50:" in line:
            val = line.split(":")[1].strip().split("ms")[0]
            metrics["lat_p50"] = float(val)
        elif "Latency p99:" in line:
            val = line.split(":")[1].strip().split("ms")[0]
            metrics["lat_p99"] = float(val)
        elif "Wall time:" in line:
            val = line.split(":")[1].strip().split("s")[0]
            metrics["wall_time"] = float(val)
        elif "Errors:" in line:
            val = line.split(":")[1].strip()
            metrics["errors"] = int(val)
        elif "Mismatches:" in line:
            val = line.split(":")[1].strip()
            metrics["mismatches"] = int(val)
        elif "Bandwidth:" in line:
            val = line.split(":")[1].strip().split()[0]
            metrics["bandwidth_mb"] = float(val)
        elif "RESULT: PASS" in line:
            metrics["pass"] = True
        elif "RESULT: FAIL" in line:
            metrics["pass"] = False

    if "msg_per_sec" not in metrics:
        metrics["msg_per_sec"] = 0
        metrics["lat_avg"] = 0
        metrics["lat_p50"] = 0
        metrics["lat_p99"] = 0
        metrics["wall_time"] = 0
        metrics["errors"] = -1
        metrics["pass"] = False

    return metrics


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--build", action="store_true", help="Build all servers before running")
    parser.add_argument("--tests", type=str, default="all",
                        help="Comma-separated test indices (0-based), or 'all'")
    args = parser.parse_args()

    if args.build:
        print("Building servers...")
        build_all()

    # Check binaries exist
    for name, binary, _, _ in SERVERS:
        if not os.path.exists(binary):
            print(f"ERROR: {binary} not found. Run with --build or build manually.")
            sys.exit(1)

    if args.tests == "all":
        selected = list(range(len(TESTS)))
    else:
        selected = [int(x) for x in args.tests.split(",")]

    W = 90
    print("=" * W)
    print("  KSVC vs Tokio vs Go — Echo Server Benchmark")
    print("=" * W)

    # all_results[test_idx] = (label, { "Go": metrics, "Tokio": metrics, "KSVC": metrics })
    all_results = []

    for tidx in selected:
        threads, messages, size, label = TESTS[tidx]
        total = threads * messages

        print(f"\n{'─' * W}")
        print(f"  Test {tidx}: {label}")
        print(f"  Total: {total:,} messages, {total * size / 1024:.0f} KB")
        print(f"{'─' * W}")

        test_results = {}

        for name, binary, port, _ in SERVERS:
            print(f"  Starting {name:<6} on :{port}...", end=" ", flush=True)
            proc = start_server(binary, port)
            print("OK", end="  ")

            print(f"Running...", end=" ", flush=True)
            m = run_test(port, threads, messages, size)
            status = "PASS" if m.get("pass") else "FAIL"
            print(f"{status}  {m.get('msg_per_sec', 0):>10,.0f} msg/s  "
                  f"p50={m.get('lat_p50', 0):.2f}ms  p99={m.get('lat_p99', 0):.2f}ms")

            stop_server(proc)
            time.sleep(0.3)
            test_results[name] = m

        all_results.append((label, test_results))

    # ── Summary: Throughput ──
    print(f"\n{'=' * W}")
    print(f"  THROUGHPUT (msg/s) — higher is better")
    print(f"{'=' * W}")
    print()
    hdr = f"  {'Test':<22} {'Go':>12} {'Tokio':>12} {'KSVC':>12}  {'Best':>6}"
    print(hdr)
    print(f"  {'─' * 22} {'─' * 12} {'─' * 12} {'─' * 12}  {'─' * 6}")

    for label, results in all_results:
        short = label.split(":")[0].strip()
        g = results.get("Go", {}).get("msg_per_sec", 0)
        t = results.get("Tokio", {}).get("msg_per_sec", 0)
        k = results.get("KSVC", {}).get("msg_per_sec", 0)
        vals = {"Go": g, "Tokio": t, "KSVC": k}
        best = max(vals, key=vals.get)
        print(f"  {short:<22} {g:>12,.0f} {t:>12,.0f} {k:>12,.0f}  {best:>6}")

    # ── Summary: Latency p99 ──
    print()
    print(f"{'=' * W}")
    print(f"  LATENCY p99 (ms) — lower is better")
    print(f"{'=' * W}")
    print()
    hdr = f"  {'Test':<22} {'Go':>12} {'Tokio':>12} {'KSVC':>12}  {'Best':>6}"
    print(hdr)
    print(f"  {'─' * 22} {'─' * 12} {'─' * 12} {'─' * 12}  {'─' * 6}")

    for label, results in all_results:
        short = label.split(":")[0].strip()
        g = results.get("Go", {}).get("lat_p99", 999)
        t = results.get("Tokio", {}).get("lat_p99", 999)
        k = results.get("KSVC", {}).get("lat_p99", 999)
        vals = {"Go": g, "Tokio": t, "KSVC": k}
        best = min(vals, key=vals.get)
        print(f"  {short:<22} {g:>11.2f}ms {t:>11.2f}ms {k:>11.2f}ms  {best:>6}")

    # ── Summary: Latency p50 ──
    print()
    print(f"{'=' * W}")
    print(f"  LATENCY p50 (ms) — lower is better")
    print(f"{'=' * W}")
    print()
    hdr = f"  {'Test':<22} {'Go':>12} {'Tokio':>12} {'KSVC':>12}  {'Best':>6}"
    print(hdr)
    print(f"  {'─' * 22} {'─' * 12} {'─' * 12} {'─' * 12}  {'─' * 6}")

    for label, results in all_results:
        short = label.split(":")[0].strip()
        g = results.get("Go", {}).get("lat_p50", 999)
        t = results.get("Tokio", {}).get("lat_p50", 999)
        k = results.get("KSVC", {}).get("lat_p50", 999)
        vals = {"Go": g, "Tokio": t, "KSVC": k}
        best = min(vals, key=vals.get)
        print(f"  {short:<22} {g:>11.2f}ms {t:>11.2f}ms {k:>11.2f}ms  {best:>6}")

    # ── Architecture notes ──
    print(f"\n{'─' * W}")
    print(f"  ARCHITECTURE:")
    print(f"  Go:    goroutine-per-conn, netpoller (epoll), M:N scheduling")
    print(f"  Tokio: task-per-conn, multi-threaded, epoll (mio)")
    print(f"  KSVC:  single-threaded, io_uring, event loop (flush_and_wait)")
    print(f"")
    print(f"  SYSCALLS PER ECHO (steady state):")
    print(f"  Go:    epoll_wait + read + write per msg  (3 syscalls)")
    print(f"  Tokio: epoll_ctl + epoll_wait + read + write  (3-4 syscalls)")
    print(f"  KSVC:  io_uring_enter per BATCH  (1 syscall for N msgs)")
    print(f"{'─' * W}")


if __name__ == "__main__":
    main()
