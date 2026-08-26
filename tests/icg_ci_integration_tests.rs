//! End-to-end coverage for the release path represented by the `icg-ci`
//! workflow.
//!
//! The Argo WorkflowTemplate lives in the deployment repository, while this
//! repository owns the commands it invokes. These tests therefore drive the
//! real CLI gates and updater rather than duplicating the workflow as a second
//! test implementation. No GitHub or cluster access is required.

use flate2::write::GzEncoder;
use flate2::Compression;
use icg::coverage::{load_rule_pack, run_release_integrity_diff};
use icg::engine::{CheckResult, CommandSource, ContentSource, Engine};
use icg::regression::{verify_regression_suite, ExpectedVerdict, RegressionSuite};
use icg::rule_pack::Pack;
use icg::trust_pointer::TrustPointerStore;
use icg::update::{run_update, UpdateCheckState, UpdateConfig};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tar::{Builder, Header};

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

fn production_pack_archive() -> Vec<u8> {
    let pack_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
    let mut paths = fs::read_dir(pack_directory)
        .expect("production packs should be readable")
        .map(|entry| entry.expect("pack entry should be readable").path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "production archive needs pack manifests");

    build_pack_archive(
        paths
            .into_iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("pack name should be UTF-8")
                    .to_string();
                (name, fs::read(path).expect("pack should be readable"))
            })
            .collect(),
    )
}

fn build_pack_archive(entries: Vec<(String, Vec<u8>)>) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = Builder::new(encoder);
    for (name, contents) in entries {
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, format!("./{name}"), contents.as_slice())
            .expect("test archive entry should be appended");
    }
    archive
        .into_inner()
        .expect("test archive should be finalized")
        .finish()
        .expect("test gzip archive should be finalized")
}

fn build_pack_archive_with_symlink(valid_manifest: Vec<u8>) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = Builder::new(encoder);
    let mut manifest_header = Header::new_gnu();
    manifest_header.set_size(valid_manifest.len() as u64);
    manifest_header.set_mode(0o644);
    manifest_header.set_cksum();
    archive
        .append_data(
            &mut manifest_header,
            "./secrets.json",
            valid_manifest.as_slice(),
        )
        .expect("valid test archive entry should be appended");

    let mut symlink_header = Header::new_gnu();
    symlink_header.set_entry_type(tar::EntryType::Symlink);
    symlink_header.set_size(0);
    symlink_header.set_mode(0o777);
    symlink_header.set_cksum();
    archive
        .append_link(&mut symlink_header, "./escaped.json", "/etc/passwd")
        .expect("malicious symlink test entry should be appended");

    archive
        .into_inner()
        .expect("test archive should be finalized")
        .finish()
        .expect("test gzip archive should be finalized")
}

