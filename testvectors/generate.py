#!/usr/bin/env python3
# Copyright (c) 2026 Erik Kline
# SPDX-License-Identifier: MIT
"""Generates conformance test vectors for the `retry` library.

This script is the reference implementation of the algorithm in SPEC.md
section 5: it is written directly against that spec (integer-nanosecond
bookkeeping, `base + base * j` evaluation order, round-half-away-from-zero
to the nearest nanosecond, truncating nanoseconds->milliseconds for the
JSON `_ms` fields). The Go and Rust implementations are expected to
reproduce these "expected" sequences exactly.

Deliberately does NOT use Python's `random` module (a language stdlib
default); instead it uses a small hand-rolled PCG32 generator (see SPEC.md
section 7.2) for vectors that call for "random-looking" jitter.

Usage:
    python3 generate.py

Regenerates every *.json file in this directory. The script is checked in
alongside its output; conformance tests only ever read the generated JSON,
never invoke this script.
"""
from __future__ import annotations

import dataclasses
import json
import math
import pathlib
from typing import Any, Dict, List, Optional, Tuple

NS_PER_MS = 1_000_000

# The fixed per-retry growth factor mandated by RFC 9915 section 15
# (RT = 2*RTprev + jitter), currently 2 (doubling). Named generically
# rather than e.g. DOUBLING_FACTOR in case a future major version needs
# to generalize it, matching the Go and Rust implementations; it is
# intentionally not a Params field today: SPEC.md section 5.1 defines
# this algorithm as doubling, not as configurable exponential backoff
# with an arbitrary base.
SCALE_FACTOR = 2


# ---------------------------------------------------------------------------
# PCG32 (PCG-XSH-RR) -- a small, portable, non-stdlib PRNG.
# Reference: https://www.pcg-random.org/
# ---------------------------------------------------------------------------
class Pcg32:
    """Minimal PCG-XSH-RR 32-bit generator, seeded explicitly for
    reproducibility. Not related to any language's stdlib RNG."""

    _MULT = 6364136223846793005
    _MASK64 = (1 << 64) - 1

    def __init__(self, seed: int, seq: int):
        self._state = 0
        self._inc = ((seq << 1) | 1) & self._MASK64
        self._next_u32()
        self._state = (self._state + seed) & self._MASK64
        self._next_u32()

    def _next_u32(self) -> int:
        old = self._state
        self._state = (old * self._MULT + self._inc) & self._MASK64
        xorshifted = (((old >> 18) ^ old) >> 27) & 0xFFFFFFFF
        rot = (old >> 59) & 0xFFFFFFFF
        return ((xorshifted >> rot) | (xorshifted << ((-rot) & 31))) & 0xFFFFFFFF

    def next_float(self) -> float:
        """Uniform float in [0, 1)."""
        return self._next_u32() / 2**32

    def uniform(self, lo: float, hi: float) -> float:
        return lo + self.next_float() * (hi - lo)


def round_half_away_from_zero(x: float) -> int:
    """Matches Go's math.Round and Rust's f64::round (both round half away
    from zero), unlike Python's banker's-rounding built-in round()."""
    if x >= 0:
        return math.floor(x + 0.5)
    return math.ceil(x - 0.5)


# ---------------------------------------------------------------------------
# Reference algorithm (mirrors SPEC.md section 5 exactly).
# ---------------------------------------------------------------------------
@dataclasses.dataclass
class Params:
    initial_rt_ms: int
    max_interval_ms: Optional[int] = None
    max_retries: Optional[int] = None
    max_duration_ms: Optional[int] = None

    def to_json(self) -> Dict[str, Any]:
        return {
            "initial_rt_ms": self.initial_rt_ms,
            "max_interval_ms": self.max_interval_ms,
            "max_retries": self.max_retries,
            "max_duration_ms": self.max_duration_ms,
        }


@dataclasses.dataclass
class State:
    retries: int = 0
    last_rt_ns: int = 0
    elapsed_ns: int = 0


