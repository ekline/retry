# retry: Specification

A pure, sans-I/O retransmission backoff calculator with parallel implementations in Go and Rust, sharing a single set of conformance test vectors.

## 1. Overview

This library computes how long a caller should wait before each retransmission of a previously-sent message, given immutable parameters and a previous state. The shape of the parameters and the doubling-with-jitter algorithm are drawn from RFC 9915 §15, but the library is **generic over protocol**: it has no DHCPv6, CoAP, TLS, or other protocol-specific behavior. It is the arithmetic core that any retransmitting protocol can wrap.

### What the library does

- Computes one retransmission timeout per call.
- Tracks the scheduled (not wall-clock) elapsed time across retransmissions, for MRD enforcement.
- Reports termination (max retries exhausted, max duration exhausted) as part of the return value.
- Accepts a caller-supplied jitter source, allowing both production randomness and deterministic test replay.

### What the library does not do

- Send, schedule, sleep, or perform any I/O.
- Generate randomness or read the clock internally.
- Handle the initial transmission of a message — that is the caller's first action, before any compute call.
- Implement protocol-specific behavior (DHCPv6 Solicit's first-message delay, RC=0 special case, Elapsed Time option encoding, message validation, etc.).
- Enforce rate limiting (RFC 9915 §14.1 is a separate concern, out of scope here).
- Persist state across process restarts (callers serialize `State` themselves if needed).
- Manage multi-interface state.

## 2. Repository

Single repository, two language implementations, shared test vectors.

```
/
├── README.md
├── SPEC.md                # this document
├── LICENSE                # MIT
├── Makefile                # shared CI/local commands; see §9
├── testvectors/
│   ├── README.md
│   └── *.json
├── go/
│   ├── go.mod
│   ├── retry.go
│   ├── sequence.go
│   ├── retry_test.go
│   ├── fuzz_test.go
│   └── conformance_test.go
└── rust/
    ├── Cargo.toml
    ├── src/
    │   ├── lib.rs
    │   ├── sequence.rs
    │   └── main.rs         # `retry` CLI; only built with --features cli
    └── tests/
        ├── basics.rs
        ├── sequence.rs
        ├── properties.rs
        ├── cli.rs
        └── conformance.rs
```

- Go module path: `github.com/ekline/retry/go` (resolved).
- Rust crate name: `retry` (resolved).
- GitHub organization/location: `github.com/ekline/retry` (resolved).
- Release tags: `go/vX.Y.Z` and `rust/vX.Y.Z` independently; both versions track the same SPEC version.

## 3. License

MIT, with the standard header included in each source file per language convention.

## 4. Types

The library exposes three plain-data types and one trait/interface. All types are immutable and copyable.

### 4.1 Params

Describes the retransmission schedule. Immutable once constructed; pass a new `Params` to `compute` to re-key (e.g., when a server advertises a new MRT).

| Field         | RFC name | Semantics                                                                                  |
|---------------|----------|--------------------------------------------------------------------------------------------|
| `InitialRT`   | IRT      | Nominal first retransmission timeout. Used as the base in the first `compute` call.        |
| `MaxInterval` | MRT      | Upper bound on the pre-jitter base interval. Optional / unbounded.                         |
| `MaxRetries`  | MRC      | Give up after this many retransmissions have been scheduled. Optional / unbounded.         |
| `MaxDuration` | MRD      | Give up once cumulative scheduled elapsed time would exceed this. Optional / unbounded.    |

Optionality:
- **Rust**: optional fields are `Option<Duration>` / `Option<u64>`. `None` = unbounded. `State.retries` and `Params.max_retries` are `u64` (not `u32`) to match Go's `int`, which is effectively unbounded for this purpose on any real platform; both saturate rather than overflow (see §5.3).
- **Go**: optional fields use zero-value sentinels. A zero `time.Duration` means unbounded for `MaxInterval` and `MaxDuration`. A negative `int` means unbounded for `MaxRetries` (`0` means "no retransmissions permitted").

### 4.2 State

History carried between `compute` calls. The zero value is the initial state; the caller passes it for the first call.

| Field     | Semantics                                                                  |
|-----------|----------------------------------------------------------------------------|
| `Retries` | Number of retransmissions scheduled so far. `0` before the first call.     |
| `LastRT`  | Most recently computed RT. `0` before the first call.                      |
| `Elapsed` | Sum of all RTs scheduled so far. `0` before the first call.                |

Callers may construct and serialize `State` freely; it has no hidden invariants.

### 4.3 Step

The return value of `compute`. Either a wait instruction or a terminal give-up signal; carries the updated state in both cases.

- **Rust**: an enum with `Wait { rt: Duration, state: State }` and `GiveUp { reason: Termination, state: State }` variants.
- **Go**: a struct with `RT time.Duration`, `Done bool`, `Reason Termination`, and `State State`. `RT` is meaningful only when `Done == false`. `Reason` is meaningful only when `Done == true`.

### 4.4 Termination

Enum of give-up reasons:

- `MaxRetries`
- `MaxDuration`

Future variants may be added in a minor release.

### 4.5 JitterSource

A trait/interface with a single method:

```
NextJitter() -> f64
```

Returns the multiplier applied to the candidate base RT to produce the actual RT. For example, for DHCPv6 the source would yield uniform values in `[-0.1, +0.1]`. The library is agnostic to the distribution — bounds, shape, and bias are entirely the source's concern.

Required built-in: **`FixedJitter`**, which replays a slice of values. Once exhausted, all subsequent calls repeat the last value forever (or `0.0`, if constructed with no values at all) rather than silently switching to unjittered behavior. Used for deterministic testing and conformance vector replay.

Optional built-in: **`UniformJitter`**, which wraps a PRNG and returns uniform values in `[-factor, +factor]`.
- **Rust**: behind the `rand` feature flag; depends on `rand_core::RngCore`.
- **Go**: a separate type wrapping `*math/rand/v2.Rand`. Not behind a build tag — Go's stdlib makes the dependency free.

## 5. Algorithm

A single public function:

```
compute(params, prev, jitter) -> Step
```

The function is pure given a deterministic `JitterSource`. It performs the following steps in order.

### 5.1 Base interval selection

```
if prev.Retries == 0:
    base = params.InitialRT
else:
    candidate = prev.LastRT * 2     # saturating
    if params.MaxInterval is bounded and candidate > params.MaxInterval:
        base = params.MaxInterval
    else:
        base = candidate
```

The detection of "first call" uses `prev.Retries == 0` rather than `prev.LastRT == 0`, so that a caller resuming from a checkpointed state behaves correctly.

The `2` is a fixed constant of this algorithm, not a tunable parameter: it comes directly from RFC 9915's `RT = 2*RTprev + jitter`, the same way DHCPv6's retransmission timing has always doubled (RFC 3315, RFC 8415 §15). Both implementations name it `scaleFactor` / `SCALE_FACTOR` -- deliberately generic names, not `doublingFactor`, in case a future major version needs to generalize it -- but deliberately do not expose it on `Params` today: doing so would turn this library from an implementation of one specific RFC 9915-shaped algorithm into a generic configurable-backoff library, which is out of scope -- the same reasoning §11.1 applies to protocol-specific `Params` presets.

### 5.2 Jitter application

```
j  = jitter.NextJitter()
rt = base + base * j
```

**Implementation note (binding):** the expression must be written exactly as `base + base * j`, not as `base * (1 + j)`. Floating-point addition and multiplication are not associative; conformance vectors are sensitive to evaluation order, and the two languages must produce bit-identical results when given the same inputs.

If `rt < 0` (which requires `j < -1.0`), saturate to zero.

If `rt` is NaN (only reachable via a non-finite `JitterSource`: a NaN jitter value directly, or a `0 * Inf` produced by an infinite jitter value when `base` is exactly zero), treat it as if jitter had been `0.0`, i.e. fall back to the unjittered `base`. A `JitterSource` is caller-supplied and this library cannot make it well-behaved, so `compute` must still return a well-formed `Step` rather than propagate NaN into `State`.

### 5.3 State update

```
new_state.Retries = prev.Retries + 1    # saturating
new_state.LastRT  = rt
new_state.Elapsed = prev.Elapsed + rt   # saturating add
```

All Duration arithmetic is saturating. Overflow yields the maximum representable Duration; this is documented behavior and not an error. The `Retries` counter saturates the same way, at the maximum representable value for its type (Go's platform-word-sized `int`; Rust's `u64`), rather than wrapping or panicking -- reachable only after billions of calls per second for centuries, but a checkpointed `State` can be constructed with any value, so both implementations guard against it explicitly rather than relying on it being unreachable in practice.

### 5.4 Termination check

Evaluated against `new_state`, in order:

1. If `params.MaxRetries` is bounded and `new_state.Retries > params.MaxRetries`: return `GiveUp { MaxRetries, new_state }`.
2. Else if `params.MaxDuration` is bounded and `new_state.Elapsed > params.MaxDuration`: return `GiveUp { MaxDuration, new_state }`.
3. Else: return `Wait { rt, new_state }`.

Termination is evaluated against the new state, not the previous one. Consequences worth noting:

- If `params.MaxRetries == 0`, the very first compute call returns `GiveUp { MaxRetries }`. The configuration "no retransmissions permitted" is expressible.
- If a single first RT exceeds `params.MaxDuration`, the very first compute call returns `GiveUp { MaxDuration }`. This is a degenerate but well-defined case.

### 5.5 Re-keying

To change any parameter mid-sequence — most commonly MRT in response to a server hint — the caller passes a new `Params` value to the next `compute` call. The library has no mutation API; new parameters take effect immediately on the next call. The `State` carries forward unchanged.

## 6. Caller pattern

The reference caller pattern is a do-while-style loop:

```
transmit(message)
state = State{}             // zero value
loop:
    step = compute(params, state, jitter)
    state = step.State
    if step.Done:
        break               // exhausted retries or budget
    
    schedule retransmit in step.RT
    wait for response or timer
    if response received:
        break               // success
    
    retransmit(message)
```

Notes:

- The compute call comes first in each loop iteration so that termination is detected before any retransmit is sent.
- A response received after the timer expires (but while the next retransmit is in flight) is the caller's concern, not the calculator's.
- If the caller learns a new MRT from a server response, it replaces `params` before the next iteration and continues.

## 7. Conformance test vectors

A file under `testvectors/` is a JSON document with this schema:

```json
{
  "name": "string",
  "description": "string",
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
      "rt_ms":       <int>,           // present when kind == "wait"
      "reason":      "max_retries" | "max_duration",   // present when kind == "giveup"
      "retries":     <int>,
      "last_rt_ms":  <int>,
      "elapsed_ms":  <int>
    }
  ]
}
```

Both implementations must produce the `expected` sequence exactly when fed `params` and a `FixedJitter` initialized with `jitter`. Comparison is exact (integer milliseconds). The conformance test in each language iterates all `*.json` files under `testvectors/` and runs each.

### 7.1 Required vectors

At minimum, the following must be present:

- **`dhcpv6_solicit.json`** — IRT=1000ms, MRT=3600000ms, MaxRetries=null, MaxDuration=null. Jitter from a uniform distribution in `[-0.1, +0.1]`, expressed as 16 fixed samples.
- **`dhcpv6_request.json`** — IRT=1000ms, MRT=30000ms, MaxRetries=10, MaxDuration=null. Should terminate with `max_retries` on the 11th compute call.
- **`mrc_zero.json`** — MaxRetries=0. First compute call returns `giveup` with reason `max_retries`.
- **`mrd_exhaustion.json`** — MaxDuration set such that the sequence is cut off mid-flight by `max_duration`.
- **`rekey.json`** — Two test vectors sharing a state shape, modeling a re-key. The implementation runs the first to a checkpoint state, swaps params, then runs the second from that state.
- **`zero_jitter.json`** — All jitter values 0.0; deterministic doubling without randomness.
- **`negative_saturation.json`** — A jitter value of `-1.5` driving RT to zero by saturation.
- **`jitter_exhaustion_repeats_last.json`** — Fewer jitter values than steps; the last value must repeat forever once exhausted (§4.5), not fall back to `0.0`.

### 7.2 Vector generation

A small script under `testvectors/generate.py` (or equivalent) seeds a portable PRNG (PCG or ChaCha8, **not** language stdlib defaults) and writes the JSON files. The script is checked in; the generated files are also checked in. Conformance does not depend on running the script — it depends only on consuming the JSON.

## 8. Per-language requirements

### 8.1 Rust

- **Crate name**: `retry` (resolved).
- **MSRV**: Rust 1.70 or later.
- **`no_std` + `alloc`** by default. `std` not required.
- **Features**:
  - default: none
  - `rand`: enables `UniformJitter`; pulls in `rand_core`.
  - `cli`: builds the `retry` binary (§13); pulls in `clap`. Affects only the binary target, not the library: a downstream `retry = "..."` library-only consumer never needs `clap`, since Cargo only resolves a dependency's binary targets (and their dependencies) when something actually asks to build them.
- **Dependencies**: zero in default build. `rand_core` only behind `rand`; `clap` only behind `cli`, and only for the binary.
- **Public API**: `Params`, `State`, `Step`, `Termination`, `JitterSource`, `FixedJitter`, `compute`. `UniformJitter` behind the `rand` feature. Additive convenience layer (§11): `Params::new`/`with_*`, `Sequence`. The `retry` CLI (§13) is not part of this API and is not covered by SemVer (§10).
- **Lints**: `#![deny(missing_docs)]`, `#![forbid(unsafe_code)]`.

### 8.2 Go

- **Module path**: `github.com/ekline/retry/go` (resolved).
- **Go version**: 1.22 or later.
- **Dependencies**: none (`math/rand/v2` is stdlib).
- **Package name**: `retry`.
- **Public API**: `Params`, `State`, `Step`, `Termination`, `JitterSource`, `FixedJitter`, `UniformJitter`, `Compute`. Additive convenience layer (§11): `NewParams`, `Option`, `With*`, `Sequence`, `JitterFunc`.
- **Lints**: `go vet`, `gofmt`, `staticcheck` (clean).

## 9. CI

GitHub Actions, matrix build. Every step shells out to a target in the root `Makefile` rather than inlining commands in the workflow YAML, so local development (`make check`) and CI run the identical commands -- one copy of each, not two kept in sync by hand.

- **Go job** (`make go-check`): `go vet ./...`, `gofmt -l .` (must produce no output), `staticcheck ./...`, `go test ./... -timeout 60s`.
- **Rust job** (`make rust-check`): `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings` (default and `rand` features), `timeout 60 cargo test` (default and `rand` features).

Both test steps are time-bounded (60s) so an accidental infinite loop in test code fails the job quickly instead of running until the platform's own job timeout. Go's `-timeout` flag is built into `go test`; Rust's stock test harness has no equivalent, so the Rust job wraps it with the standard `timeout` command instead of adding a test-runner dependency.

Path filters (see §2's tree for what lives where):

- Changes under `testvectors/` trigger both language jobs.
- Changes under `go/` trigger only the Go job.
- Changes under `rust/` trigger only the Rust job.
- Changes to `SPEC.md`, `README.md`, `Makefile`, or workflow files trigger both jobs.

## 10. SemVer

Both crates/modules follow SemVer independently. The public API surface in §8 is the SemVer-protected contract. Behavioral guarantees in §5 (algorithm) are equally protected: any change that would alter conformance vector outputs is a major version change.

- Adding new `Termination` variants: minor.
- Adding new built-in jitter sources: minor.
- Renaming `Params`, `State`, or `Step` fields: major.
- Changing the `compute` signature: major.
- Changing floating-point evaluation order in §5.2: major.

## 11. Convenience API (additive)

Sections 4-5 define the required core: pure types plus a single `compute` function. Both implementations also ship a thin, additive ergonomic layer on top of that core. This layer is optional sugar -- everything it does, callers could do themselves with `Params`, `State`, and `compute` directly -- and is SemVer-minor, not part of the core algorithmic contract in §5 or §10's major-version triggers.

### 11.1 `Params` construction

Direct struct literals remain valid in both languages, but the zero-value `Params{}` in Go is a footgun: its zero `MaxRetries` means "no retransmissions permitted," not "unbounded." Both languages provide a constructor that defaults every optional field to unbounded instead:

- **Rust**: `Params::new(initial_rt)` returns `Params` with `max_interval`/`max_retries`/`max_duration` all `None`. Chainable `with_max_interval`, `with_max_retries`, and `with_max_duration` methods consume and return `Self`:

  ```
  Params::new(Duration::from_secs(1))
      .with_max_interval(Duration::from_secs(30))
      .with_max_retries(10)
  ```

- **Go**: `NewParams(initialRT, opts ...Option) Params` defaults `MaxRetries` to unbounded (`-1`) and applies functional options:

  ```
  retry.NewParams(time.Second,
      retry.WithMaxInterval(30*time.Second),
      retry.WithMaxRetries(10),
  )
  ```

  `Option`, `WithMaxInterval`, `WithMaxRetries`, and `WithMaxDuration` are the exported pieces.

This constructor deliberately stays generic rather than growing protocol-specific presets (e.g. a hypothetical `Params::dhcpv6_solicit()`): that's a different crate's job to layer on top, not this one's.

### 11.2 `Sequence`: a stateful driver

`Sequence` threads `State` between `compute` calls so callers don't have to manage it by hand. It performs no I/O, scheduling, or sleeping -- it only replaces the manual state-threading in §6's caller pattern, which can be rewritten as:

- **Rust**: `Sequence<J>` implements `Iterator<Item = Duration>`. `Sequence::new(params, jitter)` constructs one; `.reason() -> Option<Termination>` reports why iteration stopped; `.state() -> State` and `.set_params(params)` (re-keying, §5.5) round out the type.

  ```
  let mut seq = Sequence::new(params, jitter);
  transmit(&msg);
  for rt in &mut seq {
      sleep(rt);
      if got_response() { break; }
      retransmit(&msg);
  }
  if let Some(reason) = seq.reason() { /* ... */ }
  ```

- **Go**: `*Sequence` follows the `bufio.Scanner` shape. `NewSequence(params, jitter) *Sequence` constructs one; `Next() bool` advances it; `RT() time.Duration` and `Reason() (Termination, bool)` are the accessors; `State() State` and `SetParams(params)` round it out.

  ```
  seq := retry.NewSequence(params, jitter)
  transmit(msg)
  for seq.Next() {
      time.Sleep(seq.RT())
      if gotResponse() { break }
      retransmit(msg)
  }
  if reason, gaveUp := seq.Reason(); gaveUp { /* ... */ }
  ```

This is the only concession to mutation-style convenience: `compute` itself never gains a `&mut State`-updating overload. The immutable return-the-new-state pattern remains the sole low-level API; `Sequence` is where the ergonomic state-threading lives, layered on top of `compute` rather than baked into it.

### 11.3 Closures as `JitterSource`

Both languages let a plain function act as a `JitterSource` without a named wrapper type:

- **Rust**: a blanket `impl<F: FnMut() -> f64> JitterSource for F` means any `FnMut() -> f64` closure (or function item) can be passed anywhere a `JitterSource` is expected, e.g. `compute(&params, state, &mut || rng.gen_range(-0.1..0.1))`.
- **Go**: `JitterFunc func() float64` implements `JitterSource` via a method on the function type, mirroring `http.HandlerFunc`: `retry.JitterFunc(func() float64 { return rng.Float64()*0.2 - 0.1 })`.

## 12. Property-based and fuzz testing

The shared conformance vectors (§7) check specific, hand-picked sequences. They do not explore the input space broadly, and bounds-checking review found real bugs (integer overflow, NaN propagation, a float-rounding hole at the edge of `int64`) that only showed up once specific extreme values were tried. Both implementations therefore also test properties that must hold for *any* input, not just the chosen vectors:

- `compute` never panics, for any `Params` / `State` / jitter value combination.
- `State.elapsed` never decreases from one call to the next.
- `State.retries` increases by exactly 1 per call, or stays saturated at the maximum representable value for its type once reached (never wraps, never panics) -- see §5.3.
- The termination decision (`Wait` / `GiveUp(MaxRetries)` / `GiveUp(MaxDuration)`) always matches §5.4's order exactly, for the resulting state.
- A `NaN` jitter value always produces the same result as a `0.0` jitter value (§5.2).

### 12.1 Go

`go/fuzz_test.go` uses Go's built-in fuzzing (`testing.F`, stdlib since Go 1.18; no new dependency). `go test ./...` (including in CI) runs only the seed corpus -- the hand-found edge cases plus a routine case -- as ordinary test cases. Extended, mutation-guided fuzzing is a local, opt-in activity:

```sh
go test -fuzz=FuzzCompute -fuzztime=30s ./go
```

Any input the fuzzer finds that fails an invariant is saved under `go/testdata/fuzz/FuzzCompute/`; such files are checked in and thereafter run automatically as regression cases by plain `go test`.

### 12.2 Rust

`rust/tests/properties.rs` uses `proptest` (dev-dependency only; does not affect the published crate's dependency graph). `cargo test` (including in CI) runs each property against its default number of random cases (256) every time, with a fresh seed -- this is ordinary property-based testing, not coverage-guided fuzzing, so no separate "fuzzing mode" is needed. For a deeper local search:

```sh
PROPTEST_CASES=100000 cargo test --test properties
```

Any failure is shrunk to a minimal reproducing case and saved under `rust/tests/regressions/`, replayed automatically on subsequent runs.

## 13. Command-line utility (`retry`)

A small, Rust-only `retry` binary (`rust/src/main.rs`), in the spirit of `seq`: given a `Params` configuration on the command line, it prints the sequence of computed retransmission timeouts, one per line, for manual inspection or piping into other tools.

- **Not part of the library's public API and not covered by SemVer (§10).** It exists for manual inspection and documentation, the same purpose originally sketched (as `cmd/retry-trace` or `examples/trace.rs`) before this spec's open design questions were all settled.
- **Go-less by design.** Only Rust ships this; there is no `go/cmd/retry-trace` equivalent. Nothing about the CLI is algorithm-specific enough to need cross-language parity or conformance-vector coverage -- it is a thin, disposable presentation layer over `Sequence`.
- **Built only with `--features cli`** (pulls in `clap`), so the default build and any library-only consumer stay dependency-free (§8.1).
- **Interface**: `retry <INITIAL_RT_MS> [OPTIONS]`, with `--max-interval-ms`, `--max-retries`, `--max-duration-ms` mapping directly to `Params`, `--jitter F,F,...` supplying a `FixedJitter` (repeating the last value forever once exhausted, per §4.5), `-n/--count` capping output for unbounded schedules (default 20), and `-H/--human` switching from plain-milliseconds to `Duration`'s human-readable `Debug` format. Run `retry --help` for the full, authoritative list -- this SPEC does not duplicate it field-by-field.
- **Output discipline, matching `seq`**: stdout carries only the computed values, one per line (milliseconds by default), so the output is pipeable; a give-up reason, if the schedule ends that way, is a single line on stderr, not stdout. A downstream reader closing the pipe early (e.g. `retry 1000 | head -3`) exits cleanly rather than reporting a broken-pipe error.
- **Tests**: `rust/tests/cli.rs` (gated by `#[cfg(feature = "cli")]`, mirroring how `tests/uniform_jitter.rs` gates on `rand`) runs the built binary via `std::process::Command` and checks its output.
