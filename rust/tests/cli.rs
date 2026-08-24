// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

//! Integration tests for the `retry` CLI binary. Compiles to an empty
//! test binary unless built with `--features cli`, since that's what
//! gates the binary itself.
#![cfg(feature = "cli")]

use std::process::Command;

fn retry() -> Command {
    Command::new(env!("CARGO_BIN_EXE_retry"))
}

#[test]
fn basic_doubling_no_jitter() {
    let output = retry()
        .args(["1000", "--max-retries", "3"])
        .output()
        .expect("failed to run retry");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "1000\n2000\n4000\n"
    );
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("MaxRetries"));
}

#[test]
fn max_interval_caps() {
    let output = retry()
        .args(["1000", "--max-interval-ms", "3000", "--max-retries", "4"])
        .output()
        .expect("failed to run retry");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "1000\n2000\n3000\n3000\n"
    );
}

#[test]
fn human_flag_formats_durations() {
    let output = retry()
        .args(["1000", "--max-retries", "1", "--human"])
        .output()
        .expect("failed to run retry");
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "1s\n");
}

#[test]
fn count_limits_unbounded_output() {
    let output = retry()
        .args(["1", "-n", "5"])
        .output()
        .expect("failed to run retry");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 5);
    // Fully unbounded: no give-up note on stderr, we just stopped at -n.
    assert!(output.stderr.is_empty());
}

#[test]
fn jitter_exhaustion_repeats_last_value() {
    let output = retry()
        .args(["1000", "--jitter", "0.5,-0.5", "-n", "4"])
        .output()
        .expect("failed to run retry");
    assert!(output.status.success());
    // base=1000,j=0.5->1500; base=3000,j=-0.5->1500; then j=-0.5 repeats.
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "1500\n1500\n1500\n1500\n"
    );
}

#[test]
fn missing_required_argument_fails() {
    let output = retry().output().expect("failed to run retry");
    assert!(!output.status.success());
}

#[test]
fn help_flag_exits_zero() {
    let output = retry().arg("--help").output().expect("failed to run retry");
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("Usage: retry"));
}