def compute(params: Params, prev: State, j: float) -> Tuple[str, Any, State]:
    """Returns (kind, payload, new_state), where kind is "wait" (payload =
    rt_ns) or "giveup" (payload = reason string)."""
    if prev.retries == 0:
        base_ns = params.initial_rt_ms * NS_PER_MS
    else:
        candidate_ns = prev.last_rt_ns * SCALE_FACTOR  # Python ints never overflow;
        # Go/Rust saturate at their own language's max representable
        # Duration, which these test vectors never approach.
        if (
            params.max_interval_ms is not None
            and candidate_ns > params.max_interval_ms * NS_PER_MS
        ):
            base_ns = params.max_interval_ms * NS_PER_MS
        else:
            base_ns = candidate_ns

    base_f = float(base_ns)
    rt_f = base_f + base_f * j
    rt_ns = 0 if rt_f < 0 else round_half_away_from_zero(rt_f)

    new_state = State(
        retries=prev.retries + 1,
        last_rt_ns=rt_ns,
        elapsed_ns=prev.elapsed_ns + rt_ns,
    )

    if params.max_retries is not None and new_state.retries > params.max_retries:
        return "giveup", "max_retries", new_state
    if (
        params.max_duration_ms is not None
        and new_state.elapsed_ns > params.max_duration_ms * NS_PER_MS
    ):
        return "giveup", "max_duration", new_state
    return "wait", rt_ns, new_state


def run_phase(
    params: Params, jitter: List[float], start_state: State, n_steps: int
) -> Tuple[List[Dict[str, Any]], State]:
    state = start_state
    expected: List[Dict[str, Any]] = []

    def next_jitter(i: int) -> float:
        return jitter[i] if i < len(jitter) else 0.0

    for i in range(n_steps):
        kind, payload, state = compute(params, state, next_jitter(i))
        if kind == "wait":
            expected.append(
                {
                    "kind": "wait",
                    "rt_ms": payload // NS_PER_MS,
                    "retries": state.retries,
                    "last_rt_ms": state.last_rt_ns // NS_PER_MS,
                    "elapsed_ms": state.elapsed_ns // NS_PER_MS,
                }
            )
        else:
            expected.append(
                {
                    "kind": "giveup",
                    "reason": payload,
                    "retries": state.retries,
                    "last_rt_ms": state.last_rt_ns // NS_PER_MS,
                    "elapsed_ms": state.elapsed_ns // NS_PER_MS,
                }
            )
            break
    return expected, state


def phase_json(
    params: Params, jitter: List[float], expected: List[Dict[str, Any]]
) -> Dict[str, Any]:
    return {"params": params.to_json(), "jitter": jitter, "expected": expected}


def write_vector(
    out_dir: pathlib.Path, name: str, description: str, phases: List[Dict[str, Any]]
) -> None:
    doc = {"name": name, "description": description, "phases": phases}
    path = out_dir / f"{name}.json"
    path.write_text(json.dumps(doc, indent=2) + "\n")
    print(f"wrote {path}")


