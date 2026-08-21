//! End-to-end tests for the integration workflows in `docs/examples/README.md`.
//!
//! The JSON files beside this test are intentionally the scenario inputs and
//! expected outcomes. Keeping the probes in fixtures makes it possible to
//! review the documented workflow independently from the Rust harness.

use chrono::NaiveDate;
use icg::engine::{CheckResult, ContentSource, Engine, InputSource};
use icg::overrides::{
    load_verified_override, save_override, validate_override_at, OverrideFreshness,
};
use icg::rule_pack::{load_pack, Pack};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn fixture_path(name: &str) -> PathBuf {
    Path::new(ROOT)
        .join("tests/fixtures/integration-scenarios")
        .join(name)
}

fn read_fixture<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = fixture_path(name);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|error| panic!("failed to parse fixture {}: {error}", path.display()))
}

fn pack_path(relative: &str) -> PathBuf {
    Path::new(ROOT).join(relative)
}

fn full_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_packs_from_dir(pack_path("packs"))
        .expect("all checked-in rule packs should load");
    engine
}

fn source_from_payload(tool_name: &str, tool_input: Value) -> InputSource {
    let payload = json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
    });
    let input = Engine::parse_and_validate_pre_tool_use(&payload.to_string())
        .unwrap_or_else(|error| panic!("fixture payload should parse: {error}"));
    Engine::input_source_from_pre_tool_use(input)
        .expect("validated payload should convert")
        .expect("fixture payload should identify a checkable tool")
}

fn evaluate_source(engine: &Engine, source: InputSource) -> CheckResult {
    match source {
        InputSource::Command(source) => engine.evaluate_command(&source),
        InputSource::Content(source) => engine.evaluate_content(&source),
        InputSource::ContentBatch(sources) => engine.evaluate_content_batch(&sources),
    }
}

fn assert_result(
    result: &CheckResult,
    should_deny: bool,
    expected_pack: Option<&str>,
    expected_pattern: Option<&str>,
) {
    match (should_deny, result) {
        (
            true,
            CheckResult::Denied {
                pack_id,
                pattern_id,
                ..
            },
        ) => {
            if let Some(expected) = expected_pack {
                assert_eq!(pack_id, expected);
            }
            if let Some(expected) = expected_pattern {
                assert_eq!(pattern_id, expected);
            }
        }
        (true, other) => panic!("expected denial, got {other:?}"),
        (false, CheckResult::Allowed) => {}
        (false, other) => panic!("expected allow, got {other:?}"),
    }
}

fn owned_args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

fn run_icg(args: &[String], input: Option<&str>, telemetry_path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = telemetry_path {
        command.env("ICG_TELEMETRY_PATH", path);
    }

    if let Some(input) = input {
        command.stdin(Stdio::piped());
        let mut child = command.spawn().expect("icg should start");
        child
            .stdin
            .take()
            .expect("icg stdin should be available")
            .write_all(input.as_bytes())
            .expect("fixture input should be written");
        child.wait_with_output().expect("icg should finish")
    } else {
        command.output().expect("icg should start")
    }
}

#[derive(Debug, Deserialize)]
struct MigrationFixture {
    scenario: u8,
    title: String,
    org_guard: String,
    overlap_probes: Vec<MigrationProbe>,
    org_guard_only_probes: Vec<MigrationProbe>,
    icg_only_probes: Vec<MigrationProbe>,
    migration_phases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MigrationProbe {
    name: String,
    tool_name: String,
    tool_input: Value,
    org_guard_denies: bool,
    icg_denies: bool,
    #[serde(default)]
    icg_pack: Option<String>,
    #[serde(default)]
    icg_pattern: Option<String>,
}

/// Return the checked-in org hook when this checkout is running in the host
/// environment. CI remains deterministic because the reference inventory in
/// the fixture is always tested even when the host hook is unavailable.
fn org_guard_script() -> Option<PathBuf> {
    let path = std::env::var_os("ORG_RULE_GUARD_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/coding/.claude/hooks/org-rule-guard.py"));
    if !path.is_file() {
        return None;
    }
    let python = Command::new("python3").arg("--version").output();
    python
        .ok()
        .filter(|output| output.status.success())
        .map(|_| path)
}

