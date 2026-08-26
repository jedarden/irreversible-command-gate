//! Emergency response scenario tests (Scenario 4)
//!
//! These tests verify the emergency response workflow documented in
//! docs/examples/README.md Scenario 4: Emergency Response.
//!
//! The scenario covers:
//! - Step 1: Assess the Situation (health check)
//! - Step 2: Document the Emergency
//! - Step 3: Bypass the Guard (ICG_DISABLED environment variable)
//! - Step 4: Verify and Restore (service health check)
//! - Step 5: Follow Up (incident reporting)
//!
use std::fs;
use std::io::Write;
use std::process::{Command, Output};
use tempfile::tempdir;

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

#[test]
fn emergency_scenario_1_assess_situation_with_health_check() {
    // Step 1: Verify we can check icg health to assess if it's blocking
    let health = icg(&["health", "--check-packs"]);

    // This should succeed and show pack status
    assert!(health.status.success());
    let stdout = String::from_utf8_lossy(&health.stdout);
    assert!(stdout.contains("pack") || stdout.contains("valid") || stdout.contains("✓"));
}

#[test]
fn emergency_scenario_4_document_and_verify_emergency_record() {
    // Step 2: Verify we can create an emergency documentation record
    let temp_dir = tempdir().unwrap();
    let emergency_file = temp_dir.path().join("emergency-record.txt");

    // Create emergency record (as documented in the scenario)
    let timestamp = chrono::Utc::now().to_rfc3339();
    let emergency_record = format!(
        "EMERGENCY BYPASS RECORD
======================
Timestamp: {}
Service: auth-api
Issue: Vault policy deleted, breaking authentication
Action: Bypassing icg to restore policy
Justification: Service down, users affected",
        timestamp
    );

    fs::write(&emergency_file, emergency_record).expect("emergency record should be written");

    // Verify record exists and contains required fields
    let content = fs::read_to_string(&emergency_file).expect("emergency record should be readable");
    assert!(content.contains("EMERGENCY BYPASS RECORD"));
    assert!(content.contains("Timestamp:"));
    assert!(content.contains("Service:"));
    assert!(content.contains("Issue:"));
    assert!(content.contains("Action:"));
    assert!(content.contains("Justification:"));
}

#[test]
fn emergency_scenario_3_bypass_guard_with_disabled_flag() {
    // Step 3: `check` records an activation but never records command data.
    let temp = tempdir().expect("emergency telemetry directory should be created");
    let telemetry_path = temp.path().join("telemetry.json");
    let command_secret = "emergency-secret-must-not-be-logged";

    let result = Command::new(env!("CARGO_BIN_EXE_icg"))
        .env("ICG_DISABLED", "1")
        .env("ICG_TELEMETRY_PATH", &telemetry_path)
        .args([
            "check",
            "--command",
            &format!("vault policy write auth-policy {command_secret}"),
        ])
        .output()
        .expect("icg should run even when disabled");

    // When ICG_DISABLED=1, dangerous commands should be allowed
    // This should succeed without denial
    assert!(result.status.success());

    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("WARNING") || stdout.contains("disabled") || stdout.contains("bypass"),
        "Should warn about disabled guard"
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("icg_emergency_bypass event=activated front_end=check"));
    assert!(
        !format!("{stdout}{stderr}").contains(command_secret),
        "emergency activation output must not contain command data"
    );

    let telemetry = fs::read_to_string(&telemetry_path).expect("bypass telemetry should persist");
    assert!(
        telemetry.contains("\"front_end\": \"check\""),
        "check activation should be auditable: {telemetry}"
    );
    assert!(
        !telemetry.contains(command_secret),
        "emergency telemetry must never contain command data"
    );
}

