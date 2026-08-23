// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

// Package retry implements a pure, sans-I/O retransmission backoff
// calculator, generic over protocol. The shape of the parameters and the
// doubling-with-jitter algorithm are drawn from RFC 9915 §15, but this
// package has no DHCPv6, CoAP, TLS, or other protocol-specific behavior.
//
// See SPEC.md at the repository root for the full design specification.
package retry

import (
	"math"
	"math/rand/v2"
	"time"
)

// Params describes an immutable retransmission schedule. Pass a new Params
// to Compute to re-key mid-sequence (e.g. when a server advertises a new
// MRT); State carries forward unchanged.
//
// Optional fields use zero-value sentinels: a zero MaxInterval or
// MaxDuration means "unbounded". A negative MaxRetries means "unbounded";
// zero means "no retransmissions permitted".
type Params struct {
	// InitialRT is the nominal first retransmission timeout (RFC 9915 IRT),
	// used as the base in the first Compute call.
	InitialRT time.Duration

	// MaxInterval is the upper bound on the pre-jitter base interval
	// (RFC 9915 MRT). Zero means unbounded.
	MaxInterval time.Duration

	// MaxRetries gives up once this many retransmissions have been
	// scheduled (RFC 9915 MRC). Negative means unbounded; zero means no
	// retransmissions are permitted at all.
	MaxRetries int

	// MaxDuration gives up once cumulative scheduled elapsed time would
	// exceed this (RFC 9915 MRD). Zero means unbounded.
	MaxDuration time.Duration
}

func (p Params) maxIntervalBounded() bool { return p.MaxInterval > 0 }
func (p Params) maxRetriesBounded() bool  { return p.MaxRetries >= 0 }
func (p Params) maxDurationBounded() bool { return p.MaxDuration > 0 }

// Option configures an optional Params field. See NewParams.
type Option func(*Params)

// WithMaxInterval sets MaxInterval (RFC 9915 MRT), the upper bound on the
// pre-jitter base interval.
func WithMaxInterval(d time.Duration) Option {
	return func(p *Params) { p.MaxInterval = d }
}

// WithMaxRetries sets MaxRetries (RFC 9915 MRC), the retry budget. Use 0
// to permit no retransmissions at all.
func WithMaxRetries(n int) Option {
	return func(p *Params) { p.MaxRetries = n }
}

// WithMaxDuration sets MaxDuration (RFC 9915 MRD), the cumulative
// scheduled-time budget.
func WithMaxDuration(d time.Duration) Option {
	return func(p *Params) { p.MaxDuration = d }
}

// NewParams returns Params with initialRT and every optional field
// unbounded unless overridden by opts:
//
//	params := retry.NewParams(time.Second,
//		retry.WithMaxInterval(30*time.Second),
//		retry.WithMaxRetries(10),
//	)
//
// This differs from the zero value Params{}, whose zero MaxRetries means
// "no retransmissions permitted" -- NewParams defaults MaxRetries to
// unbounded instead, so forgetting an option never silently forbids
// retries.
func NewParams(initialRT time.Duration, opts ...Option) Params {
	p := Params{InitialRT: initialRT, MaxRetries: -1}
	for _, opt := range opts {
		opt(&p)
	}
	return p
}

// State carries history between Compute calls. The zero value is the
// initial state, which the caller passes for the first call. States may be
// freely constructed and serialized by callers; there are no hidden
// invariants.
type State struct {
	// Retries is the number of retransmissions scheduled so far. Zero
	// before the first call.
	Retries int

	// LastRT is the most recently computed RT. Zero before the first call.
	LastRT time.Duration

	// Elapsed is the sum of all RTs scheduled so far. Zero before the
	// first call.
	Elapsed time.Duration
}

// Termination enumerates the reasons Compute can give up. Future variants
// may be added in a minor release.
type Termination int

const (
	// notDone is the zero value of Termination; Reason is meaningless on
	// a Step where Done is false.
	notDone Termination = iota

	// MaxRetriesExceeded means the configured MaxRetries (MRC) was
	// exhausted.
	MaxRetriesExceeded

	// MaxDurationExceeded means the configured MaxDuration (MRD) was
	// exceeded.
	MaxDurationExceeded
)

// String implements fmt.Stringer.
func (t Termination) String() string {
	switch t {
	case MaxRetriesExceeded:
		return "max_retries"
	case MaxDurationExceeded:
		return "max_duration"
	default:
		return "not_done"
	}
}

// Step is the result of a Compute call: either a wait instruction or a
// terminal give-up signal. State is always valid; RT is meaningful only
// when Done is false, and Reason is meaningful only when Done is true.
type Step struct {
	// RT is the computed retransmission timeout. Meaningful only when
	// Done is false.
	RT time.Duration

	// Done reports whether the caller should give up instead of
	// scheduling a retransmission.
	Done bool

	// Reason explains why Done is true. Meaningful only when Done is
	// true.
	Reason Termination

	// State is the updated state, valid regardless of Done.
	State State
}

