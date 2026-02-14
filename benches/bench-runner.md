### Key design points implemented
#### Common is law 
 * BR reads common/{profile}, exports every KV as gvt_{K}={V}. 
 * If an app config contains any key that also exists in common, BR rejects it with an error. No override possible.

#### App config is additive
 * exported as gvt_app_{K}={V}. 
 * Only knobs unique to that runtime (sq_entries, variant, max_gvthreads).


#### parallelism is in common
 * every app reads gvt_parallelism and maps it to their own concept (threads/workers/GOMAXPROCS). 
 * No app gets more CPU than another.

#### CPU pinning is external
 * BR runs taskset -c 0-{N-1} around the binary. 
 * Apps don't self-pin.

#### No shared library needed
 * apps just read env vars. 
 * Go needs os.Getenv("gvt_parallelism"), Rust needs std::env::var("gvt_parallelism").


### Runner invovation with CLI filtering
```bash

# Run one app, one config, one profile
python bench/bench-runner.py benches/httpd/manifest.yml --common medium --app ksvc-httpd --config 4t
python bench/bench-runner.py benches/httpd/manifest.yml                          # full matrix
python bench/bench-runner.py benches/httpd/manifest.yml --common light           # one profile
python bench/bench-runner.py benches/httpd/manifest.yml --common heavy --app go-httpd --config pooled
python bench/bench-runner.py benches/httpd/manifest.yml --list                   # show matrix
python bench/bench-runner.py benches/httpd/manifest.yml --dry-run                # plan, don't execute
python bench/bench-runner.py benches/httpd/manifest.yml --repeat 3               # statistical runs


```


