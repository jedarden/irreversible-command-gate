use std::fs;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

fn icg_with_stdin(args: &[&str], input: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = command.spawn().expect("icg should run");
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().expect("icg should finish")
}

#[test]
fn documented_check_explain_and_coverage_commands_work() {
    let coverage = icg(&["coverage", "--list"]);
    assert!(coverage.status.success());
    let coverage_stdout = String::from_utf8_lossy(&coverage.stdout);
    assert!(coverage_stdout.contains("image-tag"));
    assert!(coverage_stdout.contains("storage-class"));

    let check = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/storage-class.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"claim.yaml","content":"storageClassName: ssd-large\n"}}"#,
    );
    assert!(check.status.success());
    assert!(String::from_utf8_lossy(&check.stdout).contains("DENIED"));

    let explain = icg(&[
        "explain",
        "--pattern",
        "storage-class-ssd",
        "--pack",
        "packs/storage-class.json",
        "--show-regex",
    ]);
    assert!(explain.status.success());
    let explain_stdout = String::from_utf8_lossy(&explain.stdout);
    assert!(explain_stdout.contains("SSD storage classes are prohibited"));
    assert!(explain_stdout.contains("Regex:"));
}

#[test]
fn documented_backup_and_bug_report_commands_work() {
    let directory = tempdir().unwrap();
    let archive = directory.path().join("icg-backup.tar.gz");
    let backup = icg(&[
        "backup",
        "create",
        "--output",
        archive.to_str().unwrap(),
        "--source",
        "packs/image-tag.json",
    ]);
    assert!(
        backup.status.success(),
        "{}",
        String::from_utf8_lossy(&backup.stderr)
    );

    let verify = icg(&["backup", "verify", archive.to_str().unwrap()]);
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    assert!(String::from_utf8_lossy(&verify.stdout).contains("verified successfully"));

    let report = directory.path().join("bug-report.txt");
    let bug_report = icg(&["bug-report", "--output", report.to_str().unwrap()]);
    assert!(bug_report.status.success());
    let report_text = fs::read_to_string(report).unwrap();
    assert!(report_text.contains("icg bug report"));
    assert!(report_text.contains("command contents"));
}

#[test]
fn documented_override_workflow_creates_and_lists_release_bound_artifact() {
    let directory = tempdir().unwrap();
    let request = directory.path().join("request.json");
    let overrides = directory.path().join("overrides");

    let create = icg(&[
        "override",
        "create",
        "--repo",
        "jedarden/example",
        "--pattern-id",
        "image-tag-bare-sha",
        "--justification",
        "approved build-system exception",
        "--output",
        request.to_str().unwrap(),
    ]);
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );

    let approve = icg(&[
        "override",
        "approve",
        "--request",
        request.to_str().unwrap(),
        "--approver",
        "security-team",
        "--expiration",
        "2026-12-31",
        "--release-ref",
        "v1.2.3",
        "--pack",
        "packs/image-tag.json",
        "--output-dir",
        overrides.to_str().unwrap(),
    ]);
    assert!(
        approve.status.success(),
        "{}",
        String::from_utf8_lossy(&approve.stderr)
    );

    let list = icg(&[
        "override",
        "list",
        "--directory",
        overrides.to_str().unwrap(),
    ]);
    assert!(list.status.success());
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(list_stdout.contains("jedarden/example"));
    assert!(list_stdout.contains("image-tag-bare-sha"));
    assert!(overrides.join("example-image-tag-bare-sha.toml").is_file());
}
