// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

package retry

import "time"

// Sequence drives a retransmission schedule, threading State between
// Compute calls so the caller doesn't have to. It performs no I/O,
// scheduling, or sleeping itself -- it only replaces the manual
// State-threading in the caller pattern described in SPEC.md §6:
//
//	seq := retry.NewSequence(params, jitter)
//	transmit(message)
//	for seq.Next() {
//		time.Sleep(seq.RT())
//		if responseReceived() {
//			break
//		}
//		retransmit(message)
//	}
//	if reason, gaveUp := seq.Reason(); gaveUp {
//		log.Printf("gave up: %v", reason)
//	}
//
// A Sequence is not safe for concurrent use.
type Sequence struct {
	params Params
	state  State
	jitter JitterSource
	rt     time.Duration
	done   bool
	reason Termination
}

// NewSequence returns a Sequence starting from the zero State, ready for
// its first Next call.
func NewSequence(params Params, jitter JitterSource) *Sequence {
	return &Sequence{params: params, jitter: jitter}
}

// Next advances the schedule by one Compute call. It reports true if the
// caller should wait RT and then retransmit, or false if the schedule has
// given up (see Reason) -- including on every call after the first false.
func (s *Sequence) Next() bool {
	if s.done {
		return false
	}
	step := Compute(s.params, s.state, s.jitter)
	s.state = step.State
	if step.Done {
		s.reason = step.Reason
		s.done = true
		return false
	}
	s.rt = step.RT
	return true
}

// RT returns the retransmission timeout computed by the most recent Next
// call. Meaningful only immediately after Next has returned true.
func (s *Sequence) RT() time.Duration { return s.rt }

// Reason reports why the schedule gave up, and whether it has. It returns
// (0, false) until Next has returned false.
func (s *Sequence) Reason() (Termination, bool) { return s.reason, s.done }

// State returns the current State, reflecting every Next call so far.
func (s *Sequence) State() State { return s.state }

// SetParams re-keys the schedule for subsequent Next calls (SPEC.md §5.5),
// most commonly in response to a server-advertised MRT. Accumulated State
// carries forward unchanged, and a Sequence that has already given up
// stays given up.
func (s *Sequence) SetParams(params Params) { s.params = params }
