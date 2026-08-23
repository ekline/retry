// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

package retry

import (
	"math"
	"math/rand/v2"
	"testing"
	"time"
)

func ms(n int64) time.Duration { return time.Duration(n) * time.Millisecond }

func TestFirstCallUsesInitialRT(t *testing.T) {
	params := Params{InitialRT: ms(1000), MaxRetries: -1}
	step := Compute(params, State{}, NewFixedJitter([]float64{0}))
	if step.Done {
		t.Fatalf("expected Done=false, got Done=true (reason %v)", step.Reason)
	}
	if step.RT != ms(1000) {
		t.Errorf("RT = %v, want %v", step.RT, ms(1000))
	}
	if step.State.Retries != 1 {
		t.Errorf("Retries = %d, want 1", step.State.Retries)
	}
}

func TestDoublingWithoutCap(t *testing.T) {
	params := Params{InitialRT: ms(500), MaxRetries: -1}
	jitter := NewFixedJitter([]float64{0, 0, 0, 0})
	state := State{}
	want := []time.Duration{ms(500), ms(1000), ms(2000), ms(4000)}
	for i, w := range want {
		step := Compute(params, state, jitter)
		if step.RT != w {
			t.Errorf("step %d: RT = %v, want %v", i, step.RT, w)
		}
		state = step.State
	}
}

func TestMaxIntervalCaps(t *testing.T) {
	params := Params{InitialRT: ms(1000), MaxInterval: ms(3000), MaxRetries: -1}
	jitter := NewFixedJitter([]float64{0, 0, 0, 0})
	state := State{}
	want := []time.Duration{ms(1000), ms(2000), ms(3000), ms(3000)}
	for i, w := range want {
		step := Compute(params, state, jitter)
		if step.RT != w {
			t.Errorf("step %d: RT = %v, want %v", i, step.RT, w)
		}
		state = step.State
	}
}

func TestFirstCallDetectionUsesRetriesNotLastRT(t *testing.T) {
	// A caller resuming from a checkpointed state with Retries > 0 (even
	// if LastRT happens to be zero) must NOT be treated as a first call.
	params := Params{InitialRT: ms(1000), MaxRetries: -1}
	prev := State{Retries: 1, LastRT: 0, Elapsed: ms(1000)}
	step := Compute(params, prev, NewFixedJitter([]float64{0}))
	// base = saturatingScale(0) = 0, not InitialRT.
	if step.RT != 0 {
		t.Errorf("RT = %v, want 0 (base should double LastRT, not reuse InitialRT)", step.RT)
	}
}

func TestJitterPositive(t *testing.T) {
	params := Params{InitialRT: ms(1000), MaxRetries: -1}
	step := Compute(params, State{}, NewFixedJitter([]float64{0.1}))
	if step.RT != ms(1100) {
		t.Errorf("RT = %v, want %v", step.RT, ms(1100))
	}
}

func TestJitterNegativeSaturatesToZero(t *testing.T) {
	params := Params{InitialRT: ms(1000), MaxRetries: -1}
	step := Compute(params, State{}, NewFixedJitter([]float64{-1.5}))
	if step.RT != 0 {
		t.Errorf("RT = %v, want 0", step.RT)
	}
	if step.State.Elapsed != 0 {
		t.Errorf("Elapsed = %v, want 0", step.State.Elapsed)
	}
}

func TestMaxRetriesZeroGivesUpImmediately(t *testing.T) {
	params := Params{InitialRT: ms(1000), MaxRetries: 0}
	step := Compute(params, State{}, NewFixedJitter([]float64{0}))
	if !step.Done {
		t.Fatalf("expected Done=true")
	}
	if step.Reason != MaxRetriesExceeded {
		t.Errorf("Reason = %v, want MaxRetriesExceeded", step.Reason)
	}
	if step.State.Retries != 1 {
		t.Errorf("Retries = %d, want 1 (the give-up call still counts)", step.State.Retries)
	}
}

func TestMaxRetriesExceeded(t *testing.T) {
	params := Params{InitialRT: ms(100), MaxRetries: 2}
	jitter := NewFixedJitter([]float64{0, 0, 0})
	state := State{}
	for i := 0; i < 2; i++ {
		step := Compute(params, state, jitter)
		if step.Done {
			t.Fatalf("step %d: unexpected Done=true", i)
		}
		state = step.State
	}
	step := Compute(params, state, jitter)
	if !step.Done || step.Reason != MaxRetriesExceeded {
		t.Fatalf("3rd call: Done=%v Reason=%v, want Done=true Reason=MaxRetriesExceeded", step.Done, step.Reason)
	}
}

func TestMaxDurationExceeded(t *testing.T) {
	params := Params{InitialRT: ms(1000), MaxInterval: ms(10000), MaxDuration: ms(5000), MaxRetries: -1}
	jitter := NewFixedJitter([]float64{0, 0, 0})
	state := State{}
	// 1000, 2000 -> elapsed 3000 (ok), then 4000 -> elapsed 7000 (exceeds 5000).
	for i := 0; i < 2; i++ {
		step := Compute(params, state, jitter)
		if step.Done {
			t.Fatalf("step %d: unexpected Done=true", i)
		}
		state = step.State
	}
	step := Compute(params, state, jitter)
	if !step.Done || step.Reason != MaxDurationExceeded {
		t.Fatalf("3rd call: Done=%v Reason=%v, want Done=true Reason=MaxDurationExceeded", step.Done, step.Reason)
	}
}

