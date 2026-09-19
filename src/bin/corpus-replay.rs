//! Replay a traffic corpus against the loaded rule packs.
//!
//! Measurement harness for the coverage replays recorded in
//! `packs/coverage-justifications.md` and `docs/notes/traffic-corpus-replay.md`.
//! It evaluates every command in a corpus file (one JSON `{"command": ...}`
//! object per line, as produced by `scripts/extract-traffic-corpus`) through
//! the same `Engine::evaluate_command` path the hook uses, and writes one
//! result object per input line, in input order:
//!
//! ```json
//! {"verdict":"denied","pack_id":"git","pattern_id":"git-commit-without-pathspec",
//!  "segments":[["git","commit","-m","x"]]}
//! ```
//!
//! Result lines are positional: line N corresponds to corpus line N, and the
//! command text itself is deliberately **not** copied into the results — the
//! corpus stays the only file carrying verbatim traffic. Nothing is written
//! to the denial log or telemetry: the engine is constructed without a
//! telemetry store or state store, so evaluation is side-effect free.
//!
//! Usage: `corpus-replay <packs-dir> <corpus.jsonl> <results.jsonl>`
//!
//! Both verdict summaries and per-rule attribution go to stderr; stdout is
//! kept clean so the results file can be piped.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use icg::engine::{CheckResult, CommandSource, Engine};
use serde_json::json;

fn verdict_fields(result: &CheckResult) -> (&'static str, Option<&str>, Option<&str>) {
    match result {
        CheckResult::Allowed => ("allowed", None, None),
        CheckResult::Denied {
            pack_id,
            pattern_id,
            ..
        } => ("denied", Some(pack_id), Some(pattern_id)),
        CheckResult::Rewrite {
            pack_id,
            pattern_id,
            ..
        } => ("rewrite", Some(pack_id), Some(pattern_id)),
        CheckResult::Warning {
            pack_id,
            pattern_id,
            ..
        } => ("warning", Some(pack_id), Some(pattern_id)),
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (packs_dir, corpus_path, results_path) = match (args.next(), args.next(), args.next()) {
        (Some(p), Some(c), Some(r)) => (p, c, r),
        _ => {
            eprintln!("usage: corpus-replay <packs-dir> <corpus.jsonl> <results.jsonl>");
            return ExitCode::from(2);
        }
    };

    let mut engine = Engine::new();
    if let Err(error) = engine.load_packs_from_dir(PathBuf::from(&packs_dir)) {
        eprintln!("error: cannot load packs from {packs_dir}: {error:#}");
        return ExitCode::from(2);
    }

    let corpus_file = match File::open(&corpus_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("error: cannot open corpus {corpus_path}: {error}");
            return ExitCode::from(2);
        }
    };
    let results_file = match File::create(&results_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("error: cannot create results {results_path}: {error}");
            return ExitCode::from(2);
        }
    };
    let mut out = BufWriter::new(results_file);

    let reader = BufReader::new(corpus_file);
    let mut counts = std::collections::BTreeMap::new();
    let mut index: usize = 0;
    for (line_no, line) in reader.lines().enumerate() {
        let Ok(line) = line else {
            eprintln!("error: corpus line {} is not readable", line_no + 1);
            return ExitCode::from(2);
        };
        let command = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(value) => match value.get("command").and_then(|c| c.as_str()) {
                Some(command) => command.to_string(),
                None => {
                    eprintln!(
                        "error: corpus line {} has no string \"command\"",
                        line_no + 1
                    );
                    return ExitCode::from(2);
                }
            },
            Err(error) => {
                eprintln!("error: corpus line {} is not JSON: {error}", line_no + 1);
                return ExitCode::from(2);
            }
        };

        let source = CommandSource::Hook(command);
        let result = engine.evaluate_command(&source);
        let (verdict, pack_id, pattern_id) = verdict_fields(&result);
        let segments: Vec<Vec<String>> = engine
            .segment_command(&source)
            .into_iter()
            .map(|token| {
                let mut rendered = vec![token.executable];
                rendered.extend(token.args);
                rendered
            })
            .collect();

        *counts.entry(verdict).or_insert(0usize) += 1;
        index += 1;
        if index.is_multiple_of(5000) {
            eprintln!("... {index} evaluated");
        }

        let record = json!({
            "verdict": verdict,
            "pack_id": pack_id,
            "pattern_id": pattern_id,
            "segments": segments,
        });
        if serde_json::to_writer(&mut out, &record).is_err() || writeln!(out).is_err() {
            eprintln!("error: cannot write results line {}", index);
            return ExitCode::from(2);
        }
    }

    if let Err(error) = out.flush() {
        eprintln!("error: cannot flush results: {error}");
        return ExitCode::from(2);
    }

    eprintln!("evaluated {index} commands: {counts:?}");
    ExitCode::SUCCESS
}
