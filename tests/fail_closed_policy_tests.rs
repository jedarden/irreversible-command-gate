use icg::fail_closed::{
    PolicyEventType, PolicyMode, PolicyStore, PolicyTransition, ReconcileOutcome,
};
use icg::rollback::{check_and_rollback, PoisonPillConfig};
use icg::state_store::{DenyRatePolicy, StateStore};
use icg::trust_pointer::{TrustPointer, TrustPointerStore};
use icg::{engine::CheckResult, engine::CommandSource, engine::Engine};
use std::os::unix::fs::PermissionsExt;

fn secure_tempdir() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("temporary directory");
    let mut permissions = std::fs::metadata(directory.path())
        .expect("temporary directory metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(directory.path(), permissions).expect("secure temporary directory");
    directory
}

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
    let directory = secure_tempdir();
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
    assert_eq!(
        policy
            .load()
            .expect("policy telemetry should load")
            .events
            .last()
            .unwrap()
            .event_type,
        PolicyEventType::Graduated
    );
}

#[test]
fn poison_pill_resets_open_policy_without_editing_telemetry() {
    let directory = secure_tempdir();
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
    assert_eq!(
        policy
            .load()
            .expect("policy telemetry should load")
            .events
            .last()
            .unwrap()
            .event_type,
        PolicyEventType::PoisonPill
    );
}

/// Crash evidence recorded by the guarded process in its own state store is
/// consumed by the operator's reconciliation into the same poison-pill event
/// rollback evidence produces, exactly once (irrevers-3e6c6fde).
#[test]
fn reconcile_consumes_guard_crash_evidence_once() {
    let directory = secure_tempdir();
    let runtime = StateStore::new(directory.path().join("runtime.json"));
    let trust = TrustPointerStore::new(directory.path().join("trust.json"));
    let policy = PolicyStore::new(directory.path().join("policy.json"));
    let poison_config = test_poison_config();

    runtime
        .record_guard_crash("crash-1741234567890123456-4242")
        .expect("guard-crash evidence should persist");

    let result = policy
        .reconcile_release_health(&runtime, &trust, &poison_config)
        .expect("guard-crash evidence should reconcile");
    assert!(matches!(result, ReconcileOutcome::PoisonPill(_)));
    let state = policy.load().expect("policy should load");
    assert_eq!(
        state.last_poison_pill_event.as_deref(),
        Some("guard-crash:crash-1741234567890123456-4242")
    );
    assert_eq!(
        state.events.last().expect("event should exist").event_type,
        PolicyEventType::PoisonPill
    );

    // The counter makes the consumption idempotent; without a trust pointer
    // the replayed reconciliation is pending, not a second poison pill.
    assert!(matches!(
        policy
            .reconcile_release_health(&runtime, &trust, &poison_config)
            .expect("replayed reconciliation should succeed"),
        ReconcileOutcome::Pending { .. }
    ));

    // A second, later crash is a new event.
    runtime
        .record_guard_crash("crash-1741234599999999999-4243")
        .expect("second guard-crash evidence should persist");
    assert!(matches!(
        policy
            .reconcile_release_health(&runtime, &trust, &poison_config)
            .expect("second crash should reconcile"),
        ReconcileOutcome::PoisonPill(_)
    ));
    let state = policy.load().expect("policy should reload");
    assert_eq!(
        state.last_poison_pill_event.as_deref(),
        Some("guard-crash:crash-1741234599999999999-4243")
    );
}

/// An artifact directory owned by a non-root uid: the standing violation the
/// guarded CI pods carried for nineteen days without anything going red
/// (irrevers-beee1069). Built so exactly one condition exists on every
/// runner: unprivileged runners already own the fixture directory, and root
/// runners give it away (the probe is skipped for root either way). Mode
/// 0555 keeps the write probe from adding an unprivileged-write condition on
/// unprivileged runners.
fn artifact_dir_with_not_root_owned_violation() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("temporary directory");
    // Safe: a plain syscall wrapper; unprivileged runners already satisfy
    // the condition, so only a root runner needs the giveaway.
    if unsafe { libc::geteuid() } == 0 {
        std::os::unix::fs::chown(directory.path(), Some(65534), Some(65534))
            .expect("root should be able to chown the fixture directory");
    }
    let mut permissions = std::fs::metadata(directory.path())
        .expect("temporary directory metadata")
        .permissions();
    permissions.set_mode(0o555);
    std::fs::set_permissions(directory.path(), permissions).expect("fixture permissions");
    directory
}