fn run_org_guard(path: &Path, tool_name: &str, tool_input: &Value) -> bool {
    let mut child = Command::new("python3")
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("configured org-rule-guard.py should start");
    let payload = json!({"tool_name": tool_name, "tool_input": tool_input});
    child
        .stdin
        .take()
        .expect("org hook stdin should be available")
        .write_all(payload.to_string().as_bytes())
        .expect("org hook input should be written");
    let output = child.wait_with_output().expect("org hook should finish");
    assert!(
        output.status.success(),
        "org hook failed: {:?}",
        output.status
    );
    serde_json::from_slice::<Value>(&output.stdout)
        .ok()
        .map(|response| response["hookSpecificOutput"]["permissionDecision"] == "deny")
        .unwrap_or(false)
}

fn assert_migration_probe(engine: &Engine, probe: &MigrationProbe, live_org_guard: Option<&Path>) {
    let result = evaluate_source(
        engine,
        source_from_payload(&probe.tool_name, probe.tool_input.clone()),
    );
    assert_result(
        &result,
        probe.icg_denies,
        probe.icg_pack.as_deref(),
        probe.icg_pattern.as_deref(),
    );

    if let Some(path) = live_org_guard {
        assert_eq!(
            run_org_guard(path, &probe.tool_name, &probe.tool_input),
            probe.org_guard_denies,
            "live {} result differed for {}",
            path.display(),
            probe.name
        );
    }
}

#[test]
fn scenario_10_compares_org_guard_overlap_and_coverage_gaps() {
    let fixture: MigrationFixture = read_fixture("scenario-10-migration.json");
    assert_eq!(fixture.scenario, 10);
    assert_eq!(fixture.title, "Migrating from org-rule-guard.py");
    assert_eq!(fixture.org_guard, "org-rule-guard.py");
    assert_eq!(
        fixture.migration_phases,
        [
            "coexistence",
            "migrate-overlapping-rules",
            "retain-org-only-rules"
        ]
    );

    let engine = full_engine();
    let live_org_guard = org_guard_script();

    // The overlap is deliberately checked for both deny and allow outcomes:
    // migration must not create a false positive for a pinned image.
    for probe in &fixture.overlap_probes {
        assert_migration_probe(&engine, probe, live_org_guard.as_deref());
    }
    for probe in &fixture.org_guard_only_probes {
        assert_migration_probe(&engine, probe, live_org_guard.as_deref());
    }
    for probe in &fixture.icg_only_probes {
        assert_migration_probe(&engine, probe, live_org_guard.as_deref());
    }
}

#[derive(Debug, Deserialize)]
struct HarnessFixture {
    scenario: u8,
    title: String,
    harnesses: Vec<HarnessProbe>,
    format_differences: Vec<FormatDifference>,
    shared_response: SharedResponse,
}

#[derive(Debug, Deserialize)]
struct HarnessProbe {
    name: String,
    wire_format: String,
    pack: String,
    payload: Value,
    source: String,
    expected_decision: String,
    expected_pack: String,
    expected_pattern: String,
}

#[derive(Debug, Deserialize)]
struct FormatDifference {
    harness: String,
    tool_name_key: String,
    tool_input_key: String,
    file_path_key: String,
}

#[derive(Debug, Deserialize)]
struct SharedResponse {
    root_key: String,
    decision_key: String,
    deny_value: String,
}

