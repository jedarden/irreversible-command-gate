use icg::engine::{CheckResult, CommandSource, Engine};
use icg::state_store::StateStore;
use std::sync::Arc;

#[test]
fn engine_persists_per_release_evaluation_and_deny_counts() {
    let directory = tempfile::tempdir().expect("temporary state directory");
    let state_store = Arc::new(StateStore::new(directory.path().join("state.json")));
    let mut engine = Engine::new()
        .with_release_ref("release-1")
        .with_state_store(Arc::clone(&state_store));
    engine
        .load_pack_from_file(concat!(env!("CARGO_MANIFEST_DIR"), "/packs/openbao.json"))
        .expect("openbao pack loads");

    let denied = engine.evaluate_command(&CommandSource::Hook(
        "vault kv destroy secret/example".to_string(),
    ));
    assert!(matches!(denied, CheckResult::Denied { .. }));

    let allowed = engine.evaluate_command(&CommandSource::Hook("vault status".to_string()));
    assert_eq!(allowed, CheckResult::Allowed);

    let record = state_store
        .release_telemetry_for("release-1")
        .expect("state store loads")
        .expect("release telemetry exists");
    assert_eq!(record.evaluation_count, 2);
    assert_eq!(record.deny_count, 1);
    assert!((record.deny_rate - 0.5).abs() < f64::EPSILON);
}
