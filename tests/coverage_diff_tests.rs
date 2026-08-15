use std::path::PathBuf;

#[test]
fn test_no_regressions_clean_release() {
    let previous = PathBuf::from("tests/fixtures/previous-release.json");
    let current = PathBuf::from("tests/fixtures/current-release-clean.json");

    let diff = icg::coverage::run_coverage_diff(previous, current).unwrap();

    assert!(
        !diff.has_regressions(),
        "Clean release should not have regressions"
    );
    assert!(diff.removed_guarded_patterns.is_empty());
    assert!(diff.widened_safe_patterns.is_empty());
    assert!(diff.narrowed_guarded_patterns.is_empty());
}

#[test]
fn test_detects_removed_guarded_patterns() {
    let previous = PathBuf::from("tests/fixtures/previous-release.json");
    let current = PathBuf::from("tests/fixtures/current-release-regression.json");

    let diff = icg::coverage::run_coverage_diff(previous, current).unwrap();

    assert!(diff.has_regressions(), "Should detect regressions");

    // vault-policy-delete was removed
    assert!(diff
        .removed_guarded_patterns
        .contains(&"vault-policy-delete".to_string()));
    // vault-secrets-disable was removed
    assert!(diff
        .removed_guarded_patterns
        .contains(&"vault-secrets-disable".to_string()));
    assert_eq!(diff.removed_guarded_patterns.len(), 2);

    let removed: Vec<&str> = diff
        .removed_guarded_pattern_changes
        .iter()
        .map(|change| change.pattern_id.as_str())
        .collect();
    assert_eq!(
        removed,
        vec!["vault-policy-delete", "vault-secrets-disable"]
    );
    assert!(diff
        .removed_guarded_pattern_changes
        .iter()
        .all(|change| change.current == "<removed>"));
}

#[test]
fn test_detects_widened_safe_patterns() {
    let previous = PathBuf::from("tests/fixtures/previous-release.json");
    let current = PathBuf::from("tests/fixtures/current-release-regression.json");

    let diff = icg::coverage::run_coverage_diff(previous, current).unwrap();

    assert!(diff.has_regressions());

    // safe-list was widened from "vault.*list" to ".*" (catch-all)
    let widened_list: Vec<&str> = diff
        .widened_safe_patterns
        .iter()
        .map(|p| p.pattern_id.as_str())
        .collect();
    assert!(
        widened_list.contains(&"safe-list"),
        "Should detect widened safe-list pattern"
    );
}

#[test]
fn test_detects_narrowed_guarded_patterns_marked_destructive() {
    let previous = PathBuf::from("tests/fixtures/previous-release.json");
    let current = PathBuf::from("tests/fixtures/current-release-regression.json");

    let diff = icg::coverage::run_coverage_diff(previous, current).unwrap();

    assert!(diff.has_regressions());

    // git-force-push was narrowed from "git push.*--force" to "git push --force"
    // (removed the wildcard .* between push and --force)
    let narrowed_list: Vec<&str> = diff
        .narrowed_guarded_patterns
        .iter()
        .map(|p| p.pattern_id.as_str())
        .collect();
    assert!(
        narrowed_list.contains(&"git-force-push"),
        "Should detect narrowed git-force-push pattern"
    );
}

#[test]
fn test_pattern_widened_detection() {
    // More specific -> Less specific (widened)
    assert!(icg::coverage::is_pattern_widened("specific-thing", ".*"));
    assert!(icg::coverage::is_pattern_widened("vault.*list", ".*"));
    assert!(icg::coverage::is_pattern_widened("^git status", "git.*"));
}

#[test]
fn test_pattern_not_widened() {
    // Same or more specific (not widened)
    assert!(!icg::coverage::is_pattern_widened(".*", ".*"));
    assert!(!icg::coverage::is_pattern_widened(".*", "specific-thing"));
}

#[test]
fn test_pattern_narrowed_detection() {
    // Less specific -> More specific (narrowed)
    assert!(icg::coverage::is_pattern_narrowed(".*", "specific-thing"));
    assert!(icg::coverage::is_pattern_narrowed(
        "git push.*--force",
        "git push --force"
    ));
    assert!(icg::coverage::is_pattern_narrowed(
        "dangerous-.*",
        "dangerous-thing"
    ));
}

#[test]
fn test_pattern_not_narrowed() {
    // Same or less specific (not narrowed)
    assert!(!icg::coverage::is_pattern_narrowed(".*", ".*"));
    assert!(!icg::coverage::is_pattern_narrowed("specific-thing", ".*"));
}

#[test]
fn test_renders_layer_two_report_with_justification_field() {
    let previous = PathBuf::from("tests/fixtures/previous-release.json");
    let current = PathBuf::from("tests/fixtures/current-release-regression.json");

    let diff = icg::coverage::run_coverage_diff(previous.clone(), current.clone()).unwrap();
    let report = icg::coverage::render_coverage_diff_report(
        &previous,
        &current,
        &diff,
        Some("Reviewed removals and approved the intentional deprecation."),
    );

    assert!(report.contains("format: coverage-diff/v1"));
    assert!(report.contains("status: regressions_detected"));
    assert!(report
        .contains("justification: Reviewed removals and approved the intentional deprecation."));
    assert!(report.contains("## Removed guarded_patterns"));
    assert!(report.contains("## Widened safe_patterns"));
    assert!(report.contains("## Narrowed guarded_patterns (destructive: true)"));
    assert!(report.contains("previous: vault secrets disable"));
    assert!(report.contains("current: <removed>"));
    assert!(report.contains("previous: vault.*list"));
    assert!(report.contains("current: .*"));
    assert!(report.contains("previous: git push.*--force"));
    assert!(report.contains("current: git push --force"));

    let missing_justification =
        icg::coverage::render_coverage_diff_report(&previous, &current, &diff, None);
    assert!(missing_justification.contains(
        "justification: REQUIRED: provide --justification with the release approval rationale"
    ));
}

#[test]
fn test_justification_must_be_explicit() {
    assert!(!icg::coverage::CoverageDiff::has_explicit_justification(
        None
    ));
    assert!(!icg::coverage::CoverageDiff::has_explicit_justification(
        Some("  ")
    ));
    assert!(icg::coverage::CoverageDiff::has_explicit_justification(
        Some("intentional deprecation")
    ));
}

#[test]
fn test_load_rule_pack() {
    let path = PathBuf::from("tests/fixtures/previous-release.json");
    let pack = icg::coverage::load_rule_pack(path).unwrap();

    assert_eq!(pack.id, "test-pack-previous");
    assert_eq!(pack.safe_patterns.len(), 3);
    assert_eq!(pack.guarded_patterns.len(), 4);

    // Verify a specific pattern
    let vault_destroy = pack
        .guarded_patterns
        .iter()
        .find(|p| p.id == "vault-kv-destroy")
        .expect("Should find vault-kv-destroy pattern");

    // Extract regex from the check enum
    match &vault_destroy.check {
        icg::rule_pack::Check::CommandRegex { regex } => {
            assert_eq!(regex, "vault kv destroy");
        }
        _ => panic!("Expected CommandRegex check"),
    }

    assert!(vault_destroy.destructive);
}
