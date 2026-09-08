use icg::fail_closed::{PolicyMode, PolicyState, PolicyStore};
use icg::health::{GuardLifecycle, HealthState, HealthStore};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Serialize the tests that mutate process-global environment variables.
///
/// Recovering from poisoning is deliberate.  The guarded value is `()`, so a
/// panic while holding it leaves nothing inconsistent to protect against, and
/// propagating the poison turns one real assertion failure into five
/// unrelated ones -- which is precisely how a single stderr regression came
/// to be recorded as "fails 5/5" while its actual cause sat in one test.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The hook resolves its trust directory to a hardcoded `/etc/icg`
/// (`TrustPointerStore::default_path`) with no override, so these tests cannot
/// isolate it the way they isolate health, policy and telemetry paths.  When
/// that directory does not satisfy the guard's own security check, the guard
/// correctly reports a violation on every invocation -- a REAL fault, not test
/// noise -- and these tests would be asserting against someone else's defect.
///
/// Ask the guard's own check rather than reimplementing a condition of it.
/// The argo guarded-builder image fails it two independent ways
/// (irrevers-beee1069): it ships /etc/icg at 0777, AND its pods run as uid 0
/// with `USER` unset, which the check reports as "Current user can WRITE to
/// artifact directory" even after the mode is corrected. A guard that tested
/// only the world-writable bit would start failing again the moment the
/// Dockerfile is fixed.
fn skip_if_ambient_trust_directory_is_insecure(test: &str) -> bool {
    let Ok(default_path) = icg::trust_pointer::TrustPointerStore::default_path() else {
        return false;
    };
    let store = icg::trust_pointer::TrustPointerStore::new(default_path);
    match store.verify_artifact_directory_security() {
        Ok(()) => false,
        Err(error) => {
            eprintln!(
                "SKIP {test}: the ambient trust directory fails the guard's own security \
                 check, so it reports a violation on every call (irrevers-beee1069): {error:#}. \
                 The assertion under test is about the fail-closed policy, not that defect."
            );
            true
        }
    }
}

fn seed_stale_run(path: &std::path::Path) {
    let store = HealthStore::new(path);
    let mut state = HealthState::new();
    state.mark_start();
    state.current_run_pid = Some(u32::MAX);
    store
        .persist(&state)
        .expect("stale health marker should persist");
}

fn run_hook(directory: &TempDir, policy_mode: Option<PolicyMode>, input: &[u8]) -> Output {
    let policy_path = directory.path().join("policy.json");

    if let Some(mode) = policy_mode {
        let policy = PolicyStore::new(&policy_path);
        let mut state = PolicyState::new(3).expect("default threshold should be valid");
        state.mode = mode;
        policy.save(&state).expect("policy should persist");
    }

    run_hook_with_policy(directory, &policy_path, true, input)
}

/// Run one hook invocation against an explicit policy path.  `seed_crash`
/// controls the recovered-crash scenario; the ordinary guarded tool call has
/// a clean lifecycle marker and never recovers a crash.
fn run_hook_with_policy(
    directory: &TempDir,
    policy_path: &std::path::Path,
    seed_crash: bool,
    input: &[u8],
) -> Output {
    let health_path = directory.path().join("health.json");
    let telemetry_path = directory.path().join("telemetry.json");
    let state_path = directory.path().join("session-state.json");
    if seed_crash {
        seed_stale_run(&health_path);
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["hook"])
        .env("ICG_HEALTH_PATH", &health_path)
        .env("ICG_FAIL_CLOSED_POLICY", policy_path)
        .env("ICG_TELEMETRY_PATH", &telemetry_path)
        // The operational state store is the one artifact the guarded
        // invocation legitimately owns; keep it out of /var/cache/icg so the
        // assertions below measure this run and nothing ambient.
        .env("ICG_STATE_PATH", &state_path)
        // The CI executor configures its production pack location globally.
        // These lifecycle tests deliberately exercise crash recovery without
        // loading a pack, so do not let a missing executor-local directory
        // turn the hook's ordinary startup error into a second crash record.
        .env_remove("ICG_RULE_PACK")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook should start");
    child
        .stdin
        .take()
        .expect("hook stdin should be piped")
        .write_all(input)
        .expect("hook input should be written");
    child.wait_with_output().expect("hook should finish")
}

