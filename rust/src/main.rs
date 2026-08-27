// Copyright (c) 2026 Erik Kline
// SPDX-License-Identifier: MIT

//! `retry`: a small command-line utility for manually inspecting a
//! `Params` configuration, in the spirit of `seq` -- print a sequence of
//! computed retransmission timeouts, one per line, suitable for piping
//! into other tools. See `SPEC.md` §13.
//!
//! Not part of the `libretry` library's public API and not covered by its
//! SemVer guarantees; only built with `--features cli`.

use std::io::{self, ErrorKind, Write};
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;

use libretry::{FixedJitter, Params, Sequence};

/// Print a sequence of computed retransmission timeouts, one per line, in
/// the spirit of `seq`.
///
/// Stops when the schedule gives up (MaxRetries or MaxDuration exceeded)
/// or after --count lines, whichever comes first. Output goes to stdout
/// with nothing but the values (one per line), so it can be piped into
/// other tools; a final "gave up: ..." note, if any, goes to stderr.
#[derive(Parser)]
#[command(name = "retry", version)]
struct Args {
    /// Initial retransmission timeout, in milliseconds (RFC 9915 IRT).
    initial_rt_ms: u64,

    /// MRT: cap on the pre-jitter base interval, in milliseconds.
    #[arg(long, value_name = "MS")]
    max_interval_ms: Option<u64>,

    /// MRC: give up after this many retransmissions.
    #[arg(long, value_name = "N")]
    max_retries: Option<u64>,

    /// MRD: give up once cumulative scheduled time would exceed this, in
    /// milliseconds.
    #[arg(long, value_name = "MS")]
    max_duration_ms: Option<u64>,

    /// Comma-separated jitter values to replay; repeats the last value
    /// forever once exhausted. Default: 0.0 (no jitter).
    #[arg(long, value_delimiter = ',', value_name = "F,F,...")]
    jitter: Vec<f64>,

    /// Stop after this many lines, even if the schedule is unbounded.
    #[arg(short = 'n', long, default_value_t = 20, value_name = "N")]
    count: u32,

    /// Print human-readable durations (e.g. "1.5s") instead of milliseconds.
    #[arg(short = 'H', long)]
    human: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let mut params = Params::new(Duration::from_millis(args.initial_rt_ms));
    if let Some(ms) = args.max_interval_ms {
        params = params.with_max_interval(Duration::from_millis(ms));
    }
    if let Some(n) = args.max_retries {
        params = params.with_max_retries(n);
    }
    if let Some(ms) = args.max_duration_ms {
        params = params.with_max_duration(Duration::from_millis(ms));
    }

    let jitter_values = if args.jitter.is_empty() {
        vec![0.0]
    } else {
        args.jitter
    };
    let mut seq = Sequence::new(params, FixedJitter::new(jitter_values));

    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut printed: u32 = 0;

    for rt in &mut seq {
        let line = if args.human {
            format!("{rt:?}\n")
        } else {
            format!("{}\n", rt.as_millis())
        };
        if let Err(e) = out.write_all(line.as_bytes()) {
            // A pipe closed downstream (e.g. `retry ... | head`) is not
            // an error worth reporting -- seq and friends exit quietly.
            if e.kind() == ErrorKind::BrokenPipe {
                return ExitCode::SUCCESS;
            }
            eprintln!("retry: {e}");
            return ExitCode::FAILURE;
        }
        printed += 1;
        if printed >= args.count {
            break;
        }
    }

    if let Err(e) = out.flush() {
        if e.kind() != ErrorKind::BrokenPipe {
            eprintln!("retry: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    if let Some(reason) = seq.reason() {
        eprintln!("retry: gave up: {reason:?}");
    }

    ExitCode::SUCCESS
}
