//! End-to-end coverage for the release path represented by the `icg-ci`
//! workflow.
//!
//! The Argo WorkflowTemplate lives in the deployment repository, while this
//! repository owns the commands it invokes. These tests therefore drive the
//! real CLI gates and updater rather than duplicating the workflow as a second
//! test implementation. No GitHub or cluster access is required.

use icg::coverage::{load_rule_pack, run_release_integrity_diff};
use icg::engine::{CheckResult, CommandSource, ContentSource, Engine};
use icg::regression::{verify_regression_suite, ExpectedVerdict, RegressionSuite};
use icg::rule_pack::Pack;
use icg::trust_pointer::TrustPointerStore;
use icg::update::{run_update, UpdateCheckState, UpdateConfig};
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn run_icg(args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg binary should start")
}

fn assert_denies_generated_cases(pack: Pack, suite: &RegressionSuite) {
    verify_regression_suite(&pack, suite).expect("generated suite must pass its structural gate");

    let mut engine = Engine::new();
    engine
        .load_pack(pack)
        .expect("release pack should load in the runtime engine");

    for case in &suite.cases {
        let result = match (&case.file_path, &case.content) {
            (Some(file_path), Some(content)) => engine.evaluate_content(&ContentSource::Write {
                file_path: file_path.clone(),
                content: content.clone(),
            }),
            (None, None) => engine.evaluate_command(&CommandSource::Hook(case.command.clone())),
            _ => panic!(
                "regression case '{}' must contain either command or content input",
                case.pattern_id
            ),
        };

        assert!(
            matches!(
                result,
                CheckResult::Denied { ref pattern_id, .. } if pattern_id == &case.pattern_id
            ),
            "regression case '{}' did not deny through the runtime engine: {:?}",
            case.pattern_id,
            result
        );
        assert_eq!(case.expected, ExpectedVerdict::Deny);
    }
}

#[test]
fn icg_ci_release_candidate_runs_both_layer_one_gates_and_emits_layer_two_report() {
    let temp = tempfile::tempdir().expect("temporary CI workspace");
    let previous = fixture("previous-release.json");
    let current = fixture("current-release-clean.json");
    let regression = fixture("current-release-regression.json");
    let suite_path = temp.path().join("artifacts/regression-suite.json");

    // This is the fixed deny-regression stage of Layer 1. It generates the
    // artifact the workflow would publish, then exercises every case against
    // the actual current release pack.
    let suite_output = run_icg(&[
        "regression-suite".to_string(),
        current.display().to_string(),
        "--output".to_string(),
        suite_path.display().to_string(),
    ]);
    assert!(
        suite_output.status.success(),
        "regression gate failed: {}",
        String::from_utf8_lossy(&suite_output.stderr)
    );
    let suite: RegressionSuite =
        serde_json::from_slice(&fs::read(&suite_path).expect("suite artifact should exist"))
            .expect("suite artifact should be valid JSON");
    assert_denies_generated_cases(
        load_rule_pack(current.clone()).expect("current fixture should load"),
        &suite,
    );

    // The clean candidate produces the stable coverage-diff/v1 review input
    // with all four coverage sections present.
    let clean_diff = run_icg(&[
        "coverage-diff".to_string(),
        previous.display().to_string(),
        current.display().to_string(),
    ]);
    assert!(
        clean_diff.status.success(),
        "clean coverage gate failed: {}",
        String::from_utf8_lossy(&clean_diff.stderr)
    );
    let clean_report = String::from_utf8(clean_diff.stdout).expect("report should be UTF-8");
    for marker in [
        "format: coverage-diff/v1",
        "status: no_regressions",
        "## Removed guarded_patterns",
        "## Disabled guarded_patterns",
        "## Widened safe_patterns",
        "## Narrowed guarded_patterns (destructive: true)",
        "justification: not required",
    ] {
        assert!(clean_report.contains(marker), "report missing `{marker}`");
    }

    // A weakened candidate must stop the release before a human review can
    // approve it: the fixed suite is rejected and the coverage gate exits 2
    // until a non-blank Layer 2 rationale is supplied.
    let rejected_suite = run_icg(&[
        "regression-suite".to_string(),
        regression.display().to_string(),
    ]);
    assert!(
        !rejected_suite.status.success(),
        "weakened candidate unexpectedly generated a passing suite"
    );

    let rejected_diff = run_icg(&[
        "coverage-diff".to_string(),
        previous.display().to_string(),
        regression.display().to_string(),
    ]);
    assert_eq!(
        rejected_diff.status.code(),
        Some(2),
        "coverage regressions without rationale must block release: {}",
        String::from_utf8_lossy(&rejected_diff.stderr)
    );
    let rejected_report =
        String::from_utf8(rejected_diff.stdout).expect("rejected report should be UTF-8");
    assert!(rejected_report.contains("status: regressions_detected"));
    assert!(rejected_report.contains("justification: REQUIRED"));
    assert!(rejected_report.contains("pattern_id: vault-policy-delete"));
    assert!(rejected_report.contains("pattern_id: git-force-push"));

    // The explicit rationale is the hand-off to the Layer 2 reviewer. The
    // report remains the same structured evidence, now bound to a decision.
    let approved_diff = run_icg(&[
        "coverage-diff".to_string(),
        previous.display().to_string(),
        regression.display().to_string(),
        "--justification".to_string(),
        "Reviewed deprecation and migration coverage for the candidate.".to_string(),
    ]);
    assert!(
        approved_diff.status.success(),
        "reviewed diff should pass the explicit-rationale gate: {}",
        String::from_utf8_lossy(&approved_diff.stderr)
    );
    let approved_report =
        String::from_utf8(approved_diff.stdout).expect("approved report should be UTF-8");
    assert!(approved_report
        .contains("justification: Reviewed deprecation and migration coverage for the candidate."));

    // Keep the library result in this test as a direct assertion that the
    // report and the release-gate decision are backed by the same manifests.
    let diff = run_release_integrity_diff(previous, current, None, None)
        .expect("release integrity diff should be reproducible in-process");
    assert!(!diff.has_regressions());
}