fn assert_native_hook_decision(probe: &HarnessProbe, pack: &Path, telemetry_path: &Path) {
    let args = vec![
        "hook".to_string(),
        "--rule-pack".to_string(),
        pack.to_string_lossy().into_owned(),
    ];
    let output = run_icg(
        &args,
        Some(&probe.payload.to_string()),
        Some(telemetry_path),
    );
    assert!(
        output.status.success(),
        "{} hook failed: {}",
        probe.name,
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{} hook response was not JSON: {error}", probe.name));
    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"],
        probe.expected_decision
    );
    assert!(response["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default()
        .contains(&probe.expected_pack));
}

#[test]
fn scenario_11_parses_and_runs_both_harness_wire_formats() {
    let fixture: HarnessFixture = read_fixture("scenario-11-multi-harness.json");
    assert_eq!(fixture.scenario, 11);
    assert_eq!(fixture.title, "Setting up Multi-Harness Support");
    assert_eq!(fixture.harnesses.len(), 4);
    assert_eq!(fixture.shared_response.root_key, "hookSpecificOutput");
    assert_eq!(fixture.shared_response.decision_key, "permissionDecision");
    assert_eq!(fixture.shared_response.deny_value, "deny");

    let claude = fixture
        .format_differences
        .iter()
        .find(|format| format.harness == "claude-code")
        .expect("Claude format should be documented");
    assert_eq!(claude.tool_name_key, "toolName");
    assert_eq!(claude.tool_input_key, "toolInput");
    assert_eq!(claude.file_path_key, "filePath");
    let codex = fixture
        .format_differences
        .iter()
        .find(|format| format.harness == "codex-cli")
        .expect("Codex format should be documented");
    assert_eq!(codex.tool_name_key, "tool_name");
    assert_eq!(codex.tool_input_key, "tool_input");
    assert_eq!(codex.file_path_key, "file_path");

    let telemetry = tempdir().expect("telemetry directory should exist");
    for (index, probe) in fixture.harnesses.iter().enumerate() {
        assert_eq!(
            probe.wire_format,
            if probe.name == "claude-code" {
                "camelCase"
            } else {
                "snake_case"
            },
            "fixture should identify the wire format for {}",
            probe.name
        );
        let raw = probe.payload.to_string();
        let parsed = Engine::parse_and_validate_pre_tool_use(&raw)
            .unwrap_or_else(|error| panic!("{} payload should parse: {error}", probe.name));
        let source = Engine::input_source_from_pre_tool_use(parsed)
            .unwrap_or_else(|error| panic!("{} payload should normalize: {error}", probe.name))
            .expect("scenario payload should be checkable");
        assert_eq!(
            match &source {
                InputSource::Command(_) => "command",
                InputSource::Content(_) | InputSource::ContentBatch(_) => "content",
            },
            probe.source,
            "{} source kind",
            probe.name
        );

        let pack = pack_path(&probe.pack);
        let mut args = owned_args(&[
            "check",
            "--stdin",
            "--harness",
            &probe.name,
            "--debug",
            "--pack",
        ]);
        args.push(pack.to_string_lossy().into_owned());
        let output = run_icg(
            &args,
            Some(&raw),
            Some(&telemetry.path().join(format!("check-{index}.json"))),
        );
        assert!(
            output.status.success(),
            "{} check failed: {}",
            probe.name,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("DENIED"), "{} should be denied", probe.name);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(&format!("Harness: {}", probe.name)),
            "--harness should be visible in debug output"
        );

        // Use the native response path as well. This verifies that both
        // harnesses receive the same machine-readable deny envelope.
        assert_native_hook_decision(
            probe,
            &pack,
            &telemetry.path().join(format!("hook-{index}.json")),
        );

        let expected = match source {
            InputSource::Command(source) => full_engine().evaluate_command(&source),
            InputSource::Content(source) => full_engine().evaluate_content(&source),
            InputSource::ContentBatch(sources) => full_engine().evaluate_content_batch(&sources),
        };
        assert_result(
            &expected,
            probe.expected_decision == "deny",
            Some(&probe.expected_pack),
            Some(&probe.expected_pattern),
        );
    }
}

#[derive(Debug, Deserialize)]
struct OverrideFixture {
    scenario: u8,
    title: String,
    repository_path: String,
    repository_id: String,
    pattern_id: String,
    pack: String,
    release_ref: String,
    expiration: String,
    expired_on: String,
    justification: String,
    approver: String,
    probe_file: String,
    probe_content: String,
    safe_content: String,
    workflow: Vec<String>,
}

