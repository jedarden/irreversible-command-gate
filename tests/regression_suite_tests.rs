use std::path::PathBuf;

use icg::engine::{CheckResult, CommandSource, Engine};
use icg::regression::{
    ExpectedVerdict, generate_regression_suite, generate_regression_suite_from_manifest,
    verify_regression_suite,
};
use icg::rule_pack::load_pack;

#[test]
fn fixture_generates_a_case_for_every_guarded_pattern() {
    let manifest = PathBuf::from("tests/fixtures/previous-release.json");
    let pack = load_pack(&manifest).unwrap();
    let suite = generate_regression_suite(&pack).unwrap();

    assert_eq!(suite.cases.len(), pack.guarded_patterns.len());
    assert_eq!(
        suite
            .cases
            .iter()
            .map(|case| case.pattern_id.as_str())
            .collect::<Vec<_>>(),
        pack.guarded_patterns
            .iter()
            .map(|pattern| pattern.id.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        suite
            .cases
            .iter()
            .all(|case| case.expected == ExpectedVerdict::Deny)
    );
    verify_regression_suite(&pack, &suite).unwrap();

    let mut engine = Engine::new();
    engine.load_pack(pack).unwrap();
    for case in suite.cases {
        let result = engine.evaluate_command(&CommandSource::Hook(case.command));
        assert!(
            matches!(
                result,
                CheckResult::Denied { ref pattern_id, .. } if pattern_id == &case.pattern_id
            ),
            "regression case '{}' did not deny through the engine: {:?}",
            case.pattern_id,
            result
        );
    }
}

#[test]
fn manifest_generation_is_json_round_tripable() {
    let suite =
        generate_regression_suite_from_manifest("tests/fixtures/previous-release.json").unwrap();
    let json = suite.to_json().unwrap();
    let decoded: icg::regression::RegressionSuite = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded, suite);
}
