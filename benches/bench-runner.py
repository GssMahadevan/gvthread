#!/usr/bin/env python3
"""
bench-runner.py — Fair benchmark orchestrator for GVThread.

Reads benches/{testName}/manifest.yml, iterates common profiles × app configs,
exports gvt_* / gvt_app_* env vars, runs wrk, collects results.

Design principles:
  - Common section is LAW. Same for every app in a run. Cannot be overridden.
  - App config is ADDITIVE. Only knobs unique to that runtime/backend.
  - Port is PER-APP (at app level, not common) to avoid TCP TIME_WAIT conflicts.
  - gvt_{K}={V}       for common params (enforced equal across apps)
  - gvt_app_{K}={V}   for app-specific params (includes port)
  - taskset enforces cpu_cores (apps don't do their own pinning)
  - parallelism is always common (threads/workers/GOMAXPROCS all read gvt_parallelism)

Usage:
    python3 bench-runner.py benches/httpd/manifest.yml
    python3 bench-runner.py benches/httpd/manifest.yml --common light
    python3 bench-runner.py benches/httpd/manifest.yml --common heavy --app ksvc-httpd
    python3 bench-runner.py benches/httpd/manifest.yml --common light --app go-httpd --config pooled
    python3 bench-runner.py benches/httpd/manifest.yml --list
    python3 bench-runner.py benches/httpd/manifest.yml --dry-run
"""

import argparse
import json
import os
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

# ---------------------------------------------------------------------------
# YAML loading: try PyYAML, fall back to minimal parser
# ---------------------------------------------------------------------------

try:
    import yaml

    def load_yaml(path):
        with open(path) as f:
            return yaml.safe_load(f)

except ImportError:
    # Minimal YAML subset parser — handles our manifest structure:
    # top-level keys, nested dicts, scalars, lists of dicts.
    def load_yaml(path):
        """Parse the subset of YAML we use in manifests (no PyYAML needed)."""
        import re

        with open(path) as f:
            lines = f.readlines()

        root = {}
        stack = [(root, -1)]  # (dict, indent_level)

        for raw_line in lines:
            # Strip comments and trailing whitespace
            line = raw_line.split('#')[0].rstrip()
            if not line.strip():
                continue

            indent = len(line) - len(line.lstrip())
            stripped = line.strip()

            # Pop stack to correct level
            while len(stack) > 1 and indent <= stack[-1][1]:
                stack.pop()

            parent, _ = stack[-1]

            # List item: "- name: foo"  or  "- key: val"
            if stripped.startswith('- '):
                item_str = stripped[2:].strip()
                # If parent is a dict and the last key points to a list
                if isinstance(parent, dict):
                    # Find the key this list belongs to
                    last_key = getattr(stack[-1], '_list_key', None)
                    # Actually, the list should already exist from the key line
                    # This is the tricky part — skip for now, handle inline
                    pass

                # For simplicity, handle "- name: val" as dict in a list
                if ':' in item_str:
                    item = {}
                    # Parse all KV pairs on this line and subsequent indented lines
                    k, v = item_str.split(':', 1)
                    item[k.strip()] = _parse_value(v.strip())

                    # Find the list to append to
                    if isinstance(parent, dict):
                        for pk in reversed(list(parent.keys())):
                            if isinstance(parent[pk], list):
                                parent[pk].append(item)
                                stack.append((item, indent + 2))
                                break
                    elif isinstance(parent, list):
                        parent.append(item)
                        stack.append((item, indent + 2))
                continue

            # Key-value line: "key: value" or "key:"
            if ':' in stripped:
                k, v = stripped.split(':', 1)
                k = k.strip()
                v = v.strip()

                if v == '' or v == '|':
                    # Nested dict or list — create empty dict, will be populated
                    child = {}
                    parent[k] = child
                    stack.append((child, indent))
                elif v.startswith('['):
                    # Inline list
                    parent[k] = [_parse_value(x.strip().strip('"').strip("'"))
                                 for x in v.strip('[]').split(',') if x.strip()]
                    # Check if it should be a list of dicts
                else:
                    parent[k] = _parse_value(v)
            else:
                # Continuation or bare value — skip
                pass

        return root

    def _parse_value(v):
        """Parse a YAML scalar value."""
        if v == '' or v is None:
            return None
        if v.lower() == 'true':
            return True
        if v.lower() == 'false':
            return False
        if v.startswith('"') and v.endswith('"'):
            return v[1:-1]
        if v.startswith("'") and v.endswith("'"):
            return v[1:-1]
        try:
            return int(v)
        except ValueError:
            pass
        try:
            return float(v)
        except ValueError:
            pass
        return v


# ---------------------------------------------------------------------------
# Utility functions
# ---------------------------------------------------------------------------

ROOT_DIR = None  # Set in main() based on manifest location


def log(msg, prefix="bench"):
    ts = datetime.now().strftime("%H:%M:%S")
    print(f"[{ts}] [{prefix}] {msg}", flush=True)


