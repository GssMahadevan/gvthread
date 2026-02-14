#!/usr/bin/env python3
"""
echo-plugin.py — Plugin for echo server benchmarks.

Defines how to build, run, and benchmark echo servers (Go, Tokio, KSVC).
Used by itests/test-runner.py.
"""

import re
import socket
import subprocess
import sys
import time
import threading
from pathlib import Path

# Add itests/ to path for result_schema
# cmd/echo/echo-plugin.py → parent.parent.parent = repo root
ROOT_DIR = Path(__file__).resolve().parent.parent.parent
ITESTS_DIR = ROOT_DIR / "itests"
sys.path.insert(0, str(ITESTS_DIR))
from result_schema import TestResult, MetricResult

# Plugin metadata
KIND = "bench"  # "bench" or "smoke"

# Server definitions: how to build and run each echo server
SERVERS = {
    "go": {
        "dir": "go",
        "build": "go build -o echo-server .",
        "cmd": "echo-server",
        "args": lambda port: [str(port)],
        "env": lambda port: {},
        "startup_wait_s": 0.5,
    },
    "tokio": {
        "dir": "rust/tokio",
        "build": "cargo build --release",
        "cargo_package": "tokio-echo",
        "cmd": "target/release/tokio-echo",
        "args": lambda port: [str(port)],
        "env": lambda port: {},
        "startup_wait_s": 0.5,
    },
    "ksvc": {
        "dir": "rust/ksvc",
        "build": "cargo build --release",
        "cargo_package": "ksvc-echo",
        "cmd": "target/release/ksvc-echo",
        "args": lambda port: [str(port)],
        "env": lambda port: {},
        "startup_wait_s": 0.5,
    },
}


# ──────────────────────────────────────────────────────────────
# Benchmark scenarios
# ──────────────────────────────────────────────────────────────

SCENARIOS = [
    {"name": "light_4conn",    "connections": 4,   "msg_size": 64,   "duration_s": 10},
    {"name": "medium_50conn",  "connections": 50,  "msg_size": 64,   "duration_s": 10},
    {"name": "heavy_100conn",  "connections": 100, "msg_size": 64,   "duration_s": 10},
    {"name": "bulk_4kb",       "connections": 4,   "msg_size": 4096, "duration_s": 10},
    {"name": "storm_200conn",  "connections": 200, "msg_size": 64,   "duration_s": 10},
]


def _tcp_echo_client(host, port, msg_size, duration_s, results_slot, index):
    """Single-threaded TCP echo client. Measures per-request round-trip latency."""
    payload = b"X" * msg_size
    count = 0
    errors = 0
    latencies_ns = []  # nanosecond round-trip times (sampled)
    sample_every = 1   # adjusted after warmup
    end_time = time.monotonic() + duration_s

    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.settimeout(5.0)
        sock.connect((host, port))

        while time.monotonic() < end_time:
            record = (count % sample_every == 0)

            if record:
                t0 = time.perf_counter_ns()

            sock.sendall(payload)
            resp = b""
            while len(resp) < msg_size:
                chunk = sock.recv(msg_size - len(resp))
                if not chunk:
                    break
                resp += chunk

            if resp == payload:
                count += 1
                if record:
                    latencies_ns.append(time.perf_counter_ns() - t0)
            else:
                errors += 1

            # After 1000 requests, adjust sampling rate to cap at ~10K samples
            if count == 1000 and duration_s > 0:
                est_total = (1000 / max(time.monotonic() - (end_time - duration_s), 0.001)) * duration_s
                sample_every = max(1, int(est_total / 10000))

        sock.close()
    except Exception as e:
        errors += 1

    results_slot[index] = {"count": count, "errors": errors, "latencies_ns": latencies_ns}


def _compute_percentiles(all_latencies_ns):
    """Compute p50, p90, p99, avg from a list of nanosecond latencies."""
    if not all_latencies_ns:
        return {"p50_us": None, "p90_us": None, "p99_us": None, "avg_us": None}
    s = sorted(all_latencies_ns)
    n = len(s)
    return {
        "p50_us": s[int(n * 0.50)] / 1000.0,
        "p90_us": s[int(n * 0.90)] / 1000.0,
        "p99_us": s[min(int(n * 0.99), n - 1)] / 1000.0,
        "avg_us": (sum(s) / n) / 1000.0,
    }


def _run_python_echo_bench(port: int, scenario: dict) -> dict:
    """
    Run echo benchmark using Python threaded clients.
    Returns {throughput_rps, errors, duration_s, p50_us, p90_us, p99_us, avg_us}.
    """
    conns = scenario["connections"]
    msg_size = scenario["msg_size"]
    duration_s = scenario["duration_s"]

    results_slots = [None] * conns
    threads = []
    for i in range(conns):
        t = threading.Thread(
            target=_tcp_echo_client,
            args=("127.0.0.1", port, msg_size, duration_s, results_slots, i),
        )
        threads.append(t)

    start = time.monotonic()
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=duration_s + 10)
    elapsed = time.monotonic() - start

    total_count = sum(r["count"] for r in results_slots if r)
    total_errors = sum(r["errors"] for r in results_slots if r)
    rps = total_count / elapsed if elapsed > 0 else 0

    # Merge latency samples from all connections
    all_latencies = []
    for r in results_slots:
        if r and r.get("latencies_ns"):
            all_latencies.extend(r["latencies_ns"])
    pcts = _compute_percentiles(all_latencies)

    return {
        "throughput_rps": rps,
        "errors": total_errors,
        "duration_s": elapsed,
        "samples": len(all_latencies),
        **pcts,
    }


def run_bench(server_name: str, port: int, wrk_threads: int = 2, **kwargs) -> TestResult:
    """
    Run all echo benchmark scenarios for the given server.
    Called by test-runner.py.
    """
    result = TestResult(test_type="echo", server=server_name)

    for scenario in SCENARIOS:
        print(f"    [{server_name}] Scenario: {scenario['name']} "
              f"({scenario['connections']} conns, {scenario['msg_size']}B) ...")

        bench = _run_python_echo_bench(port, scenario)

        m = MetricResult(
            name=scenario["name"],
            throughput_rps=bench["throughput_rps"],
            p50_us=bench.get("p50_us"),
            p99_us=bench.get("p99_us"),
            duration_s=bench["duration_s"],
            threads=1 if server_name == "ksvc" else 0,  # 0 = multi/unknown
            extra={
                "connections": scenario["connections"],
                "msg_size": scenario["msg_size"],
                "errors": bench["errors"],
                "p90_us": bench.get("p90_us"),
                "avg_us": bench.get("avg_us"),
                "latency_samples": bench.get("samples", 0),
            },
        )
        result.add_metric(m)
        p50 = bench.get("p50_us")
        p99 = bench.get("p99_us")
        lat_str = f"p50={p50:,.0f}μs p99={p99:,.0f}μs" if p50 and p99 else "no latency data"
        print(f"    [{server_name}]   {bench['throughput_rps']:,.0f} msg/s  "
              f"{lat_str}  ({bench['errors']} errors)")

    return result