#[test]
fn trusted_release_update_replaces_artifact_and_deploys_the_trusted_pack() {
    let temp = tempfile::tempdir().expect("temporary deployment workspace");
    let previous_bytes = fs::read(fixture("previous-release.json")).expect("previous fixture");
    let current_bytes = fs::read(fixture("current-release-clean.json")).expect("current fixture");
    let server = FixtureServer::new(current_bytes.clone());

    let trust_path = temp.path().join("etc/icg/trust-pointer.json");
    let artifact_path = temp.path().join("etc/icg/rule-pack.json");
    let state_path = temp.path().join("etc/icg/last-update-check.json");
    fs::create_dir_all(artifact_path.parent().expect("artifact parent"))
        .expect("deployment directory should exist");
    fs::write(&artifact_path, &previous_bytes).expect("old artifact should be installed");

    // The pointer is advanced only after the Layer 1/2 evidence above has
    // been produced. The updater must use this exact reference for its API
    // lookup; it must not silently follow a latest-release alias.
    let trust_store = TrustPointerStore::new(&trust_path);
    trust_store
        .set_trusted_ref_with_justification(
            "v2.0.0",
            "Layer 1 passed and Layer 2 approved the coverage report",
        )
        .expect("trusted release pointer should be written");

    let mut config = UpdateConfig::default();
    config.repository = "test/release-repo".to_string();
    config.release_api_base_url = server.base_url();
    config.artifact_path = artifact_path.clone();
    config.trust_pointer_path = trust_path.clone();
    config.state_path = state_path.clone();

    let result = run_update(config).expect("trusted release update should succeed");
    assert!(result.updated);
    assert_eq!(result.trusted_ref, "v2.0.0");
    assert_eq!(result.release_tag, "v2.0.0");
    assert_eq!(result.previous_version.as_deref(), Some("existing"));
    assert_eq!(
        fs::read(&artifact_path).expect("deployed artifact"),
        current_bytes
    );
    assert!(
        !artifact_path.with_extension("tmp").exists(),
        "temporary artifact must be renamed away"
    );

    let state = UpdateCheckState::load(&state_path)
        .expect("update state should be readable")
        .expect("successful update should record state");
    assert_eq!(state.release_tag, "v2.0.0");
    assert_eq!(state.trusted_ref, "v2.0.0");
    assert_eq!(
        trust_store
            .get_trusted_ref()
            .expect("trust pointer should remain readable"),
        Some("v2.0.0".to_string())
    );

    let requested_paths = server.finish();
    assert_eq!(
        requested_paths,
        vec![
            "/repos/test/release-repo/releases/tags/v2.0.0".to_string(),
            "/assets/rule-pack.json".to_string(),
        ]
    );

    // Finally, load the exact artifact the updater installed and run the
    // runtime checks. This closes the trust-pointer -> artifact -> deployed
    // rule-pack path instead of stopping at a successful file download.
    let deployed_pack = load_rule_pack(artifact_path).expect("deployed pack should load");
    assert_eq!(deployed_pack.id, "test-pack-current-clean");
    let mut engine = Engine::new();
    engine
        .load_pack(deployed_pack)
        .expect("deployed pack should be accepted by the engine");
    assert!(matches!(
        engine.evaluate_command(&CommandSource::Hook(
            "vault kv destroy secret/example".to_string()
        )),
        CheckResult::Denied { ref pattern_id, .. } if pattern_id == "vault-kv-destroy"
    ));
    assert!(matches!(
        engine.evaluate_content(&ContentSource::Write {
            file_path: "deploy/app.yaml".to_string(),
            content: "image: example:latest".to_string(),
        }),
        CheckResult::Denied { ref pattern_id, .. } if pattern_id == "image-tag-latest"
    ));
}

