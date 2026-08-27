// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

package libretry

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// vectorFile mirrors the schema documented in testvectors/README.md.
type vectorFile struct {
	Name        string        `json:"name"`
	Description string        `json:"description"`
	Phases      []vectorPhase `json:"phases"`
}

type vectorPhase struct {
	Params   vectorParams   `json:"params"`
	Jitter   []float64      `json:"jitter"`
	Expected []vectorExpect `json:"expected"`
}

type vectorParams struct {
	InitialRTMs   int64  `json:"initial_rt_ms"`
	MaxIntervalMs *int64 `json:"max_interval_ms"`
	MaxRetries    *int64 `json:"max_retries"`
	MaxDurationMs *int64 `json:"max_duration_ms"`
}

type vectorExpect struct {
	Kind      string  `json:"kind"`
	RTMs      *int64  `json:"rt_ms"`
	Reason    *string `json:"reason"`
	Retries   int64   `json:"retries"`
	LastRTMs  int64   `json:"last_rt_ms"`
	ElapsedMs int64   `json:"elapsed_ms"`
}

func (p vectorParams) toParams() Params {
	params := Params{
		InitialRT:  time.Duration(p.InitialRTMs) * time.Millisecond,
		MaxRetries: -1,
	}
	if p.MaxIntervalMs != nil {
		params.MaxInterval = time.Duration(*p.MaxIntervalMs) * time.Millisecond
	}
	if p.MaxRetries != nil {
		params.MaxRetries = int(*p.MaxRetries)
	}
	if p.MaxDurationMs != nil {
		params.MaxDuration = time.Duration(*p.MaxDurationMs) * time.Millisecond
	}
	return params
}

func TestConformance(t *testing.T) {
	paths, err := filepath.Glob(filepath.Join("..", "testvectors", "*.json"))
	if err != nil {
		t.Fatalf("glob testvectors: %v", err)
	}
	if len(paths) == 0 {
		t.Fatal("no test vectors found under ../testvectors -- did generate.py run?")
	}

	for _, path := range paths {
		t.Run(filepath.Base(path), func(t *testing.T) {
			raw, err := os.ReadFile(path)
			if err != nil {
				t.Fatalf("read %s: %v", path, err)
			}
			var vf vectorFile
			if err := json.Unmarshal(raw, &vf); err != nil {
				t.Fatalf("unmarshal %s: %v", path, err)
			}

			state := State{}
			for pi, phase := range vf.Phases {
				params := phase.Params.toParams()
				jitter := NewFixedJitter(phase.Jitter)

				for ei, want := range phase.Expected {
					step := Compute(params, state, jitter)
					state = step.State

					switch want.Kind {
					case "wait":
						if step.Done {
							t.Fatalf("phase %d entry %d: got Done=true (reason %v), want Wait", pi, ei, step.Reason)
						}
						if want.RTMs == nil {
							t.Fatalf("phase %d entry %d: vector missing rt_ms for kind=wait", pi, ei)
						}
						if got := step.RT.Milliseconds(); got != *want.RTMs {
							t.Errorf("phase %d entry %d: RT = %dms, want %dms", pi, ei, got, *want.RTMs)
						}
					case "giveup":
						if !step.Done {
							t.Fatalf("phase %d entry %d: got Done=false, want GiveUp", pi, ei)
						}
						if want.Reason == nil {
							t.Fatalf("phase %d entry %d: vector missing reason for kind=giveup", pi, ei)
						}
						if got := step.Reason.String(); got != *want.Reason {
							t.Errorf("phase %d entry %d: Reason = %q, want %q", pi, ei, got, *want.Reason)
						}
					default:
						t.Fatalf("phase %d entry %d: unknown kind %q", pi, ei, want.Kind)
					}

					if got := int64(step.State.Retries); got != want.Retries {
						t.Errorf("phase %d entry %d: Retries = %d, want %d", pi, ei, got, want.Retries)
					}
					if got := step.State.LastRT.Milliseconds(); got != want.LastRTMs {
						t.Errorf("phase %d entry %d: LastRT = %dms, want %dms", pi, ei, got, want.LastRTMs)
					}
					if got := step.State.Elapsed.Milliseconds(); got != want.ElapsedMs {
						t.Errorf("phase %d entry %d: Elapsed = %dms, want %dms", pi, ei, got, want.ElapsedMs)
					}
				}
			}
		})
	}
}