fn output_text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn scenario_12_runs_override_request_approval_verification_and_expiry() {
    let fixture: OverrideFixture = read_fixture("scenario-12-repository-overrides.json");
    assert_eq!(fixture.scenario, 12);
    assert_eq!(fixture.title, "Configuring Repository Overrides");
    assert_eq!(
        fixture.workflow,
        ["identify", "request", "approve", "verify", "monitor", "expire"]
    );

    let temporary = tempdir().expect("scenario temporary directory should exist");
    let request_path = temporary.path().join("override-request.json");
    let overrides_dir = temporary.path().join("overrides");
    let artifact_path = overrides_dir.join(format!("{}.toml", fixture.repository_id));
    let pack = pack_path(&fixture.pack);

    // Scenario 12, Step 2: create the review request without enabling an
    // exemption. The request intentionally retains the absolute user-facing
    // repository path; approval normalizes it to the release artifact scope.
    let create_args = vec![
        "override".to_string(),
        "create".to_string(),
        "--repo".to_string(),
        fixture.repository_path.clone(),
        "--pattern-id".to_string(),
        fixture.pattern_id.clone(),
        "--justification".to_string(),
        fixture.justification.clone(),
        "--output".to_string(),
        request_path.to_string_lossy().into_owned(),
    ];
    let created = run_icg(&create_args, None, None);
    assert!(
        created.status.success(),
        "create failed: {}",
        output_text(&created)
    );
    let request: Value = serde_json::from_slice(
        &fs::read(&request_path).expect("create should write a request artifact"),
    )
    .expect("request artifact should be JSON");
    assert_eq!(request["schema"], "icg-override-request/v1");
    assert_eq!(request["repo"], fixture.repository_path);
    assert_eq!(request["patternId"], fixture.pattern_id);
    assert_eq!(request["justification"], fixture.justification);

    // Scenario 12, Step 4: approval creates a release-bound TOML artifact.
    // Supplying --output keeps the test in the exact overrides/<repo>.toml
    // layout required by verified loading.
    let approve_args = vec![
        "override".to_string(),
        "approve".to_string(),
        "--request".to_string(),
        request_path.to_string_lossy().into_owned(),
        "--approver".to_string(),
        fixture.approver.clone(),
        "--expiration".to_string(),
        fixture.expiration.clone(),
        "--release-ref".to_string(),
        fixture.release_ref.clone(),
        "--pack".to_string(),
        pack.to_string_lossy().into_owned(),
        "--output".to_string(),
        artifact_path.to_string_lossy().into_owned(),
    ];
    let approved = run_icg(&approve_args, None, None);
    assert!(
        approved.status.success(),
        "approve failed: {}",
        output_text(&approved)
    );
    assert!(output_text(&approved).contains("Override approved and installed"));
    assert!(
        artifact_path.is_file(),
        "approval should install TOML artifact"
    );

    let loaded_pack: Pack = load_pack(&pack).expect("image-tag pack should load");
    let manifest = load_verified_override(
        &artifact_path,
        &fixture.repository_id,
        &fixture.release_ref,
        std::slice::from_ref(&loaded_pack),
    )
    .expect("approved artifact should be verified for its release and repository");
    assert_eq!(manifest.repository, fixture.repository_id);
    assert_eq!(manifest.exempted_rule_ids, [fixture.pattern_id.clone()]);
    assert_eq!(manifest.release_ref, fixture.release_ref);
    assert_eq!(manifest.expires_at, fixture.expiration);
    assert!(
        load_verified_override(
            &artifact_path,
            "other-app",
            &fixture.release_ref,
            std::slice::from_ref(&loaded_pack),
        )
        .is_err(),
        "override must not cross repository boundaries"
    );
    assert!(
        load_verified_override(
            &artifact_path,
            &fixture.repository_id,
            "v0.1.0-different-release",
            std::slice::from_ref(&loaded_pack),
        )
        .is_err(),
        "override must be bound to the trusted release"
    );

    // Scenario 12, Step 5: the same dangerous content remains denied without
    // the repository-bound artifact and is allowed only after verification.
    let mut baseline = Engine::new();
    baseline
        .load_pack(loaded_pack.clone())
        .expect("image-tag pack should load into baseline engine");
    let content_source = ContentSource::Write {
        file_path: fixture.probe_file.clone(),
        content: fixture.probe_content.clone(),
    };
    assert_result(
        &baseline.evaluate_content(&content_source),
        true,
        Some("image-tag"),
        Some(&fixture.pattern_id),
    );

    let mut overridden = Engine::new();
    overridden
        .load_pack(loaded_pack.clone())
        .expect("image-tag pack should load into override engine");
    overridden
        .load_verified_override(&manifest, &fixture.repository_id, &fixture.release_ref)
        .expect("verified manifest should enable its exemption");
    assert_eq!(
        overridden.evaluate_content(&content_source),
        CheckResult::Allowed
    );
    assert_eq!(
        overridden.evaluate_content(&ContentSource::Write {
            file_path: fixture.probe_file.clone(),
            content: fixture.safe_content.clone(),
        }),
        CheckResult::Allowed
    );

    let telemetry = temporary.path().join("telemetry.json");
    let hook_input = json!({
        "toolName": "Write",
        "toolInput": {
            "filePath": fixture.probe_file,
            "content": fixture.probe_content
        }
    })
    .to_string();
    let hook_args = vec![
        "hook".to_string(),
        "--rule-pack".to_string(),
        pack.to_string_lossy().into_owned(),
        "--override-file".to_string(),
        artifact_path.to_string_lossy().into_owned(),
        "--repository".to_string(),
        fixture.repository_id.clone(),
        "--trusted-ref".to_string(),
        fixture.release_ref.clone(),
    ];
    let hook_allowed = run_icg(&hook_args, Some(&hook_input), Some(&telemetry));
    assert!(
        hook_allowed.status.success(),
        "verified hook failed: {}",
        output_text(&hook_allowed)
    );
    let response: Value = serde_json::from_slice(&hook_allowed.stdout).expect("hook response JSON");
    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );

    let outside_args = vec![
        "hook".to_string(),
        "--rule-pack".to_string(),
        pack.to_string_lossy().into_owned(),
    ];
    let hook_denied = run_icg(&outside_args, Some(&hook_input), Some(&telemetry));
    assert!(
        hook_denied.status.success(),
        "unscoped hook should return a decision"
    );
    let response: Value = serde_json::from_slice(&hook_denied.stdout).expect("hook response JSON");
    assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");

    // Scenario 12, Step 6: list active artifacts, then include an expired
    // artifact for the review report.
    let list_args = vec![
        "override".to_string(),
        "list".to_string(),
        "--directory".to_string(),
        overrides_dir.to_string_lossy().into_owned(),
    ];
    let listed = run_icg(&list_args, None, None);
    assert!(
        listed.status.success(),
        "list failed: {}",
        output_text(&listed)
    );
    let listed_text = output_text(&listed);
    assert!(listed_text.contains(&fixture.repository_id));
    assert!(listed_text.contains(&fixture.expiration));
    assert!(listed_text.contains("Fresh"));

    let mut expired = manifest.clone();
    expired.repository = "expired-app".to_string();
    expired.expires_at = fixture.expired_on.clone();
    save_override(&expired, overrides_dir.join("expired-app.toml"))
        .expect("expired fixture should be writable");
    let include_expired_args = vec![
        "override".to_string(),
        "list".to_string(),
        "--directory".to_string(),
        overrides_dir.to_string_lossy().into_owned(),
        "--include-expired".to_string(),
    ];
    let listed_all = run_icg(&include_expired_args, None, None);
    assert!(listed_all.status.success());
    let listed_all_text = output_text(&listed_all);
    assert!(listed_all_text.contains("expired-app"));
    assert!(listed_all_text.contains("Expired"));

    // Expiration is enforced by validation, not merely displayed by list.
    let today = NaiveDate::from_ymd_opt(2026, 8, 20).expect("fixture date should be valid");
    assert_eq!(
        expired.freshness_at(today).expect("expiry should parse"),
        OverrideFreshness::Expired
    );
    assert!(validate_override_at(
        &expired,
        "expired-app",
        &fixture.release_ref,
        std::slice::from_ref(&loaded_pack),
        today,
    )
    .is_err());
}

#[test]
fn scenario_fixtures_cover_all_three_documented_scenarios() {
    let files = [
        "scenario-10-migration.json",
        "scenario-11-multi-harness.json",
        "scenario-12-repository-overrides.json",
    ];
    let scenarios = files
        .iter()
        .map(|file| {
            let value: Value = read_fixture(file);
            value["scenario"].as_u64().expect("scenario number")
        })
        .collect::<Vec<_>>();
    assert_eq!(scenarios, [10, 11, 12]);
}
