````bash
cargo build -p wrkr --release
RUSTFLAGS="-Awarnings" cargo build --release -p wrkr

# bench-runner auto-detects target/release/wrkr
python3 benches/bench-runner.py benches/httpd/manifest.yml --common light



# Standalone usage:
target/release/wrkr http://127.0.0.1:8080/ -c50 -d5

# Bench-runner auto-detects wrkr in target/<build>/wrkr:
python3 benches/bench-runner.py benches/httpd/manifest.yml --common light

# Force wrk fallback:
python3 benches/bench-runner.py benches/httpd/manifest.yml --common light --use-wrk
# Force reqwest for comparison:
gvt_app_http=reqwest python3 benches/bench-runner.py benches/httpd/manifest.yml --common light



```
 * You can also set it per-config in the manifest if you ever want to compare:
```yml
configs:
  - name: default
    http: hyper    # → gvt_app_http=hyper passed to wrkr
```
