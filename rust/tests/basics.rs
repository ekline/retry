// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

//! Unit-style tests for the public API, mirroring go/retry_test.go.

use core::time::Duration;
use retry::{compute, FixedJitter, JitterSource, Params, State, Step, Termination};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn base_params() -> Params {
    Params {
        initial_rt: ms(1000),
        max_interval: None,
        max_retries: None,
        max_duration: None,
    }
}

#[test]
fn first_call_uses_initial_rt() {
    let params = base_params();
    let mut jitter = FixedJitter::new(vec![0.0]);
    let step = compute(&params, State::default(), &mut jitter);
    match step {
        Step::Wait { rt, state } => {
            assert_eq!(rt, ms(1000));
            assert_eq!(state.retries, 1);
        }
        Step::GiveUp { .. } => panic!("expected Wait, got {step:?}"),
    }
}

#[test]
fn doubling_without_cap() {
    let params = base_params();
    let mut jitter = FixedJitter::new(vec![0.0, 0.0, 0.0, 0.0]);
    let mut state = State::default();
    for want in [ms(1000), ms(2000), ms(4000), ms(8000)] {
        match compute(&params, state, &mut jitter) {
            Step::Wait { rt, state: s } => {
                assert_eq!(rt, want);
                state = s;
            }
            other => panic!("expected Wait, got {other:?}"),
        }
    }
}

#[test]
fn max_interval_caps() {
    let mut params = base_params();
    params.max_interval = Some(ms(3000));
    let mut jitter = FixedJitter::new(vec![0.0, 0.0, 0.0, 0.0]);
    let mut state = State::default();
    for want in [ms(1000), ms(2000), ms(3000), ms(3000)] {
        match compute(&params, state, &mut jitter) {
            Step::Wait { rt, state: s } => {
                assert_eq!(rt, want);
                state = s;
            }
            other => panic!("expected Wait, got {other:?}"),
        }
    }
}

#[test]
fn first_call_detection_uses_retries_not_last_rt() {
    // A caller resuming from a checkpointed state with retries > 0 (even
    // if last_rt happens to be zero) must NOT be treated as a first call.
    let params = base_params();
    let prev = State {
        retries: 1,
        last_rt: Duration::ZERO,
        elapsed: ms(1000),
    };
    let mut jitter = FixedJitter::new(vec![0.0]);
    match compute(&params, prev, &mut jitter) {
        Step::Wait { rt, .. } => assert_eq!(rt, Duration::ZERO),
        other => panic!("expected Wait, got {other:?}"),
    }
}

#[test]
fn jitter_positive() {
    let params = base_params();
    let mut jitter = FixedJitter::new(vec![0.1]);
    match compute(&params, State::default(), &mut jitter) {
        Step::Wait { rt, .. } => assert_eq!(rt, ms(1100)),
        other => panic!("expected Wait, got {other:?}"),
    }
}

#[test]
fn jitter_negative_saturates_to_zero() {
    let params = base_params();
    let mut jitter = FixedJitter::new(vec![-1.5]);
    match compute(&params, State::default(), &mut jitter) {
        Step::Wait { rt, state } => {
            assert_eq!(rt, Duration::ZERO);
            assert_eq!(state.elapsed, Duration::ZERO);
        }
        other => panic!("expected Wait, got {other:?}"),
    }
}

#[test]
fn max_retries_zero_gives_up_immediately() {
    let mut params = base_params();
    params.max_retries = Some(0);
    let mut jitter = FixedJitter::new(vec![0.0]);
    match compute(&params, State::default(), &mut jitter) {
        Step::GiveUp { reason, state } => {
            assert_eq!(reason, Termination::MaxRetries);
            assert_eq!(state.retries, 1);
        }
        other => panic!("expected GiveUp, got {other:?}"),
    }
}

#[test]
fn max_retries_exceeded() {
    let mut params = base_params();
    params.initial_rt = ms(100);
    params.max_retries = Some(2);
    let mut jitter = FixedJitter::new(vec![0.0, 0.0, 0.0]);
    let mut state = State::default();
    for _ in 0..2 {
        match compute(&params, state, &mut jitter) {
            Step::Wait { state: s, .. } => state = s,
            other => panic!("unexpected GiveUp: {other:?}"),
        }
    }
    match compute(&params, state, &mut jitter) {
        Step::GiveUp { reason, .. } => assert_eq!(reason, Termination::MaxRetries),
        other => panic!("expected GiveUp, got {other:?}"),
    }
}

