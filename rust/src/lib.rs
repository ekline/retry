// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

//! A pure, sans-I/O retransmission backoff calculator, generic over
//! protocol.
//!
//! The shape of the parameters and the doubling-with-jitter algorithm are
//! drawn from RFC 9915 section 15, but this crate has no DHCPv6, CoAP,
//! TLS, or other protocol-specific behavior: it is the arithmetic core
//! that any retransmitting protocol can wrap. It computes one
//! retransmission timeout per [`compute`] call, tracks scheduled (not
//! wall-clock) elapsed time for MRD enforcement, and reports termination
//! as part of its return value. It performs no I/O, scheduling,
//! sleeping, or randomness generation of its own -- callers supply a
//! [`JitterSource`].
//!
//! See `SPEC.md` at the repository root for the full design
//! specification.
#![no_std]
#![deny(missing_docs)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

mod sequence;
pub use sequence::Sequence;

/// Describes an immutable retransmission schedule.
///
/// Pass a new `Params` to [`compute`] to re-key mid-sequence (e.g. when a
/// server advertises a new MRT); [`State`] carries forward unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// Nominal first retransmission timeout (RFC 9915 IRT), used as the
    /// base in the first `compute` call.
    pub initial_rt: Duration,

    /// Upper bound on the pre-jitter base interval (RFC 9915 MRT).
    /// `None` means unbounded.
    pub max_interval: Option<Duration>,

    /// Give up once this many retransmissions have been scheduled
    /// (RFC 9915 MRC). `None` means unbounded; `Some(0)` means no
    /// retransmissions are permitted at all.
    pub max_retries: Option<u64>,

    /// Give up once cumulative scheduled elapsed time would exceed this
    /// (RFC 9915 MRD). `None` means unbounded.
    pub max_duration: Option<Duration>,
}

impl Params {
    /// Creates `Params` with `initial_rt` and every optional field
    /// unbounded, ready for `with_*` overrides:
    ///
    /// ```
    /// # use core::time::Duration;
    /// # use retry::Params;
    /// let params = Params::new(Duration::from_secs(1))
    ///     .with_max_interval(Duration::from_secs(30))
    ///     .with_max_retries(10);
    /// ```
    pub fn new(initial_rt: Duration) -> Self {
        Self {
            initial_rt,
            max_interval: None,
            max_retries: None,
            max_duration: None,
        }
    }

    /// Sets `max_interval` (RFC 9915 MRT), the upper bound on the
    /// pre-jitter base interval.
    pub fn with_max_interval(mut self, max_interval: Duration) -> Self {
        self.max_interval = Some(max_interval);
        self
    }

    /// Sets `max_retries` (RFC 9915 MRC), the retry budget. `0` permits no
    /// retransmissions at all.
    pub fn with_max_retries(mut self, max_retries: u64) -> Self {
        self.max_retries = Some(max_retries);
        self
    }

    /// Sets `max_duration` (RFC 9915 MRD), the cumulative scheduled-time
    /// budget.
    pub fn with_max_duration(mut self, max_duration: Duration) -> Self {
        self.max_duration = Some(max_duration);
        self
    }
}

/// History carried between [`compute`] calls.
///
/// The [`Default`] value is the initial state, which the caller passes
/// for the first call. States may be freely constructed and serialized
/// by callers; there are no hidden invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State {
    /// Number of retransmissions scheduled so far. Zero before the first
    /// call. Saturates at `u64::MAX` instead of overflowing (see
    /// `compute`'s use of `saturating_add`); reaching that in practice
    /// would require billions of calls per second for centuries.
    pub retries: u64,

    /// Most recently computed RT. Zero before the first call.
    pub last_rt: Duration,

    /// Sum of all RTs scheduled so far. Zero before the first call.
    pub elapsed: Duration,
}

/// Give-up reasons returned by [`compute`]. More variants may be added in
/// a minor release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Termination {
    /// The configured `max_retries` (RFC 9915 MRC) was exhausted.
    MaxRetries,
    /// The configured `max_duration` (RFC 9915 MRD) was exceeded.
    MaxDuration,
}

/// The result of a [`compute`] call: either a wait instruction or a
/// terminal give-up signal. Carries the updated [`State`] in both cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Wait `rt` before retransmitting.
    Wait {
        /// The computed retransmission timeout.
        rt: Duration,
        /// The updated state.
        state: State,
    },
    /// Give up; do not retransmit again.
    GiveUp {
        /// Why the caller should give up.
        reason: Termination,
        /// The updated state.
        state: State,
    },
}

impl Step {
    /// Returns the updated [`State`], regardless of variant.
    pub fn state(&self) -> State {
        match *self {
            Step::Wait { state, .. } => state,
            Step::GiveUp { state, .. } => state,
        }
    }
}

/// Supplies the multiplier applied to a candidate base RT to produce the
/// actual RT.
///
/// The library is agnostic to the distribution; bounds, shape, and bias
/// are entirely the source's concern. For example, for DHCPv6 the source
/// would yield uniform values in `[-0.1, +0.1]`.
pub trait JitterSource {
    /// Returns the multiplier for the next [`compute`] call.
    fn next_jitter(&mut self) -> f64;
}

impl<F: FnMut() -> f64> JitterSource for F {
    fn next_jitter(&mut self) -> f64 {
        self()
    }
}

/// Replays a fixed sequence of jitter values. Once exhausted, it keeps
/// repeating the last value forever (or `0.0`, if it was constructed with
/// no values at all) rather than silently switching to unjittered
/// behavior.
///
/// Used for deterministic testing and conformance vector replay.
#[derive(Debug, Clone, Default)]
pub struct FixedJitter {
    values: Vec<f64>,
    next: usize,
}