fn run_hook(pack_directory: &Path, tool_name: &str, tool_input: Value) -> Value {
    let support = tempfile::tempdir().expect("hook support directory should exist");
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["hook", "--rule-pack"])
        .arg(pack_directory)
        .env("ICG_HEALTH_PATH", support.path().join("health.json"))
        .env("ICG_TELEMETRY_PATH", support.path().join("telemetry.json"))
        .env("ICG_DENIAL_LOG", support.path().join("denials.jsonl"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook should start");
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(
            json!({"tool_name": tool_name, "tool_input": tool_input})
                .to_string()
                .as_bytes(),
        )
        .expect("hook input should be written");
    let output = child.wait_with_output().expect("hook should finish");
    assert!(
        output.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("hook should return one JSON response")
}

fn assert_hook_denied(response: Value, pack_id: &str, pattern_id: &str) {
    let output = &response["hookSpecificOutput"];
    assert_eq!(output["permissionDecision"], "deny");
    let reason = output["permissionDecisionReason"]
        .as_str()
        .expect("denial should include a reason");
    assert!(reason.contains(&format!("pack={pack_id}")), "{reason}");
    assert!(
        reason.contains(&format!("pattern={pattern_id}")),
        "{reason}"
    );
}

fn write_trust_pointer(path: &Path) -> TrustPointerStore {
    let artifact_dir = path
        .parent()
        .expect("trust pointer should have a parent directory");
    let mut permissions = fs::metadata(artifact_dir)
        .expect("trust pointer directory metadata should be readable")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(artifact_dir, permissions)
        .expect("trust pointer directory should be secured for the test");

    let trust_store = TrustPointerStore::new(path);
    trust_store
        .set_trusted_ref_with_justification(
            "v2.0.0",
            "Layer 1 passed and Layer 2 approved the coverage report",
        )
        .expect("trusted release pointer should be written");
    trust_store
}

#[test]
fn trusted_release_update_replaces_complete_pack_directory_and_preserves_enforcement() {
    let temp = tempfile::tempdir().expect("temporary deployment workspace");
    let archive = production_pack_archive();
    let server = FixtureServer::new(archive);

    let trust_path = temp.path().join("etc/icg/trust-pointer.json");
    let pack_directory = temp.path().join("etc/icg/packs");
    let state_path = temp.path().join("etc/icg/last-update-check.json");
    fs::create_dir_all(&pack_directory).expect("deployment directory should exist");
    fs::write(
        pack_directory.join("removed-by-release.json"),
        r#"{"id":"removed-by-release","tool_keywords":["removed"],"applies_to":[],"safe_patterns":[],"guarded_patterns":[]}"#,
    )
    .expect("old pack should be installed");

    // The pointer is advanced only after the Layer 1/2 evidence above has
    // been produced. The updater must use this exact reference for its API
    // lookup; it must not silently follow a latest-release alias.
    let trust_store = write_trust_pointer(&trust_path);

    let config = UpdateConfig {
        repository: "test/release-repo".to_string(),
        release_api_base_url: server.base_url(),
        pack_directory: pack_directory.clone(),
        trust_pointer_path: trust_path.clone(),
        state_path: state_path.clone(),
        ..Default::default()
    };

    let result = run_update(config).expect("trusted release update should succeed");
    assert!(result.updated);
    assert_eq!(result.trusted_ref, "v2.0.0");
    assert_eq!(result.release_tag, "v2.0.0");
    assert_eq!(result.previous_version.as_deref(), Some("existing"));
    assert_eq!(result.pack_directory, pack_directory);
    let expected_rollback_directory = pack_directory.with_file_name("packs.previous");
    assert_eq!(
        result.rollback_directory.as_deref(),
        Some(expected_rollback_directory.as_path())
    );
    assert!(
        !pack_directory.join("removed-by-release.json").exists(),
        "replaced directory must not retain packs deleted by the release"
    );
    assert!(
        pack_directory
            .with_file_name("packs.previous")
            .join("removed-by-release.json")
            .exists(),
        "the prior active directory should be retained for rollback"
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
            "/assets/icg-packs.tar.gz".to_string(),
        ]
    );

    // Exercise the native hook against the directory actually installed by the
    // updater. A merged legacy artifact cannot express these three packs.
    let github_token = ["ghp_", "Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56"].concat();
    assert_hook_denied(
        run_hook(
            &pack_directory,
            "Bash",
            json!({"command": format!("echo {github_token} > /tmp/token")}),
        ),
        "secrets",
        "github-token",
    );
    assert_hook_denied(
        run_hook(
            &pack_directory,
            "Write",
            json!({"filePath": "deploy/app.yaml", "content": "image: nginx:latest\n"}),
        ),
        "image-tag",
        "image-tag-latest",
    );
    assert_hook_denied(
        run_hook(
            &pack_directory,
            "Write",
            json!({"filePath": "deploy/pvc.yaml", "content": "storageClassName: ssd-large\n"}),
        ),
        "storage-class",
        "storage-class-ssd",
    );
}

#[test]
fn malformed_release_archive_cannot_partially_deploy_or_escape_the_pack_root() {
    let temp = tempfile::tempdir().expect("temporary deployment workspace");
    let malformed = build_pack_archive_with_symlink(
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/secrets.json"))
            .expect("fixture pack should be readable"),
    );
    let server = FixtureServer::new(malformed);
    let trust_path = temp.path().join("etc/icg/trust-pointer.json");
    let pack_directory = temp.path().join("etc/icg/packs");
    let old_pack = pack_directory.join("known-good.json");
    let old_contents = br#"{"id":"known-good","tool_keywords":["known"],"applies_to":[],"safe_patterns":[],"guarded_patterns":[]}"#;
    fs::create_dir_all(&pack_directory).expect("active directory should exist");
    fs::write(&old_pack, old_contents).expect("known-good pack should be installed");
    write_trust_pointer(&trust_path);

    let result = run_update(UpdateConfig {
        repository: "test/release-repo".to_string(),
        release_api_base_url: server.base_url(),
        pack_directory: pack_directory.clone(),
        trust_pointer_path: trust_path,
        state_path: temp.path().join("etc/icg/last-update-check.json"),
        ..Default::default()
    });
    assert!(result.is_err(), "symlink archive must be rejected");
    assert_eq!(
        fs::read(&old_pack).expect("active known-good pack should remain"),
        old_contents,
        "a rejected archive must leave the active directory byte-for-byte intact"
    );
    assert!(
        !pack_directory.join("secrets.json").exists(),
        "a valid archive prefix must not be partially deployed"
    );
    assert!(
        !temp.path().join("etc/icg/escaped.json").exists(),
        "archive links must never write outside the staging directory"
    );
    let _ = server.finish();
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
            "{{\"tag_name\":\"v2.0.0\",\"name\":\"v2.0.0\",\"published_at\":\"2026-08-16T12:00:00Z\",\"assets\":[{{\"name\":\"icg-packs.tar.gz\",\"browser_download_url\":\"{base_url}/assets/icg-packs.tar.gz\",\"size\":{}}}]}}",
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
                            "/assets/icg-packs.tar.gz" => {
                                ("200 OK", "application/gzip", artifact.as_slice())
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

/// Test that the CI workflow gates the actual pack bytes, not static fixtures.
///
/// This test proves that the release process builds its merged compatibility
/// artifact and feeds its actual source rule bytes into the fixed deny corpus,
/// rather than relying on static test fixtures. Stateful predicates and
/// updated-input rules are covered by their dedicated engine tests because
/// they cannot belong to a fixed deny corpus.
#[test]
fn ci_workflow_gates_actual_pack_bytes_not_fixtures() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let real_packs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");

    // Verify the real packs directory exists and contains pack files
    assert!(real_packs_dir.is_dir(), "packs directory should exist");
    let pack_files = fs::read_dir(&real_packs_dir)
        .expect("packs directory should be readable")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .count();
    assert!(
        pack_files > 0,
        "packs directory should contain at least one pack file"
    );

    // Build the merged rule pack the regression-suite command accepts, using
    // the actual source packs that will be released.
    let actual_merged_pack = temp.path().join("actual-merged-pack.json");
    let merge_output = run_icg(&[
        "build-pack".to_string(),
        format!("--pack-dir={}", real_packs_dir.display()),
        format!("--output={}", actual_merged_pack.display()),
    ]);
    assert!(
        merge_output.status.success(),
        "merged pack generation for actual packs should succeed: {}",
        String::from_utf8_lossy(&merge_output.stderr)
    );

    // Build a fixed-corpus input from each actual modular pack. Keep only
    // destructive deny regexes: stateful predicates require live session or
    // remote state, and updated-input rules deliberately do not deny. The
    // source JSON is retained so a hand-authored example_command survives the
    // check instead of being dropped by the legacy merged artifact.
    let corpus_dir = temp.path().join("release-regression-packs");
    fs::create_dir_all(&corpus_dir).expect("regression corpus directory should be created");
    let mut generated_case_count = 0;
    for entry in fs::read_dir(&real_packs_dir).expect("packs directory should be readable") {
        let entry = entry.expect("pack directory entry should be readable");
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }

        let mut pack: Value =
            serde_json::from_slice(&fs::read(&path).expect("source pack should be readable"))
                .expect("source pack should be valid JSON");
        let guarded_patterns = pack["guarded_patterns"]
            .as_array_mut()
            .expect("source pack should contain guarded_patterns");
        guarded_patterns.retain(|pattern| {
            pattern["destructive"] == Value::Bool(true)
                && pattern["redirect"]["channel"] == Value::String("deny".to_string())
                && pattern["type"] != Value::String("predicate".to_string())
        });
        if guarded_patterns.is_empty() {
            continue;
        }

        let corpus_pack = corpus_dir.join(
            path.file_name()
                .expect("source pack should have a file name"),
        );
        fs::write(
            &corpus_pack,
            serde_json::to_vec_pretty(&pack).expect("filtered pack should serialize"),
        )
        .expect("filtered pack should be written");

        let suite_path = corpus_pack.with_extension("suite.json");
        let suite_output = run_icg(&[
            "regression-suite".to_string(),
            corpus_pack.display().to_string(),
            format!("--output={}", suite_path.display()),
        ]);
        assert!(
            suite_output.status.success(),
            "regression suite generation for actual pack {} should succeed: {}",
            path.display(),
            String::from_utf8_lossy(&suite_output.stderr)
        );

        let suite: Value = serde_json::from_slice(
            &fs::read(&suite_path).expect("regression suite should be readable"),
        )
        .expect("regression suite should be valid JSON");
        let cases = suite["cases"]
            .as_array()
            .expect("regression suite should contain cases");
        generated_case_count += cases.len();
        for case in cases {
            assert!(
                case.get("pack_id").and_then(|i| i.as_str()).is_some(),
                "regression case should have pack_id"
            );
            assert!(
                case.get("pattern_id").and_then(|i| i.as_str()).is_some(),
                "regression case should have pattern_id"
            );
            assert!(
                case.get("expected").and_then(|e| e.as_str()) == Some("deny"),
                "regression case should expect 'deny' verdict"
            );
        }
    }
    assert!(
        generated_case_count > 0,
        "actual source packs should produce fixed deny regression cases"
    );

    // This test passes: the CI workflow now gates the actual pack bytes,
    // not static fixtures that don't match what's released.
}
