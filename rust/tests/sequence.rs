// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

//! Tests for the `Sequence` convenience driver.

use core::time::Duration;

use retry::{FixedJitter, Params, Sequence, Termination};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn sequence_basic() {
    let params = Params::new(ms(500));
    let mut seq = Sequence::new(params, FixedJitter::new(vec![0.0, 0.0, 0.0]));
    let want = [ms(500), ms(1000), ms(2000)];

    // params is fully unbounded, so bound the loop with `.take()` --
    // otherwise this would iterate forever.
    let got: Vec<Duration> = seq.by_ref().take(3).collect();

    assert_eq!(got, want);
    assert_eq!(seq.state().retries, 3);
    assert_eq!(seq.reason(), None);
}

#[test]
fn sequence_gives_up_and_stays_given_up() {
    let params = Params::new(ms(1000)).with_max_retries(1);
    let mut seq = Sequence::new(params, FixedJitter::new(vec![0.0, 0.0, 0.0]));

    assert_eq!(seq.next(), Some(ms(1000)));
    assert_eq!(seq.next(), None);
    assert_eq!(seq.reason(), Some(Termination::MaxRetries));

    // Once given up, the iterator keeps yielding None without recomputing.
    assert_eq!(seq.next(), None);
    assert_eq!(seq.reason(), Some(Termination::MaxRetries));
}

#[test]
fn sequence_set_params_rekeys() {
    let params = Params::new(ms(1000)).with_max_interval(ms(5000));
    let mut seq = Sequence::new(params, FixedJitter::new(vec![0.0, 0.0, 0.0]));

    let checkpoint_rt = seq.by_ref().take(3).last();
    assert_eq!(checkpoint_rt, Some(ms(4000)));

    seq.set_params(Params::new(ms(1000)).with_max_interval(ms(3000)));
    assert_eq!(seq.next(), Some(ms(3000)));
}

#[test]
fn sequence_is_a_real_iterator() {
    // Exercises standard Iterator combinators, not just manual .next().
    let params = Params::new(ms(100));
    let seq = Sequence::new(params, FixedJitter::new(vec![0.0; 10]));
    let first_three: Vec<Duration> = seq.take(3).collect();
    assert_eq!(first_three, [ms(100), ms(200), ms(400)]);
}