def main() -> None:
    out_dir = pathlib.Path(__file__).parent

    # -- dhcpv6_solicit: IRT=1s, MRT=1h, unbounded MRC/MRD. 16 PCG32-derived
    # uniform samples in [-0.1, 0.1], matching RFC 8415's Solicit RAND. --
    rng = Pcg32(seed=915_1969, seq=1)
    solicit_jitter = [round(rng.uniform(-0.1, 0.1), 6) for _ in range(16)]
    params = Params(initial_rt_ms=1000, max_interval_ms=3_600_000)
    expected, _ = run_phase(params, solicit_jitter, State(), n_steps=16)
    write_vector(
        out_dir,
        "dhcpv6_solicit",
        "DHCPv6 Solicit-shaped schedule: IRT=1s, MRT=1h, unbounded MRC/MRD, "
        "16 PCG32-derived uniform jitter samples in [-0.1, 0.1]. All calls "
        "are expected to succeed (no termination).",
        [phase_json(params, solicit_jitter, expected)],
    )

    # -- dhcpv6_request: IRT=1s, MRT=30s, MRC=10 -> terminates with
    # max_retries on the 11th compute call. Zero jitter for a
    # hand-verifiable doubling/capping sequence. --
    params = Params(initial_rt_ms=1000, max_interval_ms=30_000, max_retries=10)
    jitter = [0.0] * 11
    expected, _ = run_phase(params, jitter, State(), n_steps=11)
    write_vector(
        out_dir,
        "dhcpv6_request",
        "DHCPv6 Request-shaped schedule: IRT=1s, MRT=30s, MRC=10, zero "
        "jitter. Doubles and caps at MRT, then gives up with max_retries "
        "on the 11th compute call.",
        [phase_json(params, jitter, expected)],
    )

    # -- mrc_zero: MaxRetries=0 -> the very first compute call gives up. --
    params = Params(initial_rt_ms=1000, max_retries=0)
    jitter = [0.0]
    expected, _ = run_phase(params, jitter, State(), n_steps=1)
    write_vector(
        out_dir,
        "mrc_zero",
        "MaxRetries=0: the configuration 'no retransmissions permitted'. "
        "The very first compute call must give up with max_retries.",
        [phase_json(params, jitter, expected)],
    )

    # -- mrd_exhaustion: MaxDuration cuts the sequence off mid-flight. --
    params = Params(initial_rt_ms=1000, max_interval_ms=10_000, max_duration_ms=5_000)
    jitter = [0.0, 0.0, 0.0]
    expected, _ = run_phase(params, jitter, State(), n_steps=3)
    write_vector(
        out_dir,
        "mrd_exhaustion",
        "IRT=1s, MRT=10s, MRD=5s, zero jitter. Doubling (1s, 2s, 4s) pushes "
        "cumulative elapsed past the 5s budget on the 3rd compute call, "
        "which gives up with max_duration.",
        [phase_json(params, jitter, expected)],
    )

    # -- rekey: two phases sharing a state shape. Phase 1 runs to a
    # checkpoint; phase 2 swaps in a lower MRT (as if a server advertised a
    # new hint) and continues from that checkpoint state. --
    phase1_params = Params(initial_rt_ms=1000, max_interval_ms=5_000)
    phase1_jitter = [0.0, 0.0, 0.0]
    phase1_expected, phase1_end_state = run_phase(
        phase1_params, phase1_jitter, State(), n_steps=3
    )
    phase2_params = Params(initial_rt_ms=1000, max_interval_ms=3_000)
    phase2_jitter = [0.0, 0.0]
    phase2_expected, _ = run_phase(
        phase2_params, phase2_jitter, phase1_end_state, n_steps=2
    )
    write_vector(
        out_dir,
        "rekey",
        "Two phases modeling a mid-sequence re-key. Phase 1 (MRT=5s) runs "
        "3 steps to a checkpoint state. Phase 2 swaps in MRT=3s (e.g. a "
        "server-advertised hint) and continues from that state for 2 more "
        "steps. State.Retries/LastRT/Elapsed carry forward unchanged across "
        "the params swap; only the new MRT affects subsequent base "
        "selection.",
        [
            phase_json(phase1_params, phase1_jitter, phase1_expected),
            phase_json(phase2_params, phase2_jitter, phase2_expected),
        ],
    )

    # -- zero_jitter: deterministic doubling with no jitter at all. --
    params = Params(initial_rt_ms=500)
    jitter = [0.0] * 6
    expected, _ = run_phase(params, jitter, State(), n_steps=6)
    write_vector(
        out_dir,
        "zero_jitter",
        "IRT=500ms, unbounded MRT/MRC/MRD, all jitter 0.0: pure doubling "
        "(500, 1000, 2000, 4000, 8000, 16000ms) with no randomness and no "
        "termination.",
        [phase_json(params, jitter, expected)],
    )

    # -- negative_saturation: a jitter value below -1.0 drives RT to zero
    # by saturation, not a negative Duration. --
    params = Params(initial_rt_ms=1000)
    jitter = [-1.5]
    expected, _ = run_phase(params, jitter, State(), n_steps=1)
    write_vector(
        out_dir,
        "negative_saturation",
        "IRT=1s, a single jitter value of -1.5 (base + base*-1.5 = "
        "-500ms), which must saturate to rt=0 rather than go negative.",
        [phase_json(params, jitter, expected)],
    )


if __name__ == "__main__":
    main()
