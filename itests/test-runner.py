#!/usr/bin/env python3
"""
test-runner.py — Integration test orchestrator for GVThread.

Discovers cmd/{test_type}/{test_type}-plugin.py files, imports them,
and runs the build → start → bench → stop → compare cycle.

Usage:
    python3 itests/test-runner.py echo                  # run echo benchmarks
    python3 itests/test-runner.py httpd --servers ksvc   # only KSVC httpd
    python3 itests/test-runner.py --all                  # every test type
    python3 itests/test-runner.py echo --baseline        # compare vs baseline
    python3 itests/test-runner.py echo --save-baseline   # save current as baseline
    python3 itests/test-runner.py echo --list            # list available servers
    python3 itests/test-runner.py smoke                  # run smoke (pass/fail only)
"""

import argparse
import importlib.util
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

# Ensure itests/ is on the path for result_schema
ITESTS_DIR = Path(__file__).resolve().parent
ROOT_DIR = ITESTS_DIR.parent
CMD_DIR = ROOT_DIR / "cmd"
sys.path.insert(0, str(ITESTS_DIR))

from result_schema import (
    TestResult, compare_results, get_baseline,
    print_comparison_table, RESULTS_DIR
)


def discover_plugins() -> dict:
    """
    Scan cmd/ for directories containing a {name}-plugin.py.
    Returns {test_type: plugin_module}.
    """
    plugins = {}
    if not CMD_DIR.is_dir():
        return plugins
    for d in sorted(CMD_DIR.iterdir()):
        if not d.is_dir():
            continue
        plugin_file = d / f"{d.name}-plugin.py"
        if plugin_file.exists():
            spec = importlib.util.spec_from_file_location(
                f"plugin_{d.name}", str(plugin_file)
            )
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
            plugins[d.name] = mod
    return plugins


def build_server(test_type: str, server_name: str, server_cfg: dict) -> bool:
    """Build a single server. Returns True on success."""
    server_dir = CMD_DIR / test_type / server_cfg["dir"]
    print(f"  [{test_type}/{server_name}] Building in {server_dir} ...")

    build_cmd = server_cfg["build"]
    # For cargo builds, run from repo root so workspace resolution works
    if build_cmd.startswith("cargo"):
        cwd = ROOT_DIR
        # Add -p package if specified
        pkg = server_cfg.get("cargo_package")
        if pkg and f"-p {pkg}" not in build_cmd:
            build_cmd += f" -p {pkg}"
    else:
        cwd = server_dir

    result = subprocess.run(
        build_cmd, shell=True, cwd=str(cwd),
        capture_output=True, text=True
    )
    if result.returncode != 0:
        print(f"  [{test_type}/{server_name}] BUILD FAILED:")
        print(result.stderr[-500:] if len(result.stderr) > 500 else result.stderr)
        return False
    print(f"  [{test_type}/{server_name}] Build OK")
    return True


def resolve_binary(test_type: str, server_cfg: dict) -> str:
    """Resolve the server binary path (handles cargo target/ directory)."""
    cmd = server_cfg["cmd"]
    if cmd.startswith("target/"):
        return str(ROOT_DIR / cmd)
    return str(CMD_DIR / test_type / server_cfg["dir"] / cmd)


