//! The fixed regression suite for the two hook-front-end content guards.
//!
//! The `.github/workflows` write denial and the `kind: Job`/`CronJob`
//! content denial are built into the engine's content path
//! (`evaluate_content_inner`) rather than loaded from a pack, so the
//! pack-derived suites (`icg regression-suite`, `--release-gate`) cannot
//! see them: their generator is regex-only over pack manifests and records
//! predicates as reasoned skips. The feature suites
//! (`github_workflows_hook_integration_tests.rs`,
//! `job_cronjob_hook_integration_tests.rs`) drive the modules' full fixture
//! tables; wide tables track the predicates' behavior. Neither gives the
//! denials a small, permanent home that fails if they stop firing.
//!
//! This suite is that home. The cases are data
//! (`tests/fixtures/hook-predicate-regression-suite.json`, in the
//! `RegressionSuite` schema the repo already publishes) and are checked
//! through both layers a later change could break:
//!
//! 1. the engine loaded with the **real shipped `packs/` directory** — the
//!    production configuration, so a content-pack change or a reordering of
//!    `evaluate_content_inner` that lets a pack answer before a guard
//!    fails here, not just in the empty-pack feature tests; and
//! 2. the compiled `icg hook` binary reading that same directory — the full
//!    hook pipeline, so a front-end change (stdin parsing, tool-input
//!    normalization, response rendering) that drops the denial fails too.
//!
//! Every check is a plain assertion: there is no `#[ignore]`, no
//! conditional skip, and a missing or emptied fixture is a load failure,
//! so either denial going quiet fails the suite loudly.

use icg::engine::{CheckResult, ContentSource, Engine};
use icg::regression::{ExpectedVerdict, RegressionSuite};
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::tempdir;

/// The fixed suite artifact. Data, not code: the cases survive refactors of
/// the predicate modules and are pinned in the suite's own published schema.
const SUITE_PATH: &str = "tests/fixtures/hook-predicate-regression-suite.json";

/// The schema version the artifact was written against. Bumping
/// `regression`'s internal `SUITE_VERSION` must consciously migrate this
/// file, not re-read it silently.
const SUITE_VERSION: u32 = 1;

/// Pack/pattern attribution pinned as literals, per the rule the
/// `job-cronjob-yaml` module states: an accidental rename must fail a test
/// instead of silently breaking the denial's downstream consumers.
/// (`github-workflows` has no exported constants — its attribution is
/// emitted inline by the engine — so the literals are the only pin for it.)
const GITHUB_WORKFLOWS_PACK: &str = "github-workflows";
const GITHUB_WORKFLOWS_PATTERN: &str = "github-workflows-protected";
const JOB_CRONJOB_PACK: &str = "job-cronjob-yaml";
const JOB_CRONJOB_PATTERN: &str = "kind-job-cronjob";

/// The shipped pack directory, i.e. the production configuration the hook
/// runs with in a checkout.
fn packs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("packs")
}

fn load_fixed_suite() -> RegressionSuite {
    let raw = std::fs::read_to_string(SUITE_PATH)
        .unwrap_or_else(|error| panic!("fixed regression suite {SUITE_PATH} must exist: {error}"));
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("fixed regression suite {SUITE_PATH} must parse: {error}"))
}

/// The suite must keep covering both rule families with their pinned
/// attribution, every case must be a content-mode deny case, and nothing
/// outside the two families may sneak in. Deleting a family's cases — the
/// "silent removal" this suite exists to prevent — fails right here.
#[test]
fn fixed_suite_covers_both_hook_predicate_families() {
    let suite = load_fixed_suite();
    assert_eq!(
        suite.version, SUITE_VERSION,
        "fixed suite schema version drifted; migrate {SUITE_PATH} deliberately"
    );
    assert!(
        !suite.cases.is_empty(),
        "fixed regression suite must not be emptied"
    );

    let mut workflows_cases = 0;
    let mut job_cronjob_cases = 0;
    for case in &suite.cases {
        assert_eq!(
            case.expected,
            ExpectedVerdict::Deny,
            "every fixed case is a deny case; '{}' drifted",
            case.pattern_id
        );
        assert!(
            case.command.is_empty(),
            "hook-predicate cases are content-mode; '{}' carries a command",
            case.pattern_id
        );
        assert!(
            case.file_path.is_some() && case.content.is_some(),
            "fixed case '{}' must carry both file_path and content",
            case.pattern_id
        );

        match (case.pack_id.as_str(), case.pattern_id.as_str()) {
            (GITHUB_WORKFLOWS_PACK, GITHUB_WORKFLOWS_PATTERN) => workflows_cases += 1,
            (JOB_CRONJOB_PACK, JOB_CRONJOB_PATTERN) => job_cronjob_cases += 1,
            (pack_id, pattern_id) => panic!(
                "fixed suite case ({pack_id}, {pattern_id}) is outside the two \
                 hook-predicate families this suite owns"
            ),
        }
    }
    assert!(
        workflows_cases > 0,
        "the .github/workflows denial lost its fixed regression case"
    );
    assert!(
        job_cronjob_cases > 0,
        "the kind: Job/CronJob denial lost its fixed regression case"
    );
}

