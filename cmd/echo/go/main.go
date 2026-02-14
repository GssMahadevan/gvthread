// go-echo: TCP echo server using goroutines
//
// One goroutine per connection — the gold standard for
// "simple code, good performance" that GVThread aims to match.
//
// Build: go build -o go-echo main.go
// Run:   ./go-echo [port]
// Test:  python3 cmd/ksvc-echo/test_echo.py --port 9997

package main

import (
	"fmt"
	"io"
	"net"
	"os"
	"os/signal"
	"sync/atomic"
	"syscall"
	"time"
)

var (
	accepts  uint64
	bytesIn  uint64
	bytesOut uint64
	active   int64
	errors   uint64
)

func handleConn(conn net.Conn) {
	atomic.AddInt64(&active, 1)
	defer func() {
		conn.Close()
		atomic.AddInt64(&active, -1)
	}()

	buf := make([]byte, 4096)
	for {
		n, err := conn.Read(buf)
		if err != nil {
			if err != io.EOF {
				atomic.AddUint64(&errors, 1)
			}
			return
		}
		atomic.AddUint64(&bytesIn, uint64(n))

		written := 0
		for written < n {
			w, err := conn.Write(buf[written:n])
			if err != nil {
				atomic.AddUint64(&errors, 1)
				return
			}
			written += w
		}
		atomic.AddUint64(&bytesOut, uint64(n))
	}
}

func main() {
	port := "9997"
	if len(os.Args) > 1 {
		port = os.Args[1]
	}

	ln, err := net.Listen("tcp", "0.0.0.0:"+port)
	if err != nil {
		fmt.Fprintf(os.Stderr, "go-echo: listen failed: %v\n", err)
		os.Exit(1)
	}

	fmt.Fprintf(os.Stderr, "go-echo: listening on 0.0.0.0:%s (goroutine-per-conn)\n", port)

	// Stats printer
	start := time.Now()
	go func() {
		for {
			time.Sleep(5 * time.Second)
			elapsed := time.Since(start).Seconds()
			fmt.Fprintf(os.Stderr,
				"[%.1fs] active=%d accepts=%d bytes_in=%d bytes_out=%d err=%d\n",
				elapsed,
				atomic.LoadInt64(&active),
				atomic.LoadUint64(&accepts),
				atomic.LoadUint64(&bytesIn),
				atomic.LoadUint64(&bytesOut),
				atomic.LoadUint64(&errors),
			)
		}
	}()

	// Graceful shutdown on SIGINT/SIGTERM
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)
	go func() {
		<-sigCh
		fmt.Fprintf(os.Stderr, "\ngo-echo: shutting down...\n")
		ln.Close()
	}()

	for {
		conn, err := ln.Accept()
		if err != nil {
			// Check if listener was closed (shutdown)
			if opErr, ok := err.(*net.OpError); ok && !opErr.Temporary() {
				break
			}
			atomic.AddUint64(&errors, 1)
			continue
		}
		atomic.AddUint64(&accepts, 1)
		go handleConn(conn)
	}

	fmt.Fprintf(os.Stderr, "go-echo: done. accepts=%d bytes_in=%d bytes_out=%d err=%d\n",
		atomic.LoadUint64(&accepts),
		atomic.LoadUint64(&bytesIn),
		atomic.LoadUint64(&bytesOut),
		atomic.LoadUint64(&errors),
	)
}
