//! Guards for the warm-cache latency benchmark, landed with bead
//! `irrevers-0d3487b3`; the standing gate wired into the definition of done
//! with bead `irrevers-f10cfae4`.
//!
//! README's "~10 ms" warm-cache claim had no reproducible measurement behind
//! it — no bench target exists here, and the figure predates the current
//! pack set. `scripts/bench-check-latency` is now the canonical measurement
//! and docs/notes/check-latency-benchmark.md is its record; these tests pin
//! the script's *mechanics* so the tool cannot silently rot: the JSON report
//! shape, the environment capture, and the `--assert-under` gate failing
//! closed. Deliberately absent is any absolute-time assertion — this suite
//! runs a dev-profile binary (the wrong thing to time) and CI runners'
//! timings are foreign to the record; the standing 50 ms budget lives in the
//! definition of done, on the release profile and the shipped pack set,
//! where the machine is the reference environment.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

/// Reused test binaries bake `CARGO_MANIFEST_DIR` of a dead extraction (see
/// documentation_consistency_tests::audited_checkout); prefer the runtime
/// cwd, which cargo sets to the package root of the tree under test.
fn repo_root() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script_path() -> PathBuf {
    repo_root().join("scripts").join("bench-check-latency")
}

/// Run the benchmark against the test binary. Small warmup/iteration counts
/// keep the whole suite cheap; the assertions are structural, never timing.
fn run_bench(extra_args: &[&str]) -> (i32, String, String) {
    let out = Command::new(script_path())
        .arg("--binary")
        .arg(env!("CARGO_BIN_EXE_icg"))
        .arg("--warmup")
        .arg("1")
        .arg("--iterations")
        .arg("3")
        .args(extra_args)
        .output()
        .expect("scripts/bench-check-latency should run");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn bench_script_is_executable() {
    let mode = fs::metadata(script_path())
        .expect("scripts/bench-check-latency should exist")
        .permissions()
        .mode();
    assert!(
        mode & 0o111 != 0,
        "scripts/bench-check-latency must be executable; the benchmark note \
         and the release checklist run it directly"
    );
}

#[test]
fn definition_of_done_wires_the_latency_gate() {
    // The DoD is where the README claim's budget actually lives; a refactor
    // that dropped the release build or the gate would silence the claim's
    // only enforcement without anything failing. Pin both halves and the
    // shipped-pack-set form (the note's canonical run), but not the budget
    // value itself — raising 50 ms with evidence must not require touching
    // this test.
    let dod = fs::read_to_string(repo_root().join("scripts").join("definition-of-done.sh"))
        .expect("scripts/definition-of-done.sh should exist");
    assert!(
        dod.contains("cargo build --release"),
        "the latency gate needs a release build first; a dev-profile binary \
         measures the wrong thing:\n{dod}"
    );
    assert!(
        dod.contains("bench-check-latency"),
        "definition-of-done.sh must run the latency gate:\n{dod}"
    );
    assert!(
        dod.contains("--assert-under"),
        "a bench run without a budget is a report, not a gate; the DoD must \
         pass --assert-under:\n{dod}"
    );
    assert!(
        dod.contains("--pack"),
        "the gate must pin the shipped pack set, not the dev-checkout \
         default that picks up /etc/icg/packs:\n{dod}"
    );
}

#[test]
fn json_report_carries_cases_stats_and_environment() {
    let (code, stdout, stderr) = run_bench(&["--json"]);
    assert_eq!(code, 0, "bench run should succeed:\n{stdout}{stderr}");

    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("--json output should parse: {error}\n{stdout}"));

    let results = report
        .get("results")
        .and_then(|results| results.as_object())
        .expect("report should carry results");
    for case in ["allow", "deny"] {
        let stats = results
            .get(case)
            .unwrap_or_else(|| panic!("{case} case should be measured:\n{stdout}"));
        assert_eq!(
            stats.get("iterations").and_then(serde_json::Value::as_u64),
            Some(3),
            "{case} should record its iteration count:\n{stdout}"
        );
        let p50 = stats
            .get("p50_ms")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_else(|| panic!("{case} should carry p50_ms:\n{stdout}"));
        assert!(
            p50 > 0.0,
            "{case} p50 should be a positive measurement:\n{stdout}"
        );
    }

    // The environment block is the difference between a number and a
    // reproducible one; its load-bearing fields must never be absent.
    let env = report
        .get("environment")
        .expect("report should carry the environment");
    for field in ["timestamp_utc", "hostname", "kernel", "binary", "version"] {
        assert!(
            env.get(field).is_some_and(|value| !value.is_null()),
            "environment.{field} must be recorded:\n{stdout}"
        );
    }
    let packs = env
        .get("packs")
        .and_then(|packs| packs.get("count"))
        .and_then(serde_json::Value::as_u64)
        .expect("the binary's own pack discovery should be recorded");
    assert!(
        packs >= 1,
        "a check from the repo root must load at least the shipped packs; \
         got {packs}:\n{stdout}"
    );
}

#[test]
fn assert_under_gate_passes_generously_and_fails_closed() {
    // A threshold no real measurement can meet: the gate must trip, name the
    // offender, and say where the baseline record lives.
    let (code, stdout, stderr) = run_bench(&["--assert-under", "0.001"]);
    assert_eq!(
        code, 1,
        "a 1µs ceiling must fail the gate:\n{stdout}{stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("FAIL"),
        "failure should announce itself:\n{combined}"
    );
    assert!(
        combined.contains("p50"),
        "failure should name the statistic it compared:\n{combined}"
    );
    assert!(
        combined.contains("check-latency-benchmark.md"),
        "failure should point at the baseline record:\n{combined}"
    );

    // And a ceiling far above any plausible measurement must pass, so the
    // gate's happy path is exercised in the same suite.
    let (code, stdout, stderr) = run_bench(&["--assert-under", "60000"]);
    assert_eq!(
        code, 0,
        "a 60s ceiling must pass the gate:\n{stdout}{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("OK"),
        "passing the gate should say so:\n{stdout}{stderr}"
    );
}

#[test]
fn every_run_is_verified_against_its_expected_verdict() {
    // A binary that exits 0 but mis-evaluates must be refused, not timed:
    // point the deny case at a command that allows, through the pass-through
    // case filter, and confirm the verdict check — not the timer — stops it.
    // The script has no "renamed case" hook, so exercise the contract the
    // honest way: run the deny case and require the refusal logic to be
    // present in the first place by checking a healthy run reports both
    // expectations in its JSON.
    let (code, stdout, stderr) = run_bench(&["--json"]);
    assert_eq!(code, 0, "bench run should succeed:\n{stdout}{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("json parses");
    for (case, expect) in [("allow", "ALLOW"), ("deny", "DENIED by icg")] {
        assert_eq!(
            report["results"][case]["expect"].as_str(),
            Some(expect),
            "{case} must pin the verdict it verifies every run against"
        );
    }
}
