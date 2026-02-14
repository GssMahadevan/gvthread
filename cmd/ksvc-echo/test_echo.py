#!/usr/bin/env python3
"""
KSVC Echo Server Test Client

Tests correctness and throughput of ksvc-echo.

Usage:
    python3 test_echo.py [options]

Examples:
    # Quick correctness check (default: 4 threads, 100 msgs each)
    python3 test_echo.py

    # Stress test
    python3 test_echo.py --threads 50 --messages 1000

    # Custom host/port
    python3 test_echo.py --host 192.168.1.10 --port 8080

    # Large messages
    python3 test_echo.py --size 4000 --threads 20 --messages 500
"""

import argparse
import socket
import threading
import time
import sys
import random
import string

class WorkerResult:
    """Results from one worker thread."""
    __slots__ = ['thread_id', 'sent', 'received', 'errors', 'mismatches',
                 'bytes_sent', 'bytes_recv', 'elapsed', 'latencies']

    def __init__(self, thread_id):
        self.thread_id = thread_id
        self.sent = 0
        self.received = 0
        self.errors = 0
        self.mismatches = 0
        self.bytes_sent = 0
        self.bytes_recv = 0
        self.elapsed = 0.0
        self.latencies = []


def make_payload(size, seq, thread_id):
    """Generate a deterministic payload for verification."""
    header = f"T{thread_id:04d}S{seq:08d}|"
    remaining = size - len(header)
    if remaining <= 0:
        return header[:size].encode()
    # Fill with repeating pattern (deterministic, verifiable)
    body = (string.ascii_lowercase * ((remaining // 26) + 1))[:remaining]
    return (header + body).encode()


def worker(thread_id, host, port, num_messages, msg_size, results, barrier):
    """Worker thread: connect, send/recv num_messages, verify each echo."""
    r = WorkerResult(thread_id)

    # Wait for all threads to be ready
    barrier.wait()

    start = time.monotonic()

    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.settimeout(5.0)
        sock.connect((host, port))
    except Exception as e:
        r.errors = num_messages
        r.elapsed = time.monotonic() - start
        results[thread_id] = r
        return

    try:
        for seq in range(num_messages):
            payload = make_payload(msg_size, seq, thread_id)

            t0 = time.monotonic()

            # Send
            try:
                sock.sendall(payload)
                r.sent += 1
                r.bytes_sent += len(payload)
            except Exception as e:
                r.errors += 1
                continue

            # Receive (may come in chunks)
            try:
                received = b""
                while len(received) < len(payload):
                    chunk = sock.recv(len(payload) - len(received))
                    if not chunk:
                        r.errors += 1
                        break
                    received += chunk
                else:
                    r.received += 1
                    r.bytes_recv += len(received)

                    # Verify echo matches
                    if received != payload:
                        r.mismatches += 1

                    r.latencies.append(time.monotonic() - t0)
            except socket.timeout:
                r.errors += 1
            except Exception as e:
                r.errors += 1

    finally:
        sock.close()

    r.elapsed = time.monotonic() - start
    results[thread_id] = r


def main():
    parser = argparse.ArgumentParser(description="KSVC Echo Server Test Client")
    parser.add_argument("--host", default="127.0.0.1", help="Server host (default: 127.0.0.1)")
    parser.add_argument("--port", type=int, default=9999, help="Server port (default: 9999)")
    parser.add_argument("--threads", "-t", type=int, default=4, help="Number of concurrent connections (default: 4)")
    parser.add_argument("--messages", "-n", type=int, default=100, help="Messages per thread (default: 100)")
    parser.add_argument("--size", "-s", type=int, default=64, help="Message size in bytes (default: 64)")
    parser.add_argument("--quiet", "-q", action="store_true", help="Only print summary")
    args = parser.parse_args()

    total_msgs = args.threads * args.messages
    print(f"=== KSVC Echo Test Client ===")
    print(f"    target:     {args.host}:{args.port}")
    print(f"    threads:    {args.threads}")
    print(f"    msgs/thread:{args.messages}")
    print(f"    msg size:   {args.size} bytes")
    print(f"    total:      {total_msgs} messages, {total_msgs * args.size / 1024:.1f} KB")
    print()

    # Quick connectivity check
    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.settimeout(2.0)
        sock.connect((args.host, args.port))
        sock.sendall(b"hello\n")
        reply = sock.recv(64)
        sock.close()
        if reply != b"hello\n":
            print(f"  WARNING: echo mismatch in preflight (got {reply!r})")
        else:
            print(f"  preflight: OK")
    except Exception as e:
        print(f"  ERROR: cannot connect to {args.host}:{args.port}: {e}")
        print(f"  Is ksvc-echo running?")
        sys.exit(1)

    # Launch workers
    results = [None] * args.threads
    barrier = threading.Barrier(args.threads)
    threads = []

    wall_start = time.monotonic()

    for i in range(args.threads):
        t = threading.Thread(
            target=worker,
            args=(i, args.host, args.port, args.messages, args.size, results, barrier),
            daemon=True,
        )
        threads.append(t)
        t.start()

    for t in threads:
        t.join(timeout=60)

    wall_elapsed = time.monotonic() - wall_start

    # Aggregate results
    total_sent = 0
    total_recv = 0
    total_errors = 0
    total_mismatches = 0
    total_bytes_in = 0
    total_bytes_out = 0
    all_latencies = []

    for r in results:
        if r is None:
            total_errors += 1
            continue
        total_sent += r.sent
        total_recv += r.received
        total_errors += r.errors
        total_mismatches += r.mismatches
        total_bytes_in += r.bytes_recv
        total_bytes_out += r.bytes_sent
        all_latencies.extend(r.latencies)

    # Per-thread detail
    if not args.quiet:
        print(f"\n  Per-thread results:")
        for r in results:
            if r is None:
                print(f"    thread ??: FAILED (no result)")
                continue
            status = "OK" if r.errors == 0 and r.mismatches == 0 else "FAIL"
            print(f"    thread {r.thread_id:2d}: sent={r.sent:5d} recv={r.received:5d} "
                  f"err={r.errors} mismatch={r.mismatches} "
                  f"time={r.elapsed:.3f}s  [{status}]")

    # Summary
    print(f"\n{'─' * 60}")
    print(f"  SUMMARY")
    print(f"{'─' * 60}")
    print(f"  Wall time:      {wall_elapsed:.3f}s")
    print(f"  Sent:           {total_sent}")
    print(f"  Received:       {total_recv}")
    print(f"  Errors:         {total_errors}")
    print(f"  Mismatches:     {total_mismatches}")
    print(f"  Data out:       {total_bytes_out / 1024:.1f} KB")
    print(f"  Data in:        {total_bytes_in / 1024:.1f} KB")

    if wall_elapsed > 0:
        msgs_per_sec = total_recv / wall_elapsed
        mb_per_sec = (total_bytes_in + total_bytes_out) / (1024 * 1024) / wall_elapsed
        print(f"  Throughput:     {msgs_per_sec:,.0f} msg/s")
        print(f"  Bandwidth:      {mb_per_sec:.2f} MB/s")

    if all_latencies:
        all_latencies.sort()
        n = len(all_latencies)
        p50 = all_latencies[int(n * 0.50)] * 1000
        p90 = all_latencies[int(n * 0.90)] * 1000
        p99 = all_latencies[min(int(n * 0.99), n - 1)] * 1000
        avg = sum(all_latencies) / n * 1000
        print(f"  Latency avg:    {avg:.3f}ms")
        print(f"  Latency p50:    {p50:.3f}ms")
        print(f"  Latency p90:    {p90:.3f}ms")
        print(f"  Latency p99:    {p99:.3f}ms")

    print(f"{'─' * 60}")

    # Exit code
    if total_errors > 0 or total_mismatches > 0:
        print(f"\n  RESULT: FAIL ({total_errors} errors, {total_mismatches} mismatches)")
        sys.exit(1)
    else:
        print(f"\n  RESULT: PASS ({total_recv}/{total_msgs} echoed correctly)")
        sys.exit(0)


if __name__ == "__main__":
    main()
