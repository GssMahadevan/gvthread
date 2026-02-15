// go-httpd: Multi-variant HTTP/1.1 benchmark server
//
// Variants (selected via gvt_app_variant env var):
//
//   naive  — net.Listener + Accept + goroutine per conn, manual HTTP parse
//            This is the purest goroutine-per-connection model.
//            No stdlib HTTP overhead, no router, no header map allocations.
//
//   mux    — stdlib net/http with http.ServeMux
//            The idiomatic Go way. Uses net/http's goroutine-per-conn
//            plus full HTTP/1.1 parsing, chunked encoding, etc.
//
//   fiber  — gofiber/fiber v2 (fasthttp underneath)
//            Worker-pool model, not goroutine-per-conn.
//            Reuses goroutines, zero-alloc header parsing.
//
// Build: cd cmd/httpd/go && go build -o httpd-server .
// Run:   gvt_app_variant=naive gvt_app_port=8083 ./httpd-server

package main

import (
	"bufio"
	"flag"
	"fmt"
	"net"
	"net/http"
	"os"
	"runtime"
	"strconv"
	"sync/atomic"
	"time"

	"github.com/gofiber/fiber/v2"
)

// ── Shared state ──

var (
	requests  uint64
	responses uint64
)

const helloBody = "Hello from Go!\n"

// ── Main ──

func main() {
	port := flag.Int("port", 8083, "Listen port")
	variant := flag.String("variant", "naive", "Server variant: naive|mux|fiber")
	flag.Parse()

	// Bench-runner env vars override defaults
	if v := os.Getenv("gvt_app_port"); v != "" {
		if p, err := strconv.Atoi(v); err == nil {
			*port = p
		}
	}
	if v := os.Getenv("gvt_app_variant"); v != "" {
		*variant = v
	}
	if v := os.Getenv("gvt_parallelism"); v != "" {
		if p, err := strconv.Atoi(v); err == nil && p > 0 {
			runtime.GOMAXPROCS(p)
		}
	}

	fmt.Fprintf(os.Stderr, "go-httpd: port=%d variant=%s GOMAXPROCS=%d\n",
		*port, *variant, runtime.GOMAXPROCS(0))

	// Stats printer
	go statsLoop()

	addr := fmt.Sprintf("0.0.0.0:%d", *port)

	switch *variant {
	case "naive":
		runNaive(addr)
	case "mux":
		runMux(addr)
	case "fiber":
		runFiber(addr)
	default:
		fmt.Fprintf(os.Stderr, "go-httpd: unknown variant %q (use naive|mux|fiber)\n", *variant)
		os.Exit(1)
	}
}

// ── Stats ──

func statsLoop() {
	start := time.Now()
	var lastResp uint64
	for {
		time.Sleep(5 * time.Second)
		resp := atomic.LoadUint64(&responses)
		delta := resp - lastResp
		rps := float64(delta) / 5.0
		fmt.Fprintf(os.Stderr, "[%.1fs] resp=%d rps=%.0f\n",
			time.Since(start).Seconds(), resp, rps)
		lastResp = resp
	}
}

// ════════════════════════════════════════════════════════════════════
// Variant: naive — raw net.Listener, manual HTTP
// ════════════════════════════════════════════════════════════════════
//
// Closest to what gvthread-httpd does:
//   - Accept loop → goroutine per connection
//   - Read until \r\n\r\n
//   - Write fixed response
//   - Keep-alive loop
//
// No stdlib net/http, no router, no header map, no bufio.Writer pool.

var naiveResponse = []byte(fmt.Sprintf(
	"HTTP/1.1 200 OK\r\n"+
		"Content-Type: text/plain\r\n"+
		"Content-Length: %d\r\n"+
		"Connection: keep-alive\r\n"+
		"Server: go-httpd\r\n"+
		"\r\n%s",
	len(helloBody), helloBody,
))

