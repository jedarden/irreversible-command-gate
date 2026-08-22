//! End-to-end tests for the four developer workflows in
//! `docs/examples/README.md` (Scenarios 6–9).
//!
//! The smaller scenario-specific test files exercise individual behaviors.
//! This file keeps the documented workflows honest by walking each scenario
//! from its fixture/scaffold through intermediate artifacts and final checks.

use icg::coverage::run_coverage_diff;
use icg::engine::{CheckResult, CommandSource, ContentSource, Engine};
use icg::regression::{
    generate_regression_suite_from_manifest, verify_regression_suite, ExpectedVerdict,
    RegressionSuite,
};
use icg::rule_pack::{load_pack, Check};
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/developer-scenarios")
        .join(name)
}

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

fn icg_with_stdin(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("icg should run");
    child
        .stdin
        .take()
        .expect("stdin should be available")
        .write_all(input.as_bytes())
        .expect("stdin should accept the request");
    child.wait_with_output().expect("icg should finish")
}

fn output_text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn pack_fixture(name: &str) -> icg::rule_pack::Pack {
    load_pack(fixture(name)).unwrap_or_else(|error| panic!("{name} should load: {error:#}"))
}

#[test]
fn scenario_6_new_pack_scaffold_and_local_validation() {
    let temp = tempdir().unwrap();
    let output_dir = temp.path().to_string_lossy().into_owned();

    let scaffold = icg(&[
        "new-pack",
        "kubectl-demo",
        "--pack-type",
        "command",
        "--output-dir",
        &output_dir,
    ]);
    assert!(scaffold.status.success(), "{}", output_text(&scaffold));
    let scaffold_output = output_text(&scaffold);
    assert!(scaffold_output.contains("Pack scaffold created"));
    assert!(scaffold_output.contains("Test stub created"));
    assert!(scaffold_output.contains("Run `cargo test`"));

    let generated_pack_path = temp.path().join("kubectl-demo.json");
    let generated_test_path = temp.path().join("kubectl-demo_pack_tests.rs");
    let generated_pack = load_pack(&generated_pack_path).expect("scaffold must be loadable");
    assert_eq!(generated_pack.id, "kubectl-demo");
    assert!(!generated_pack.safe_patterns.is_empty());
    assert!(!generated_pack.guarded_patterns.is_empty());
    assert!(fs::read_to_string(generated_test_path)
        .unwrap()
        .contains("guarded_pattern_detects_dangerous_operations"));

    let fixture_pack_path = fixture("creating-rule-pack-new.json");
    let fixture_pack_arg = fixture_pack_path.to_string_lossy().into_owned();
    let safe = icg(&[
        "check",
        "--command",
        "kubectl get pods -n payments",
        "--pack",
        &fixture_pack_arg,
    ]);
    assert!(safe.status.success(), "{}", output_text(&safe));
    assert!(output_text(&safe).contains("ALLOW"));

    let dangerous = icg(&[
        "check",
        "--command",
        "kubectl delete pvc payments-data",
        "--pack",
        &fixture_pack_arg,
    ]);
    assert!(dangerous.status.success(), "{}", output_text(&dangerous));
    let dangerous_output = output_text(&dangerous);
    assert!(dangerous_output.contains("DENIED"));
    assert!(dangerous_output.contains("kubectl-delete-pvc"));
    assert!(dangerous_output.contains("Data cannot be recovered"));

    // The scaffold command is intentionally non-destructive: it refuses to
    // overwrite a pack created by an earlier step.
    let duplicate = icg(&[
        "new-pack",
        "kubectl-demo",
        "--pack-type",
        "command",
        "--output-dir",
        &output_dir,
    ]);
    assert!(!duplicate.status.success());
    let duplicate_output = output_text(&duplicate);
    assert!(duplicate_output.contains("overwrite") || duplicate_output.contains("exists"));
}