/// The nineteen-day blind spot, end to end (irrevers-91694e78): a security
/// violation of the artifact directory is recorded by the guarded boundary
/// as crash evidence in the state store it owns, the operator's
/// reconciliation consumes it as exactly one poison-pill policy event, and
/// reconciling the same crash count again produces none.
#[test]
fn artifact_dir_violation_reconciles_into_exactly_one_poison_pill() {
    let artifact = artifact_dir_with_not_root_owned_violation();
    let directory = secure_tempdir();
    let runtime = StateStore::new(directory.path().join("runtime.json"));
    let trust = TrustPointerStore::new(artifact.path().join("trust.json"));
    let policy = PolicyStore::new(directory.path().join("policy.json"));
    let poison_config = test_poison_config();
    let expected_crash_id = format!(
        "artifact-dir-security:not-root-owned:{}",
        artifact.path().display()
    );

    // The guarded boundary: the check itself only warns for a custom-path
    // condition, and the violation is recorded anyway.
    let report = check_and_rollback(&runtime, &trust, &poison_config)
        .expect("a warn-only artifact-dir condition must not fail the guarded boundary");
    assert!(report.is_none(), "no telemetry means no rollback");

    let evidence = runtime
        .guard_crash_state()
        .expect("guard-crash evidence should persist");
    assert_eq!(
        evidence.crash_count, 1,
        "exactly one violation exists in the fixture"
    );
    assert_eq!(
        evidence.last_crash_id.as_deref(),
        Some(expected_crash_id.as_str())
    );

    // Restore write access so the temporary directory can still be removed.
    let mut permissions = std::fs::metadata(artifact.path())
        .expect("fixture metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(artifact.path(), permissions).expect("fixture restoration");

    // The operator's reconciliation turns the evidence into one poison-pill
    // event...
    let outcome = policy
        .reconcile_release_health(&runtime, &trust, &poison_config)
        .expect("reconciliation should succeed");
    let ReconcileOutcome::PoisonPill(PolicyTransition::PoisonPill { event_ref, .. }) = outcome
    else {
        panic!("the recorded violation must reconcile into a poison pill");
    };
    assert_eq!(event_ref, format!("guard-crash:{expected_crash_id}"));
    assert_eq!(
        policy
            .load()
            .expect("policy should load")
            .events
            .iter()
            .filter(|event| event.event_type == PolicyEventType::PoisonPill)
            .count(),
        1
    );

    // ...and reconciling the same crash count again produces none:
    // last_processed_guard_crash_count has already read this far.
    assert!(matches!(
        policy
            .reconcile_release_health(&runtime, &trust, &poison_config)
            .expect("replayed reconciliation should succeed"),
        ReconcileOutcome::Pending { .. }
    ));
    let replayed = policy.load().expect("policy should reload");
    assert_eq!(
        replayed
            .events
            .iter()
            .filter(|event| event.event_type == PolicyEventType::PoisonPill)
            .count(),
        1,
        "the same crash count must not produce a second poison-pill event"
    );
    assert_eq!(
        replayed.last_processed_guard_crash_count, 1,
        "the policy tracks how far it has read"
    );
}

#[test]
fn operator_force_graduate_and_force_revert_are_durable() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let policy = PolicyStore::new(directory.path().join("policy.json"));

    assert!(matches!(
        policy
            .force_graduate("approved fail-closed rollout")
            .expect("force graduation should succeed"),
        PolicyTransition::ForcedGraduation { .. }
    ));
    assert_eq!(
        policy.load().expect("policy should load").mode,
        PolicyMode::FailClosed
    );

    assert!(matches!(
        policy
            .force_revert("guard incident")
            .expect("force revert should succeed"),
        PolicyTransition::EmergencyDemotion { .. }
    ));
    let state = policy.load().expect("policy should reload");
    assert_eq!(state.mode, PolicyMode::FailOpen);
    assert_eq!(state.clean_release_streak, 0);
    assert_eq!(state.events.len(), 2);
    assert_eq!(
        state.events[0].event_type,
        PolicyEventType::ManualGraduation
    );
    assert_eq!(
        state.events[1].event_type,
        PolicyEventType::EmergencyDemotion
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
