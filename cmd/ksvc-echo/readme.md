```bash
cargo build --release -p ksvc-echo
# Terminal 1 — server:

./target/release/ksvc-echo 9999


# Terminal 2 — test client:


```bash

# Quick correctness (4 threads × 100 messages)
python3 cmd/ksvc-echo/test_echo.py

# Stress test (50 concurrent connections, 1000 messages each)
python3 cmd/ksvc-echo/test_echo.py -t 50 -n 1000

# Large messages
python3 cmd/ksvc-echo/test_echo.py -t 20 -n 500 -s 4000

# Full blast
python3 cmd/ksvc-echo/test_echo.py -t 100 -n 5000 -s 128
```

**What the client verifies per message:**
```
1. Generate deterministic payload: "T0042S00000123|abcdefg..."
2. Send entire payload
3. Recv until all bytes back (handles chunking)
4. Compare byte-for-byte: recv == sent
5. Record latency (send → last recv byte)
```

**Output looks like:**
```
=== KSVC Echo Test Client ===
    target:     127.0.0.1:9999
    threads:    50
    msgs/thread:1000
    total:      50000 messages, 3125.0 KB

  Per-thread results:
    thread  0: sent= 1000 recv= 1000 err=0 mismatch=0 time=1.234s  [OK]
    ...
────────────────────────────────────────────────────────────
  Throughput:     40,521 msg/s
  Latency p50:    0.082ms
  Latency p99:    1.234ms
────────────────────────────────────────────────────────────
  RESULT: PASS (50000/50000 echoed correctly)
  ```