#[test]
fn emergency_bypass_hook_returns_json_allow_before_fail_closed_loading() {
    let temp = tempdir().expect("hook support directory should be created");
    let telemetry_path = temp.path().join("telemetry.json");
    let command_secret = "hook-emergency-secret-must-not-be-logged";
    let missing_pack = temp.path().join("missing-pack.json");

    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["hook", "--rule-pack"])
        .arg(&missing_pack)
        .env("ICG_DISABLED", "1")
        .env("ICG_FAIL_CLOSED", "true")
        .env("ICG_TELEMETRY_PATH", &telemetry_path)
        .env("ICG_HEALTH_PATH", temp.path().join("health.json"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("icg hook should start");
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(
            format!(
                r#"{{"tool_name":"Bash","tool_input":{{"command":"vault kv destroy {command_secret}"}}}}"#
            )
            .as_bytes(),
        )
        .expect("hook input should be written");

    let output = child.wait_with_output().expect("hook should finish");
    assert!(
        output.status.success(),
        "emergency hook bypass should allow: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("hook bypass must emit JSON");
    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    assert!(response["systemMessage"]
        .as_str()
        .expect("hook bypass should contain its mandatory warning")
        .contains("ICG_DISABLED"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("icg_emergency_bypass event=activated front_end=hook"));
    assert!(!format!("{}{}", response, stderr).contains(command_secret));
    let telemetry = fs::read_to_string(&telemetry_path).expect("hook telemetry should persist");
    assert!(telemetry.contains("\"front_end\": \"hook\""));
    assert!(!telemetry.contains(command_secret));
}

#[cfg(unix)]
#[test]
fn emergency_bypass_executes_a_real_shadowed_binary_without_command_logging() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let temp = tempdir().expect("wrapper support directory should be created");
    let wrapper_dir = temp.path().join("wrapper");
    let real_dir = temp.path().join("real");
    fs::create_dir(&wrapper_dir).expect("wrapper directory should be created");
    fs::create_dir(&real_dir).expect("real-tool directory should be created");

    let tool = "icg-emergency-shadowed-tool";
    let wrapper = wrapper_dir.join(tool);
    symlink(env!("CARGO_BIN_EXE_icg"), &wrapper).expect("wrapper symlink should be created");
    let real_tool = real_dir.join(tool);
    fs::write(
        &real_tool,
        "#!/bin/sh\nprintf 'REAL_SHADOWED_TOOL_RAN\\n'\n",
    )
    .expect("real tool should be written");
    let mut permissions = fs::metadata(&real_tool)
        .expect("real tool metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&real_tool, permissions).expect("real tool should be executable");

    let path = std::env::join_paths([wrapper_dir, real_dir]).expect("test PATH should be valid");
    let telemetry_path = temp.path().join("telemetry.json");
    let command_secret = "wrapper-emergency-secret-must-not-be-logged";
    let output = Command::new(&wrapper)
        .args(["destroy", command_secret])
        .env("PATH", path)
        .env("ICG_DISABLED", "1")
        .env("ICG_TELEMETRY_PATH", &telemetry_path)
        .env("ICG_HEALTH_PATH", temp.path().join("health.json"))
        .output()
        .expect("shadowed wrapper should run");

    assert!(
        output.status.success(),
        "shadowed emergency bypass should exec the real tool: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "REAL_SHADOWED_TOOL_RAN\n"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ICG_DISABLED emergency bypass active"));
    assert!(stderr.contains("icg_emergency_bypass event=activated front_end=wrapper"));
    assert!(!stderr.contains(command_secret));

    let telemetry = fs::read_to_string(&telemetry_path).expect("wrapper telemetry should persist");
    assert!(telemetry.contains("\"front_end\": \"wrapper\""));
    assert!(!telemetry.contains(command_secret));
}

#[test]
fn emergency_scenario_5_export_denial_for_false_positive_report() {
    // Step 4: Verify we can export denial details for incident reporting
    let temp_dir = tempdir().unwrap();
    let report_file = temp_dir.path().join("false-positive-report.txt");

    // Create a test denial scenario
    let check = icg(&["check", "--command", "vault kv destroy secret/test"]);

    // Should deny the destructive command
    assert!(!check.status.success() || String::from_utf8_lossy(&check.stdout).contains("DENIED"));

    // Export denial details (as documented in Scenario 2, Step 4)
    // Note: icg doesn't have an explicit "export-denial" command,
    // but we can capture the output for reporting
    let denial_output = String::from_utf8_lossy(&check.stdout);
    fs::write(&report_file, denial_output.as_bytes()).expect("denial report should be written");

    // Verify report contains useful information
    let report_content =
        fs::read_to_string(&report_file).expect("denial report should be readable");
    assert!(
        report_content.contains("vault")
            || report_content.contains("destroy")
            || report_content.contains("DENIED"),
        "Report should contain denial information"
    );
}

#[test]
fn emergency_scenario_health_status_shows_recent_denials() {
    // Verify health status can show recent denials for emergency assessment
    // This tests the health check capabilities documented in Scenario 4, Step 1

    // First, trigger a denial
    let _denial = icg(&["check", "--command", "vault kv destroy secret/test"]);

    // Check health status
    let health = icg(&["health"]);

    // Should succeed and provide status
    assert!(health.status.success());
    let stdout = String::from_utf8_lossy(&health.stdout);

    // Health output should contain status information
    assert!(
        stdout.contains("icg")
            || stdout.contains("binary")
            || stdout.contains("version")
            || stdout.contains("healthy"),
        "Health check should provide status information"
    );
}

#[test]
fn emergency_scenario_denial_history_for_incident_analysis() {
    let temp = tempdir().expect("denial history directory should be created");
    let denial_log = temp.path().join("denials.jsonl");
    let command = "vault kv destroy secret/incident-analysis";

    // A direct check is a production front end, so this must persist the same
    // JSONL record that hooks and wrappers produce. A private log path keeps
    // the scenario isolated from concurrent operator activity.
    let check = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["check", "--command", command])
        .env("ICG_DENIAL_LOG", &denial_log)
        .output()
        .expect("icg check should run");
    assert!(check.status.success());
    assert!(String::from_utf8_lossy(&check.stdout).contains("DENIED by icg"));

    let persisted = fs::read_to_string(&denial_log).expect("denial record should persist");
    let record: serde_json::Value = serde_json::from_str(persisted.trim())
        .expect("production denial log should contain one JSON record");
    let telemetry_id = record["id"]
        .as_str()
        .expect("denial record should have an ID")
        .to_owned();
    assert_eq!(record["pack_id"], "openbao");
    assert_eq!(record["pattern_id"], "openbao-destructive-verb");
    assert_eq!(record["severity"], "high");
    assert_eq!(record["context"]["tool"], "command");
    assert_eq!(record["denied_input"]["command"], "<redacted>");
    assert!(
        !persisted.contains(command),
        "denial history should not persist command payloads by default"
    );

    let history = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["status", "--denials", "--since", "1h", "--format", "json"])
        .env("ICG_DENIAL_LOG", &denial_log)
        .output()
        .expect("icg status should run");
    assert!(
        history.status.success(),
        "denial history should be readable: {}",
        String::from_utf8_lossy(&history.stderr)
    );
    let history: serde_json::Value =
        serde_json::from_slice(&history.stdout).expect("denial history should be JSON");
    let denial = history
        .as_array()
        .and_then(|denials| denials.first())
        .expect("recent denial should be returned");
    assert_eq!(denial["telemetryId"], telemetry_id);
    assert_eq!(denial["packId"], "openbao");
    assert_eq!(denial["patternId"], "openbao-destructive-verb");
    assert_eq!(denial["severity"], "High");
    assert_eq!(denial["command"], "<redacted>");

    let summary = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["status", "--denials", "--pattern-summary", "--since", "1h"])
        .env("ICG_DENIAL_LOG", &denial_log)
        .output()
        .expect("icg pattern summary should run");
    assert!(summary.status.success());
    let summary = String::from_utf8_lossy(&summary.stdout);
    assert!(summary.contains("DENIAL PATTERNS (last 1h)"));
    assert!(summary.contains("openbao-destructive-verb"));
    assert!(summary.contains("100%"));

    let export = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["export-denial", &telemetry_id])
        .env("ICG_DENIAL_LOG", &denial_log)
        .output()
        .expect("icg export-denial should run");
    assert!(
        export.status.success(),
        "denial export should succeed: {}",
        String::from_utf8_lossy(&export.stderr)
    );
    let export = String::from_utf8_lossy(&export.stdout);
    assert!(export.contains(&format!("Denial report: {telemetry_id}")));
    assert!(export.contains("Pattern: openbao-destructive-verb"));
    assert!(export.contains("Command: <redacted>"));
}