/// The exported job-cronjob attribution constants must still equal the
/// literals the suite pins. The workflows attribution has no constants to
/// compare against; its literals are exercised end-to-end below.
#[test]
fn exported_attribution_constants_match_the_pinned_literals() {
    use icg::job_cronjob_yaml::{PACK_ID, PATTERN_ID};
    assert_eq!(PACK_ID, JOB_CRONJOB_PACK, "job-cronjob pack id was renamed");
    assert_eq!(
        PATTERN_ID, JOB_CRONJOB_PATTERN,
        "job-cronjob pattern id was renamed"
    );
}

/// Every fixed case must still deny through the engine loaded with the real
/// shipped packs — the production configuration, where a guard answers
/// before any pack is consulted. A refactor that reorders that or a pack
/// change that shadows the denial fails here with the offending case named.
#[test]
fn every_fixed_case_denies_through_the_production_engine() {
    let suite = load_fixed_suite();
    let mut engine = Engine::new();
    engine
        .load_packs_from_dir(packs_dir())
        .expect("shipped packs/ directory should load");

    for case in suite.cases {
        let file_path = case
            .file_path
            .clone()
            .expect("structural test pins content-mode cases");
        let content = case
            .content
            .clone()
            .expect("structural test pins content-mode cases");
        let result = engine.evaluate_content(&ContentSource::Write { file_path, content });
        assert!(
            matches!(
                &result,
                CheckResult::Denied { pack_id, pattern_id, .. }
                    if pack_id == &case.pack_id && pattern_id == &case.pattern_id
            ),
            "fixed regression case '{}' ({}) no longer denies through the \
             production engine: {result:?}",
            case.pattern_id,
            case.pack_id
        );
    }
}

/// Spawn `icg hook` against the real shipped packs directory, exactly as a
/// harness would, and return the response JSON. Support-file paths are
/// redirected into a temporary directory so the run leaves no local state
/// behind (the same discipline as `hook_pack_directory_tests`).
fn run_hook(tool_name: &str, tool_input: Value) -> Value {
    let support = tempdir().expect("hook support directory should exist");
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["hook", "--rule-pack"])
        .arg(packs_dir())
        .env("ICG_HEALTH_PATH", support.path().join("health.json"))
        .env("ICG_TELEMETRY_PATH", support.path().join("telemetry.json"))
        .env("ICG_DENIAL_LOG", support.path().join("denials.jsonl"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");

    let payload = json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
    });
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(payload.to_string().as_bytes())
        .expect("hook input should be written");

    let output = child
        .wait_with_output()
        .expect("hook process should finish");
    assert!(
        output.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("hook stdout should be one JSON object")
}

/// Every fixed case must still deny through the full hook pipeline reading
/// the real shipped packs: a front-end change (stdin parsing, Write-input
/// normalization, response rendering) that drops either denial fails here.
#[test]
fn every_fixed_case_denies_through_the_hook_pipeline() {
    let suite = load_fixed_suite();
    for case in suite.cases {
        let file_path = case
            .file_path
            .clone()
            .expect("structural test pins content-mode cases");
        let content = case
            .content
            .clone()
            .expect("structural test pins content-mode cases");
        let response = run_hook(
            "Write",
            json!({
                "filePath": file_path,
                "content": content,
            }),
        );

        let hook_output = &response["hookSpecificOutput"];
        assert_eq!(
            hook_output["permissionDecision"], "deny",
            "fixed regression case '{}' ({}) no longer denies through the hook \
             pipeline: {response:?}",
            case.pattern_id, case.pack_id
        );
        let reason = hook_output["permissionDecisionReason"]
            .as_str()
            .unwrap_or_else(|| {
                panic!(
                    "deny for fixed case '{}' must carry a reason string",
                    case.pattern_id
                )
            });
        assert!(
            reason.contains(&format!("pack={}", case.pack_id))
                && reason.contains(&format!("pattern={}", case.pattern_id)),
            "deny for fixed case '{}' must keep its pack/pattern attribution, \
             got: {reason}",
            case.pattern_id
        );
    }
}