/// A recovered crash is recorded as evidence in the operational state store
/// the guarded process owns; the poison-pill policy event is the operator's
/// reconciliation of that evidence, never a guarded write.  The old contract
/// -- the hook itself persisting `last_poison_pill_event` -- only held on an
/// agent-writable policy directory, which the hardened deployment
/// deliberately does not provide (irrevers-3e6c6fde).
#[test]
fn recovered_guard_crash_records_evidence_and_reconciles_into_policy() {
    let _lock = env_lock();
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = br#"{"toolName":"Bash","toolInput":{"command":"printf safe"}}"#;
    let output = run_hook(&directory, None, input);
    assert!(
        output.status.success(),
        "fail-open hook should continue: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response = String::from_utf8_lossy(&output.stdout);
    assert!(response.contains("\"permissionDecision\":\"allow\""));

    let health = HealthStore::new(directory.path().join("health.json"))
        .load_or_create()
        .expect("health state should recover");
    assert_eq!(health.total_crashes, 1);

    let state_store =
        icg::state_store::StateStore::new(directory.path().join("session-state.json"));
    let evidence = state_store
        .guard_crash_state()
        .expect("guard-crash evidence should persist");
    assert_eq!(evidence.crash_count, 1);
    let crash_id = evidence
        .last_crash_id
        .expect("recorded evidence should identify the crash");

    let policy_path = directory.path().join("policy.json");
    assert!(
        !policy_path.exists(),
        "a guarded invocation must not create fail-closed policy state"
    );

    let state_path = directory.path().join("session-state.json");
    let trust_path = directory.path().join("trust-pointer.json");
    let run_reconcile = || {
        Command::new(env!("CARGO_BIN_EXE_icg"))
            .args(["policy", "reconcile"])
            .arg("--path")
            .arg(&policy_path)
            .arg("--state-store-path")
            .arg(&state_path)
            .arg("--trust-pointer-path")
            .arg(&trust_path)
            .env(
                "ICG_TELEMETRY_PATH",
                directory.path().join("telemetry.json"),
            )
            .output()
            .expect("policy reconcile should run")
    };

    let reconcile = run_reconcile();
    assert!(
        reconcile.status.success(),
        "policy reconcile should succeed: {}",
        String::from_utf8_lossy(&reconcile.stderr)
    );
    let reconcile_output = String::from_utf8_lossy(&reconcile.stdout);
    assert!(
        reconcile_output.contains("PoisonPill"),
        "reconciliation should consume the crash evidence: {reconcile_output}"
    );
    let policy = PolicyStore::new(&policy_path)
        .load()
        .expect("reconciled policy should load");
    assert_eq!(policy.mode, PolicyMode::FailOpen);
    assert_eq!(
        policy.last_poison_pill_event.as_deref(),
        Some(format!("guard-crash:{crash_id}").as_str())
    );

    // The counter makes consumption idempotent: replaying the same evidence
    // against an already-caught-up policy does not poison twice.
    let replay = run_reconcile();
    assert!(
        replay.status.success(),
        "replayed policy reconcile should succeed: {}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let replay_output = String::from_utf8_lossy(&replay.stdout);
    assert!(
        replay_output.contains("Pending"),
        "replayed reconciliation must not consume the same crash twice: {replay_output}"
    );
}

#[test]
fn recovered_guard_crash_denies_in_fail_closed_mode() {
    let _lock = env_lock();
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = br#"{"toolName":"Bash","toolInput":{"command":"printf unsafe"}}"#;
    let policy_path = directory.path().join("policy.json");
    let lock_path = directory.path().join("policy.lock");

    let policy = PolicyStore::new(&policy_path);
    let mut state = PolicyState::new(3).expect("default threshold should be valid");
    state.mode = PolicyMode::FailClosed;
    policy.save(&state).expect("deployed policy should persist");
    // Seeding went through the writer, which legitimately takes the lock.
    // Clear it so the assertions below measure the hook, not the setup.
    std::fs::remove_file(&lock_path).expect("seed lock should be removable");

    let output = run_hook_with_policy(&directory, &policy_path, true, input);
    assert!(
        output.status.success(),
        "hook protocol should return a denial response: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response = String::from_utf8_lossy(&output.stdout);
    assert!(response.contains("\"permissionDecision\":\"deny\""));
    assert!(response.contains("guard-crash"));

    let health = HealthStore::new(directory.path().join("health.json"))
        .load_or_create()
        .expect("health state should recover");
    assert_eq!(health.total_crashes, 1);
    let evidence = icg::state_store::StateStore::new(directory.path().join("session-state.json"))
        .guard_crash_state()
        .expect("guard-crash evidence should persist");
    assert_eq!(evidence.crash_count, 1);

    // The denial comes from reading the durable Fail-Closed posture, not
    // from the guarded process writing policy state to justify itself.
    assert!(
        !lock_path.exists(),
        "the hook must not create the fail-closed policy lock"
    );
    let policy = PolicyStore::new(&policy_path)
        .load()
        .expect("policy should remain readable after enforcement");
    assert_eq!(policy.mode, PolicyMode::FailClosed);
    assert!(
        policy.last_poison_pill_event.is_none(),
        "the denial must come from reading the durable policy, not from the \
         guarded process writing a poison-pill event: {:?}",
        policy.last_poison_pill_event
    );
    assert!(
        policy.events.is_empty(),
        "the guarded process must not write policy events: {:?}",
        policy.events
    );
}

#[test]
fn lifecycle_reports_recovered_crash_once() {
    let _lock = env_lock();
    let directory = tempfile::tempdir().expect("temporary directory");
    let health_path = directory.path().join("health.json");
    seed_stale_run(&health_path);

    std::env::set_var("ICG_HEALTH_PATH", &health_path);
    let lifecycle = GuardLifecycle::start().expect("lifecycle should recover stale marker");
    assert!(lifecycle.recovered_crash());
    assert_eq!(
        lifecycle
            .startup_crash()
            .expect("crash evidence should be available")
            .crash_type,
        icg::health::CrashType::Unknown
    );
    lifecycle
        .store()
        .load_or_create()
        .expect("health state should be readable");
    std::env::remove_var("ICG_HEALTH_PATH");
}

/// A normally-installed host keeps the policy directory administrator-owned
/// (the deployment ownership table lists `/etc/icg/fail-closed-policy.json`
/// as `root:root`) while `/var/cache/icg` stays writable by the guarded
/// agent.  Graduation is an operator action, so an ordinary hook invocation
/// must not even attempt the policy lock, must leave the deployed state
/// untouched, and must keep the stderr channel clear for faults that matter.
#[test]
fn hook_invocation_leaves_administrator_owned_policy_untouched() {
    if skip_if_ambient_trust_directory_is_insecure(
        "hook_invocation_leaves_administrator_owned_policy_untouched",
    ) {
        return;
    }
    let _lock = env_lock();
    let policy_directory = tempfile::tempdir().expect("policy directory");
    let state_directory = tempfile::tempdir().expect("guard state directory");
    let policy_path = policy_directory.path().join("fail-closed-policy.json");
    let lock_path = policy_directory.path().join("fail-closed-policy.lock");

    let policy = PolicyStore::new(&policy_path);
    let mut state = PolicyState::new(3).expect("default threshold should be valid");
    state.mode = PolicyMode::FailOpen;
    policy.save(&state).expect("deployed policy should persist");
    // Seeding went through the writer, which legitimately takes the lock.
    // Clear it so the assertion below measures the hook rather than the
    // setup -- on a real host the operator's last write leaves the same file
    // behind, but here we want to catch the hook creating it.
    std::fs::remove_file(&lock_path).expect("seed lock should be removable");

    let set_policy_directory_mode = |mode: u32| {
        let mut permissions = std::fs::metadata(policy_directory.path())
            .expect("policy directory metadata")
            .permissions();
        permissions.set_mode(mode);
        std::fs::set_permissions(policy_directory.path(), permissions)
            .expect("policy directory permissions should apply");
    };
    set_policy_directory_mode(0o555);

    let input = br#"{"toolName":"Bash","toolInput":{"command":"printf safe"}}"#;
    let output = run_hook_with_policy(&state_directory, &policy_path, false, input);

    // Restore write access so the temporary directory can still be removed.
    set_policy_directory_mode(0o700);

    assert!(
        output.status.success(),
        "fail-open hook should continue: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Nothing may reach stderr at all.  The guarded process neither locks nor
    // writes the policy, and a run that starts uneventfully is not an event
    // worth a line -- the channel belongs to faults.  Asserting emptiness
    // rather than the absence of one known string is deliberate: it is what
    // makes this test catch the NEXT thing that decides to narrate itself
    // here.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty(),
        "a guarded invocation must leave stderr clear for real faults: {stderr}"
    );
    assert!(
        !lock_path.exists(),
        "the hook must not create the fail-closed policy lock"
    );
    let response = String::from_utf8_lossy(&output.stdout);
    assert!(response.contains("\"permissionDecision\":\"allow\""));

    let deployed = PolicyStore::new(&policy_path)
        .load()
        .expect("policy should remain readable");
    assert_eq!(deployed.mode, PolicyMode::FailOpen);
    assert!(
        deployed.events.is_empty(),
        "the guarded process must not write policy events: {:?}",
        deployed.events
    );
}

/// The invocation right after a recovered crash is the moment the guard is
/// most degraded, and it must still keep the hardened ownership boundary:
/// crash evidence goes to the operational state store the guarded agent
/// owns, while the administrator-owned policy directory is neither locked
/// nor written (irrevers-3e6c6fde).
#[test]
fn recovered_guard_crash_keeps_administrator_owned_policy_untouched() {
    if skip_if_ambient_trust_directory_is_insecure(
        "recovered_guard_crash_keeps_administrator_owned_policy_untouched",
    ) {
        return;
    }
    let _lock = env_lock();
    let policy_directory = tempfile::tempdir().expect("policy directory");
    let state_directory = tempfile::tempdir().expect("guard state directory");
    let policy_path = policy_directory.path().join("fail-closed-policy.json");
    let lock_path = policy_directory.path().join("fail-closed-policy.lock");

    let policy = PolicyStore::new(&policy_path);
    let mut state = PolicyState::new(3).expect("default threshold should be valid");
    state.mode = PolicyMode::FailOpen;
    policy.save(&state).expect("deployed policy should persist");
    std::fs::remove_file(&lock_path).expect("seed lock should be removable");

    let set_policy_directory_mode = |mode: u32| {
        let mut permissions = std::fs::metadata(policy_directory.path())
            .expect("policy directory metadata")
            .permissions();
        permissions.set_mode(mode);
        std::fs::set_permissions(policy_directory.path(), permissions)
            .expect("policy directory permissions should apply");
    };
    set_policy_directory_mode(0o555);

    let input = br#"{"toolName":"Bash","toolInput":{"command":"printf safe"}}"#;
    let output = run_hook_with_policy(&state_directory, &policy_path, true, input);

    // Restore write access so the temporary directory can still be removed.
    set_policy_directory_mode(0o700);

    assert!(
        output.status.success(),
        "fail-open hook should continue: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("icg_health_event event=crash_detected"),
        "the recovered crash must still be reported: {stderr}"
    );
    assert!(
        !stderr.contains("permission denied"),
        "recording crash evidence must stay in the state store the guarded \
         agent owns, not reach for the policy directory: {stderr}"
    );
    let response = String::from_utf8_lossy(&output.stdout);
    assert!(response.contains("\"permissionDecision\":\"allow\""));

    assert!(
        !lock_path.exists(),
        "the post-crash invocation must not create the fail-closed policy lock"
    );
    let deployed = PolicyStore::new(&policy_path)
        .load()
        .expect("policy should remain readable");
    assert_eq!(deployed.mode, PolicyMode::FailOpen);
    assert!(
        deployed.last_poison_pill_event.is_none() && deployed.events.is_empty(),
        "the guarded process must not write policy state: {:?}",
        deployed.events
    );

    let evidence =
        icg::state_store::StateStore::new(state_directory.path().join("session-state.json"))
            .guard_crash_state()
            .expect("guard-crash evidence should persist");
    assert_eq!(
        evidence.crash_count, 1,
        "the crash evidence itself must not be dropped on the hardened layout"
    );
}

/// Graduation, inspection, and demotion are operator actions against the
/// administrator-controlled store.  Those commands keep working after the
/// guarded invocation stopped reconciling on every tool call.
#[test]
fn operator_policy_commands_manage_the_durable_policy() {
    if skip_if_ambient_trust_directory_is_insecure(
        "operator_policy_commands_manage_the_durable_policy",
    ) {
        return;
    }
    let _lock = env_lock();
    let directory = tempfile::tempdir().expect("temporary directory");
    let policy_path = directory.path().join("fail-closed-policy.json");

    let policy = PolicyStore::new(&policy_path);
    let mut state = PolicyState::new(3).expect("default threshold should be valid");
    state.mode = PolicyMode::FailOpen;
    policy.save(&state).expect("deployed policy should persist");

    let status = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["policy", "status"])
        .arg("--path")
        .arg(&policy_path)
        .env(
            "ICG_TELEMETRY_PATH",
            directory.path().join("telemetry.json"),
        )
        .output()
        .expect("policy status should run");
    assert!(
        status.status.success(),
        "policy status should succeed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_output = String::from_utf8_lossy(&status.stdout);
    assert!(
        status_output.contains("**Mode:** FailOpen"),
        "{status_output}"
    );

    let reconcile = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["policy", "reconcile"])
        .arg("--path")
        .arg(&policy_path)
        .arg("--state-store-path")
        .arg(directory.path().join("session-state.json"))
        .arg("--trust-pointer-path")
        .arg(directory.path().join("trust-pointer.json"))
        .env(
            "ICG_TELEMETRY_PATH",
            directory.path().join("telemetry.json"),
        )
        .output()
        .expect("policy reconcile should run");
    assert!(
        reconcile.status.success(),
        "policy reconcile should succeed: {}",
        String::from_utf8_lossy(&reconcile.stderr)
    );
    let reconcile_output = String::from_utf8_lossy(&reconcile.stdout);
    assert!(
        reconcile_output.contains("Fail-closed policy reconciliation: Pending"),
        "reconciliation without a trust pointer is pending, not clean: {reconcile_output}"
    );
}

/// The counterpart to the assertion above: stderr goes quiet for uneventful
/// runs, so it must still carry the events that are not uneventful.  A
/// recovered crash is the loudest thing the lifecycle can report, and it
/// keeps reporting.
#[test]
fn a_recovered_crash_still_announces_itself_on_stderr() {
    let _lock = env_lock();
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = br#"{"toolName":"Bash","toolInput":{"command":"printf safe"}}"#;
    let output = run_hook(&directory, Some(PolicyMode::FailOpen), input);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("icg_health_event event=crash_detected"),
        "a recovered crash must still be reported: {stderr}"
    );
    assert!(
        !stderr.contains("event=run_started"),
        "starting a run is not an event: {stderr}"
    );
    let response = String::from_utf8_lossy(&output.stdout);
    assert!(response.contains("\"permissionDecision\":\"allow\""));
}
