#!/usr/bin/env python3
"""
httpd-plugin.py — Plugin for HTTP server benchmarks.

Uses wrk (C-based HTTP benchmarking tool) for accurate load generation.
Falls back to Python if wrk is not installed.
Used by itests/test-runner.py.
"""

import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ITESTS_DIR = Path(__file__).resolve().parent.parent / "itests"
sys.path.insert(0, str(ITESTS_DIR))
from result_schema import TestResult, MetricResult

# Plugin metadata
KIND = "bench"

SERVERS = {
    "go": {
        "dir": "go",
        "build": "go build -o httpd-server .",
        "cmd": "httpd-server",
        "args": lambda port: [f"-port={port}"],
        "env": lambda port: {},
        "startup_wait_s": 0.5,
    },
    "tokio": {
        "dir": "rust/tokio",
        "build": "cargo build --release",
        "cargo_package": "tokio-httpd",
        "cmd": "target/release/tokio-httpd",
        "args": lambda port: [f"--port={port}"],
        "env": lambda port: {"PORT": str(port)},
        "startup_wait_s": 0.5,
    },
    "ksvc": {
        "dir": "rust/ksvc",
        "build": "cargo build --release",
        "cargo_package": "ksvc-httpd",
        "cmd": "target/release/ksvc-httpd",
        "args": lambda port: [f"--port={port}"],
        "env": lambda port: {"PORT": str(port)},
        "startup_wait_s": 0.5,
    },
}


# ──────────────────────────────────────────────────────────────
# Benchmark scenarios
# ──────────────────────────────────────────────────────────────

SCENARIOS = [
    {"name": "light",  "threads": 2,  "connections": 10,  "duration_s": 10},
    {"name": "medium", "threads": 4,  "connections": 50,  "duration_s": 10},
    {"name": "heavy",  "threads": 8,  "connections": 100, "duration_s": 10},
    {"name": "storm",  "threads": 16, "connections": 100, "duration_s": 10},
    {"name": "max",    "threads": 16, "connections": 200, "duration_s": 10},
]


def _check_wrk():
    """Check if wrk is available."""
    return shutil.which("wrk") is not None


def _parse_wrk_output(output: str) -> dict:
    """
    Parse wrk output for throughput and latency.
    Example wrk output:
        Latency    42.13us  120.51us   1.53ms   87.60%
        Req/Sec   115.18k    10.19k  135.46k    70.00%
        229906 requests in 10.00s, ...
        Requests/sec: 229906.12
        Transfer/sec:     27.21MB
    """
    result = {"throughput_rps": 0, "avg_latency_us": 0, "p99_us": 0}

    # Requests/sec line
    m = re.search(r"Requests/sec:\s+([\d.]+)", output)
    if m:
        result["throughput_rps"] = float(m.group(1))

    # Latency line (avg)
    m = re.search(r"Latency\s+([\d.]+)(us|ms|s)", output)
    if m:
        val = float(m.group(1))
        unit = m.group(2)
        if unit == "ms":
            val *= 1000
        elif unit == "s":
            val *= 1_000_000
        result["avg_latency_us"] = val

    # Try to find latency distribution if --latency flag was used
    # 99%   1.53ms
    m = re.search(r"99%\s+([\d.]+)(us|ms|s)", output)
    if m:
        val = float(m.group(1))
        unit = m.group(2)
        if unit == "ms":
            val *= 1000
        elif unit == "s":
            val *= 1_000_000
        result["p99_us"] = val

    # Socket errors
    m = re.search(r"Socket errors:.*?(\d+) connect.*?(\d+) read.*?(\d+) write.*?(\d+) timeout", output)
    if m:
        result["errors"] = sum(int(x) for x in m.groups())
    else:
        result["errors"] = 0

    return result


def _run_wrk(port: int, scenario: dict) -> dict:
    """Run wrk against the server."""
    cmd = [
        "wrk",
        f"-t{scenario['threads']}",
        f"-c{scenario['connections']}",
        f"-d{scenario['duration_s']}s",
        "--latency",
        f"http://127.0.0.1:{port}/",
    ]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=scenario["duration_s"] + 30)
    return _parse_wrk_output(result.stdout + result.stderr)


def _run_python_http_bench(port: int, scenario: dict) -> dict:
    """Fallback: simple Python HTTP benchmark (much lower throughput than wrk)."""
    import urllib.request
    import threading

    total_count = 0
    total_errors = 0
    lock = threading.Lock()
    end_time = time.monotonic() + scenario["duration_s"]

    def worker():
        nonlocal total_count, total_errors
        count = 0
        errors = 0
        while time.monotonic() < end_time:
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=5)
                count += 1
            except Exception:
                errors += 1
        with lock:
            total_count += count
            total_errors += errors

    threads = []
    for _ in range(min(scenario["connections"], 50)):
        t = threading.Thread(target=worker)
        threads.append(t)
        t.start()
    for t in threads:
        t.join(timeout=scenario["duration_s"] + 10)

    return {
        "throughput_rps": total_count / scenario["duration_s"],
        "errors": total_errors,
        "avg_latency_us": 0,
        "p99_us": 0,
    }


def run_bench(server_name: str, port: int, wrk_threads: int = 2, **kwargs) -> TestResult:
    """Run all HTTP benchmark scenarios for the given server."""
    has_wrk = _check_wrk()
    if not has_wrk:
        print("    WARNING: wrk not found — using Python fallback (results will be client-limited)")

    result = TestResult(test_type="httpd", server=server_name)

    for scenario in SCENARIOS:
        # Override wrk threads from CLI if specified
        if wrk_threads and has_wrk:
            scenario = {**scenario, "threads": wrk_threads}

        print(f"    [{server_name}] Scenario: {scenario['name']} "
              f"({scenario['threads']}t/{scenario['connections']}c, {scenario['duration_s']}s) ...")

        if has_wrk:
            bench = _run_wrk(port, scenario)
        else:
            bench = _run_python_http_bench(port, scenario)

        m = MetricResult(
            name=scenario["name"],
            throughput_rps=bench["throughput_rps"],
            p50_us=bench.get("avg_latency_us"),
            p99_us=bench.get("p99_us"),
            duration_s=scenario["duration_s"],
            threads=1 if server_name == "ksvc" else 0,
            extra={
                "wrk_threads": scenario["threads"],
                "connections": scenario["connections"],
                "errors": bench.get("errors", 0),
                "client": "wrk" if has_wrk else "python",
            },
        )
        result.add_metric(m)
        print(f"    [{server_name}]   {bench['throughput_rps']:,.0f} req/s  "
              f"p99={bench.get('p99_us', 0):.0f}μs  "
              f"({bench.get('errors', 0)} errors)")

    return result