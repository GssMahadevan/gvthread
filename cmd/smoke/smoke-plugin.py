#!/usr/bin/env python3
"""
smoke-plugin.py — Plugin for KSVC smoke tests (correctness, not performance).

Runs cmd/smoke/ksvc binary which executes all 33 smoke tests internally.
Parses output for pass/fail counts.
Used by itests/test-runner.py.
"""

import re
import subprocess
import sys
from pathlib import Path

ITESTS_DIR = Path(__file__).resolve().parent.parent.parent / "itests"
ROOT_DIR = ITESTS_DIR.parent
sys.path.insert(0, str(ITESTS_DIR))
from result_schema import TestResult, MetricResult

# Plugin metadata
KIND = "smoke"  # This is a correctness test, not a benchmark

SERVERS = {
    "ksvc": {
        "dir": "ksvc",
        "build": "cargo build --release",
        "cargo_package": "ksvc-smoke",
        "cmd": "target/release/ksvc-smoke",
        "args": [],
    },
}


def run_smoke(server_name: str, **kwargs) -> TestResult:
    """
    Run ksvc-smoke binary and parse its output.
    Expected output format per test line:
        [PASS] Test 01: Kernel module loaded
        [FAIL] Test 19: statx read ...
    And a summary:
        Result: 33/33 PASS
    """
    result = TestResult(test_type="smoke", server=server_name)

    binary = str(ROOT_DIR / SERVERS[server_name]["cmd"])
    try:
        proc = subprocess.run(
            [binary],
            capture_output=True, text=True, timeout=120
        )
        output = proc.stdout + proc.stderr
    except subprocess.TimeoutExpired:
        result.passed = False
        result.add_metric(MetricResult(
            name="timeout",
            throughput_rps=0,
            extra={"passed": False, "error": "Smoke test timed out after 120s"},
        ))
        return result
    except FileNotFoundError:
        result.passed = False
        result.add_metric(MetricResult(
            name="not_found",
            throughput_rps=0,
            extra={"passed": False, "error": f"Binary not found: {binary}"},
        ))
        return result

    # Parse individual test lines
    test_pattern = re.compile(r"\[(PASS|FAIL)\]\s*(.*)")
    for line in output.splitlines():
        m = test_pattern.search(line)
        if m:
            status = m.group(1)
            desc = m.group(2).strip()
            result.add_metric(MetricResult(
                name=desc[:60],
                throughput_rps=0,
                extra={"passed": status == "PASS"},
            ))

    # Parse summary line
    summary = re.search(r"(\d+)/(\d+)\s+PASS", output)
    if summary:
        passed = int(summary.group(1))
        total = int(summary.group(2))
        result.passed = (passed == total)
        result.metadata["passed_count"] = passed
        result.metadata["total_count"] = total
    else:
        # If no summary found, infer from individual tests
        all_passed = all(
            t.get("extra", {}).get("passed", False) for t in result.tests
        )
        result.passed = all_passed and len(result.tests) > 0

    result.metadata["exit_code"] = proc.returncode
    if proc.returncode != 0:
        result.passed = False

    return result