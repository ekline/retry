// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

//! Property-based tests for `compute`, checking invariants that should
//! hold for any input -- not just the hand-picked cases in `tests/basics.rs`
//! and the shared conformance vectors.
//!
//! Run `PROPTEST_CASES=100000 cargo test --test properties` locally for a
//! deeper search than CI's default (256 cases per property).

use core::time::Duration;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

use libretry::{compute, Params, State, Step, Termination};

/// Durations up to `u64::MAX` nanoseconds (~584 years), the range in
/// which this library's jitter arithmetic is exact; see SPEC.md §5.1's
/// note on why durations beyond that are a documented (not fuzzed)
/// precision limit of `f64` itself, not something this library can fix.
fn duration_strategy() -> impl Strategy<Value = Duration> {
    any::<u64>().prop_map(Duration::from_nanos)
}

fn opt_duration_strategy() -> impl Strategy<Value = Option<Duration>> {
    prop_oneof![Just(None), duration_strategy().prop_map(Some)]
}

fn opt_u64_strategy() -> impl Strategy<Value = Option<u64>> {
    prop_oneof![Just(None), any::<u64>().prop_map(Some)]
}

/// Jitter values: a typical-ish range weighted most common, plus the
/// specific non-finite values that historically broke things, plus fully
/// arbitrary bit patterns for good measure.
fn jitter_strategy() -> impl Strategy<Value = f64> {
    prop_oneof![
        6 => -10.0..10.0f64,
        1 => Just(f64::NAN),
        1 => Just(f64::INFINITY),
        1 => Just(f64::NEG_INFINITY),
        1 => any::<f64>(),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_failure_persistence(
        // Default failure persistence looks for a sibling `src/` to mirror
        // a path under, which doesn't exist for a `tests/` integration
        // test; store any regressions next to this file instead.
        FileFailurePersistence::WithSource("regressions"),
    ))]

    /// `compute` must never panic for any input, `Elapsed` must never
    /// decrease, `Retries` must increase by exactly 1 (or stay saturated
    /// at `u64::MAX`), and the termination decision must match SPEC.md
    /// §5.4's order exactly.
    #[test]
    fn compute_never_panics_and_upholds_invariants(
        initial_rt in duration_strategy(),
        max_interval in opt_duration_strategy(),
        max_retries in opt_u64_strategy(),
        max_duration in opt_duration_strategy(),
        prev_retries in any::<u64>(),
        prev_last_rt in duration_strategy(),
        prev_elapsed in duration_strategy(),
        jitter in jitter_strategy(),
    ) {
        let params = Params { initial_rt, max_interval, max_retries, max_duration };
        let prev = State { retries: prev_retries, last_rt: prev_last_rt, elapsed: prev_elapsed };

        let step = compute(&params, prev, &mut || jitter);
        let state = step.state();

        prop_assert!(
            state.elapsed >= prev.elapsed,
            "Elapsed decreased: {:?} -> {:?}",
            prev.elapsed,
            state.elapsed
        );

        if prev.retries == u64::MAX {
            prop_assert_eq!(state.retries, u64::MAX, "Retries at u64::MAX should stay saturated");
        } else {
            prop_assert_eq!(state.retries, prev.retries + 1);
        }

        let want_max_retries = max_retries.is_some_and(|mr| state.retries > mr);
        let want_max_duration =
            !want_max_retries && max_duration.is_some_and(|md| state.elapsed > md);

        match (&step, want_max_retries, want_max_duration) {
            (Step::GiveUp { reason, .. }, true, _) => {
                prop_assert_eq!(*reason, Termination::MaxRetries);
            }
            (Step::GiveUp { reason, .. }, false, true) => {
                prop_assert_eq!(*reason, Termination::MaxDuration);
            }
            (Step::Wait { .. }, false, false) => {}
            (other, want_mr, want_md) => {
                prop_assert!(
                    false,
                    "unexpected step {:?} (want_max_retries={}, want_max_duration={})",
                    other,
                    want_mr,
                    want_md
                );
            }
        }
    }

    /// NaN jitter must behave exactly like 0.0 jitter (SPEC.md §5.2),
    /// regardless of the rest of the input.
    #[test]
    fn nan_jitter_equals_zero_jitter(
        initial_rt in duration_strategy(),
        prev_retries in any::<u64>(),
        prev_last_rt in duration_strategy(),
        prev_elapsed in duration_strategy(),
    ) {
        let params = Params::new(initial_rt);
        let prev = State { retries: prev_retries, last_rt: prev_last_rt, elapsed: prev_elapsed };

        let nan_step = compute(&params, prev, &mut || f64::NAN);
        let zero_step = compute(&params, prev, &mut || 0.0);
        prop_assert_eq!(nan_step, zero_step);
    }
}