func TestFirstRTExceedingMaxDurationGivesUpImmediately(t *testing.T) {
	params := Params{InitialRT: ms(1000), MaxDuration: ms(500), MaxRetries: -1}
	step := Compute(params, State{}, NewFixedJitter([]float64{0}))
	if !step.Done || step.Reason != MaxDurationExceeded {
		t.Fatalf("Done=%v Reason=%v, want Done=true Reason=MaxDurationExceeded", step.Done, step.Reason)
	}
}

func TestRekeyMidSequence(t *testing.T) {
	params := Params{InitialRT: ms(1000), MaxInterval: ms(5000), MaxRetries: -1}
	jitter := NewFixedJitter([]float64{0, 0, 0})
	state := State{}
	for i := 0; i < 3; i++ {
		step := Compute(params, state, jitter)
		state = step.State
	}
	if state.LastRT != ms(4000) {
		t.Fatalf("checkpoint LastRT = %v, want %v", state.LastRT, ms(4000))
	}

	// Re-key: lower MRT to 3000ms. State carries forward unchanged.
	params.MaxInterval = ms(3000)
	step := Compute(params, state, NewFixedJitter([]float64{0}))
	if step.RT != ms(3000) {
		t.Errorf("post-rekey RT = %v, want %v (capped by new MRT)", step.RT, ms(3000))
	}
}

func TestFixedJitterExhaustionRepeatsLastValue(t *testing.T) {
	fj := NewFixedJitter([]float64{0.5, -0.25})
	if got := fj.NextJitter(); got != 0.5 {
		t.Fatalf("1st NextJitter() = %v, want 0.5", got)
	}
	if got := fj.NextJitter(); got != -0.25 {
		t.Fatalf("2nd NextJitter() = %v, want -0.25", got)
	}
	for i := 0; i < 3; i++ {
		if got := fj.NextJitter(); got != -0.25 {
			t.Fatalf("post-exhaustion NextJitter() #%d = %v, want -0.25 (the last value, repeated)", i, got)
		}
	}
}

func TestFixedJitterEmptyReturnsZero(t *testing.T) {
	// An empty FixedJitter has no "last value" to repeat, so it must fall
	// back to 0.0 -- the only sensible default when there's nothing to
	// replay.
	fj := NewFixedJitter(nil)
	for i := 0; i < 3; i++ {
		if got := fj.NextJitter(); got != 0.0 {
			t.Fatalf("NextJitter() #%d = %v, want 0.0", i, got)
		}
	}
}

func TestFixedJitterCopiesInput(t *testing.T) {
	values := []float64{1, 2, 3}
	fj := NewFixedJitter(values)
	values[0] = 99
	if got := fj.NextJitter(); got != 1 {
		t.Fatalf("NextJitter() = %v, want 1 (should be unaffected by later mutation)", got)
	}
}

func TestUniformJitterWithinBounds(t *testing.T) {
	rng := rand.New(rand.NewPCG(1, 2))
	uj := NewUniformJitter(rng, 0.1)
	for i := 0; i < 1000; i++ {
		v := uj.NextJitter()
		if v < -0.1 || v > 0.1 {
			t.Fatalf("NextJitter() = %v, out of bounds [-0.1, 0.1]", v)
		}
	}
}

func TestTerminationString(t *testing.T) {
	cases := map[Termination]string{
		MaxRetriesExceeded:  "max_retries",
		MaxDurationExceeded: "max_duration",
		notDone:             "not_done",
	}
	for term, want := range cases {
		if got := term.String(); got != want {
			t.Errorf("%v.String() = %q, want %q", int(term), got, want)
		}
	}
}

func TestNewParamsDefaultsUnbounded(t *testing.T) {
	params := NewParams(ms(1000))
	if params.maxIntervalBounded() || params.maxRetriesBounded() || params.maxDurationBounded() {
		t.Fatalf("NewParams with no options should be fully unbounded, got %+v", params)
	}
	// Unlike NewParams, the zero value is NOT fully unbounded (MaxRetries
	// defaults to 0, "no retransmissions permitted"). This test documents
	// that NewParams exists specifically to avoid that footgun.
	if (Params{}).maxRetriesBounded() != true {
		t.Fatalf("expected the zero value's MaxRetries to be bounded (0), documenting the footgun NewParams avoids")
	}
}

func TestNewParamsWithOptions(t *testing.T) {
	params := NewParams(ms(1000),
		WithMaxInterval(ms(30000)),
		WithMaxRetries(10),
		WithMaxDuration(ms(60000)),
	)
	want := Params{InitialRT: ms(1000), MaxInterval: ms(30000), MaxRetries: 10, MaxDuration: ms(60000)}
	if params != want {
		t.Fatalf("NewParams(...) = %+v, want %+v", params, want)
	}
}