func runNaive(addr string) {
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		fmt.Fprintf(os.Stderr, "go-httpd: listen: %v\n", err)
		os.Exit(1)
	}
	fmt.Fprintf(os.Stderr, "go-httpd [naive]: listening on http://%s/\n", addr)

	for {
		conn, err := ln.Accept()
		if err != nil {
			continue
		}
		go handleNaive(conn)
	}
}

func handleNaive(conn net.Conn) {
	defer conn.Close()

	reader := bufio.NewReaderSize(conn, 4096)

	for {
		// Read until \r\n\r\n (end of HTTP headers)
		if !readUntilHeaderEnd(reader) {
			return // EOF or error
		}

		atomic.AddUint64(&requests, 1)

		// Write fixed response
		_, err := conn.Write(naiveResponse)
		if err != nil {
			return
		}

		atomic.AddUint64(&responses, 1)
	}
}

// readUntilHeaderEnd reads byte-by-byte until \r\n\r\n or error.
func readUntilHeaderEnd(r *bufio.Reader) bool {
	consecutive := 0
	for {
		b, err := r.ReadByte()
		if err != nil {
			return false
		}
		switch {
		case consecutive == 0 && b == '\r':
			consecutive = 1
		case consecutive == 1 && b == '\n':
			consecutive = 2
		case consecutive == 2 && b == '\r':
			consecutive = 3
		case consecutive == 3 && b == '\n':
			return true
		default:
			consecutive = 0
			if b == '\r' {
				consecutive = 1
			}
		}
	}
}

// ════════════════════════════════════════════════════════════════════
// Variant: mux — stdlib net/http with ServeMux
// ════════════════════════════════════════════════════════════════════
//
// The idiomatic Go HTTP server. Uses net/http's built-in:
//   - Goroutine per connection (inside ListenAndServe)
//   - Full HTTP/1.1 parsing with bufio pools
//   - ServeMux routing
//   - ResponseWriter with header map

func runMux(addr string) {
	helloBytes := []byte(helloBody)
	contentLen := strconv.Itoa(len(helloBytes))

	mux := http.NewServeMux()
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		atomic.AddUint64(&requests, 1)
		w.Header().Set("Content-Type", "text/plain")
		w.Header().Set("Connection", "keep-alive")
		w.Header().Set("Server", "go-httpd")
		w.Header().Set("Content-Length", contentLen)
		w.Write(helloBytes)
		atomic.AddUint64(&responses, 1)
	})

	server := &http.Server{
		Addr:    addr,
		Handler: mux,
	}

	fmt.Fprintf(os.Stderr, "go-httpd [mux]: listening on http://%s/\n", addr)

	if err := server.ListenAndServe(); err != nil {
		fmt.Fprintf(os.Stderr, "go-httpd: %v\n", err)
		os.Exit(1)
	}
}

// ════════════════════════════════════════════════════════════════════
// Variant: fiber — gofiber/fiber v2 (fasthttp)
// ════════════════════════════════════════════════════════════════════
//
// Different architecture from net/http:
//   - Worker pool (not goroutine-per-conn)
//   - Reuses goroutines across connections
//   - Zero-alloc header/path parsing
//   - Pre-allocated buffers

func runFiber(addr string) {
	app := fiber.New(fiber.Config{
		ServerHeader:          "go-httpd",
		DisableStartupMessage: true,
		Prefork:               false,
	})

	helloBytes := []byte(helloBody)

	app.Get("/", func(c *fiber.Ctx) error {
		atomic.AddUint64(&requests, 1)
		c.Set("Content-Type", "text/plain")
		c.Set("Connection", "keep-alive")
		err := c.Send(helloBytes)
		atomic.AddUint64(&responses, 1)
		return err
	})

	fmt.Fprintf(os.Stderr, "go-httpd [fiber]: listening on http://%s/\n", addr)

	if err := app.Listen(addr); err != nil {
		fmt.Fprintf(os.Stderr, "go-httpd: %v\n", err)
		os.Exit(1)
	}
}