def log_err(msg, prefix="bench"):
    ts = datetime.now().strftime("%H:%M:%S")
    print(f"[{ts}] [{prefix}] ERROR: {msg}", file=sys.stderr, flush=True)


def wait_for_port(port, timeout=10.0):
    """Wait until a TCP port is accepting connections."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except (ConnectionRefusedError, OSError):
            time.sleep(0.1)
    return False


def kill_on_port(port):
    """Kill any process listening on a port (cleanup from previous runs)."""
    try:
        result = subprocess.run(
            ["fuser", f"{port}/tcp"],
            capture_output=True, text=True, timeout=5
        )
        if result.stdout.strip():
            subprocess.run(
                ["fuser", "-k", f"{port}/tcp"],
                capture_output=True, timeout=5
            )
            time.sleep(0.5)
    except (FileNotFoundError, subprocess.TimeoutExpired):
        pass


def collect_system_info():
    """Collect system metadata for result records."""
    info = {
        "hostname": platform.node(),
        "kernel": platform.release(),
        "arch": platform.machine(),
        "python": platform.python_version(),
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }
    # CPU model
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    info["cpu_model"] = line.split(":", 1)[1].strip()
                    break
    except FileNotFoundError:
        pass

    # Total cores
    try:
        info["total_cores"] = os.cpu_count()
    except Exception:
        pass

    # Total memory
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemTotal"):
                    kb = int(line.split()[1])
                    info["memory_gb"] = round(kb / 1024 / 1024, 1)
                    break
    except FileNotFoundError:
        pass

    # Git SHA
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True, timeout=5,
            cwd=str(ROOT_DIR) if ROOT_DIR else None,
        )
        if result.returncode == 0:
            info["git_sha"] = result.stdout.strip()
    except (FileNotFoundError, subprocess.TimeoutExpired):
        pass

    return info


def build_taskset_prefix(cpu_cores):
    """Build taskset command prefix for CPU pinning."""
    if not cpu_cores or cpu_cores < 1:
        return []
    # Pin to cores 0..(cpu_cores-1)
    total = os.cpu_count() or 1
    if cpu_cores > total:
        log(f"Warning: cpu_cores={cpu_cores} > available={total}, using all")
        return []
    cpuset = f"0-{cpu_cores - 1}"
    if shutil.which("taskset"):
        return ["taskset", "-c", cpuset]
    else:
        log("Warning: taskset not found, CPU pinning disabled")
        return []


# ---------------------------------------------------------------------------
# wrk output parser
# ---------------------------------------------------------------------------

def parse_wrk_output(output):
    """Parse wrk stdout into a dict of metrics."""
    result = {}

    # Requests/sec: 123456.78
    m = re.search(r'Requests/sec:\s+([\d.]+)', output)
    if m:
        result["requests_per_sec"] = float(m.group(1))

    # Transfer/sec: 12.34MB
    m = re.search(r'Transfer/sec:\s+([\d.]+)(\w+)', output)
    if m:
        val = float(m.group(1))
        unit = m.group(2)
        if unit == "GB":
            val *= 1024
        elif unit == "KB":
            val /= 1024
        result["transfer_mb_per_sec"] = val

    # Latency   Avg    Stdev   Max   +/- Stdev
    # 50%  123.00us
    for pct in ["50%", "75%", "90%", "99%", "99.9%"]:
        # wrk --latency output: "  50%  123.00us"
        pattern = rf'^\s*{re.escape(pct)}\s+([\d.]+)(us|ms|s)\s*$'
        m = re.search(pattern, output, re.MULTILINE)
        if m:
            val = float(m.group(1))
            unit = m.group(2)
            if unit == "ms":
                val *= 1000
            elif unit == "s":
                val *= 1_000_000
            key = f"p{pct.replace('%', '').replace('.', '_')}_us"
            result[key] = val

    # Avg latency
    m = re.search(r'Latency\s+([\d.]+)(us|ms|s)', output)
    if m:
        val = float(m.group(1))
        unit = m.group(2)
        if unit == "ms":
            val *= 1000
        elif unit == "s":
            val *= 1_000_000
        result["avg_latency_us"] = val

    # Total requests
    m = re.search(r'(\d+)\s+requests\s+in', output)
    if m:
        result["total_requests"] = int(m.group(1))

    # Errors
    m = re.search(r'Socket errors:\s+connect\s+(\d+),\s+read\s+(\d+),\s+write\s+(\d+),\s+timeout\s+(\d+)', output)
    if m:
        result["errors"] = {
            "connect": int(m.group(1)),
            "read": int(m.group(2)),
            "write": int(m.group(3)),
            "timeout": int(m.group(4)),
        }

    # Non-2xx
    m = re.search(r'Non-2xx or 3xx responses:\s+(\d+)', output)
    if m:
        result["non_2xx"] = int(m.group(1))

    return result


def parse_wrkr_json(output):
    """Parse wrkr JSON stdout into the same metric dict format as parse_wrk_output."""
    try:
        data = json.loads(output)
    except json.JSONDecodeError:
        log_err(f"Failed to parse wrkr JSON output")
        return {}

    result = {}
    result["requests_per_sec"] = data.get("requests_per_sec", 0)
    result["total_requests"] = data.get("total_requests", 0)

    lat = data.get("latency_us", {})
    result["avg_latency_us"] = lat.get("avg")
    result["p50_us"] = lat.get("p50")
    result["p75_us"] = lat.get("p75")
    result["p90_us"] = lat.get("p90")
    result["p99_us"] = lat.get("p99")
    result["p99_9_us"] = lat.get("p99.9")

    errs = data.get("errors", {})
    if any(v > 0 for v in errs.values()):
        result["errors"] = errs

    if data.get("total_errors", 0) > 0:
        result["non_2xx"] = data.get("total_errors", 0)

    return result


def find_wrkr(args):
    """Find the wrkr binary.  Returns Path or None."""
    # Explicit --wrkr flag
    if hasattr(args, 'wrkr') and args.wrkr:
        p = Path(args.wrkr)
        if p.exists():
            return p
        log_err(f"wrkr not found at {p}")
        return None

    # Auto-detect in target/<build>/
    if ROOT_DIR:
        p = Path(ROOT_DIR) / "target" / args.build / "wrkr"
        if p.exists():
            return p

    # Check PATH
    w = shutil.which("wrkr")
    if w:
        return Path(w)

    return None


def run_load_generator(wrkr_path, port, threads, connections, duration_sec,
                       keepalive, cell_tag, is_warmup=False):
    """Run the load generator (wrkr or wrk) and return (metrics_dict, raw_stdout).

    If wrkr_path is not None, uses wrkr (JSON output).
    Otherwise falls back to wrk.
    """
    url = f"http://127.0.0.1:{port}/"
    timeout = duration_sec + 60

    if wrkr_path:
        cmd = [
            str(wrkr_path),
            url,
            "-c", str(connections),
            "-d", str(duration_sec),
        ]
        if not keepalive:
            cmd.append("--no-keepalive")

        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout,
        )

        if result.returncode != 0:
            if not is_warmup:
                log_err(f"[{cell_tag}] wrkr failed (rc={result.returncode})")
                if result.stderr:
                    log_err(f"  wrkr stderr: {result.stderr[:300]}")
            return None, result.stdout

        if is_warmup:
            return {}, result.stdout

        metrics = parse_wrkr_json(result.stdout)
        return metrics, result.stdout

    else:
        # Fallback: wrk
        cmd = [
            "wrk",
            f"-t{threads}",
            f"-c{connections}",
            f"-d{duration_sec}s",
            "--latency",
        ]
        if not keepalive:
            cmd.extend(["-H", "Connection: close"])
        cmd.append(url)

        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout,
        )

        if result.returncode != 0:
            if not is_warmup:
                log_err(f"[{cell_tag}] wrk failed (rc={result.returncode})")
                if result.stderr:
                    log_err(f"  wrk stderr: {result.stderr[:300]}")
            return None, result.stdout

        if is_warmup:
            return {}, result.stdout

        metrics = parse_wrk_output(result.stdout)
        return metrics, result.stdout


# ---------------------------------------------------------------------------
# Core: manifest loading + validation
# ---------------------------------------------------------------------------

def load_manifest(path):
    """Load and validate a benchmark manifest."""
    manifest = load_yaml(str(path))
    if not manifest:
        log_err(f"Empty or unparseable manifest: {path}")
        sys.exit(1)

    if "common" not in manifest:
        log_err(f"Manifest missing 'common' section: {path}")
        sys.exit(1)

    if "apps" not in manifest:
        log_err(f"Manifest missing 'apps' section: {path}")
        sys.exit(1)

    # Guard: port must NOT be in common (it's per-app to avoid TIME_WAIT)
    for profile_name, profile in manifest["common"].items():
        if isinstance(profile, dict) and "port" in profile:
            log_err(f"'port' found in common/{profile_name}. "
                    f"Port must be per-app (avoids TCP TIME_WAIT between runs).")
            sys.exit(1)

    # Guard: every app must have a port
    for app_name, app_def in manifest["apps"].items():
        if isinstance(app_def, dict) and "port" not in app_def:
            log_err(f"App '{app_name}' missing 'port'. "
                    f"Each app needs its own port to avoid TIME_WAIT conflicts.")
            sys.exit(1)

    return manifest


def validate_no_overlap(common_keys, app_config):
    """
    Ensure no app-config key collides with a common key.
    This is the apples-vs-oranges guardrail.
    """
    violations = []
    for k in app_config:
        if k == "name":
            continue  # 'name' is metadata, not a tuning param
        if k in common_keys:
            violations.append(k)
    return violations


# ---------------------------------------------------------------------------
# Core: build environment for a single run
# ---------------------------------------------------------------------------

def build_env(common_profile, app_config):
    """
    Build the environment dict for a single benchmark run.

    common_profile keys → gvt_{K}={V}
    app_config keys     → gvt_app_{K}={V}

    Returns (env_dict, description_string).
    """
    env = os.environ.copy()

    # Common — gvt_*
    common_keys = set()
    for k, v in common_profile.items():
        if k == "desc":
            continue
        env_key = f"gvt_{k}"
        env[env_key] = str(v)
        common_keys.add(k)

    # App-specific — gvt_app_*
    for k, v in app_config.items():
        if k == "name":
            continue
        env_key = f"gvt_app_{k}"
        env[env_key] = str(v)

    return env


def _dump_server_output(server_proc, cell_tag):
    """Read and display any buffered stdout/stderr from a server process.

    IMPORTANT: process must be dead before calling this.
    .read() on pipes of a live process blocks forever.
    Safety: kill if still alive.
    """
    # Safety: ensure process is dead before reading pipes
    if server_proc.poll() is None:
        server_proc.kill()
        try:
            server_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass

    try:
        stdout = server_proc.stdout.read().decode(errors="replace") if server_proc.stdout else ""
        stderr = server_proc.stderr.read().decode(errors="replace") if server_proc.stderr else ""
    except Exception:
        stdout = stderr = ""

    if stderr:
        log_err(f"  [{cell_tag}] Server stderr:")
        for line in stderr.strip().splitlines()[-20:]:  # last 20 lines
            log_err(f"    {line}")
    if stdout:
        log(f"  [{cell_tag}] Server stdout:")
        for line in stdout.strip().splitlines()[-10:]:  # last 10 lines
            log(f"    {line}")


# ---------------------------------------------------------------------------
# Core: single benchmark cell
# ---------------------------------------------------------------------------

def run_one_cell(
    app_name, app_def, app_config,
    common_name, common_profile,
    bench_dir, system_info,
    dry_run=False,
    build_profile="release",
    wrkr_path=None,
):
    """
    Run one benchmark cell: one app × one config × one common profile.

    Returns a result dict or None on failure.
    """
    config_name = app_config.get("name", "default")
    cell_tag = f"{common_name}/{app_name}/{config_name}"

    # ── Validate no overlap ──
    common_keys = {k for k in common_profile if k != "desc"}
    violations = validate_no_overlap(common_keys, app_config)
    if violations:
        log_err(f"[{cell_tag}] App config overrides common keys: {violations}")
        log_err(f"  This violates apples-to-apples comparison. Fix manifest.")
        return None

    # ── Build env ──
    env = build_env(common_profile, app_config)

    # Port is per-app (avoids TCP TIME_WAIT between sequential runs)
    port = app_def.get("port")
    if port is None:
        log_err(f"[{cell_tag}] No 'port' in app definition. "
                f"Each app must have its own port to avoid TIME_WAIT conflicts.")
        return None
    env["gvt_app_port"] = str(port)

    cpu_cores = common_profile.get("cpu_cores")
    parallelism = common_profile.get("parallelism")
    warmup_sec = common_profile.get("warmup_sec", 3)
    measure_sec = common_profile.get("measure_sec", 10)
    keepalive = common_profile.get("keepalive", True)
    wrk_threads = common_profile.get("wrk_threads", 2)
    wrk_connections = common_profile.get("wrk_connections", 100)

    binary = app_def.get("binary", "")
    if not binary:
        log_err(f"[{cell_tag}] No binary specified")
        return None

    # Rewrite binary path for build profile (release → debug)
    if build_profile != "release":
        binary = binary.replace("target/release/", f"target/{build_profile}/")

    # Resolve binary path relative to repo root
    binary_path = Path(ROOT_DIR) / binary if ROOT_DIR else Path(binary)

    # ── Print plan ──
    desc = common_profile.get("desc", "")
    log(f"{'─' * 60}")
    log(f"Cell: {cell_tag}")
    log(f"  Common: {common_name} — {desc}")
    log(f"  App:    {app_name} (lang={app_def.get('lang', '?')}, "
        f"model={app_def.get('model', '?')}, io={app_def.get('io', '?')})")
    log(f"  Config: {config_name} — "
        f"{', '.join(f'{k}={v}' for k, v in app_config.items() if k != 'name') or '(defaults)'}")
    log(f"  HW:     cores={cpu_cores}, parallelism={parallelism}")
    log(f"  Port:   {port} (per-app, avoids TIME_WAIT)")
    if wrkr_path:
        log(f"  Load:   wrkr -c{wrk_connections} -d{measure_sec} "
            f"{'(keepalive)' if keepalive else '(no keepalive)'}")
    else:
        log(f"  Load:   wrk -t{wrk_threads} -c{wrk_connections} -d{measure_sec}s "
            f"{'(keepalive)' if keepalive else '(no keepalive)'}")
    log(f"  Binary: {binary_path} ({build_profile})")

    # Print exported env vars
    gvt_vars = sorted(k for k in env if k.startswith("gvt_"))
    log(f"  Env:    {' '.join(f'{k}={env[k]}' for k in gvt_vars)}")

    if dry_run:
        log(f"  [DRY RUN] Skipping execution")
        return {"cell": cell_tag, "dry_run": True}

    # ── Check binary exists ──
    if not binary_path.exists():
        log_err(f"[{cell_tag}] Binary not found: {binary_path}")
        log_err(f"  Build it first (e.g. cargo build -p {app_name} --{build_profile})")
        return None

    # ── Check load generator exists ──
    load_gen_name = "wrkr" if wrkr_path else "wrk"
    if not wrkr_path and not shutil.which("wrk"):
        log_err(f"[{cell_tag}] No load generator found. "
                f"Build wrkr (cargo build -p wrkr --release) or install wrk (apt install wrk)")
        return None

    # ── Kill stale processes on port ──
    kill_on_port(port)

    # ── Build server command ──
    taskset = build_taskset_prefix(cpu_cores)
    server_cmd = taskset + [str(binary_path)]

    # ── Start server ──
    log(f"  Starting: {' '.join(server_cmd)}")
    try:
        server_proc = subprocess.Popen(
            server_cmd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except Exception as e:
        log_err(f"[{cell_tag}] Failed to start server: {e}")
        return None

    # Wait for port to open
    if not wait_for_port(port, timeout=10.0):
        log_err(f"[{cell_tag}] Server did not open port {port} within 10s")
        rc = server_proc.poll()
        if rc is not None:
            sig_name = ""
            if rc < 0:
                import signal as sig_mod
                try:
                    sig_name = f" ({sig_mod.Signals(-rc).name})"
                except (ValueError, AttributeError):
                    sig_name = f" (signal {-rc})"
            log_err(f"  Server CRASHED during startup (exit={rc}{sig_name})")
        else:
            log_err(f"  Server is running but not on port {port} — killing")
            server_proc.kill()
            server_proc.wait(timeout=5)
        _dump_server_output(server_proc, cell_tag)
        return None

    log(f"  Server ready (pid={server_proc.pid}), load-gen={load_gen_name}")

    result = None
    server_output_dumped = False
    try:
        # ── Warmup ──
        if warmup_sec > 0:
            log(f"  Warming up ({warmup_sec}s) ...")
            run_load_generator(
                wrkr_path, port, wrk_threads, wrk_connections,
                warmup_sec, keepalive, cell_tag, is_warmup=True,
            )

        # ── Check server survived warmup ──
        rc = server_proc.poll()
        if rc is not None:
            sig_name = ""
            if rc < 0:
                import signal as sig_mod
                try:
                    sig_name = f" ({sig_mod.Signals(-rc).name})"
                except (ValueError, AttributeError):
                    sig_name = f" (signal {-rc})"
            log_err(f"[{cell_tag}] Server CRASHED during warmup (exit={rc}{sig_name})")
            _dump_server_output(server_proc, cell_tag)
            server_output_dumped = True
            return None

        # ── Measurement ──
        log(f"  Measuring ({measure_sec}s) ...")
        metrics, wrk_stdout = run_load_generator(
            wrkr_path, port, wrk_threads, wrk_connections,
            measure_sec, keepalive, cell_tag, is_warmup=False,
        )

        if metrics is None:
            # Check if server died during measurement
            rc = server_proc.poll()
            if rc is not None:
                sig_name = ""
                if rc < 0:
                    import signal as sig_mod
                    try:
                        sig_name = f" ({sig_mod.Signals(-rc).name})"
                    except (ValueError, AttributeError):
                        sig_name = f" (signal {-rc})"
                log_err(f"  Server CRASHED during measurement (exit={rc}{sig_name})")
            else:
                # Server still alive — kill before reading pipes (avoids blocking)
                server_proc.kill()
                server_proc.wait(timeout=5)
            _dump_server_output(server_proc, cell_tag)
            server_output_dumped = True
            return None

        rps = metrics.get("requests_per_sec", 0)
        p50 = metrics.get("p50_us")
        p99 = metrics.get("p99_us")

        log(f"  Result: {rps:,.0f} req/s  "
            f"p50={p50:,.0f}μs  p99={p99:,.0f}μs" if p50 and p99 else
            f"  Result: {rps:,.0f} req/s")

        # ── Collect server RSS ──
        rss_kb = None
        try:
            with open(f"/proc/{server_proc.pid}/status") as f:
                for line in f:
                    if line.startswith("VmRSS:"):
                        rss_kb = int(line.split()[1])
                        break
        except (FileNotFoundError, ProcessLookupError):
            pass

        # ── Build result ──
        result = {
            "cell": cell_tag,
            "common_profile": common_name,
            "common_desc": desc,
            "app": app_name,
            "config": config_name,
            "lang": app_def.get("lang"),
            "model": app_def.get("model"),
            "io_backend": app_def.get("io"),
            "binary": str(binary_path),
            "build_profile": build_profile,

            # Common params (every app got the same)
            "common_params": {k: v for k, v in common_profile.items() if k != "desc"},

            # App-specific params (unique to this app)
            "app_params": {k: v for k, v in app_config.items() if k != "name"},

            # wrk/wrkr config
            "load_gen": load_gen_name,
            "wrk_threads": wrk_threads,
            "wrk_connections": wrk_connections,
            "measure_sec": measure_sec,
            "keepalive": keepalive,

            # Results
            "metrics": metrics,
            "rps": rps,
            "p50_us": p50,
            "p99_us": p99,
            "p99_9_us": metrics.get("p99_9_us"),
            "rss_kb": rss_kb,

            # Per-core efficiency
            "rps_per_core": rps / parallelism if parallelism else None,

            # Provenance
            "wrk_raw": wrk_stdout,
            "system": system_info,
        }

    except subprocess.TimeoutExpired:
        log_err(f"[{cell_tag}] wrk timed out")
    except Exception as e:
        log_err(f"[{cell_tag}] Unexpected error: {e}")
        import traceback
        traceback.print_exc()
    finally:
        # ── Stop server ──
        rc = server_proc.poll()
        if rc is not None:
            # Already dead (crash or natural exit)
            sig_name = ""
            if rc < 0:
                import signal as sig_mod
                try:
                    sig_name = f" ({sig_mod.Signals(-rc).name})"
                except (ValueError, AttributeError):
                    sig_name = f" (signal {-rc})"
            log(f"  Server already exited (code={rc}{sig_name})")
            if result is None and not server_output_dumped:
                _dump_server_output(server_proc, cell_tag)
        else:
            log(f"  Stopping server (pid={server_proc.pid}) ...")
            try:
                server_proc.send_signal(signal.SIGTERM)
                server_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server_proc.kill()
                server_proc.wait()

        # Capture server stderr for result record (successful runs)
        if result is not None and not server_output_dumped:
            try:
                server_stderr = server_proc.stderr.read().decode(errors="replace")
                if server_stderr:
                    result["server_stderr"] = server_stderr[-2048:]
            except Exception:
                pass

    return result


# ---------------------------------------------------------------------------
# Results: save + report
# ---------------------------------------------------------------------------

def save_result(result, results_dir):
    """Save a single cell result as JSON."""
    ts = datetime.now().strftime("%Y%m%dT%H%M%S")
    cell = result["cell"].replace("/", "__")
    filename = f"{cell}__{ts}.json"
    path = results_dir / filename

    # Remove raw wrk output from saved file (verbose)
    save_data = {k: v for k, v in result.items() if k != "wrk_raw"}
    with open(path, "w") as f:
        json.dump(save_data, f, indent=2, default=str)

    return path


def print_summary_table(all_results, common_name):
    """Print a comparison table for all results in one common profile."""
    if not all_results:
        return

    # Group by app+config
    rows = []
    for r in all_results:
        if r.get("dry_run"):
            continue
        rows.append(r)

    if not rows:
        return

    # Find column widths
    max_app = max(len(r.get("app", "")) for r in rows)
    max_cfg = max(len(r.get("config", "")) for r in rows)
    max_model = max(len(r.get("model", "") or "") for r in rows)

    hdr_app = "App".ljust(max_app)
    hdr_cfg = "Config".ljust(max_cfg)
    hdr_model = "Model".ljust(max_model)

    sep = "─"
    header = (f"  {hdr_app}  {hdr_cfg}  {hdr_model}  "
              f"{'IO':>7}  {'req/s':>12}  {'rps/core':>10}  "
              f"{'p50μs':>8}  {'p99μs':>8}  {'RSS MB':>7}")
    width = len(header) + 4

    print(f"\n{'═' * width}")
    desc = rows[0].get("common_desc", "")
    common_params = rows[0].get("common_params", {})
    par = common_params.get("parallelism", "?")
    cores = common_params.get("cpu_cores", "?")
    conns = common_params.get("wrk_connections", "?")
    print(f"  Profile: {common_name} — {desc}")
    print(f"  Params:  cores={cores}  parallelism={par}  connections={conns}")
    print(f"{'═' * width}")
    print(header)
    print(f"  {sep * (width - 4)}")

    # Sort by rps descending
    rows.sort(key=lambda r: r.get("rps", 0), reverse=True)

    best_rps = rows[0].get("rps", 0) if rows else 0

    for r in rows:
        app = r.get("app", "?").ljust(max_app)
        cfg = r.get("config", "?").ljust(max_cfg)
        model = (r.get("model", "") or "?").ljust(max_model)
        io_be = (r.get("io_backend", "") or "?")
        rps = r.get("rps", 0)
        rps_pc = r.get("rps_per_core")
        p50 = r.get("p50_us")
        p99 = r.get("p99_us")
        rss = r.get("rss_kb")

        rps_str = f"{rps:>12,.0f}" if rps else f"{'N/A':>12}"
        rps_pc_str = f"{rps_pc:>10,.0f}" if rps_pc else f"{'—':>10}"
        p50_str = f"{p50:>8,.0f}" if p50 else f"{'—':>8}"
        p99_str = f"{p99:>8,.0f}" if p99 else f"{'—':>8}"
        rss_str = f"{rss / 1024:>7.1f}" if rss else f"{'—':>7}"

        winner = " ◀" if rps == best_rps and len(rows) > 1 else ""
        print(f"  {app}  {cfg}  {model}  {io_be:>7}  {rps_str}  "
              f"{rps_pc_str}  {p50_str}  {p99_str}  {rss_str}{winner}")

    print(f"  {sep * (width - 4)}")
    ts = rows[0].get("system", {}).get("timestamp", "")
    print(f"  Timestamp: {ts}")
    print()


def generate_markdown_report(all_results, common_name, report_path):
    """Generate a markdown report file."""
    if not all_results:
        return

    rows = [r for r in all_results if not r.get("dry_run")]
    if not rows:
        return

    rows.sort(key=lambda r: r.get("rps", 0), reverse=True)

    with open(report_path, "w") as f:
        desc = rows[0].get("common_desc", "")
        cp = rows[0].get("common_params", {})
        sys_info = rows[0].get("system", {})

        f.write(f"# Benchmark: {common_name}\n\n")
        f.write(f"> {desc}\n\n")

        f.write("## Common Parameters\n\n")
        f.write("| Key | Value |\n|-----|-------|\n")
        for k, v in sorted(cp.items()):
            f.write(f"| {k} | {v} |\n")

        f.write(f"\n## System\n\n")
        for k, v in sorted(sys_info.items()):
            f.write(f"- **{k}**: {v}\n")

        f.write(f"\n## Results\n\n")
        f.write("| App | Config | Lang | Model | IO | req/s | rps/core | p50 μs | p99 μs | RSS MB |\n")
        f.write("|-----|--------|------|-------|----|------:|--------:|-------:|-------:|-------:|\n")

        for r in rows:
            rps = r.get("rps", 0)
            rps_pc = r.get("rps_per_core")
            p50 = r.get("p50_us")
            p99 = r.get("p99_us")
            rss = r.get("rss_kb")
            f.write(
                f"| {r.get('app', '?')} "
                f"| {r.get('config', '?')} "
                f"| {r.get('lang', '?')} "
                f"| {r.get('model', '?')} "
                f"| {r.get('io_backend', '?')} "
                f"| {rps:,.0f} "
                f"| {rps_pc:,.0f} " if rps_pc else f"| — "
                f"| {p50:,.0f} " if p50 else f"| — "
                f"| {p99:,.0f} " if p99 else f"| — "
                f"| {rss / 1024:.1f} |\n" if rss else f"| — |\n"
            )

        # App-specific params
        f.write(f"\n## App-Specific Parameters\n\n")
        for r in rows:
            ap = r.get("app_params", {})
            if ap:
                f.write(f"- **{r['app']}/{r['config']}**: "
                        f"{', '.join(f'{k}={v}' for k, v in ap.items())}\n")

    log(f"Report: {report_path}")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    global ROOT_DIR

    parser = argparse.ArgumentParser(
        description="GVThread fair benchmark runner",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  %(prog)s benches/httpd/manifest.yml                         # all profiles × all apps
  %(prog)s benches/httpd/manifest.yml --common light          # one profile, all apps
  %(prog)s benches/httpd/manifest.yml --common heavy --app go-httpd
  %(prog)s benches/httpd/manifest.yml --build debug           # use target/debug/ binaries
  %(prog)s benches/httpd/manifest.yml --list                  # show matrix
  %(prog)s benches/httpd/manifest.yml --dry-run               # plan only
        """,
    )
    parser.add_argument("manifest", help="Path to manifest.yml")
    parser.add_argument("--common", help="Run only this common profile (default: all)")
    parser.add_argument("--app", help="Run only this app (default: all)")
    parser.add_argument("--config", help="Run only this config within the app (default: all)")
    parser.add_argument("--list", action="store_true", help="List the execution matrix")
    parser.add_argument("--dry-run", action="store_true", help="Plan only, don't execute")
    parser.add_argument("--no-save", action="store_true", help="Don't save result JSON files")
    parser.add_argument("--no-report", action="store_true", help="Don't generate markdown reports")
    parser.add_argument("--repeat", type=int, default=1, help="Repeat each cell N times (default: 1)")
    parser.add_argument("--build", choices=["release", "debug"], default="release",
                        help="Cargo build profile for binary path resolution (default: release)")
    parser.add_argument("--wrkr", metavar="PATH",
                        help="Path to wrkr binary (default: auto-detect in target/<build>/)")
    parser.add_argument("--use-wrk", action="store_true",
                        help="Force using wrk instead of wrkr")

    args = parser.parse_args()

    manifest_path = Path(args.manifest).resolve()
    if not manifest_path.exists():
        log_err(f"Manifest not found: {manifest_path}")
        sys.exit(1)

    bench_dir = manifest_path.parent
    results_dir = bench_dir / "results"
    reports_dir = bench_dir / "reports"

    # Infer repo root: walk up from bench_dir looking for Cargo.toml
    p = bench_dir
    while p != p.parent:
        if (p / "Cargo.toml").exists():
            ROOT_DIR = p
            break
        p = p.parent
    if ROOT_DIR is None:
        ROOT_DIR = bench_dir.parent.parent  # fallback: benches/{name} → repo root

    # ── Load manifest ──
    manifest = load_manifest(manifest_path)
    common_profiles = manifest["common"]
    apps = manifest["apps"]

    # ── Filter by CLI args ──
    if args.common:
        if args.common not in common_profiles:
            log_err(f"Unknown common profile '{args.common}'. "
                    f"Available: {list(common_profiles.keys())}")
            sys.exit(1)
        common_profiles = {args.common: common_profiles[args.common]}

    if args.app:
        if args.app not in apps:
            log_err(f"Unknown app '{args.app}'. Available: {list(apps.keys())}")
            sys.exit(1)
        apps = {args.app: apps[args.app]}

    # ── List mode ──
    if args.list:
        total_cells = 0
        print(f"\nManifest: {manifest_path}\n")
        print("Common profiles:")
        for name, profile in common_profiles.items():
            desc = profile.get("desc", "")
            par = profile.get("parallelism", "?")
            cores = profile.get("cpu_cores", "?")
            conns = profile.get("wrk_connections", "?")
            dur = profile.get("measure_sec", "?")
            print(f"  {name:<16} cores={cores} par={par} conns={conns} dur={dur}s — {desc}")

        print(f"\nApps:")
        for app_name, app_def in apps.items():
            configs = app_def.get("configs", [{"name": "default"}])
            config_names = [c.get("name", "default") for c in configs]
            port = app_def.get("port", "?")
            print(f"  {app_name:<20} lang={app_def.get('lang', '?'):<6} "
                  f"model={app_def.get('model', '?'):<14} "
                  f"io={app_def.get('io', '?'):<10} "
                  f"port={port:<6} configs={config_names}")

        print(f"\nExecution matrix:")
        for cname in common_profiles:
            for app_name, app_def in apps.items():
                configs = app_def.get("configs", [{"name": "default"}])
                for cfg in configs:
                    cfg_name = cfg.get("name", "default")
                    if args.config and cfg_name != args.config:
                        continue
                    print(f"  {cname}/{app_name}/{cfg_name}")
                    total_cells += 1

        print(f"\nTotal cells: {total_cells} × {args.repeat} repeats = "
              f"{total_cells * args.repeat} runs\n")
        sys.exit(0)

    # ── Collect system info ──
    system_info = collect_system_info()

    log(f"Manifest: {manifest_path}")
    log(f"Repo root: {ROOT_DIR}")
    log(f"Profiles: {list(common_profiles.keys())}")
    log(f"Apps: {list(apps.keys())}")

    # ── Create output dirs ──
    results_dir.mkdir(parents=True, exist_ok=True)
    reports_dir.mkdir(parents=True, exist_ok=True)

    # ── Detect load generator ──
    wrkr_path = None
    if not args.use_wrk:
        wrkr_path = find_wrkr(args)
    if wrkr_path:
        log(f"Load generator: wrkr ({wrkr_path})")
    elif shutil.which("wrk"):
        log(f"Load generator: wrk (fallback)")
    else:
        log_err("No load generator found. Build wrkr (cargo build -p wrkr --release) "
                "or install wrk (apt install wrk)")
        sys.exit(1)

    # ── Run the matrix ──
    all_results_by_profile = {}

    for common_name, common_profile in common_profiles.items():
        log(f"\n{'═' * 60}")
        log(f"  Profile: {common_name} — {common_profile.get('desc', '')}")
        log(f"{'═' * 60}")

        profile_results = []

        for app_name, app_def in apps.items():
            configs = app_def.get("configs", [{"name": "default"}])

            for cfg in configs:
                cfg_name = cfg.get("name", "default")

                # Filter by --config
                if args.config and cfg_name != args.config:
                    continue

                for repeat_i in range(args.repeat):
                    if args.repeat > 1:
                        log(f"  Repeat {repeat_i + 1}/{args.repeat}")

                    result = run_one_cell(
                        app_name=app_name,
                        app_def=app_def,
                        app_config=cfg,
                        common_name=common_name,
                        common_profile=common_profile,
                        bench_dir=bench_dir,
                        system_info=system_info,
                        dry_run=args.dry_run,
                        build_profile=args.build,
                        wrkr_path=wrkr_path,
                    )

                    if result:
                        profile_results.append(result)

                        # Save individual result
                        if not args.no_save and not args.dry_run and not result.get("dry_run"):
                            path = save_result(result, results_dir)
                            log(f"  Saved: {path.name}")

        all_results_by_profile[common_name] = profile_results

        # Print summary table for this profile
        if not args.dry_run:
            print_summary_table(profile_results, common_name)

        # Generate markdown report
        if not args.no_report and not args.dry_run:
            report_path = reports_dir / f"{common_name}.md"
            generate_markdown_report(profile_results, common_name, report_path)

    # ── Final summary ──
    total_cells = sum(len(v) for v in all_results_by_profile.values())
    total_ok = sum(
        1 for results in all_results_by_profile.values()
        for r in results
        if r and not r.get("dry_run") and r.get("rps", 0) > 0
    )
    log(f"\nDone. {total_ok}/{total_cells} cells completed successfully.")


if __name__ == "__main__":
    main()