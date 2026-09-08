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
    if seed_crash {
        seed_stale_run(&health_path);
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["hook"])
        .env("ICG_HEALTH_PATH", &health_path)
        .env("ICG_FAIL_CLOSED_POLICY", policy_path)
        .env("ICG_TELEMETRY_PATH", &telemetry_path)
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

#[test]
fn recovered_guard_crash_is_fail_open_by_default_and_persisted() {
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
    let policy = PolicyStore::new(directory.path().join("policy.json"))
        .load()
        .expect("poison-pill policy event should persist");
    assert!(policy.last_poison_pill_event.is_some());
    assert_eq!(policy.mode, PolicyMode::FailOpen);
}

#[test]
fn recovered_guard_crash_denies_in_fail_closed_mode() {
    let _lock = env_lock();
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = br#"{"toolName":"Bash","toolInput":{"command":"printf unsafe"}}"#;
    let output = run_hook(&directory, Some(PolicyMode::FailClosed), input);
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
    let policy = PolicyStore::new(directory.path().join("policy.json"))
        .load()
        .expect("policy should remain readable after enforcement");
    assert_eq!(policy.mode, PolicyMode::FailClosed);
    assert!(policy.last_poison_pill_event.is_some());
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
    // Nothing about the policy may reach stderr: the guarded process neither
    // locks it nor writes it, so it has nothing to report.  `icg_health_event`
    // lines are the guard's own lifecycle telemetry, which also does not
    // belong on the fault channel but has a separate cause and a separate
    // bead (irrevers-0aa08f4e); filter those rather than weakening this to a
    // substring match, so any NEW warning still fails here.  When that bead
    // lands, delete the filter and assert `stderr.is_empty()`.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let unexpected: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("icg_health_event "))
        .collect();
    assert!(
        unexpected.is_empty(),
        "a guarded invocation must not warn about the administrator-owned policy: {unexpected:#?}"
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

/// Graduation, inspection, and demotion are operator actions against the
/// administrator-controlled store.  Those commands keep working after the
/// guarded invocation stopped reconciling on every tool call.
#[test]
fn operator_policy_commands_manage_the_durable_policy() {
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
