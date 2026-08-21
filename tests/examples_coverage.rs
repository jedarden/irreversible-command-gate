//! Static audit of the scenario-to-fixture/test index in `docs/examples/README.md`.
//!
//! The executable scenario tests prove behavior. This small guard prevents the
//! documentation index from silently losing a scenario, fixture, or test owner.

use std::fs;
use std::path::Path;

struct ScenarioAudit {
    heading: &'static str,
    fixtures: &'static [&'static str],
    test_file: &'static str,
    test_name: &'static str,
    expected_marker: &'static str,
}

#[test]
fn examples_coverage_audit_maps_all_documented_scenarios() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme_path = root.join("docs/examples/README.md");
    let readme = fs::read_to_string(&readme_path).expect("examples README should be readable");

    let scenarios = [
        ScenarioAudit {
            heading: "### Scenario 1: First-time Installation",
            fixtures: &[
                "tests/fixtures/operator-scenarios/installation.json",
                "tests/fixtures/operator-scenarios/installation-packs/",
            ],
            test_file: "tests/operator_scenarios.rs",
            test_name: "first_time_installation_validates_documented_commands_and_outputs",
            expected_marker: "vault-kv-destroy",
        },
        ScenarioAudit {
            heading: "### Scenario 2: Daily Operations",
            fixtures: &["tests/fixtures/operator-scenarios/daily-operations.json"],
            test_file: "tests/operator_scenarios.rs",
            test_name: "daily_operations_queries_fixture_for_tables_json_and_reports",
            expected_marker: "den-abc123",
        },
        ScenarioAudit {
            heading: "### Scenario 3: Handling Denials",
            fixtures: &[
                "tests/fixtures/operator-scenarios/handling-denials.json",
                "tests/fixtures/operator-scenarios/handling-denials-pack.json",
            ],
            test_file: "tests/operator_scenarios.rs",
            test_name: "handling_denials_checks_format_redirect_and_safe_alternatives",
            expected_marker: "both safe alternatives allow",
        },
        ScenarioAudit {
            heading: "### Scenario 4: Emergency Response",
            fixtures: &["tests/fixtures/operator-scenarios/emergency-response.json"],
            test_file: "tests/operator_scenarios.rs",
            test_name: "emergency_response_records_state_bypasses_once_and_restores_protection",
            expected_marker: "ICG_DISABLED=1",
        },
        ScenarioAudit {
            heading: "### Scenario 5: Maintenance Tasks",
            fixtures: &[
                "tests/fixtures/operator-scenarios/maintenance.json",
                "tests/fixtures/operator-scenarios/installation-packs/",
            ],
            test_file: "tests/operator_scenarios.rs",
            test_name: "maintenance_commands_validate_health_trends_updates_and_backup",
            expected_marker: "corrupt archive is rejected",
        },
        ScenarioAudit {
            heading: "### Scenario 6: Creating a New Rule Pack",
            fixtures: &["tests/fixtures/developer-scenarios/creating-rule-pack-new.json"],
            test_file: "tests/developer_scenarios.rs",
            test_name: "scenario_6_new_pack_scaffold_and_local_validation",
            expected_marker: "PVC deletion denies",
        },
        ScenarioAudit {
            heading: "### Scenario 7: Testing Pattern Changes",
            fixtures: &[
                "tests/fixtures/developer-scenarios/testing-pattern-changes-baseline.json",
                "tests/fixtures/developer-scenarios/testing-pattern-changes-updated.json",
                "tests/fixtures/developer-scenarios/regression-suite-baseline.json",
                "tests/fixtures/developer-scenarios/regression-suite-updated.json",
            ],
            test_file: "tests/developer_scenarios.rs",
            test_name: "scenario_7_regression_generation_verification_and_coverage_diff",
            expected_marker: "narrowed diff is rejected without justification",
        },
        ScenarioAudit {
            heading: "### Scenario 8: Debugging False Positives",
            fixtures: &[
                "tests/fixtures/developer-scenarios/debugging-false-positives-overly-broad.json",
                "tests/fixtures/developer-scenarios/debugging-false-positives-fixed.json",
            ],
            test_file: "tests/developer_scenarios.rs",
            test_name: "scenario_8_debug_trace_reproduce_fix_and_verify_false_positive",
            expected_marker: "malformed packs fail",
        },
        ScenarioAudit {
            heading: "### Scenario 9: Adding Custom Predicates",
            fixtures: &["tests/fixtures/developer-scenarios/adding-custom-predicates.json"],
            test_file: "tests/developer_scenarios.rs",
            test_name: "scenario_9_custom_predicates_evaluate_shared_checkout_scope",
            expected_marker: "A `.beads/` write in the shared checkout denies",
        },
        ScenarioAudit {
            heading: "### Scenario 10: Migrating from org-rule-guard.py",
            fixtures: &["tests/fixtures/integration-scenarios/scenario-10-migration.json"],
            test_file: "tests/integration_scenarios.rs",
            test_name: "scenario_10_compares_org_guard_overlap_and_coverage_gaps",
            expected_marker: "icg-only OpenBao destructive probe denies",
        },
        ScenarioAudit {
            heading: "### Scenario 11: Setting up Multi-Harness Support",
            fixtures: &["tests/fixtures/integration-scenarios/scenario-11-multi-harness.json"],
            test_file: "tests/integration_scenarios.rs",
            test_name: "scenario_11_parses_and_runs_both_harness_wire_formats",
            expected_marker: "CamelCase and snake_case payloads parse",
        },
        ScenarioAudit {
            heading: "### Scenario 12: Configuring Repository Overrides",
            fixtures: &[
                "tests/fixtures/integration-scenarios/scenario-12-repository-overrides.json",
            ],
            test_file: "tests/integration_scenarios.rs",
            test_name: "scenario_12_runs_override_request_approval_verification_and_expiry",
            expected_marker: "release-bound TOML artifact",
        },
    ];

    assert_eq!(scenarios.len(), 12, "the README documents twelve scenarios");
    for scenario in scenarios {
        assert!(
            readme.contains(scenario.heading),
            "coverage audit is missing README heading: {}",
            scenario.heading
        );
        assert!(
            readme.contains(scenario.test_name),
            "coverage audit is missing scenario test name: {}",
            scenario.test_name
        );
        assert!(
            readme.contains(scenario.expected_marker),
            "coverage audit is missing expected outcome marker: {}",
            scenario.expected_marker
        );

        let test_path = root.join(scenario.test_file);
        let test_source = fs::read_to_string(&test_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", test_path.display()));
        assert!(
            test_source.contains(scenario.test_name),
            "README names a test that is absent from {}: {}",
            scenario.test_file,
            scenario.test_name
        );

        for fixture in scenario.fixtures {
            let fixture_path = root.join(fixture.trim_end_matches('/'));
            assert!(
                fixture_path.exists(),
                "README names a missing scenario fixture: {}",
                fixture
            );
            assert!(
                readme.contains(fixture),
                "coverage audit is missing fixture path: {}",
                fixture
            );
        }
    }
}
