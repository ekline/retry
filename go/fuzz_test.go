// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

package libretry

import (
	"math"
	"testing"
	"time"
)

// FuzzCompute checks properties of Compute that should hold for any
// input, not just the hand-picked cases in the other tests and the
// shared conformance vectors:
//
//   - Compute never panics.
//   - Elapsed never decreases.
//   - Retries increases by exactly 1, or stays saturated at math.MaxInt.
//   - RT is never negative when the step is not Done.
//   - The termination decision matches SPEC.md §5.4's order exactly.
//
// Run `go test -fuzz=FuzzCompute -fuzztime=30s` locally to fuzz beyond
// the seed corpus; `go test ./...` (including in CI) only replays the
// seeds below plus anything saved under testdata/fuzz/FuzzCompute/.
func FuzzCompute(f *testing.F) {
	// Seed corpus: the hand-found edge cases from bounds-checking review,
	// plus a routine case for contrast.
	f.Add(int64(time.Second), int64(30*time.Second), 10, int64(60*time.Second), 0, int64(0), int64(0), 0.1)
	f.Add(int64(math.MaxInt64), int64(0), -1, int64(0), 0, int64(0), int64(0), 0.0)
	f.Add(int64(0), int64(0), -1, int64(0), 0, int64(0), int64(0), math.Inf(1))
	f.Add(int64(0), int64(0), -1, int64(0), 0, int64(0), int64(0), math.NaN())
	f.Add(int64(time.Second), int64(0), -1, int64(0), math.MaxInt, int64(time.Second), int64(time.Second), 0.0)
	f.Add(int64(-time.Second), int64(-time.Second), -5, int64(-time.Second), -5, int64(-time.Second), int64(-time.Second), -1e300)

	f.Fuzz(func(t *testing.T,
		initialRTNs int64,
		maxIntervalNs int64,
		maxRetries int,
		maxDurationNs int64,
		prevRetries int,
		prevLastRTNs int64,
		prevElapsedNs int64,
		jitter float64,
	) {
		params := Params{
			InitialRT:   time.Duration(initialRTNs),
			MaxInterval: time.Duration(maxIntervalNs),
			MaxRetries:  maxRetries,
			MaxDuration: time.Duration(maxDurationNs),
		}
		prev := State{
			Retries: prevRetries,
			LastRT:  time.Duration(prevLastRTNs),
			Elapsed: time.Duration(prevElapsedNs),
		}

		// Must never panic for any input; that alone is the primary
		// property under test here (Go's fuzzer treats a panic as a
		// failing input).
		step := Compute(params, prev, JitterFunc(func() float64 { return jitter }))

		if step.State.Elapsed < prev.Elapsed {
			t.Fatalf("Elapsed decreased: %v -> %v", prev.Elapsed, step.State.Elapsed)
		}

		if prev.Retries == math.MaxInt {
			if step.State.Retries != math.MaxInt {
				t.Fatalf("Retries at MaxInt should stay saturated, got %d", step.State.Retries)
			}
		} else if step.State.Retries != prev.Retries+1 {
			t.Fatalf("Retries = %d, want %d", step.State.Retries, prev.Retries+1)
		}

		if !step.Done && step.RT < 0 {
			t.Fatalf("RT is negative: %v", step.RT)
		}

		wantMaxRetries := params.maxRetriesBounded() && step.State.Retries > params.MaxRetries
		wantMaxDuration := !wantMaxRetries && params.maxDurationBounded() && step.State.Elapsed > params.MaxDuration
		switch {
		case wantMaxRetries:
			if !step.Done || step.Reason != MaxRetriesExceeded {
				t.Fatalf("want GiveUp(MaxRetries), got Done=%v Reason=%v", step.Done, step.Reason)
			}
		case wantMaxDuration:
			if !step.Done || step.Reason != MaxDurationExceeded {
				t.Fatalf("want GiveUp(MaxDuration), got Done=%v Reason=%v", step.Done, step.Reason)
			}
		default:
			if step.Done {
				t.Fatalf("want Wait, got GiveUp(%v)", step.Reason)
			}
		}
	})
}
