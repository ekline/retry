// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

//! Exercises `UniformJitter`, gated behind the `rand` feature. This file
//! compiles to an empty test binary when `rand` is not enabled.
#![cfg(feature = "rand")]

use rand_core::{Error, RngCore};
use retry::{JitterSource, UniformJitter};

/// A tiny deterministic xorshift64* generator implementing `RngCore`,
/// used only to exercise `UniformJitter` without an extra dev-dependency
/// on a full RNG crate.
struct XorShift64(u64);

impl RngCore for XorShift64 {
    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

#[test]
fn uniform_jitter_stays_within_bounds() {
    let mut uj = UniformJitter::new(XorShift64(0x9e37_79b9_7f4a_7c15), 0.1);
    for _ in 0..10_000 {
        let v = uj.next_jitter();
        assert!((-0.1..=0.1).contains(&v), "jitter {v} out of bounds");
    }
}

#[test]
fn uniform_jitter_is_deterministic_for_a_given_seed() {
    let mut a = UniformJitter::new(XorShift64(42), 0.1);
    let mut b = UniformJitter::new(XorShift64(42), 0.1);
    for _ in 0..100 {
        assert_eq!(a.next_jitter(), b.next_jitter());
    }
}
