#!/usr/bin/env python3
"""
KSVC vs Tokio Echo Server Benchmark

Starts each server, runs identical load tests, prints side-by-side comparison.

Usage:
    python3 cmd/ksvc-echo/bench_compare.py [--build]

Requires: both ksvc-echo and tokio-echo built in target/release/
"""

import subprocess
import time
import sys
import os
import signal
import socket
import json
import argparse

KSVC_PORT = 9999
TOKIO_PORT = 9998

# Test configurations: (threads, messages, size, label)
TESTS = [
    (4,   1000,   64,  "light:  4 conn × 1K msgs × 64B"),
    (50,  1000,   64,  "medium: 50 conn × 1K msgs × 64B"),
    (100, 2000,  128,  "heavy:  100 conn × 2K msgs × 128B"),
    (20,   500, 4000,  "bulk:   20 conn × 500 msgs × 4KB"),
    (200, 1000,   64,  "storm:  200 conn × 1K msgs × 64B"),
]


def build_if_needed(do_build):
    if not do_build:
        return
    print("Building...")
    r = subprocess.run(
        ["cargo", "build", "--release", "-p", "ksvc-echo", "-p", "tokio-echo"],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        print(f"Build failed:\n{r.stderr}")
        sys.exit(1)
    print("Build OK.\n")


def wait_for_port(port, timeout=5.0):
    """Wait until a server is listening on the given port."""
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
    """Start a server process, wait for it to be ready."""
    proc = subprocess.Popen(
        [binary, str(port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        preexec_fn=os.setsid,  # new process group for clean kill
    )
    if not wait_for_port(port):
        proc.kill()
        raise RuntimeError(f"Server {binary} failed to start on port {port}")
    return proc


def stop_server(proc):
    """Stop a server process cleanly."""
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
    """Run test_echo.py and parse results."""
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

    # Parse output for key metrics
    lines = result.stdout.strip().split("\n")
    metrics = {}
    for line in lines:
        line = line.strip()
        if "Throughput:" in line:
            # "Throughput:     31,776 msg/s"
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


def fmt_ratio(ksvc_val, tokio_val):
    """Format as ratio: '0.85×' or '1.20×'."""
    if tokio_val == 0:
        return "N/A"
    ratio = ksvc_val / tokio_val
    return f"{ratio:.2f}x"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--build", action="store_true", help="Build before running")
    parser.add_argument("--tests", type=str, default="all",
                        help="Comma-separated test indices (0-based), or 'all'")
    args = parser.parse_args()

    build_if_needed(args.build)

    ksvc_bin = "./target/release/ksvc-echo"
    tokio_bin = "./target/release/tokio-echo"

    for b in [ksvc_bin, tokio_bin]:
        if not os.path.exists(b):
            print(f"ERROR: {b} not found. Run: cargo build --release -p ksvc-echo -p tokio-echo")
            sys.exit(1)

    # Select tests
    if args.tests == "all":
        selected = list(range(len(TESTS)))
    else:
        selected = [int(x) for x in args.tests.split(",")]

    print("=" * 78)
    print("  KSVC Echo vs Tokio Echo — Side-by-Side Benchmark")
    print("=" * 78)

    all_results = []

    for tidx in selected:
        threads, messages, size, label = TESTS[tidx]
        total = threads * messages

        print(f"\n{'─' * 78}")
        print(f"  Test {tidx}: {label}")
        print(f"  Total: {total:,} messages, {total * size / 1024:.0f} KB")
        print(f"{'─' * 78}")

        # --- Tokio ---
        print(f"  Starting tokio-echo on :{TOKIO_PORT}...", end=" ", flush=True)
        tokio_proc = start_server(tokio_bin, TOKIO_PORT)
        print("OK")

        print(f"  Running test against Tokio...", end=" ", flush=True)
        tokio_m = run_test(TOKIO_PORT, threads, messages, size)
        status = "PASS" if tokio_m.get("pass") else "FAIL"
        print(f"{status} ({tokio_m.get('msg_per_sec', 0):,.0f} msg/s)")

        stop_server(tokio_proc)
        time.sleep(0.3)  # let port release

        # --- KSVC ---
        print(f"  Starting ksvc-echo on :{KSVC_PORT}...", end=" ", flush=True)
        ksvc_proc = start_server(ksvc_bin, KSVC_PORT)
        print("OK")

        print(f"  Running test against KSVC...", end=" ", flush=True)
        ksvc_m = run_test(KSVC_PORT, threads, messages, size)
        status = "PASS" if ksvc_m.get("pass") else "FAIL"
        print(f"{status} ({ksvc_m.get('msg_per_sec', 0):,.0f} msg/s)")

        stop_server(ksvc_proc)
        time.sleep(0.3)

        all_results.append((label, ksvc_m, tokio_m))

    # ── Summary table ──
    print(f"\n{'=' * 78}")
    print(f"  RESULTS SUMMARY")
    print(f"{'=' * 78}")
    print()
    print(f"  {'Test':<38} {'Tokio':>10} {'KSVC':>10} {'Ratio':>8} {'Winner':>8}")
    print(f"  {'─' * 38} {'─' * 10} {'─' * 10} {'─' * 8} {'─' * 8}")

    for label, ksvc_m, tokio_m in all_results:
        short = label.split(":")[0].strip()

        # Throughput
        t_tput = tokio_m.get("msg_per_sec", 0)
        k_tput = ksvc_m.get("msg_per_sec", 0)
        ratio = fmt_ratio(k_tput, t_tput)
        winner = "KSVC" if k_tput > t_tput else "Tokio"
        print(f"  {short + ' msg/s':<38} {t_tput:>10,.0f} {k_tput:>10,.0f} {ratio:>8} {winner:>8}")

    print()
    print(f"  {'Latency (p99)':<38} {'Tokio':>10} {'KSVC':>10} {'Ratio':>8} {'Winner':>8}")
    print(f"  {'─' * 38} {'─' * 10} {'─' * 10} {'─' * 8} {'─' * 8}")

    for label, ksvc_m, tokio_m in all_results:
        short = label.split(":")[0].strip()
        t_p99 = tokio_m.get("lat_p99", 0)
        k_p99 = ksvc_m.get("lat_p99", 0)
        # For latency, lower is better — ratio < 1 means KSVC wins
        ratio = fmt_ratio(k_p99, t_p99) if t_p99 > 0 else "N/A"
        winner = "KSVC" if k_p99 < t_p99 else "Tokio"
        print(f"  {short:<38} {t_p99:>9.2f}ms {k_p99:>9.2f}ms {ratio:>8} {winner:>8}")

    # Notes
    print(f"\n{'─' * 78}")
    print(f"  NOTES:")
    print(f"  • Tokio: multi-threaded runtime, epoll, task-per-connection")
    print(f"  • KSVC:  single-threaded, io_uring, event loop with 50μs idle sleep")
    print(f"  • Ratio = KSVC / Tokio (throughput: >1 = KSVC wins; latency: <1 = KSVC wins)")
    print(f"  • Known KSVC bottleneck: 50μs sleep when idle, vs Tokio's epoll_wait()")
    print(f"  • Fix: replace sleep with io_uring_enter(min_complete=1) — ~3-5x improvement")
    print(f"{'─' * 78}")


if __name__ == "__main__":
    main()
