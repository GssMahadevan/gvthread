#!/usr/bin/env python3
"""
KSVC vs Tokio vs Go — HTTP Benchmark (using wrk)

Usage:
    python3 cmd/ksvc-httpd/bench_http.py [--build] [--duration 10] [--threads 4]

Requires: wrk installed (sudo apt install wrk)
"""

import subprocess
import time
import sys
import os
import signal
import socket
import argparse
import re

# Server definitions: (name, binary, default_port, build_cmd, extra_args)
SERVERS = [
    ("Go",    "./cmd/go-httpd/go-httpd",      8082,
     ["go", "build", "-o", "./cmd/go-httpd/go-httpd", "./cmd/go-httpd/main.go"],
     ["--port"]),
    ("Tokio", "./target/release/tokio-httpd",  8081,
     ["cargo", "build", "--release", "-p", "tokio-httpd"],
     ["--port"]),
    ("KSVC",  "./target/release/ksvc-httpd",   8080,
     ["cargo", "build", "--release", "-p", "ksvc-httpd"],
     ["--port"]),
]

# wrk test configurations: (connections, threads, label)
WRK_TESTS = [
    (10,   2,  "light:   10 conns, 2 threads"),
    (50,   4,  "medium:  50 conns, 4 threads"),
    (100,  4,  "heavy:   100 conns, 4 threads"),
    (500,  4,  "storm:   500 conns, 4 threads"),
    (1000, 4,  "blast:   1000 conns, 4 threads"),
]


def check_wrk():
    r = subprocess.run(["which", "wrk"], capture_output=True)
    if r.returncode != 0:
        print("ERROR: wrk not found. Install with: sudo apt install wrk")
        sys.exit(1)


def build_all():
    for name, binary, _, cmd, _ in SERVERS:
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


