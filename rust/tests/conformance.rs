// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

//! Replays the shared `testvectors/*.json` files against `retry::compute`
//! and asserts exact agreement, mirroring `go/conformance_test.go`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use retry::{compute, FixedJitter, Params, State, Step, Termination};

/// Mirrors the schema documented in `testvectors/README.md`.
#[derive(Debug, Deserialize)]
struct VectorFile {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    description: String,
    phases: Vec<VectorPhase>,
}

#[derive(Debug, Deserialize)]
struct VectorPhase {
    params: VectorParams,
    jitter: Vec<f64>,
    expected: Vec<VectorExpect>,
}

#[derive(Debug, Deserialize)]
struct VectorParams {
    initial_rt_ms: i64,
    max_interval_ms: Option<i64>,
    max_retries: Option<i64>,
    max_duration_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct VectorExpect {
    kind: String,
    rt_ms: Option<i64>,
    reason: Option<String>,
    retries: i64,
    last_rt_ms: i64,
    elapsed_ms: i64,
}

fn ms(n: i64) -> Duration {
    Duration::from_millis(n as u64)
}

impl VectorParams {
    fn to_params(&self) -> Params {
        Params {
            initial_rt: ms(self.initial_rt_ms),
            max_interval: self.max_interval_ms.map(ms),
            max_retries: self.max_retries.map(|n| n as u64),
            max_duration: self.max_duration_ms.map(ms),
        }
    }
}

fn vector_paths() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../testvectors");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {}", dir.display(), e))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "no test vectors found under {} -- did generate.py run?",
        dir.display()
    );
    paths
}

#[test]
fn conformance() {
    let mut failures = Vec::new();

    for path in vector_paths() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let raw =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        let doc: VectorFile = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {}", path.display(), e));

        let mut state = State::default();

        for (pi, phase) in doc.phases.iter().enumerate() {
            let params = phase.params.to_params();
            let mut jitter = FixedJitter::new(phase.jitter.clone());

            for (ei, want) in phase.expected.iter().enumerate() {
                let step = compute(&params, state, &mut jitter);
                state = step.state();
                let label = format!("{name} phase {pi} entry {ei}");

                match (&step, want.kind.as_str()) {
                    (Step::Wait { rt, .. }, "wait") => {
                        let want_rt = want.rt_ms.unwrap_or_else(|| {
                            panic!("{label}: vector missing rt_ms for kind=wait")
                        });
                        if rt.as_millis() as i64 != want_rt {
                            failures.push(format!(
                                "{label}: rt = {}ms, want {}ms",
                                rt.as_millis(),
                                want_rt
                            ));
                        }
                    }
                    (Step::GiveUp { reason, .. }, "giveup") => {
                        let want_reason = want.reason.as_deref().unwrap_or_else(|| {
                            panic!("{label}: vector missing reason for kind=giveup")
                        });
                        let got_reason = match reason {
                            Termination::MaxRetries => "max_retries",
                            Termination::MaxDuration => "max_duration",
                            _ => "unknown",
                        };
                        if got_reason != want_reason {
                            failures.push(format!(
                                "{label}: reason = {got_reason}, want {want_reason}"
                            ));
                        }
                    }
                    (got, kind) => {
                        failures.push(format!("{label}: got {got:?}, want kind={kind}"));
                    }
                }

                if state.retries as i64 != want.retries {
                    failures.push(format!(
                        "{label}: retries = {}, want {}",
                        state.retries, want.retries
                    ));
                }
                if state.last_rt.as_millis() as i64 != want.last_rt_ms {
                    failures.push(format!(
                        "{label}: last_rt = {}ms, want {}ms",
                        state.last_rt.as_millis(),
                        want.last_rt_ms
                    ));
                }
                if state.elapsed.as_millis() as i64 != want.elapsed_ms {
                    failures.push(format!(
                        "{label}: elapsed = {}ms, want {}ms",
                        state.elapsed.as_millis(),
                        want.elapsed_ms
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "conformance failures:\n{}",
        failures.join("\n")
    );
}
