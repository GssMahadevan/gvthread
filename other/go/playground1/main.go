// Equivalent Go program to compare with GVThread
//
// Usage:
//   go build -o go_benchmark go_benchmark.go
//   GVT_WORKERS=4 GVT_GVTHREADS=2000 GVT_YIELDS=300 GVT_SLEEP_MS=100 ./go_benchmark
//
// Or with GOMAXPROCS to match worker count:
//   GOMAXPROCS=4 ./go_benchmark -goroutines=2000 -yields=300 -sleep=100

package main

import (
	"flag"
	"fmt"
	"os"
	"runtime"
	"strconv"
	"sync"
	"sync/atomic"
	"time"
)

func getEnvInt(key string, defaultVal int) int {
	if val := os.Getenv(key); val != "" {
		if i, err := strconv.Atoi(val); err == nil {
			return i
		}
	}
	return defaultVal
}

func main() {
	// Parse flags (can also use GVT_* env vars for compatibility)
	numGoroutines := flag.Int("goroutines", getEnvInt("GVT_GVTHREADS", 2000), "Number of goroutines")
	yieldsPerGoroutine := flag.Int("yields", getEnvInt("GVT_YIELDS", 300), "Yields per goroutine")
	sleepMs := flag.Int("sleep", getEnvInt("GVT_SLEEP_MS", 100), "Sleep time in ms per yield")
	workers := flag.Int("workers", getEnvInt("GVT_WORKERS", 4), "GOMAXPROCS (worker threads)")
	flag.Parse()

	// Set GOMAXPROCS to match worker count
	runtime.GOMAXPROCS(*workers)

	fmt.Println("=== Go Goroutine Benchmark ===")
	fmt.Println()
	fmt.Printf("Configuration:\n")
	fmt.Printf("  GOMAXPROCS (workers): %d\n", *workers)
	fmt.Printf("  Goroutines: %d\n", *numGoroutines)
	fmt.Printf("  Yields per goroutine: %d\n", *yieldsPerGoroutine)
	fmt.Printf("  Sleep per yield: %dms\n", *sleepMs)
	fmt.Println()

	var started int64
	var completed int64
	var totalYields int64
	var wg sync.WaitGroup

	sleepDuration := time.Duration(*sleepMs) * time.Millisecond
	expectedYields := int64(*numGoroutines) * int64(*yieldsPerGoroutine)

	fmt.Printf("[INFO] Spawning %d goroutines, each yielding %d times\n", *numGoroutines, *yieldsPerGoroutine)

	startTime := time.Now()

	// Spawn all goroutines
	for i := 0; i < *numGoroutines; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			atomic.AddInt64(&started, 1)

			for j := 0; j < *yieldsPerGoroutine; j++ {
				time.Sleep(sleepDuration)
				atomic.AddInt64(&totalYields, 1)
			}

			atomic.AddInt64(&completed, 1)
		}(i)

		// Progress indicator every 1000
		if (i+1)%1000 == 0 {
			fmt.Printf("[SPAWN] %d/%d\n", i+1, *numGoroutines)
		}
	}

	fmt.Printf("[INFO] Spawned %d goroutines\n", *numGoroutines)

	// Progress reporter
	done := make(chan struct{})
	go func() {
		ticker := time.NewTicker(5 * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-ticker.C:
				c := atomic.LoadInt64(&completed)
				y := atomic.LoadInt64(&totalYields)
				fmt.Printf("[PROGRESS] completed=%d/%d, yields=%d/%d\n",
					c, *numGoroutines, y, expectedYields)
			case <-done:
				return
			}
		}
	}()

	// Wait for all to complete
	wg.Wait()
	close(done)

	elapsed := time.Since(startTime)

	fmt.Println()
	fmt.Println("=== Results ===")
	fmt.Printf("Started:   %d/%d\n", started, *numGoroutines)
	fmt.Printf("Completed: %d/%d\n", completed, *numGoroutines)
	fmt.Printf("Yields:    %d (expected: %d)\n", totalYields, expectedYields)
	fmt.Printf("Time:      %v\n", elapsed)

	if completed == int64(*numGoroutines) && totalYields == expectedYields {
		fmt.Println("*** SUCCESS ***")
	} else {
		fmt.Println("*** FAILED ***")
	}
}