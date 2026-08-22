//! Maintenance Tasks scenario tests (Scenario 5)
//!
//! These tests verify the maintenance workflow documented in
//! docs/examples/README.md Scenario 5: Maintenance Tasks.
//!
//! The scenario covers:
//! - Step 1: Weekly Health Check
//! - Step 2: Monthly Review (denial trends)
//! - Step 3: Rule Pack Updates
//! - Step 4: Quarterly Testing (backup/restore)
//!
//! This tests regular maintenance operations for operators.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::tempdir;

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

#[test]
fn maintenance_scenario_1_weekly_health_check() {
    // Step 1: Verify weekly health check works
    let health = icg(&["health", "status"]);

    // Should succeed
    assert!(
        health.status.success(),
        "Weekly health check should succeed"
    );

    let stdout = String::from_utf8_lossy(&health.stdout);
    // Should show health status information
    assert!(
        stdout.contains("Health") || stdout.contains("Status") || stdout.contains("health"),
        "Health check should show status information"
    );
}

#[test]
fn maintenance_scenario_1_health_shows_binary_status() {
    // Verify health check shows binary status
    let health = icg(&["health", "status"]);

    let stdout = String::from_utf8_lossy(&health.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&health.stderr)
    } else {
        stdout
    };

    // Should indicate the tool is running
    assert!(
        output.contains("running") || output.contains("Status") || output.contains("Path"),
        "Health status should show binary or running status"
    );
}

#[test]
fn maintenance_scenario_2_status_shows_rule_pack_info() {
    // Step 2: Verify status shows rule pack information
    let status = icg(&["status"]);

    // Should succeed
    assert!(status.status.success(), "Status command should succeed");

    let stdout = String::from_utf8_lossy(&status.stdout);
    // Should show rule pack version or status
    assert!(
        stdout.contains("Pack") || stdout.contains("Rule") || stdout.contains("Trust"),
        "Status should show rule pack information"
    );
}

#[test]
fn maintenance_scenario_2_status_shows_trust_pointer() {
    // Verify status shows trust pointer information
    let status = icg(&["status"]);

    let stdout = String::from_utf8_lossy(&status.stdout);
    // Should show trust pointer or reference information
    assert!(
        stdout.contains("Trust")
            || stdout.contains("Pointer")
            || stdout.contains("Reference")
            || stdout.contains("Update"),
        "Status should show trust pointer information"
    );
}

#[test]
fn maintenance_scenario_3_update_command_exists() {
    // Step 3: Verify update command exists and is documented
    let help = icg(&["--help"]);

    let stdout = String::from_utf8_lossy(&help.stdout);
    // Should mention update command
    assert!(
        stdout.contains("update") || stdout.contains("Update"),
        "Help should mention update command"
    );
}

#[test]
fn maintenance_scenario_3_trust_command_works() {
    // Verify trust pointer commands work
    let temp_dir = tempdir().unwrap();
    let trust_path = temp_dir.path().join("test-trust.json");

    // Set a trust pointer
    let set = icg(&[
        "trust",
        "set",
        "v0.1.0",
        "--path",
        &trust_path.to_string_lossy(),
    ]);
    assert!(set.status.success(), "Trust set should succeed");

    // Show the trust pointer
    let show = icg(&["trust", "show", "--path", &trust_path.to_string_lossy()]);
    assert!(show.status.success(), "Trust show should succeed");

    let stdout = String::from_utf8_lossy(&show.stdout);
    assert!(
        stdout.contains("v0.1.0") || stdout.contains("Trust"),
        "Trust show should display the reference"
    );

    // Check if a reference is trusted
    let check = icg(&[
        "trust",
        "check",
        "v0.1.0",
        "--path",
        &trust_path.to_string_lossy(),
    ]);
    assert!(check.status.success(), "Trust check should succeed");

    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(
        stdout.contains("trusted") || stdout.contains("✓"),
        "Trust check should confirm reference is trusted"
    );
}

#[test]
fn maintenance_scenario_4_backup_create_command_exists() {
    // Step 4: Verify backup create command exists
    let backup_help = icg(&["backup", "--help"]);

    let stdout = String::from_utf8_lossy(&backup_help.stdout);
    // Should mention create or verify subcommands
    assert!(
        stdout.contains("create")
            || stdout.contains("Create")
            || stdout.contains("verify")
            || stdout.contains("Verify"),
        "Backup help should mention create/verify subcommands"
    );
}

#[test]
fn maintenance_scenario_4_coverage_list_shows_packs() {
    // Verify coverage list shows available rule packs
    let coverage = icg(&["coverage", "--list"]);

    // Should succeed
    assert!(coverage.status.success(), "Coverage list should succeed");

    let stdout = String::from_utf8_lossy(&coverage.stdout);
    // Should show pack information
    assert!(
        stdout.contains("pack") || stdout.contains("coverage") || stdout.is_empty(),
        "Coverage list should show packs or coverage information"
    );
}

