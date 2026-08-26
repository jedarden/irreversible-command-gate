use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rollback::PoisonPillConfig;
use icg::state_store::{DenyRatePolicy, StateStore};
use icg::trust_pointer::{TrustPointer, TrustPointerStore};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

fn secure_tempdir() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("temporary state directory");
    let mut permissions = std::fs::metadata(directory.path())
        .expect("temporary directory metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(directory.path(), permissions).expect("secure temporary directory");
    directory
}

#[test]
fn engine_persists_per_release_evaluation_and_deny_counts() {
    let directory = secure_tempdir();
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

#[test]
fn engine_telemetry_feeds_poison_pill_rollback() {
    let directory = secure_tempdir();
    let state_store = Arc::new(StateStore::new(directory.path().join("state.json")));
    let trust_store = TrustPointerStore::new(directory.path().join("trust-pointer.json"));

    fn adopt(state_store: &StateStore, trust_store: &TrustPointerStore, release_ref: &str) {
        let pointer = TrustPointer::new(release_ref);
        trust_store
            .save(&pointer)
            .expect("trust pointer should save");
        state_store
            .save_trust_pointer(&pointer)
            .expect("trust pointer observation should save");
    }

    fn evaluate(state_store: &Arc<StateStore>, release_ref: &str, command: &str, count: usize) {
        let mut engine = Engine::new()
            .with_release_ref(release_ref)
            .with_state_store(Arc::clone(state_store));
        engine
            .load_pack_from_file(concat!(env!("CARGO_MANIFEST_DIR"), "/packs/openbao.json"))
            .expect("openbao pack loads");
        for _ in 0..count {
            let _ = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
        }
    }

    for release_ref in ["release-1", "release-2", "release-3"] {
        adopt(&state_store, &trust_store, release_ref);
        evaluate(&state_store, release_ref, "vault status", 3);
    }
    adopt(&state_store, &trust_store, "release-4");
    evaluate(
        &state_store,
        "release-4",
        "vault kv destroy secret/example",
        3,
    );

    let report = icg::rollback::check_and_rollback(
        &state_store,
        &trust_store,
        &PoisonPillConfig {
            max_current_evaluations: 100,
            policy: DenyRatePolicy {
                minimum_baseline_releases: 3,
                minimum_current_evaluations: 3,
                minimum_baseline_evaluations: 9,
                ..DenyRatePolicy::default()
            },
            ..PoisonPillConfig::default()
        },
    )
    .expect("rollback check should succeed")
    .expect("release-4 should be identified as a poison pill");

    assert_eq!(report.release_ref, "release-4");
    assert_eq!(report.previous_release, "release-3");
    assert_eq!(
        trust_store.get_trusted_ref().expect("pointer should load"),
        Some("release-3".to_string())
    );
}