func TestJitterFunc(t *testing.T) {
	calls := 0
	jitter := JitterFunc(func() float64 {
		calls++
		return 0.25
	})
	if got := jitter.NextJitter(); got != 0.25 {
		t.Errorf("NextJitter() = %v, want 0.25", got)
	}
	if calls != 1 {
		t.Errorf("underlying function called %d times, want 1", calls)
	}
}

func TestSequenceBasic(t *testing.T) {
	params := NewParams(ms(500))
	seq := NewSequence(params, NewFixedJitter([]float64{0, 0, 0}))
	want := []time.Duration{ms(500), ms(1000), ms(2000)}
	for i, w := range want {
		if !seq.Next() {
			reason, _ := seq.Reason()
			t.Fatalf("step %d: Next() = false unexpectedly (reason %v)", i, reason)
		}
		if seq.RT() != w {
			t.Errorf("step %d: RT() = %v, want %v", i, seq.RT(), w)
		}
	}
	if got := seq.State().Retries; got != 3 {
		t.Errorf("State().Retries = %d, want 3", got)
	}
}

func TestSequenceGivesUpAndStaysGivenUp(t *testing.T) {
	params := NewParams(ms(1000), WithMaxRetries(1))
	seq := NewSequence(params, NewFixedJitter([]float64{0, 0, 0}))
	if !seq.Next() {
		t.Fatalf("1st Next() = false, want true")
	}
	if seq.Next() {
		t.Fatalf("2nd Next() = true, want false")
	}
	reason, gaveUp := seq.Reason()
	if !gaveUp || reason != MaxRetriesExceeded {
		t.Fatalf("Reason() = (%v, %v), want (MaxRetriesExceeded, true)", reason, gaveUp)
	}
	// Once given up, Next keeps returning false without recomputing.
	if seq.Next() {
		t.Fatalf("3rd Next() = true, want false (should stay given up)")
	}
}

func TestSequenceSetParamsRekeys(t *testing.T) {
	params := NewParams(ms(1000), WithMaxInterval(ms(5000)))
	seq := NewSequence(params, NewFixedJitter([]float64{0, 0, 0}))
	for i := 0; i < 3; i++ {
		if !seq.Next() {
			t.Fatalf("step %d: Next() = false unexpectedly", i)
		}
	}
	if seq.RT() != ms(4000) {
		t.Fatalf("checkpoint RT() = %v, want %v", seq.RT(), ms(4000))
	}

	seq.SetParams(NewParams(ms(1000), WithMaxInterval(ms(3000))))
	if !seq.Next() {
		t.Fatalf("post-rekey Next() = false unexpectedly")
	}
	if seq.RT() != ms(3000) {
		t.Errorf("post-rekey RT() = %v, want %v (capped by new MRT)", seq.RT(), ms(3000))
	}
}

func TestApplyJitterNaNFallsBackToUnjitteredBase(t *testing.T) {
	params := NewParams(ms(1000))
	step := Compute(params, State{}, JitterFunc(func() float64 { return math.NaN() }))
	if step.RT != ms(1000) {
		t.Errorf("NaN jitter: RT = %v, want %v (should fall back to unjittered base)", step.RT, ms(1000))
	}
}

func TestApplyJitterNaNFromZeroTimesInfinityDoesNotLeak(t *testing.T) {
	// InitialRT=0 combined with +Inf jitter produces 0 * +Inf = NaN in the
	// underlying float math; this must not leak through as NaN.
	params := NewParams(0)
	step := Compute(params, State{}, JitterFunc(func() float64 { return math.Inf(1) }))
	if step.RT != 0 {
		t.Errorf("RT = %v, want 0", step.RT)
	}
}

func TestApplyJitterNearInt64MaxDoesNotGoNegative(t *testing.T) {
	// Regression: float64(math.MaxInt64) rounds up to exactly 2^63 (a
	// float64 mantissa cannot hold all 63 significant bits), so the
	// overflow guard must be >=, not >, or this silently produces a huge
	// negative Duration instead of saturating.
	params := NewParams(math.MaxInt64)
	step := Compute(params, State{}, NewFixedJitter([]float64{0}))
	if step.RT < 0 {
		t.Fatalf("RT went negative: %v", step.RT)
	}
	if step.RT != math.MaxInt64 {
		t.Errorf("RT = %v, want %v (saturated)", step.RT, time.Duration(math.MaxInt64))
	}
}

func TestRetriesSaturatesInsteadOfWrapping(t *testing.T) {
	params := NewParams(ms(1000))
	prev := State{Retries: math.MaxInt, LastRT: ms(1000), Elapsed: ms(1000)}
	step := Compute(params, prev, NewFixedJitter([]float64{0}))
	if step.State.Retries < 0 {
		t.Fatalf("Retries went negative: %d (int overflow wraparound)", step.State.Retries)
	}
	if step.State.Retries != math.MaxInt {
		t.Errorf("Retries = %d, want %d (saturated, not wrapped)", step.State.Retries, math.MaxInt)
	}
}