/// Small deterministic HTTP fixture for the two requests made by the updater:
/// the release metadata lookup and the release asset download.
struct FixtureServer {
    base_url: String,
    stop: Option<Sender<()>>,
    join: Option<JoinHandle<Vec<String>>>,
}

impl FixtureServer {
    fn new(artifact: Vec<u8>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("fixture server should bind");
        listener
            .set_nonblocking(true)
            .expect("fixture server should be non-blocking");
        let address = listener.local_addr().expect("fixture server address");
        let base_url = format!("http://{address}");
        let release_body = format!(
            "{{\"tag_name\":\"v2.0.0\",\"name\":\"v2.0.0\",\"published_at\":\"2026-08-16T12:00:00Z\",\"assets\":[{{\"name\":\"rule-pack.json\",\"browser_download_url\":\"{base_url}/assets/rule-pack.json\",\"size\":{}}}]}}",
            artifact.len()
        )
        .into_bytes();
        let (stop, stop_receiver) = mpsc::channel();

        let join = thread::spawn(move || {
            let mut requested_paths = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(5);

            while Instant::now() < deadline {
                if stop_receiver.try_recv().is_ok() {
                    break;
                }

                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let path = read_request_path(&mut stream);
                        let (status, content_type, body) = match path.as_str() {
                            "/repos/test/release-repo/releases/tags/v2.0.0" => {
                                ("200 OK", "application/json", release_body.as_slice())
                            }
                            "/assets/rule-pack.json" => {
                                ("200 OK", "application/json", artifact.as_slice())
                            }
                            _ => ("404 Not Found", "text/plain", b"not found".as_slice()),
                        };
                        let headers = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        stream
                            .write_all(headers.as_bytes())
                            .and_then(|_| stream.write_all(body))
                            .expect("fixture response should be written");
                        stream
                            .shutdown(Shutdown::Both)
                            .expect("fixture connection should close");
                        requested_paths.push(path);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture server accept failed: {error}"),
                }
            }

            requested_paths
        });

        Self {
            base_url,
            stop: Some(stop),
            join: Some(join),
        }
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn finish(mut self) -> Vec<String> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.join
            .take()
            .expect("fixture server thread")
            .join()
            .expect("fixture server should exit cleanly")
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn read_request_path(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(error) => panic!("fixture request read failed: {error}"),
        }
    }

    String::from_utf8_lossy(&request)
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_string()
}
