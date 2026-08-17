//! Fixture-backed integration coverage for the five operator scenarios in
//! `docs/examples/README.md`.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/operator-scenarios"
);

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

fn fixture_json(name: &str) -> Value {
    serde_json::from_str(
        &fs::read_to_string(fixture(name)).expect("operator fixture should be readable"),
    )
    .expect("operator fixture should be valid JSON")
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

fn run_with_env(args: &[&str], envs: &[(&str, &Path)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("icg should run")
}

fn run_with_text_env(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("icg should run")
}

fn run_stdin(args: &[&str], input: &str, envs: &[(&str, &Path)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("icg should run");
    let mut stdin = child.stdin.take().expect("stdin should be available");
    std::io::Write::write_all(&mut stdin, input.as_bytes())
        .expect("fixture input should be written");
    drop(stdin);
    child.wait_with_output().expect("icg should finish")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn first_time_installation_validates_documented_commands_and_outputs() {
    let expected = fixture_json("installation.json");
    let packs = fixture("installation-packs");
    let temp = tempdir().expect("installation temp directory should be created");
    let hook_config = temp.path().join("settings.json");
    fs::write(
        &hook_config,
        r#"{"hooks":{"PreToolUse":{"command":"icg"}}}"#,
    )
    .expect("hook fixture should be written");

    let version = run(&["--version"]);
    assert!(version.status.success(), "{}", stderr(&version));
    assert_eq!(stdout(&version).trim(), "icg 0.1.0");

    let pack_check = run_with_env(&["health", "--check-packs"], &[("ICG_PACK_DIR", &packs)]);
    assert!(pack_check.status.success(), "{}", stderr(&pack_check));
    assert!(stdout(&pack_check).contains(expected["health"]["packs"].as_str().unwrap()));

    let hook_check = run_with_env(
        &["health", "--check-hooks"],
        &[("ICG_PACK_DIR", &packs), ("ICG_HOOK_CONFIG", &hook_config)],
    );
    assert!(hook_check.status.success(), "{}", stderr(&hook_check));
    assert!(stdout(&hook_check).contains(expected["health"]["hooks"].as_str().unwrap()));

    let dangerous_input =
        r#"{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}"#;
    let denied = run_stdin(
        &[
            "check",
            "--stdin",
            "--harness",
            "claude-code",
            "--pack",
            fixture("installation-packs/vault.json").to_str().unwrap(),
        ],
        dangerous_input,
        &[],
    );
    assert!(denied.status.success(), "{}", stderr(&denied));
    let denied_text = stdout(&denied);
    assert!(denied_text.contains("DENIED by icg"));
    assert!(denied_text.contains("Pack: vault"));
    assert!(denied_text.contains("Pattern: vault-kv-destroy"));
    assert!(denied_text.contains("Severity: Critical"));
    assert!(
        denied_text.contains("Explanation: This operation would permanently destroy secret data")
    );

    let safe_input = r#"{"toolName":"Bash","toolInput":{"command":"vault kv get secret/test"}}"#;
    let allowed = run_stdin(
        &[
            "check",
            "--stdin",
            "--harness",
            "claude-code",
            "--pack",
            fixture("installation-packs/vault.json").to_str().unwrap(),
        ],
        safe_input,
        &[],
    );
    assert!(allowed.status.success(), "{}", stderr(&allowed));
    assert_eq!(stdout(&allowed).trim(), "ALLOW: no configured rule matched");

    let verbose = run_with_text_env(
        &["health", "--verbose"],
        &[
            ("ICG_PACK_DIR", packs.to_str().unwrap()),
            ("ICG_HOOK_CONFIG", hook_config.to_str().unwrap()),
        ],
    );
    assert!(verbose.status.success(), "{}", stderr(&verbose));
    for expected_line in expected["health"]["verbose"].as_array().unwrap() {
        assert!(
            stdout(&verbose).contains(expected_line.as_str().unwrap()),
            "missing documented health line: {}",
            expected_line
        );
    }

    let bad_hooks = run_with_text_env(
        &["health", "--check-hooks"],
        &[
            ("ICG_PACK_DIR", packs.to_str().unwrap()),
            (
                "ICG_HOOK_CONFIG",
                temp.path().join("missing.json").to_str().unwrap(),
            ),
        ],
    );
    assert!(!bad_hooks.status.success());
}

#[test]
fn daily_operations_queries_fixture_for_tables_json_and_reports() {
    let denial_log = fixture("daily-operations.json");
    let now = "2026-08-16T10:30:00Z";

    let recent = run_with_text_env(
        &["status", "--denials", "--since", "1h"],
        &[
            ("ICG_DENIAL_LOG", denial_log.to_str().unwrap()),
            ("ICG_OPERATOR_NOW", now),
        ],
    );
    assert!(recent.status.success(), "{}", stderr(&recent));
    let recent_text = stdout(&recent);
    assert!(recent_text.contains("DENIALS (last 1h)"));
    assert!(recent_text.contains("2026-08-16 10:23:45"));
    assert!(recent_text.contains("vault-kv-destroy"));
    assert!(recent_text.contains("git-force-push"));
    assert!(recent_text.contains("latest-tag"));

    let summary = run_with_text_env(
        &["status", "--denials", "--pattern-summary", "--since", "7d"],
        &[
            ("ICG_DENIAL_LOG", denial_log.to_str().unwrap()),
            ("ICG_OPERATOR_NOW", now),
        ],
    );
    assert!(summary.status.success(), "{}", stderr(&summary));
    let summary_text = stdout(&summary);
    assert!(summary_text.contains("DENIAL PATTERNS (last 7d)"));
    assert!(summary_text.contains("vault-kv-destroy"));
    assert!(summary_text.contains("git-force-push"));
    assert!(summary_text.contains("latest-tag"));
    assert!(summary_text.contains("33%"));

    let json_output = run_with_text_env(
        &["status", "--denials", "--since", "1h", "--format", "json"],
        &[
            ("ICG_DENIAL_LOG", denial_log.to_str().unwrap()),
            ("ICG_OPERATOR_NOW", now),
        ],
    );
    assert!(json_output.status.success(), "{}", stderr(&json_output));
    let records: Value = serde_json::from_str(&stdout(&json_output)).expect("JSON output expected");
    assert_eq!(records.as_array().unwrap().len(), 3);
    assert_eq!(records[0]["telemetryId"], "den-abc123");
    assert_eq!(records[0]["packId"], "vault");

    let report = run_with_text_env(
        &["export-denial", "den-abc123"],
        &[("ICG_DENIAL_LOG", denial_log.to_str().unwrap())],
    );
    assert!(report.status.success(), "{}", stderr(&report));
    assert!(stdout(&report).contains("Denial report: den-abc123"));
    assert!(stdout(&report).contains("Command: vault kv destroy secret/app/api-key"));
    assert!(stdout(&report).contains("Reason: vault kv destroy is permanently destructive"));

    let missing = run_with_text_env(
        &["export-denial", "den-missing"],
        &[("ICG_DENIAL_LOG", "/tmp/icg-operator-scenario-missing.json")],
    );
    assert!(!missing.status.success());
}

#[test]
fn handling_denials_checks_format_redirect_and_safe_alternatives() {
    let scenario = fixture_json("handling-denials.json");
    let pack = fixture("handling-denials-pack.json");
    let denied_command = scenario["denied_command"].as_str().unwrap();

    let denied = run_with_text_env(
        &[
            "check",
            "--command",
            denied_command,
            "--pack",
            pack.to_str().unwrap(),
        ],
        &[],
    );
    assert!(denied.status.success(), "{}", stderr(&denied));
    let denied_text = stdout(&denied);
    assert!(denied_text.contains("DENIED by icg"));
    assert!(denied_text.contains("Severity: Critical"));
    assert!(denied_text.contains("Redirect: Use 'vault kv patch'"));

    let explanation = run_with_text_env(
        &[
            "explain",
            "--pattern",
            scenario["explain_pattern"].as_str().unwrap(),
            "--pack",
            pack.to_str().unwrap(),
        ],
        &[],
    );
    assert!(explanation.status.success(), "{}", stderr(&explanation));
    let explanation_text = stdout(&explanation);
    assert!(explanation_text.contains("Pattern: vault-kv-destroy"));
    assert!(explanation_text.contains("Severity: Critical"));
    assert!(explanation_text.contains("Why: This operation would permanently destroy secret data"));
    assert!(explanation_text.contains("Alternative: Use 'vault kv patch'"));

    for safe_command in scenario["safe_commands"].as_array().unwrap() {
        let allowed = run_with_text_env(
            &[
                "check",
                "--command",
                safe_command.as_str().unwrap(),
                "--pack",
                pack.to_str().unwrap(),
            ],
            &[],
        );
        assert!(allowed.status.success(), "{}", stderr(&allowed));
        assert_eq!(stdout(&allowed).trim(), "ALLOW: no configured rule matched");
    }

    let missing_pattern = run_with_text_env(
        &[
            "explain",
            "--pattern",
            "not-a-real-pattern",
            "--pack",
            pack.to_str().unwrap(),
        ],
        &[],
    );
    assert!(!missing_pattern.status.success());
}

#[test]
fn emergency_response_records_state_bypasses_once_and_restores_protection() {
    let scenario = fixture_json("emergency-response.json");
    let denial_log = fixture("daily-operations.json");
    let pack = fixture("installation-packs/vault.json");
    let temp = tempdir().expect("emergency temp directory should be created");
    let incident = temp.path().join("emergency-record.txt");
    let incident_text = format!(
        "EMERGENCY BYPASS RECORD\n======================\nService: {}\nIssue: {}\nAction: {}\nJustification: {}",
        scenario["incident"]["service"],
        scenario["incident"]["issue"],
        scenario["incident"]["action"],
        scenario["incident"]["justification"],
    );
    fs::write(&incident, incident_text).expect("incident record should be written");
    let saved_incident = fs::read_to_string(&incident).expect("incident record should be readable");
    assert!(saved_incident.contains("EMERGENCY BYPASS RECORD"));
    assert!(saved_incident.contains("auth-api"));
    assert!(saved_incident.contains("Service down, users affected"));

    let health = run_with_text_env(
        &["status", "--health"],
        &[("ICG_DENIAL_LOG", denial_log.to_str().unwrap())],
    );
    assert!(health.status.success(), "{}", stderr(&health));
    assert!(stdout(&health).contains("✓ icg is healthy and running"));
    assert!(stdout(&health).contains("Recent denials: 3 in last 5m"));

    let bypass = run_with_text_env(
        &[
            "check",
            "--command",
            scenario["bypass_command"].as_str().unwrap(),
            "--pack",
            pack.to_str().unwrap(),
        ],
        &[("ICG_DISABLED", "1")],
    );
    assert!(bypass.status.success(), "{}", stderr(&bypass));
    assert!(stdout(&bypass).contains("WARNING: icg guard disabled for this command"));
    assert!(stdout(&bypass).contains("ALLOW: emergency bypass active"));

    let restored = run_with_text_env(
        &[
            "check",
            "--command",
            "vault kv destroy secret/test",
            "--pack",
            pack.to_str().unwrap(),
        ],
        &[],
    );
    assert!(restored.status.success(), "{}", stderr(&restored));
    assert!(stdout(&restored).contains("DENIED by icg"));

    let active = run_with_text_env(
        &["status", "--health"],
        &[("ICG_DENIAL_LOG", denial_log.to_str().unwrap())],
    );
    assert!(active.status.success(), "{}", stderr(&active));
    assert!(stdout(&active).contains(scenario["restored_health"].as_str().unwrap()));

    let export = run_with_text_env(
        &["export-denial", scenario["denial_id"].as_str().unwrap()],
        &[("ICG_DENIAL_LOG", denial_log.to_str().unwrap())],
    );
    assert!(export.status.success(), "{}", stderr(&export));
    assert!(stdout(&export).contains("Denial report: den-abc123"));
}

#[test]
fn maintenance_commands_validate_health_trends_updates_and_backup() {
    let maintenance = fixture_json("maintenance.json");
    let packs = fixture("installation-packs");
    let temp = tempdir().expect("maintenance temp directory should be created");
    let archive = temp.path().join("icg-backup-20260816.tar.gz");
    let update_fixture = fixture("maintenance.json");

    let health = run_with_text_env(
        &["health", "--verbose"],
        &[("ICG_PACK_DIR", packs.to_str().unwrap())],
    );
    assert!(health.status.success(), "{}", stderr(&health));
    assert!(stdout(&health).contains("✓ Rule packs: 3 packs loaded"));
    assert!(stdout(&health).contains("✓ State store: /var/lib/icg/state.db"));

    let trend = run_with_text_env(
        &["status", "--denials", "--trend", "--since", "30d"],
        &[
            (
                "ICG_DENIAL_LOG",
                fixture("daily-operations.json").to_str().unwrap(),
            ),
            ("ICG_OPERATOR_NOW", "2026-08-16T10:30:00Z"),
        ],
    );
    assert!(trend.status.success(), "{}", stderr(&trend));
    assert!(stdout(&trend).contains(maintenance["trend"]["header"].as_str().unwrap()));
    assert!(stdout(&trend).contains(maintenance["trend"]["summary"].as_str().unwrap()));

    let update = run_with_text_env(
        &["update", "--check-only"],
        &[("ICG_UPDATE_FIXTURE", update_fixture.to_str().unwrap())],
    );
    assert!(update.status.success(), "{}", stderr(&update));
    let update_text = stdout(&update);
    assert!(update_text.contains("Updates available:"));
    assert!(update_text.contains("vault: v0.1.0 → v0.1.1"));
    assert!(update_text.contains("git: v0.1.0 → v0.1.2"));

    let backup = run_with_text_env(
        &["backup", "create", "--output", archive.to_str().unwrap()],
        &[("ICG_BACKUP_SOURCE", FIXTURES)],
    );
    assert!(backup.status.success(), "{}", stderr(&backup));
    assert!(archive.is_file());

    let verify = run(&["backup", "verify", archive.to_str().unwrap()]);
    assert!(verify.status.success(), "{}", stderr(&verify));
    assert!(stdout(&verify).contains(maintenance["backup_success"].as_str().unwrap()));

    let corrupt = temp.path().join("corrupt-backup.tar.gz");
    fs::write(&corrupt, b"not a tar archive").expect("corrupt backup should be written");
    let failed_verify = run(&["backup", "verify", corrupt.to_str().unwrap()]);
    assert!(!failed_verify.status.success());
}
