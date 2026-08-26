use icg::fail_closed::{PolicyMode, PolicyState, PolicyStore};
use icg::health::{GuardLifecycle, HealthState, HealthStore};
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("environment lock should not be poisoned")
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
    let health_path = directory.path().join("health.json");
    let policy_path = directory.path().join("policy.json");
    let telemetry_path = directory.path().join("telemetry.json");
    seed_stale_run(&health_path);

    if let Some(mode) = policy_mode {
        let policy = PolicyStore::new(&policy_path);
        let mut state = PolicyState::new(3).expect("default threshold should be valid");
        state.mode = mode;
        policy.save(&state).expect("policy should persist");
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["hook"])
        .env("ICG_HEALTH_PATH", &health_path)
        .env("ICG_FAIL_CLOSED_POLICY", &policy_path)
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