// JitterSource supplies the multiplier applied to a candidate base RT to
// produce the actual RT. The library is agnostic to the distribution;
// bounds, shape, and bias are entirely the source's concern.
type JitterSource interface {
	// NextJitter returns the multiplier for the next Compute call.
	NextJitter() float64
}

// FixedJitter replays a fixed sequence of jitter values, returning 0.0
// once exhausted. Used for deterministic testing and conformance vector
// replay.
type FixedJitter struct {
	values []float64
	next   int
}

// NewFixedJitter returns a FixedJitter that replays values in order. The
// slice is copied; later mutations of values by the caller have no effect.
func NewFixedJitter(values []float64) *FixedJitter {
	cp := make([]float64, len(values))
	copy(cp, values)
	return &FixedJitter{values: cp}
}

// NextJitter implements JitterSource.
func (f *FixedJitter) NextJitter() float64 {
	if f.next >= len(f.values) {
		return 0.0
	}
	v := f.values[f.next]
	f.next++
	return v
}

// UniformJitter wraps a *math/rand/v2.Rand and returns uniform values in
// [-Factor, +Factor]. Go's stdlib makes this dependency free, so unlike
// the Rust crate's equivalent, it is not gated behind a build tag.
type UniformJitter struct {
	Rng    *rand.Rand
	Factor float64
}

// NewUniformJitter returns a UniformJitter using rng and the given bound.
func NewUniformJitter(rng *rand.Rand, factor float64) *UniformJitter {
	return &UniformJitter{Rng: rng, Factor: factor}
}

// NextJitter implements JitterSource.
func (u *UniformJitter) NextJitter() float64 {
	return -u.Factor + u.Rng.Float64()*2*u.Factor
}

// JitterFunc adapts a plain function to a JitterSource, mirroring
// http.HandlerFunc. Useful for one-off jitter sources that don't warrant
// a named type:
//
//	jitter := retry.JitterFunc(func() float64 {
//		return rng.Float64()*0.2 - 0.1
//	})
type JitterFunc func() float64

// NextJitter implements JitterSource.
func (f JitterFunc) NextJitter() float64 { return f() }

// Compute performs one retransmission-timeout computation and returns the
// resulting Step. It is pure given a deterministic JitterSource.
//
// See SPEC.md §5 for the full algorithm description.
func Compute(params Params, prev State, jitter JitterSource) Step {
	base := selectBase(params, prev)

	j := jitter.NextJitter()
	rt := applyJitter(base, j)

	newState := State{
		Retries: prev.Retries + 1,
		LastRT:  rt,
		Elapsed: saturatingAdd(prev.Elapsed, rt),
	}

	// Termination is evaluated against newState, not prev, in this order.
	if params.maxRetriesBounded() && newState.Retries > params.MaxRetries {
		return Step{Done: true, Reason: MaxRetriesExceeded, State: newState}
	}
	if params.maxDurationBounded() && newState.Elapsed > params.MaxDuration {
		return Step{Done: true, Reason: MaxDurationExceeded, State: newState}
	}
	return Step{RT: rt, Done: false, State: newState}
}

// selectBase implements SPEC.md §5.1.
func selectBase(params Params, prev State) time.Duration {
	if prev.Retries == 0 {
		return params.InitialRT
	}
	candidate := saturatingDouble(prev.LastRT)
	if params.maxIntervalBounded() && candidate > params.MaxInterval {
		return params.MaxInterval
	}
	return candidate
}

// applyJitter implements SPEC.md §5.2. It computes base + base*j in
// float64 -- deliberately not base*(1+j); the two are not equivalent under
// floating-point arithmetic, and conformance vectors are sensitive to
// evaluation order -- then rounds to the nearest nanosecond (half away
// from zero, matching Rust's f64::round). Results below zero (which
// requires j < -1.0) saturate to zero.
func applyJitter(base time.Duration, j float64) time.Duration {
	baseF := float64(base)
	rtF := baseF + baseF*j
	if rtF < 0 {
		return 0
	}
	if rtF > float64(math.MaxInt64) {
		return math.MaxInt64
	}
	return time.Duration(math.Round(rtF))
}

// saturatingDouble doubles d, saturating at the maximum representable
// time.Duration instead of overflowing.
func saturatingDouble(d time.Duration) time.Duration {
	if d > math.MaxInt64/2 {
		return math.MaxInt64
	}
	return d * 2
}

// saturatingAdd adds a and b (both non-negative time.Durations by
// construction), saturating at the maximum representable time.Duration
// instead of overflowing.
func saturatingAdd(a, b time.Duration) time.Duration {
	if a > math.MaxInt64-b {
		return math.MaxInt64
	}
	return a + b
}