def start_server(test_type: str, server_name: str, server_cfg: dict,
                 port: int) -> subprocess.Popen:
    """Start a server process. Returns the Popen handle."""
    binary = resolve_binary(test_type, server_cfg)
    args = server_cfg.get("args", [])
    if callable(args):
        args = args(port=port)
    elif isinstance(args, str):
        args = args.format(port=port).split()

    cmd_line = [binary] + args
    env = os.environ.copy()
    env["PORT"] = str(port)
    # Allow plugin to set extra env
    extra_env = server_cfg.get("env", {})
    if callable(extra_env):
        extra_env = extra_env(port=port)
    env.update(extra_env)

    print(f"  [{test_type}/{server_name}] Starting on port {port}: {' '.join(cmd_line)}")
    proc = subprocess.Popen(
        cmd_line, env=env,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    # Wait for server to be ready
    startup_wait = server_cfg.get("startup_wait_s", 1.0)
    time.sleep(startup_wait)
    if proc.poll() is not None:
        stderr = proc.stderr.read().decode(errors="replace")
        print(f"  [{test_type}/{server_name}] Server exited immediately! stderr: {stderr[-300:]}")
        return None
    return proc


def stop_server(proc: subprocess.Popen, server_name: str):
    """Gracefully stop a server process."""
    if proc is None:
        return
    try:
        proc.send_signal(signal.SIGTERM)
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    print(f"  [{server_name}] Stopped (pid={proc.pid})")


def run_benchmarks(plugin, test_type: str, servers: list, port_base: int,
                   wrk_threads: int = 2) -> dict:
    """
    Run benchmarks for all requested servers in a test-type.
    Returns {server_name: TestResult}.
    """
    all_servers = plugin.SERVERS
    results = {}

    for i, server_name in enumerate(servers):
        if server_name not in all_servers:
            print(f"  WARNING: Unknown server '{server_name}', skipping. "
                  f"Available: {list(all_servers.keys())}")
            continue

        cfg = all_servers[server_name]
        port = port_base + i

        # Build
        if not build_server(test_type, server_name, cfg):
            continue

        # Start
        proc = start_server(test_type, server_name, cfg, port)
        if proc is None:
            continue

        try:
            # Run benchmark — plugin defines this
            print(f"  [{test_type}/{server_name}] Running benchmarks ...")
            result = plugin.run_bench(
                server_name=server_name,
                port=port,
                wrk_threads=wrk_threads,
            )
            if isinstance(result, TestResult):
                results[server_name] = result
            else:
                print(f"  [{test_type}/{server_name}] Plugin returned unexpected type: {type(result)}")
        except Exception as e:
            print(f"  [{test_type}/{server_name}] Benchmark error: {e}")
            import traceback; traceback.print_exc()
        finally:
            stop_server(proc, server_name)

    return results


def run_smoke(plugin, test_type: str, servers: list) -> dict:
    """
    Run smoke/correctness tests (no server start needed, or plugin handles it).
    Returns {server_name: TestResult}.
    """
    results = {}
    all_servers = plugin.SERVERS
    for server_name in servers:
        if server_name not in all_servers:
            continue
        cfg = all_servers[server_name]
        if not build_server(test_type, server_name, cfg):
            continue
        print(f"  [{test_type}/{server_name}] Running smoke tests ...")
        try:
            result = plugin.run_smoke(server_name=server_name)
            if isinstance(result, TestResult):
                results[server_name] = result
        except Exception as e:
            print(f"  [{test_type}/{server_name}] Smoke error: {e}")
    return results


def main():
    parser = argparse.ArgumentParser(
        description="GVThread integration test runner",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  %(prog)s echo                     Run all echo benchmarks
  %(prog)s echo --servers ksvc go   Only KSVC and Go
  %(prog)s httpd --baseline         Compare against saved baseline
  %(prog)s echo --save-baseline     Save current run as baseline
  %(prog)s --all                    Run every discovered test type
  %(prog)s --list                   List all discovered plugins and servers
        """,
    )
    parser.add_argument("test_type", nargs="?", help="Test type to run (echo, httpd, smoke, ...)")
    parser.add_argument("--all", action="store_true", help="Run all discovered test types")
    parser.add_argument("--list", action="store_true", help="List plugins and servers")
    parser.add_argument("--servers", nargs="+", help="Only run these servers (default: all)")
    parser.add_argument("--port-base", type=int, default=9100, help="Base port (default: 9100)")
    parser.add_argument("--wrk-threads", type=int, default=2, help="wrk thread count (default: 2)")
    parser.add_argument("--baseline", action="store_true", help="Compare against baseline")
    parser.add_argument("--save-baseline", action="store_true", help="Save results as baseline")
    parser.add_argument("--threshold", type=float, default=5.0,
                        help="Regression threshold %% (default: 5.0)")
    parser.add_argument("--save", action="store_true", default=True,
                        help="Save results to results/ (default: true)")
    parser.add_argument("--no-save", dest="save", action="store_false")

    args = parser.parse_args()

    # Discover
    plugins = discover_plugins()
    if not plugins:
        print(f"ERROR: No plugins found in {CMD_DIR}/*/{{name}}-plugin.py")
        sys.exit(1)

    # List mode
    if args.list:
        print("\nDiscovered test types:")
        for name, mod in sorted(plugins.items()):
            servers = list(mod.SERVERS.keys())
            kind = getattr(mod, "KIND", "bench")
            print(f"  {name:<12} servers={servers}  kind={kind}")
        print()
        sys.exit(0)

    # Determine which test types to run
    if args.all:
        test_types = list(plugins.keys())
    elif args.test_type:
        if args.test_type not in plugins:
            print(f"ERROR: Unknown test type '{args.test_type}'. "
                  f"Available: {list(plugins.keys())}")
            sys.exit(1)
        test_types = [args.test_type]
    else:
        parser.print_help()
        sys.exit(1)

    # Run each test type
    exit_code = 0
    for tt in test_types:
        plugin = plugins[tt]
        all_server_names = list(plugin.SERVERS.keys())
        servers = args.servers if args.servers else all_server_names
        kind = getattr(plugin, "KIND", "bench")

        print(f"\n{'='*60}")
        print(f"  {tt.upper()} — {kind} — servers: {servers}")
        print(f"{'='*60}")

        if kind == "smoke":
            results = run_smoke(plugin, tt, servers)
        else:
            results = run_benchmarks(
                plugin, tt, servers,
                port_base=args.port_base,
                wrk_threads=args.wrk_threads,
            )

        if not results:
            print(f"\n  No results for {tt}.")
            continue

        # Print comparison table (benchmarks only)
        if kind == "bench" and len(results) > 1:
            print_comparison_table(results, tt)

        # Save results
        if args.save:
            for server_name, result in results.items():
                path = result.save()
                print(f"  Saved: {path}")

        # Save as baseline
        if args.save_baseline:
            for server_name, result in results.items():
                bl_dir = RESULTS_DIR / tt / server_name
                bl_dir.mkdir(parents=True, exist_ok=True)
                bl_path = bl_dir / "baseline.json"
                import json
                with open(bl_path, "w") as f:
                    json.dump(result.to_dict(), f, indent=2)
                print(f"  Baseline saved: {bl_path}")

        # Compare against baseline
        if args.baseline:
            for server_name, result in results.items():
                bl = get_baseline(tt, server_name)
                if bl is None:
                    print(f"  [{tt}/{server_name}] No baseline found — skipping comparison")
                    continue
                regressions = compare_results(result, bl, args.threshold)
                if regressions:
                    exit_code = 1
                    print(f"\n  REGRESSIONS in {tt}/{server_name} (threshold={args.threshold}%):")
                    for r in regressions:
                        print(f"    {r['name']}.{r['metric']}: "
                              f"{r['baseline']:.0f} → {r['current']:.0f} "
                              f"({r['delta_pct']:+.1f}%)")
                else:
                    print(f"  [{tt}/{server_name}] No regressions vs baseline")

        # For smoke tests, report pass/fail summary
        if kind == "smoke":
            for server_name, result in results.items():
                total = len(result.tests)
                passed = sum(1 for t in result.tests if t.get("extra", {}).get("passed", True))
                status = "PASS" if result.passed else "FAIL"
                print(f"  [{tt}/{server_name}] {status} ({passed}/{total} tests)")
                if not result.passed:
                    exit_code = 1

    sys.exit(exit_code)


if __name__ == "__main__":
    main()