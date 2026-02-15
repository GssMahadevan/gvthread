````bash
cargo build -p wrkr --release
# bench-runner auto-detects target/release/wrkr
python3 benches/bench-runner.py benches/httpd/manifest.yml --common light



# Standalone usage:
target/release/wrkr http://127.0.0.1:8080/ -c50 -d5

# Bench-runner auto-detects wrkr in target/<build>/wrkr:
python3 benches/bench-runner.py benches/httpd/manifest.yml --common light

# Force wrk fallback:
python3 benches/bench-runner.py benches/httpd/manifest.yml --common light --use-wrk


```