impl FixedJitter {
    /// Creates a `FixedJitter` that replays `values` in order.
    pub fn new(values: impl Into<Vec<f64>>) -> Self {
        Self {
            values: values.into(),
            next: 0,
        }
    }
}

impl JitterSource for FixedJitter {
    fn next_jitter(&mut self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        let idx = self.next.min(self.values.len() - 1);
        let v = self.values[idx];
        if self.next < self.values.len() {
            self.next += 1;
        }
        v
    }
}

/// Performs one retransmission-timeout computation. Pure given a
/// deterministic [`JitterSource`].
///
/// See `SPEC.md` section 5 for the full algorithm description.
pub fn compute<J: JitterSource + ?Sized>(params: &Params, prev: State, jitter: &mut J) -> Step {
    let base = select_base(params, &prev);

    let j = jitter.next_jitter();
    let rt = apply_jitter(base, j);

    let new_state = State {
        retries: prev.retries.saturating_add(1),
        last_rt: rt,
        elapsed: saturating_add(prev.elapsed, rt),
    };

    // Termination is evaluated against new_state, not prev, in this order.
    if let Some(max_retries) = params.max_retries {
        if new_state.retries > max_retries {
            return Step::GiveUp {
                reason: Termination::MaxRetries,
                state: new_state,
            };
        }
    }
    if let Some(max_duration) = params.max_duration {
        if new_state.elapsed > max_duration {
            return Step::GiveUp {
                reason: Termination::MaxDuration,
                state: new_state,
            };
        }
    }
    Step::Wait {
        rt,
        state: new_state,
    }
}

/// Implements SPEC.md section 5.1.
fn select_base(params: &Params, prev: &State) -> Duration {
    if prev.retries == 0 {
        return params.initial_rt;
    }
    let candidate = saturating_scale(prev.last_rt);
    match params.max_interval {
        Some(max_interval) if candidate > max_interval => max_interval,
        _ => candidate,
    }
}

/// Implements SPEC.md section 5.2: computes `base + base * j` in `f64`
/// (deliberately not `base * (1 + j)`; the two are not equivalent under
/// floating-point arithmetic, and conformance vectors are sensitive to
/// evaluation order), then rounds to the nearest nanosecond, half away
/// from zero. Results below zero (which requires `j < -1.0`) saturate to
/// zero. A NaN result is treated as if jitter were 0.0.
fn apply_jitter(base: Duration, j: f64) -> Duration {
    let base_f = base.as_nanos() as f64;
    let mut rt_f = base_f + base_f * j;
    if rt_f.is_nan() {
        // A NaN result (whether from a NaN jitter value, or a 0 * +-Inf
        // edge case when base is zero and jitter is infinite) is treated
        // as if jitter were 0.0, falling back to the unjittered base
        // rather than propagating NaN into a nonsensical Duration.
        rt_f = base_f;
    }
    if rt_f < 0.0 {
        return Duration::ZERO;
    }
    // rt_f is non-negative here, so "half away from zero" and "half up"
    // coincide: floor(rt_f + 0.5). This deliberately avoids `f64::round`,
    // which is a `std`-only method not available under `no_std` without a
    // `libm` dependency; the `as u64` cast truncates toward zero, which
    // is floor() for non-negative operands.
    let biased = rt_f + 0.5;
    if biased >= u64::MAX as f64 {
        return Duration::from_nanos(u64::MAX);
    }
    Duration::from_nanos(biased as u64)
}

/// The fixed per-retry growth factor mandated by RFC 9915 section 15
/// (`RT = 2*RTprev + jitter`), currently 2 (doubling). Named generically
/// rather than e.g. `DOUBLING_FACTOR` in case a future major version needs
/// to generalize it; it is intentionally not a `Params` field today --
/// `SPEC.md` section 5.1 defines this algorithm as doubling, not as
/// configurable exponential backoff with an arbitrary base.
const SCALE_FACTOR: u32 = 2;

/// Multiplies `d` by [`SCALE_FACTOR`], saturating at the maximum
/// representable `Duration` instead of overflowing.
fn saturating_scale(d: Duration) -> Duration {
    d.checked_mul(SCALE_FACTOR).unwrap_or(Duration::MAX)
}

/// Adds `a` and `b`, saturating at the maximum representable `Duration`
/// instead of overflowing.
fn saturating_add(a: Duration, b: Duration) -> Duration {
    a.checked_add(b).unwrap_or(Duration::MAX)
}

#[cfg(feature = "rand")]
mod uniform {
    use super::JitterSource;
    use rand_core::RngCore;

    /// Wraps a PRNG and returns uniform values in `[-factor, +factor]`.
    ///
    /// Enabled by the `rand` feature. Depends only on `rand_core::RngCore`
    /// rather than a specific generator, so callers may plug in any
    /// `rand`-ecosystem RNG (or a custom one).
    #[derive(Debug, Clone, Copy)]
    pub struct UniformJitter<R> {
        rng: R,
        factor: f64,
    }

    impl<R: RngCore> UniformJitter<R> {
        /// Creates a `UniformJitter` using `rng` and the given bound.
        pub fn new(rng: R, factor: f64) -> Self {
            Self { rng, factor }
        }

        fn next_f64(&mut self) -> f64 {
            // 53 bits of randomness, matching f64's mantissa precision.
            let hi53 = self.rng.next_u64() >> 11;
            hi53 as f64 / (1u64 << 53) as f64
        }
    }

    impl<R: RngCore> JitterSource for UniformJitter<R> {
        fn next_jitter(&mut self) -> f64 {
            -self.factor + self.next_f64() * 2.0 * self.factor
        }
    }
}

#[cfg(feature = "rand")]
pub use uniform::UniformJitter;
