#!/usr/bin/env python3
"""
httpd-plugin.py — Plugin for HTTP server benchmarks.

Uses wrk (C-based HTTP benchmarking tool) for accurate load generation.
Falls back to Python if wrk is not installed.
Used by itests/test-runner.py.
"""

import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

# cmd/httpd/httpd-plugin.py → parent.parent.parent = repo root
ROOT_DIR = Path(__file__).resolve().parent.parent.parent
ITESTS_DIR = ROOT_DIR / "itests"
sys.path.insert(0, str(ITESTS_DIR))
from result_schema import TestResult, MetricResult

# Plugin metadata
KIND = "bench"

SERVERS = {
    "go": {
        "dir": "go",
        "build": "go build -o httpd-server .",
        "cmd": "httpd-server",
        "args": lambda port: ["--port", str(port)],
        "env": lambda port: {},
        "startup_wait_s": 0.5,
    },
    "tokio": {
        "dir": "rust/tokio",
        "build": "cargo build --release",
        "cargo_package": "tokio-httpd",
        "cmd": "target/release/tokio-httpd",
        "args": lambda port: ["--port", str(port)],
        "env": lambda port: {},
        "startup_wait_s": 0.5,
    },
    "ksvc": {
        "dir": "rust/ksvc",
        "build": "cargo build --release",
        "cargo_package": "ksvc-httpd",
        "cmd": "target/release/ksvc-httpd",
        "args": lambda port: ["--port", str(port)],
        "env": lambda port: {
            # KSVC_THREADS flows from shell → make → test-runner → server
            # Server reads it for multi-ring io_uring scaling
            k: v for k, v in {"KSVC_THREADS": os.environ.get("KSVC_THREADS", "")}.items() if v
        },
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


def _parse_latency_value(val_str, unit_str):
    """Convert wrk latency value + unit to microseconds."""
    val = float(val_str)
    if unit_str == "ms":
        val *= 1000
    elif unit_str == "s":
        val *= 1_000_000
    # "us" stays as-is
    return val


def _parse_wrk_output(output: str) -> dict:
    """
    Parse wrk output for throughput and latency distribution.

    wrk --latency output format:
        Latency Distribution
           50%   42.13us
           75%   85.20us
           90%  150.33us
           99%    1.53ms
        Latency    45.12us  120.51us   1.53ms   87.60%
        Req/Sec   115.18k    10.19k  135.46k    70.00%
        229906 requests in 10.00s, ...
        Requests/sec: 229906.12
        Transfer/sec:     27.21MB
    """
    result = {
        "throughput_rps": 0,
        "avg_us": None,
        "p50_us": None,
        "p75_us": None,
        "p90_us": None,
        "p99_us": None,
        "errors": 0,
    }

    # Requests/sec line
    m = re.search(r"Requests/sec:\s+([\d.]+)", output)
    if m:
        result["throughput_rps"] = float(m.group(1))

    # Latency Distribution percentiles (from --latency flag)
    pct_pattern = re.compile(r"(\d+)%\s+([\d.]+)(us|ms|s)")
    pct_map = {"50": "p50_us", "75": "p75_us", "90": "p90_us", "99": "p99_us"}
    for m in pct_pattern.finditer(output):
        pct = m.group(1)
        if pct in pct_map:
            result[pct_map[pct]] = _parse_latency_value(m.group(2), m.group(3))

    # Avg latency from the summary "Latency  avg  stdev  max  +/- stdev" line
    m = re.search(r"Latency\s+([\d.]+)(us|ms|s)\s+([\d.]+)(us|ms|s)", output)
    if m:
        result["avg_us"] = _parse_latency_value(m.group(1), m.group(2))

    # Socket errors
    m = re.search(
        r"Socket errors:.*?(\d+) connect.*?(\d+) read.*?(\d+) write.*?(\d+) timeout",
        output,
    )
    if m:
        result["errors"] = sum(int(x) for x in m.groups())

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
    """Fallback: Python HTTP benchmark with latency tracking."""
    import urllib.request
    import threading

    lock = threading.Lock()
    all_results = []  # list of {count, errors, latencies_ns}
    end_time = time.monotonic() + scenario["duration_s"]

    def worker():
        count = 0
        errors = 0
        latencies_ns = []
        sample_every = 1
        while time.monotonic() < end_time:
            record = (count % sample_every == 0)
            try:
                if record:
                    t0 = time.perf_counter_ns()
                urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=5)
                count += 1
                if record:
                    latencies_ns.append(time.perf_counter_ns() - t0)
            except Exception:
                errors += 1
            # Adjust sampling after warmup
            if count == 500 and scenario["duration_s"] > 0:
                elapsed = time.monotonic() - (end_time - scenario["duration_s"])
                est_total = (500 / max(elapsed, 0.001)) * scenario["duration_s"]
                sample_every = max(1, int(est_total / 5000))
        with lock:
            all_results.append({"count": count, "errors": errors, "latencies_ns": latencies_ns})

    threads = []
    for _ in range(min(scenario["connections"], 50)):
        t = threading.Thread(target=worker)
        threads.append(t)
        t.start()
    for t in threads:
        t.join(timeout=scenario["duration_s"] + 10)

    total_count = sum(r["count"] for r in all_results)
    total_errors = sum(r["errors"] for r in all_results)
    all_latencies = []
    for r in all_results:
        all_latencies.extend(r["latencies_ns"])

    pcts = _compute_percentiles(all_latencies)
    return {
        "throughput_rps": total_count / scenario["duration_s"] if scenario["duration_s"] > 0 else 0,
        "errors": total_errors,
        **pcts,
    }


def _compute_percentiles(all_latencies_ns):
    """Compute p50, p75, p90, p99, avg from nanosecond latencies."""
    if not all_latencies_ns:
        return {"p50_us": None, "p75_us": None, "p90_us": None, "p99_us": None, "avg_us": None}
    s = sorted(all_latencies_ns)
    n = len(s)
    return {
        "p50_us": s[int(n * 0.50)] / 1000.0,
        "p75_us": s[int(n * 0.75)] / 1000.0,
        "p90_us": s[int(n * 0.90)] / 1000.0,
        "p99_us": s[min(int(n * 0.99), n - 1)] / 1000.0,
        "avg_us": (sum(s) / n) / 1000.0,
    }


def run_bench(server_name: str, port: int, wrk_threads: int = 2, **kwargs) -> TestResult:
    """Run all HTTP benchmark scenarios for the given server."""
    has_wrk = _check_wrk()
    if not has_wrk:
        print("    WARNING: wrk not found — using Python fallback (results will be client-limited)")

    # Capture KSVC server thread count for metadata
    ksvc_threads = int(os.environ.get("KSVC_THREADS", "1")) if server_name == "ksvc" else 0

    result = TestResult(test_type="httpd", server=server_name)
    if ksvc_threads > 1:
        result.metadata["ksvc_threads"] = ksvc_threads

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
            p50_us=bench.get("p50_us"),
            p99_us=bench.get("p99_us"),
            duration_s=scenario["duration_s"],
            threads=ksvc_threads if server_name == "ksvc" else 0,
            extra={
                "wrk_threads": scenario["threads"],
                "connections": scenario["connections"],
                "errors": bench.get("errors", 0),
                "client": "wrk" if has_wrk else "python",
                "p75_us": bench.get("p75_us"),
                "p90_us": bench.get("p90_us"),
                "avg_us": bench.get("avg_us"),
            },
        )
        result.add_metric(m)
        p50 = bench.get("p50_us")
        p99 = bench.get("p99_us")
        lat_str = f"p50={p50:,.0f}μs p99={p99:,.0f}μs" if p50 and p99 else "no latency data"
        print(f"    [{server_name}]   {bench['throughput_rps']:,.0f} req/s  "
              f"{lat_str}  ({bench.get('errors', 0)} errors)")

    return result