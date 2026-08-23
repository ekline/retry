# retry

A pure, sans-I/O retransmission backoff calculator, with parallel Go and Rust
implementations sharing a single set of conformance test vectors.

The algorithm's shape (doubling-with-jitter, MRC/MRD bookkeeping) is drawn
from [RFC 9915](https://www.rfc-editor.org/rfc/rfc9915) §15, but the library
itself is protocol-agnostic: no DHCPv6, CoAP, TLS, or other protocol-specific
behavior lives here. It's the arithmetic core that any retransmitting
protocol can wrap.

See [`SPEC.md`](./SPEC.md) for the full design specification.

## Quick start

The core API is three plain types (`Params`, `State`, `Step`) plus one pure
function (`compute`) -- see `SPEC.md` §4-6. Both languages also offer a
small additive convenience layer (`SPEC.md` §12) that removes the most
common boilerplate: constructing `Params` and threading `State` between
calls by hand.

**Rust:**

```rust
use retry::{FixedJitter, Params, Sequence};

let params = Params::new(Duration::from_secs(1))
    .with_max_interval(Duration::from_secs(30))
    .with_max_retries(10);
let mut seq = Sequence::new(params, FixedJitter::new(vec![0.0; 11]));

transmit(&msg);
for rt in &mut seq {
    sleep(rt);
    if got_response() {
        break;
    }
    retransmit(&msg);
}
if let Some(reason) = seq.reason() {
    eprintln!("gave up: {reason:?}");
}
```

**Go:**

```go
params := retry.NewParams(time.Second,
    retry.WithMaxInterval(30*time.Second),
    retry.WithMaxRetries(10),
)
seq := retry.NewSequence(params, retry.NewFixedJitter(make([]float64, 11)))

transmit(msg)
for seq.Next() {
    time.Sleep(seq.RT())
    if gotResponse() {
        break
    }
    retransmit(msg)
}
if reason, gaveUp := seq.Reason(); gaveUp {
    log.Printf("gave up: %v", reason)
}
```

Neither `Sequence` nor `NewSequence` does any I/O, scheduling, or sleeping
themselves -- they only remove the manual `State`-threading boilerplate: it
is still the caller's job to sleep, send, and detect a response.

## Repository layout

```
/
├── SPEC.md                # design specification
├── LICENSE                # MIT
├── Makefile                # `make check` etc. -- see Building & testing below
├── testvectors/           # shared conformance test vectors (JSON) + generator
├── go/                     # Go implementation (module github.com/ekline/retry/go)
└── rust/                   # Rust implementation (crate `retry`)
```

## Building & testing

Run everything CI runs, from the repo root:

```sh
make setup   # one-time: installs staticcheck
make check   # go-check + rust-check
```

CI (`.github/workflows/go.yml`, `.github/workflows/rust.yml`) invokes these
same `make` targets, so there's exactly one copy of each command -- running
`make check` locally *is* what CI runs, not just something kept in sync
with it by hand. `make help` lists every target, including the finer-grained
ones (`make go-vet`, `make rust-clippy`, `make rust-test-rand`, etc.) for
running a single piece while iterating.

Both languages' test suites include a conformance test that replays every
`testvectors/*.json` file against `compute` and checks exact agreement; see
[`testvectors/README.md`](./testvectors/README.md) for the vector schema.
CI scopes each workflow to the parts of the repo that changed, and bounds
every test run (`go test -timeout 60s`; `timeout 60 cargo test`) so an
accidental infinite loop fails fast instead of hanging the job.

They also include property/fuzz tests for invariants that must hold for
*any* input (never panics, `Elapsed` never decreases, `Retries` saturates
instead of overflowing, NaN jitter behaves like `0.0`) -- see `SPEC.md`
§13. `make check` only runs the fast, seeded versions of these; for a
deeper local search:

```sh
make go-fuzz             # extended Go fuzzing, 30s
make rust-proptest-deep  # 100k proptest cases
```

### Without `make`

```sh
cd go && go vet ./... && gofmt -l . && staticcheck ./... && go test ./... -timeout 60s
cd rust && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --features rand -- -D warnings && timeout 60 cargo test && timeout 60 cargo test --features rand
```

## Status

Both implementations are complete per `SPEC.md` and pass the shared
conformance suite. See `SPEC.md` §11 for open items to resolve before a 1.0
release (this repo has already settled on `retry` as the crate/module name).

## License

MIT. See [`LICENSE`](./LICENSE).