#[test]
fn scenario_7_regression_generation_verification_and_coverage_diff() {
    let baseline_path = fixture("testing-pattern-changes-baseline.json");
    let updated_path = fixture("testing-pattern-changes-updated.json");

    let baseline = pack_fixture("testing-pattern-changes-baseline.json");
    let updated = pack_fixture("testing-pattern-changes-updated.json");
    assert_eq!(baseline.id, "git-baseline");
    assert_eq!(updated.id, "git-updated");

    let baseline_suite = generate_regression_suite_from_manifest(&baseline_path).unwrap();
    assert_eq!(
        baseline_suite.cases.len(),
        baseline
            .guarded_patterns
            .iter()
            .filter(|pattern| pattern.enabled)
            .count()
    );
    assert!(baseline_suite
        .cases
        .iter()
        .all(|case| case.expected == ExpectedVerdict::Deny));
    verify_regression_suite(&baseline, &baseline_suite).unwrap();

    let updated_suite = generate_regression_suite_from_manifest(&updated_path).unwrap();
    verify_regression_suite(&updated, &updated_suite).unwrap();
    assert!(updated_suite
        .cases
        .iter()
        .any(|case| case.pattern_id == "git-force-push"));

    let output_path = tempdir().unwrap().path().join("git-regression.json");
    let baseline_arg = baseline_path.to_string_lossy().into_owned();
    let output_arg = output_path.to_string_lossy().into_owned();
    let generated = icg(&["regression-suite", &baseline_arg, "--output", &output_arg]);
    assert!(generated.status.success(), "{}", output_text(&generated));
    let generated_json: RegressionSuite = serde_json::from_str(
        &fs::read_to_string(&output_path).expect("regression output should exist"),
    )
    .expect("CLI output should use the regression-suite schema");
    assert_eq!(generated_json, baseline_suite);
    assert!(output_text(&generated).contains("Generated"));

    let diff = run_coverage_diff(baseline_path.clone(), updated_path.clone()).unwrap();
    assert_eq!(
        diff.removed_guarded_patterns,
        vec!["git-push-f-flag".to_string()]
    );
    assert!(diff.disabled_guarded_patterns.is_empty());

    let baseline_diff_arg = baseline_path.to_string_lossy().into_owned();
    let updated_diff_arg = updated_path.to_string_lossy().into_owned();
    let rejected_diff = icg(&["coverage-diff", &baseline_diff_arg, &updated_diff_arg]);
    assert!(!rejected_diff.status.success());
    assert!(output_text(&rejected_diff).contains("Coverage regressions detected"));
    let approved_diff = icg(&[
        "coverage-diff",
        &baseline_diff_arg,
        &updated_diff_arg,
        "--justification",
        "The short force flag is retired after the replacement rule was verified.",
    ]);
    assert!(
        approved_diff.status.success(),
        "{}",
        output_text(&approved_diff)
    );

    // Verification must fail when a developer deletes a case or changes its
    // fixed input so a narrowed rule cannot silently ship.
    let mut missing_case = baseline_suite.clone();
    missing_case.cases.pop();
    let missing_error = verify_regression_suite(&baseline, &missing_case).unwrap_err();
    assert!(missing_error.to_string().contains("cases"));

    let mut changed_input = baseline_suite;
    changed_input.cases[0].command = "git status --short".to_string();
    let changed_error = verify_regression_suite(&baseline, &changed_input).unwrap_err();
    assert!(changed_error.to_string().contains("does not match"));
}

#[test]
fn scenario_8_debug_trace_reproduce_fix_and_verify_false_positive() {
    let broad = pack_fixture("debugging-false-positives-overly-broad.json");
    let fixed = pack_fixture("debugging-false-positives-fixed.json");
    assert!(fixed
        .guarded_patterns
        .iter()
        .any(|pattern| pattern.id == "kubectl-delete-pvc"));

    let mut broad_engine = Engine::new();
    broad_engine.load_pack(broad).unwrap();
    for command in [
        "kubectl delete pod payments-7f8d",
        "kubectl delete deployment payments",
        "kubectl delete pvc payments-data",
    ] {
        assert!(matches!(
            broad_engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Denied { .. }
        ));
    }

    let fixed_path = fixture("debugging-false-positives-fixed.json");
    let fixed_arg = fixed_path.to_string_lossy().into_owned();
    for command in [
        "kubectl delete pod payments-7f8d",
        "kubectl delete deployment payments",
    ] {
        let allowed = icg(&["check", "--command", command, "--pack", &fixed_arg]);
        assert!(allowed.status.success(), "{}", output_text(&allowed));
        assert!(output_text(&allowed).contains("ALLOW"));
    }

    let debugged = icg(&[
        "check",
        "--command",
        "kubectl delete pvc payments-data",
        "--pack",
        &fixed_arg,
        "--debug",
    ]);
    assert!(debugged.status.success(), "{}", output_text(&debugged));
    let trace = output_text(&debugged);
    for marker in [
        "DEBUG: Pattern matching trace",
        "Pack dispatched: kubectl-fixed",
        "Safe patterns checked:",
        "Guarded patterns checked:",
        "kubectl-delete-pvc: MATCH",
        "Final verdict: DENY",
    ] {
        assert!(
            trace.contains(marker),
            "debug trace missing {marker:?}: {trace}"
        );
    }

    let explain = icg(&[
        "explain",
        "--pattern",
        "kubectl-delete-pvc",
        "--pack",
        &fixed_arg,
        "--show-regex",
    ]);
    assert!(explain.status.success(), "{}", output_text(&explain));
    let explanation = output_text(&explain);
    assert!(explanation.contains("kubectl-delete-pvc"));
    assert!(explanation.contains("kubectl delete pvc"));
    assert!(explanation.contains("persistent data"));

    // A malformed rule pack is an actionable error at the CLI boundary and
    // must not be mistaken for a clean debugging result.
    let corrupt = fixture("../../fixtures/corrupt-rule-pack.json");
    let corrupt_arg = corrupt.to_string_lossy().into_owned();
    let invalid = icg(&[
        "check",
        "--command",
        "kubectl delete pvc payments-data",
        "--pack",
        &corrupt_arg,
    ]);
    assert!(!invalid.status.success());
    assert!(output_text(&invalid).contains("failed to load rule pack"));
}