#[test]
fn max_duration_exceeded() {
    let mut params = base_params();
    params.max_interval = Some(ms(10_000));
    params.max_duration = Some(ms(5_000));
    let mut jitter = FixedJitter::new(vec![0.0, 0.0, 0.0]);
    let mut state = State::default();
    for _ in 0..2 {
        match compute(&params, state, &mut jitter) {
            Step::Wait { state: s, .. } => state = s,
            other => panic!("unexpected GiveUp: {other:?}"),
        }
    }
    match compute(&params, state, &mut jitter) {
        Step::GiveUp { reason, .. } => assert_eq!(reason, Termination::MaxDuration),
        other => panic!("expected GiveUp, got {other:?}"),
    }
}

#[test]
fn first_rt_exceeding_max_duration_gives_up_immediately() {
    let mut params = base_params();
    params.max_duration = Some(ms(500));
    let mut jitter = FixedJitter::new(vec![0.0]);
    match compute(&params, State::default(), &mut jitter) {
        Step::GiveUp { reason, .. } => assert_eq!(reason, Termination::MaxDuration),
        other => panic!("expected GiveUp, got {other:?}"),
    }
}

#[test]
fn rekey_mid_sequence() {
    let mut params = base_params();
    params.max_interval = Some(ms(5000));
    let mut jitter = FixedJitter::new(vec![0.0, 0.0, 0.0]);
    let mut state = State::default();
    for _ in 0..3 {
        state = compute(&params, state, &mut jitter).state();
    }
    assert_eq!(state.last_rt, ms(4000));

    // Re-key: lower MRT to 3000ms. State carries forward unchanged.
    params.max_interval = Some(ms(3000));
    let mut jitter2 = FixedJitter::new(vec![0.0]);
    match compute(&params, state, &mut jitter2) {
        Step::Wait { rt, .. } => assert_eq!(rt, ms(3000)),
        other => panic!("expected Wait, got {other:?}"),
    }
}

#[test]
fn fixed_jitter_exhaustion_returns_zero() {
    let mut fj = FixedJitter::new(vec![0.5]);
    assert_eq!(fj.next_jitter(), 0.5);
    for _ in 0..3 {
        assert_eq!(fj.next_jitter(), 0.0);
    }
}

#[test]
fn fixed_jitter_accepts_vec_or_slice() {
    let mut from_vec = FixedJitter::new(vec![1.0, 2.0]);
    let mut from_slice = FixedJitter::new([1.0, 2.0].as_slice());
    assert_eq!(from_vec.next_jitter(), from_slice.next_jitter());
}

#[test]
fn params_new_defaults_to_unbounded() {
    let params = Params::new(ms(1000));
    assert_eq!(params.max_interval, None);
    assert_eq!(params.max_retries, None);
    assert_eq!(params.max_duration, None);
}

#[test]
fn params_builder_with_methods() {
    let params = Params::new(ms(1000))
        .with_max_interval(ms(30_000))
        .with_max_retries(10)
        .with_max_duration(ms(60_000));
    assert_eq!(
        params,
        Params {
            initial_rt: ms(1000),
            max_interval: Some(ms(30_000)),
            max_retries: Some(10),
            max_duration: Some(ms(60_000)),
        }
    );
}

#[test]
fn closures_work_as_jitter_sources() {
    let mut calls = 0;
    let mut jitter = || {
        calls += 1;
        0.25
    };
    assert_eq!(jitter.next_jitter(), 0.25);
    assert_eq!(calls, 1);

    // A closure can be passed directly to `compute`, with no wrapper type.
    let params = base_params();
    match compute(&params, State::default(), &mut || 0.1) {
        Step::Wait { rt, .. } => assert_eq!(rt, ms(1100)),
        other => panic!("expected Wait, got {other:?}"),
    }
}
