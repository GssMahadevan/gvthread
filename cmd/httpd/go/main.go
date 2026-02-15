// go-httpd: HTTP/1.1 server using Go's net/http (goroutine-per-conn)
//
// Same functionality as ksvc-httpd for fair comparison.
//
// Build: go build -o go-httpd main.go
// Run:   ./go-httpd [--port 8082] [--dir ./www]
// Bench: wrk -t4 -c100 -d10s http://127.0.0.1:8082/

package main

import (
	"flag"
	"fmt"
	"net/http"
	"os"
	"runtime"
	"sync/atomic"
	"time"
)

var (
	requests  uint64
	responses uint64
	errors    uint64
)

func main() {
	port := flag.Int("port", 8082, "Listen port")
	dir := flag.String("dir", "", "Directory to serve (default: hello world mode)")
	flag.Parse()

	// Also accept bare port as first positional arg
	if flag.NArg() > 0 {
		var p int
		if _, err := fmt.Sscanf(flag.Arg(0), "%d", &p); err == nil {
			*port = p
		}
	}

	// Bench-runner env vars override CLI defaults (but not explicit flags)
	if envPort := os.Getenv("gvt_app_port"); envPort != "" {
		var p int
		if _, err := fmt.Sscanf(envPort, "%d", &p); err == nil {
			*port = p
		}
	}

	// GOMAXPROCS from gvt_parallelism
	if envPar := os.Getenv("gvt_parallelism"); envPar != "" {
		var p int
		if _, err := fmt.Sscanf(envPar, "%d", &p); err == nil && p > 0 {
			runtime.GOMAXPROCS(p)
		}
	}

	fileMode := *dir != ""
	modeStr := "hello"
	if fileMode {
		modeStr = fmt.Sprintf("file(%s)", *dir)
	}
	fmt.Fprintf(os.Stderr, "go-httpd: port=%d mode=%s GOMAXPROCS=%d\n", *port, modeStr, runtime.GOMAXPROCS(0))

	// Stats printer
	start := time.Now()
	go func() {
		for {
			time.Sleep(5 * time.Second)
			elapsed := time.Since(start).Seconds()
			resp := atomic.LoadUint64(&responses)
			rps := float64(resp) / elapsed
			fmt.Fprintf(os.Stderr, "[%.1fs] req=%d resp=%d rps=%.0f err=%d\n",
				elapsed,
				atomic.LoadUint64(&requests),
				resp, rps,
				atomic.LoadUint64(&errors),
			)
		}
	}()

	helloBody := []byte("Hello from Go!\n")
	helloResp := fmt.Sprintf(
		"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: %d\r\nConnection: keep-alive\r\nServer: go-httpd\r\n\r\n%s",
		len(helloBody), helloBody,
	)
	_ = helloResp // We'll use http.Handler instead for fair comparison

	mux := http.NewServeMux()

	if fileMode {
		fs := http.FileServer(http.Dir(*dir))
		mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
			atomic.AddUint64(&requests, 1)
			w.Header().Set("Connection", "keep-alive")
			w.Header().Set("Server", "go-httpd")
			fs.ServeHTTP(w, r)
			atomic.AddUint64(&responses, 1)
		})
	} else {
		mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
			atomic.AddUint64(&requests, 1)
			w.Header().Set("Content-Type", "text/plain")
			w.Header().Set("Connection", "keep-alive")
			w.Header().Set("Server", "go-httpd")
			w.Header().Set("Content-Length", fmt.Sprintf("%d", len(helloBody)))
			w.Write(helloBody)
			atomic.AddUint64(&responses, 1)
		})
	}

	server := &http.Server{
		Addr:    fmt.Sprintf("0.0.0.0:%d", *port),
		Handler: mux,
	}

	fmt.Fprintf(os.Stderr, "go-httpd: listening on http://0.0.0.0:%d/\n", *port)

	if err := server.ListenAndServe(); err != nil {
		fmt.Fprintf(os.Stderr, "go-httpd: %v\n", err)
		os.Exit(1)
	}
}