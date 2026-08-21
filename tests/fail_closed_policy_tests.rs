use icg::fail_closed::{PolicyMode, PolicyStore, PolicyTransition, ReconcileOutcome};
use icg::rollback::PoisonPillConfig;
use icg::state_store::{DenyRatePolicy, StateStore};
use icg::trust_pointer::{TrustPointer, TrustPointerStore};
use icg::{engine::CheckResult, engine::CommandSource, engine::Engine};

fn adopt(store: &StateStore, trust: &TrustPointerStore, release_ref: &str) {
    let pointer = TrustPointer::new(release_ref);
    trust.save(&pointer).expect("trust pointer should save");
    store
        .save_trust_pointer(&pointer)
        .expect("trust pointer observation should save");
}

fn record_release(store: &StateStore, release_ref: &str, count: usize) {
    for _ in 0..count {
        store
            .record_release_evaluation(release_ref, false)
            .expect("release telemetry should save");
    }
}

fn test_poison_config() -> PoisonPillConfig {
    PoisonPillConfig {
        max_current_evaluations: 100,
        policy: DenyRatePolicy {
            minimum_baseline_releases: 3,
            minimum_current_evaluations: 3,
            minimum_baseline_evaluations: 9,
            ..DenyRatePolicy::default()
        },
        ..PoisonPillConfig::default()
    }
}

#[test]
fn policy_reconciles_unique_clean_releases_and_graduates() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let runtime = StateStore::new(directory.path().join("runtime.json"));
    let trust = TrustPointerStore::new(directory.path().join("trust.json"));
    let policy = PolicyStore::new(directory.path().join("policy.json"));
    let poison_config = test_poison_config();

    record_release(&runtime, "v1", 3);
    record_release(&runtime, "v2", 3);
    record_release(&runtime, "v3", 3);
    adopt(&runtime, &trust, "v4");
    record_release(&runtime, "v4", 3);

    policy.set_threshold(2).expect("threshold should configure");
    let first = policy
        .reconcile_release_health(&runtime, &trust, &poison_config)
        .expect("first release should reconcile");
    assert!(matches!(
        first,
        ReconcileOutcome::Clean(PolicyTransition::CleanRelease {
            clean_streak: 1,
            ..
        })
    ));

    // Replaying the same release is idempotent.
    assert_eq!(
        policy
            .reconcile_release_health(&runtime, &trust, &poison_config)
            .expect("duplicate reconciliation should succeed"),
        ReconcileOutcome::NoChange
    );

    adopt(&runtime, &trust, "v5");
    record_release(&runtime, "v5", 3);
    let second = policy
        .reconcile_release_health(&runtime, &trust, &poison_config)
        .expect("second release should reconcile");
    assert!(matches!(
        second,
        ReconcileOutcome::Clean(PolicyTransition::Graduated { .. })
    ));
    assert_eq!(
        policy.load().expect("policy should load").mode,
        PolicyMode::FailClosed
    );
}

#[test]
fn poison_pill_resets_open_policy_without_editing_telemetry() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let runtime = StateStore::new(directory.path().join("runtime.json"));
    let trust = TrustPointerStore::new(directory.path().join("trust.json"));
    let policy = PolicyStore::new(directory.path().join("policy.json"));
    let poison_config = test_poison_config();

    record_release(&runtime, "v1", 3);
    record_release(&runtime, "v2", 3);
    record_release(&runtime, "v3", 3);
    adopt(&runtime, &trust, "v4");
    record_release(&runtime, "v4", 3);
    policy.set_threshold(2).expect("threshold should configure");
    policy
        .reconcile_release_health(&runtime, &trust, &poison_config)
        .expect("clean release should reconcile");
    let telemetry_before = runtime.release_telemetry().expect("telemetry should load");

    runtime
        .record_rollback("v4", "v3", "test poison pill")
        .expect("rollback event should persist");
    let result = policy
        .reconcile_release_health(&runtime, &trust, &poison_config)
        .expect("poison pill should reconcile");
    assert!(matches!(result, ReconcileOutcome::PoisonPill(_)));
    assert_eq!(
        policy
            .load()
            .expect("policy should load")
            .clean_release_streak,
        0
    );
    assert_eq!(
        runtime.release_telemetry().expect("telemetry should load"),
        telemetry_before,
        "policy reconciliation must not mutate poison-pill telemetry"
    );
}

#[test]
fn engine_uses_fail_closed_mode_for_guard_load_failure() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let broken_pack = directory.path().join("broken.json");
    std::fs::write(
        &broken_pack,
        r#"{
          "id": "broken",
          "tool_keywords": ["vault"],
          "guarded_patterns": [{
            "id": "broken-regex",
            "check": {"type": "command_regex", "regex": "["},
            "tier": "tier1",
            "severity": "High",
            "explanation": "test",
            "redirect": {"channel": "deny", "reason_template": "test"}
          }]
        }"#,
    )
    .expect("broken pack should write");

    let mut fail_open = Engine::new().with_fail_closed(false);
    fail_open
        .load_pack_from_file(&broken_pack)
        .expect("pack failures are handled by the engine");
    assert_eq!(
        fail_open.evaluate_command(&CommandSource::Hook("vault status".into())),
        CheckResult::Allowed
    );

    let mut fail_closed = Engine::new().with_fail_closed(true);
    fail_closed
        .load_pack_from_file(&broken_pack)
        .expect("pack failures are handled by the engine");
    assert!(matches!(
        fail_closed.evaluate_command(&CommandSource::Hook("vault status".into())),
        CheckResult::Denied {
            ref pack_id,
            ref pattern_id,
            ..
        } if pack_id == "fail-closed" && pattern_id == "guard-crash"
    ));
}
