// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

//! The [`Sequence`] convenience driver.

use core::time::Duration;

use crate::{compute, JitterSource, Params, State, Step, Termination};

/// Drives a retransmission schedule, threading [`State`] between
/// [`compute`] calls so the caller doesn't have to. It performs no I/O,
/// scheduling, or sleeping itself -- it only replaces the manual
/// state-threading in the caller pattern described in `SPEC.md` section 6:
///
/// ```
/// # use core::time::Duration;
/// # use libretry::{FixedJitter, Params, Sequence};
/// # fn transmit(_msg: &str) {}
/// # fn sleep(_d: Duration) {}
/// # fn got_response() -> bool { true }
/// # fn retransmit(_msg: &str) {}
/// let params = Params::new(Duration::from_secs(1)).with_max_retries(5);
/// let mut seq = Sequence::new(params, FixedJitter::new(vec![0.0; 6]));
///
/// transmit("hello");
/// for rt in &mut seq {
///     sleep(rt);
///     if got_response() {
///         break;
///     }
///     retransmit("hello");
/// }
/// if let Some(reason) = seq.reason() {
///     eprintln!("gave up: {reason:?}");
/// }
/// ```
///
/// `Sequence` implements [`Iterator<Item = Duration>`](Iterator): each
/// [`next`](Iterator::next) call is one [`compute`] call, yielding `Some(rt)`
/// to wait and retransmit, or `None` once the schedule has given up (see
/// [`reason`](Sequence::reason)).
pub struct Sequence<J> {
    params: Params,
    state: State,
    jitter: J,
    reason: Option<Termination>,
}

impl<J: JitterSource> Sequence<J> {
    /// Returns a `Sequence` starting from the zero [`State`], ready for
    /// its first [`next`](Iterator::next) call.
    pub fn new(params: Params, jitter: J) -> Self {
        Self {
            params,
            state: State::default(),
            jitter,
            reason: None,
        }
    }

    /// The current [`State`], reflecting every completed step so far.
    pub fn state(&self) -> State {
        self.state
    }

    /// Why iteration stopped, once it has. `None` until the underlying
    /// [`compute`] call returns [`Step::GiveUp`].
    pub fn reason(&self) -> Option<Termination> {
        self.reason
    }

    /// Re-keys the schedule for subsequent steps (`SPEC.md` section 5.5),
    /// most commonly in response to a server-advertised MRT. Accumulated
    /// [`State`] carries forward unchanged, and a `Sequence` that has
    /// already given up stays given up.
    pub fn set_params(&mut self, params: Params) {
        self.params = params;
    }
}

impl<J: JitterSource> Iterator for Sequence<J> {
    type Item = Duration;

    fn next(&mut self) -> Option<Duration> {
        if self.reason.is_some() {
            return None;
        }
        match compute(&self.params, self.state, &mut self.jitter) {
            Step::Wait { rt, state } => {
                self.state = state;
                Some(rt)
            }
            Step::GiveUp { reason, state } => {
                self.state = state;
                self.reason = Some(reason);
                None
            }
        }
    }
}
