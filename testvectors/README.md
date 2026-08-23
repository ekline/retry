# Conformance test vectors

Shared JSON test vectors consumed by both the Go and Rust implementations.
See `SPEC.md` §7 for the conceptual schema and the list of required
vectors. `generate.py` is the reference implementation of the algorithm in
`SPEC.md` §5 (integer-nanosecond bookkeeping, exact `base + base * j`
evaluation order, round-half-away-from-zero) and is the source of truth for
every `expected` sequence below; it is checked in alongside its output, but
conformance tests only ever read the generated `*.json` files.

## Schema

Each file is a JSON document:

```json
{
  "name": "string",
  "description": "string",
  "phases": [
    {
      "params": {
        "initial_rt_ms":   <int>,
        "max_interval_ms": <int | null>,
        "max_retries":     <int | null>,
        "max_duration_ms": <int | null>
      },
      "jitter": [<float>, ...],
      "expected": [
        {
          "kind":        "wait" | "giveup",
          "rt_ms":       <int>,                              // present when kind == "wait"
          "reason":      "max_retries" | "max_duration",     // present when kind == "giveup"
          "retries":     <int>,
          "last_rt_ms":  <int>,
          "elapsed_ms":  <int>
        }
      ]
    }
  ]
}
```

This wraps SPEC.md §7's illustrative single-sequence schema in a `phases`
array so that one file format covers every vector, including `rekey.json`
(SPEC.md §7.1), which needs two sequences that share carried-forward state
across a params swap. Every vector has at least one phase; only `rekey.json`
has more than one.

A conformant implementation replays each phase in order against a single
`FixedJitter`, carrying `State` forward both within a phase (call to call)
and across phases within the same file (only resetting state to its zero
value between different files), and asserts each `expected` entry against
the corresponding `compute` call's result. Comparison is exact on the
integer millisecond fields.

## Required vectors

- `dhcpv6_solicit.json` — IRT=1s, MRT=1h, unbounded MRC/MRD, 16 PCG32-derived
  uniform jitter samples in `[-0.1, 0.1]`.
- `dhcpv6_request.json` — IRT=1s, MRT=30s, MRC=10; terminates with
  `max_retries` on the 11th compute call.
- `mrc_zero.json` — MaxRetries=0; the first compute call gives up with
  `max_retries`.
- `mrd_exhaustion.json` — MaxDuration set so the sequence is cut off
  mid-flight with `max_duration`.
- `rekey.json` — two phases: run to a checkpoint, swap params (lower MRT),
  continue from that state.
- `zero_jitter.json` — all jitter 0.0; deterministic doubling only.
- `negative_saturation.json` — a jitter value of `-1.5` driving `rt` to
  zero by saturation.
- `jitter_exhaustion_repeats_last.json` — fewer jitter values than
  steps; the last value must repeat forever once exhausted, not fall
  back to `0.0`.

## Regenerating

```sh
python3 generate.py
```

This overwrites every `*.json` file in this directory. Only `generate.py`
uses Python; nothing in `go/` or `rust/` depends on Python or on this
script running.