def start_server(binary, port, port_arg_style):
    """Start server with port argument."""
    cmd = [binary, port_arg_style, str(port)]
    proc = subprocess.Popen(
        cmd,
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
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait(timeout=2)


def run_wrk(port, connections, threads, duration):
    """Run wrk and parse results."""
    url = f"http://127.0.0.1:{port}/"
    result = subprocess.run(
        ["wrk", f"-t{threads}", f"-c{connections}", f"-d{duration}s", "--latency", url],
        capture_output=True, text=True,
        timeout=duration + 30,
    )

    output = result.stdout
    metrics = {
        "req_per_sec": 0.0,
        "transfer_per_sec": "",
        "lat_avg": "",
        "lat_p50": "",
        "lat_p90": "",
        "lat_p99": "",
        "total_requests": 0,
        "errors": 0,
        "raw": output,
    }

    for line in output.split("\n"):
        line = line.strip()

        # "Requests/sec:  45678.90"
        m = re.match(r'Requests/sec:\s+([\d.]+)', line)
        if m:
            metrics["req_per_sec"] = float(m.group(1))

        # "Transfer/sec:      5.23MB"
        m = re.match(r'Transfer/sec:\s+(.+)', line)
        if m:
            metrics["transfer_per_sec"] = m.group(1).strip()

        # "    Avg     Stdev   Max   +/- Stdev"
        # "  123.45us  67.89us  5.00ms  78.90%"
        # Latency line (after "Latency" header)
        m = re.match(r'Latency\s+([\d.]+\w+)\s+([\d.]+\w+)\s+([\d.]+\w+)', line)
        if m:
            metrics["lat_avg"] = m.group(1)

        # Percentile lines: "50%  123.00us"
        m = re.match(r'50%\s+([\d.]+\w+)', line)
        if m:
            metrics["lat_p50"] = m.group(1)
        m = re.match(r'90%\s+([\d.]+\w+)', line)
        if m:
            metrics["lat_p90"] = m.group(1)
        m = re.match(r'99%\s+([\d.]+\w+)', line)
        if m:
            metrics["lat_p99"] = m.group(1)

        # "12345 requests in 10.00s"
        m = re.match(r'(\d+)\s+requests?\s+in', line)
        if m:
            metrics["total_requests"] = int(m.group(1))

        # Socket/read errors
        m = re.match(r'Socket errors:.*', line)
        if m:
            nums = re.findall(r'\d+', line)
            metrics["errors"] = sum(int(n) for n in nums)

        # Non-2xx
        m = re.match(r'Non-2xx.*?(\d+)', line)
        if m:
            metrics["errors"] += int(m.group(1))

    return metrics


def parse_latency_us(s):
    """Convert wrk latency string to microseconds for comparison."""
    if not s:
        return 0
    s = s.strip()
    if s.endswith("ms"):
        return float(s[:-2]) * 1000
    elif s.endswith("us"):
        return float(s[:-2])
    elif s.endswith("s"):
        return float(s[:-1]) * 1000000
    return 0


def fmt_rps(val):
    if val >= 1000:
        return f"{val:,.0f}"
    return f"{val:.1f}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--build", action="store_true")
    parser.add_argument("--duration", "-d", type=int, default=10, help="wrk duration in seconds")
    parser.add_argument("--tests", type=str, default="all")
    parser.add_argument("--warmup", type=int, default=2, help="Warmup seconds before each test")
    args = parser.parse_args()

    check_wrk()

    if args.build:
        print("Building servers...")
        build_all()

    for name, binary, _, _, _ in SERVERS:
        if not os.path.exists(binary):
            print(f"ERROR: {binary} not found. Run with --build")
            sys.exit(1)

    if args.tests == "all":
        selected = list(range(len(WRK_TESTS)))
    else:
        selected = [int(x) for x in args.tests.split(",")]

    W = 92
    print("=" * W)
    print(f"  KSVC vs Tokio vs Go — HTTP Benchmark (wrk, {args.duration}s per test)")
    print("=" * W)

    all_results = []

    for tidx in selected:
        conns, threads, label = WRK_TESTS[tidx]

        print(f"\n{'─' * W}")
        print(f"  Test {tidx}: {label}")
        print(f"{'─' * W}")

        test_results = {}

        for name, binary, port, _, port_arg in SERVERS:
            print(f"  {name:<6}", end=" ", flush=True)

            proc = start_server(binary, port, port_arg)

            # Warmup
            if args.warmup > 0:
                print(f"warmup({args.warmup}s)...", end=" ", flush=True)
                run_wrk(port, min(conns, 20), 2, args.warmup)

            # Actual test
            print(f"running({args.duration}s)...", end=" ", flush=True)
            m = run_wrk(port, conns, threads, args.duration)

            rps = m["req_per_sec"]
            p99 = m["lat_p99"]
            errs = m["errors"]
            print(f"{rps:>12,.0f} req/s  p99={p99:>10}  err={errs}")

            stop_server(proc)
            time.sleep(0.5)
            test_results[name] = m

        all_results.append((label, test_results))

    # ── Summary: Throughput ──
    print(f"\n{'=' * W}")
    print(f"  THROUGHPUT (req/s) — higher is better")
    print(f"{'=' * W}")
    print()
    print(f"  {'Test':<20} {'Go':>14} {'Tokio':>14} {'KSVC':>14}  {'Best':>6}")
    print(f"  {'─' * 20} {'─' * 14} {'─' * 14} {'─' * 14}  {'─' * 6}")

    for label, results in all_results:
        short = label.split(":")[0].strip()
        vals = {}
        for name in ["Go", "Tokio", "KSVC"]:
            vals[name] = results.get(name, {}).get("req_per_sec", 0)
        best = max(vals, key=vals.get)
        print(f"  {short:<20} {vals['Go']:>14,.0f} {vals['Tokio']:>14,.0f} {vals['KSVC']:>14,.0f}  {best:>6}")

    # ── Summary: Latency p99 ──
    print()
    print(f"{'=' * W}")
    print(f"  LATENCY p99 — lower is better")
    print(f"{'=' * W}")
    print()
    print(f"  {'Test':<20} {'Go':>14} {'Tokio':>14} {'KSVC':>14}  {'Best':>6}")
    print(f"  {'─' * 20} {'─' * 14} {'─' * 14} {'─' * 14}  {'─' * 6}")

    for label, results in all_results:
        short = label.split(":")[0].strip()
        vals = {}
        strs = {}
        for name in ["Go", "Tokio", "KSVC"]:
            s = results.get(name, {}).get("lat_p99", "N/A")
            strs[name] = s
            vals[name] = parse_latency_us(s)
        nonzero = {k: v for k, v in vals.items() if v > 0}
        best = min(nonzero, key=nonzero.get) if nonzero else "N/A"
        print(f"  {short:<20} {strs['Go']:>14} {strs['Tokio']:>14} {strs['KSVC']:>14}  {best:>6}")

    # ── Summary: Latency p50 ──
    print()
    print(f"{'=' * W}")
    print(f"  LATENCY p50 — lower is better")
    print(f"{'=' * W}")
    print()
    print(f"  {'Test':<20} {'Go':>14} {'Tokio':>14} {'KSVC':>14}  {'Best':>6}")
    print(f"  {'─' * 20} {'─' * 14} {'─' * 14} {'─' * 14}  {'─' * 6}")

    for label, results in all_results:
        short = label.split(":")[0].strip()
        vals = {}
        strs = {}
        for name in ["Go", "Tokio", "KSVC"]:
            s = results.get(name, {}).get("lat_p50", "N/A")
            strs[name] = s
            vals[name] = parse_latency_us(s)
        nonzero = {k: v for k, v in vals.items() if v > 0}
        best = min(nonzero, key=nonzero.get) if nonzero else "N/A"
        print(f"  {short:<20} {strs['Go']:>14} {strs['Tokio']:>14} {strs['KSVC']:>14}  {best:>6}")

    # ── Notes ──
    print(f"\n{'─' * W}")
    print(f"  ARCHITECTURE:")
    print(f"  Go:    net/http (goroutine-per-conn, epoll, M:N scheduling)")
    print(f"  Tokio: async tasks (multi-threaded, epoll via mio)")
    print(f"  KSVC:  single-threaded event loop (io_uring, flush_and_wait)")
    print(f"")
    print(f"  SYSCALLS PER HTTP REQUEST (steady state with keep-alive):")
    print(f"  Go:    epoll_wait + read + write  (3 syscalls/req)")
    print(f"  Tokio: epoll_ctl + epoll_wait + read + write  (3-4 syscalls/req)")
    print(f"  KSVC:  io_uring_enter per BATCH  (1 syscall for N reqs)")
    print(f"{'─' * W}")


if __name__ == "__main__":
    main()