#[test]
fn scenario_9_custom_predicates_evaluate_shared_checkout_scope() {
    let pack = pack_fixture("adding-custom-predicates.json");
    let mut predicate_names = HashSet::new();
    for pattern in &pack.safe_patterns {
        if let Check::Predicate { predicate_name, .. } = &pattern.check {
            predicate_names.insert(predicate_name.as_str());
        }
    }
    for pattern in &pack.guarded_patterns {
        if let Check::Predicate { predicate_name, .. } = &pattern.check {
            predicate_names.insert(predicate_name.as_str());
        }
    }
    assert!(predicate_names.contains("is_shared_checkout"));
    assert!(predicate_names.contains("is_worktree"));
    assert!(predicate_names.contains("has_uncommitted_changes"));

    let mut engine = Engine::new();
    engine.load_pack(pack).unwrap();

    let shared_checkout_write = ContentSource::Write {
        file_path: ".beads/checkpoint/developer-scenario.json".to_string(),
        content: "{\"case\":\"realistic\"}\n".to_string(),
    };
    assert!(matches!(
        engine.evaluate_content(&shared_checkout_write),
        CheckResult::Denied {
            ref pack_id,
            ref pattern_id,
            ..
        } if pack_id == "beads-predicates" && pattern_id == "beads-shared-checkout-write"
    ));

    let unrelated_write = ContentSource::Write {
        file_path: "docs/developer-notes.md".to_string(),
        content: "The checkout is healthy.\n".to_string(),
    };
    assert_eq!(
        engine.evaluate_content(&unrelated_write),
        CheckResult::Allowed
    );

    let pack_arg = fixture("adding-custom-predicates.json")
        .to_string_lossy()
        .into_owned();
    let request = r#"{"toolName":"Write","toolInput":{"filePath":".beads/checkpoint/developer-scenario.json","content":"{}"}}"#;
    let denied = icg_with_stdin(&["check", "--stdin", "--pack", &pack_arg], request);
    assert!(denied.status.success(), "{}", output_text(&denied));
    assert!(output_text(&denied).contains("beads-shared-checkout-write"));

    let safe_request = r#"{"toolName":"Write","toolInput":{"filePath":"docs/developer-notes.md","content":"safe"}}"#;
    let allowed = icg_with_stdin(&["check", "--stdin", "--pack", &pack_arg], safe_request);
    assert!(allowed.status.success(), "{}", output_text(&allowed));
    assert!(output_text(&allowed).contains("ALLOW"));
}

#[test]
fn developer_rule_pack_fixtures_validate_and_readme_commands_are_executable() {
    let pack_names = [
        "creating-rule-pack-new.json",
        "testing-pattern-changes-baseline.json",
        "testing-pattern-changes-updated.json",
        "debugging-false-positives-overly-broad.json",
        "debugging-false-positives-fixed.json",
        "adding-custom-predicates.json",
    ];

    for name in pack_names {
        let pack = pack_fixture(name);
        let mut ids: HashSet<String> = HashSet::new();
        let mut validate_pattern = |id: &str, check: &Check| {
            assert!(ids.insert(id.to_string()), "duplicate pattern id in {name}");
            match check {
                Check::CommandRegex { regex } | Check::ContentRegex { regex } => {
                    Regex::new(regex)
                        .unwrap_or_else(|error| panic!("invalid regex in {name}/{id}: {error}"));
                }
                Check::Predicate { predicate_name, .. } => {
                    assert!(!predicate_name.trim().is_empty());
                }
            }
        };
        for pattern in &pack.safe_patterns {
            validate_pattern(&pattern.id, &pattern.check);
        }
        for pattern in &pack.guarded_patterns {
            validate_pattern(&pattern.id, &pattern.check);
        }
        assert!(
            !pack.guarded_patterns.is_empty(),
            "{name} needs guarded coverage"
        );
    }

    let corrupt = fixture("../../fixtures/corrupt-rule-pack.json");
    assert!(load_pack(corrupt).is_err());

    let baseline_snapshot: Value = serde_json::from_str(
        &fs::read_to_string(fixture("regression-suite-baseline.json")).unwrap(),
    )
    .unwrap();
    let updated_snapshot: Value = serde_json::from_str(
        &fs::read_to_string(fixture("regression-suite-updated.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(baseline_snapshot["packId"], "git");
    assert_eq!(updated_snapshot["packId"], "git");
    assert!(baseline_snapshot["cases"].as_array().unwrap().len() >= 4);
    assert!(updated_snapshot["cases"].as_array().unwrap().len() >= 4);

    let readme =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/examples/README.md"))
            .unwrap();
    for marker in [
        "### Scenario 6: Creating a New Rule Pack",
        "### Scenario 7: Testing Pattern Changes",
        "### Scenario 8: Debugging False Positives",
        "### Scenario 9: Adding Custom Predicates",
        "new-pack",
        "regression-suite",
        "coverage-diff",
        "--debug",
        "is_shared_checkout",
    ] {
        assert!(
            readme.contains(marker),
            "README missing developer marker {marker:?}"
        );
    }
}