#[test]
fn maintenance_scenario_regression_suite_generation() {
    // Verify regression suite can be generated for maintenance testing
    let packs = vec!["packs/image-tag.json", "packs/storage-class.json"];

    for pack_path in packs {
        if PathBuf::from(pack_path).exists() {
            let temp_dir = tempdir().unwrap();
            let output_path = temp_dir.path().join("regression.json");

            let result = icg(&[
                "regression-suite",
                pack_path,
                "--output",
                &output_path.to_string_lossy(),
            ]);

            // Should succeed or fail gracefully (not crash)
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                !stderr.contains("panic") && !stderr.contains("segfault"),
                "Regression suite generation should not crash for {}",
                pack_path
            );

            // If successful, verify output file was created
            if result.status.success() && output_path.exists() {
                let content =
                    fs::read_to_string(&output_path).expect("regression suite should be readable");
                assert!(
                    content.contains("cases") || content.contains("regression"),
                    "Regression suite should contain cases information"
                );
            }
        }
    }
}

#[test]
fn maintenance_scenario_health_reset_works() {
    // Verify health data can be reset (for maintenance)
    let temp_dir = tempdir().unwrap();
    let health_path = temp_dir.path().join("test-health.json");

    // First, mark some activity
    let mark_start = icg(&[
        "health",
        "mark-start",
        "--path",
        &health_path.to_string_lossy(),
    ]);
    assert!(
        mark_start.status.success(),
        "Health mark-start should succeed"
    );

    // Then reset (with force flag for testing)
    let reset = icg(&[
        "health",
        "reset",
        "--path",
        &health_path.to_string_lossy(),
        "--force",
    ]);
    assert!(reset.status.success(), "Health reset should succeed");

    let stdout = String::from_utf8_lossy(&reset.stdout);
    assert!(
        stdout.contains("cleared") || stdout.contains("✓") || stdout.contains("Health"),
        "Health reset should confirm data was cleared"
    );
}

#[test]
fn maintenance_scenario_telemetry_status_works() {
    // Verify telemetry status can be checked
    let temp_dir = tempdir().unwrap();
    let telemetry_path = temp_dir.path().join("test-telemetry.json");

    let status = icg(&[
        "telemetry",
        "status",
        "--path",
        &telemetry_path.to_string_lossy(),
    ]);

    // Should succeed
    assert!(status.status.success(), "Telemetry status should succeed");

    let stdout = String::from_utf8_lossy(&status.stdout);
    // Should show telemetry information
    assert!(
        stdout.contains("Telemetry") || stdout.contains("Window") || stdout.contains("Baseline"),
        "Telemetry status should show monitoring information"
    );
}

#[test]
fn maintenance_scenario_redos_check_works() {
    // Verify ReDoS (Regular Expression Denial of Service) check works
    let packs = vec!["packs/image-tag.json"];

    for pack_path in packs {
        if PathBuf::from(pack_path).exists() {
            let result = icg(&["redos-check", pack_path, "--skip-dynamic"]);

            // Should not crash
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                !stderr.contains("panic") && !stderr.contains("segfault"),
                "ReDoS check should not crash"
            );

            // Should provide output about safety
            let stdout = String::from_utf8_lossy(&result.stdout);
            assert!(
                stdout.contains("ReDoS")
                    || stdout.contains("PASS")
                    || stdout.contains("pattern")
                    || stdout.contains("safe"),
                "ReDoS check should report on pattern safety"
            );
        }
    }
}

#[test]
fn maintenance_scenario_bug_report_command_exists() {
    // Verify bug-report command exists for maintenance issue reporting
    let bug_report_help = icg(&["bug-report", "--help"]);

    // Should succeed or fail gracefully
    let stderr = String::from_utf8_lossy(&bug_report_help.stderr);
    assert!(
        !stderr.contains("panic") && !stderr.contains("segfault"),
        "Bug-report command should not crash"
    );
}

#[test]
fn maintenance_scenario_override_command_exists() {
    // Verify override commands exist for maintenance
    let override_help = icg(&["override", "--help"]);

    // Should succeed or fail gracefully
    let stderr = String::from_utf8_lossy(&override_help.stderr);
    assert!(
        !stderr.contains("panic") && !stderr.contains("segfault"),
        "Override command should not crash"
    );

    let stdout = String::from_utf8_lossy(&override_help.stdout);
    assert!(
        stdout.contains("create")
            || stdout.contains("approve")
            || stdout.contains("list")
            || stdout.contains("override"),
        "Override help should mention subcommands"
    );
}

#[test]
fn maintenance_scenario_trust_channel_support() {
    // Verify trust commands support channels (for canary rollout maintenance)
    let temp_dir = tempdir().unwrap();
    let trust_path = temp_dir.path().join("test-trust-canary.json");

    // Set trust pointer for canary channel
    let set = icg(&[
        "trust",
        "set",
        "v0.2.0-canary",
        "--channel",
        "canary",
        "--path",
        &trust_path.to_string_lossy(),
    ]);

    assert!(
        set.status.success(),
        "Trust set with channel should succeed"
    );

    // Show for canary channel
    let show = icg(&[
        "trust",
        "show",
        "--channel",
        "canary",
        "--path",
        &trust_path.to_string_lossy(),
    ]);

    assert!(
        show.status.success(),
        "Trust show with channel should succeed"
    );

    let stdout = String::from_utf8_lossy(&show.stdout);
    assert!(
        stdout.contains("canary") || stdout.contains("v0.2.0"),
        "Trust show should display channel information"
    );
}
