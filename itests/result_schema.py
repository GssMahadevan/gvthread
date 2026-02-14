#!/usr/bin/env python3
"""
result_schema.py — Shared result format for all itest plugins.

Every plugin emits TestResult objects. test-runner.py collects them,
serializes to JSON, and optionally compares against baselines.
"""

import json
import os
import sys
from dataclasses import dataclass, field, asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

RESULTS_DIR = Path(__file__).resolve().parent.parent / "results"


@dataclass
class MetricResult:
    """One benchmark measurement (e.g., 'light_4conn')."""
    name: str
    throughput_rps: float
    p50_us: Optional[float] = None
    p99_us: Optional[float] = None
    cpu_pct: Optional[float] = None
    threads: int = 1
    duration_s: float = 10.0
    extra: dict = field(default_factory=dict)


@dataclass
class TestResult:
    """Full result from one server in one test-type."""
    test_type: str          # "echo", "httpd", "smoke"
    server: str             # "ksvc", "tokio", "go"
    timestamp: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    passed: bool = True
    tests: list = field(default_factory=list)   # list of MetricResult dicts
    metadata: dict = field(default_factory=dict) # git sha, hostname, etc.

    def add_metric(self, m: MetricResult):
        self.tests.append(asdict(m))

    def to_dict(self):
        return asdict(self)

    def save(self, base_dir: Optional[Path] = None):
        """Save to results/{test_type}/{server}/{timestamp}.json"""
        base = base_dir or RESULTS_DIR
        out_dir = base / self.test_type / self.server
        out_dir.mkdir(parents=True, exist_ok=True)
        # Use a filesystem-safe timestamp
        ts = self.timestamp.replace(":", "-").replace("+", "_")
        path = out_dir / f"{ts}.json"
        with open(path, "w") as f:
            json.dump(self.to_dict(), f, indent=2)
        return path

    @classmethod
    def load(cls, path: Path) -> "TestResult":
        with open(path) as f:
            d = json.load(f)
        r = cls(
            test_type=d["test_type"],
            server=d["server"],
            timestamp=d["timestamp"],
            passed=d.get("passed", True),
            tests=d.get("tests", []),
            metadata=d.get("metadata", {}),
        )
        return r


def get_baseline(test_type: str, server: str, base_dir: Optional[Path] = None) -> Optional[TestResult]:
    """Load the baseline.json for a given test_type/server, if it exists."""
    base = base_dir or RESULTS_DIR
    path = base / test_type / server / "baseline.json"
    if path.exists():
        return TestResult.load(path)
    return None


def compare_results(current: TestResult, baseline: TestResult,
                    threshold_pct: float = 5.0) -> list:
    """
    Compare current vs baseline. Returns list of regressions.
    Each regression is a dict: {name, metric, current, baseline, delta_pct}.
    """
    baseline_by_name = {t["name"]: t for t in baseline.tests}
    regressions = []
    for test in current.tests:
        name = test["name"]
        if name not in baseline_by_name:
            continue
        bl = baseline_by_name[name]
        # Check throughput regression
        cur_rps = test.get("throughput_rps", 0)
        bl_rps = bl.get("throughput_rps", 0)
        if bl_rps > 0:
            delta = ((cur_rps - bl_rps) / bl_rps) * 100
            if delta < -threshold_pct:
                regressions.append({
                    "name": name,
                    "metric": "throughput_rps",
                    "current": cur_rps,
                    "baseline": bl_rps,
                    "delta_pct": round(delta, 2),
                })
        # Check p99 regression (higher is worse)
        cur_p99 = test.get("p99_us")
        bl_p99 = bl.get("p99_us")
        if cur_p99 and bl_p99 and bl_p99 > 0:
            delta = ((cur_p99 - bl_p99) / bl_p99) * 100
            if delta > threshold_pct:
                regressions.append({
                    "name": name,
                    "metric": "p99_us",
                    "current": cur_p99,
                    "baseline": bl_p99,
                    "delta_pct": round(delta, 2),
                })
    return regressions


def print_comparison_table(results: dict, test_type: str):
    """
    Pretty-print a comparison table across servers.
    results: {server_name: TestResult}
    """
    if not results:
        print("No results to compare.")
        return

    # Collect all test names (union across servers)
    servers = sorted(results.keys())
    all_names = []
    for r in results.values():
        for t in r.tests:
            if t["name"] not in all_names:
                all_names.append(t["name"])

    # Header
    hdr = f"{'Test':<25}"
    for s in servers:
        hdr += f"  {s:>12} req/s"
    print(f"\n{'='*len(hdr)}")
    print(f"  {test_type.upper()} Benchmark Comparison")
    print(f"{'='*len(hdr)}")
    print(hdr)
    print("-" * len(hdr))

    # Rows
    for name in all_names:
        row = f"{name:<25}"
        vals = {}
        for s in servers:
            by_name = {t["name"]: t for t in results[s].tests}
            v = by_name.get(name, {}).get("throughput_rps")
            vals[s] = v
            row += f"  {v:>12,.0f}    " if v else f"  {'N/A':>12}    "
        # Winner
        valid = {s: v for s, v in vals.items() if v}
        if valid:
            winner = max(valid, key=valid.get)
            row += f"  << {winner}"
        print(row)

    print("-" * len(hdr))
    ts = next(iter(results.values())).timestamp
    print(f"  Timestamp: {ts}\n")