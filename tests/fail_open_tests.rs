use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run_hook(stdin: &[u8], rule_pack: Option<&Path>) -> Output {
    let telemetry_dir = tempfile::tempdir().expect("failed to create telemetry directory");
    let telemetry_path = telemetry_dir.path().join("telemetry.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command.arg("hook");
    command.env("ICG_TELEMETRY_PATH", telemetry_path);
    if let Some(rule_pack) = rule_pack {
        command.arg("--rule-pack").arg(rule_pack);
    }

    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start icg hook");
    child
        .stdin
        .take()
        .expect("hook stdin was not piped")
        .write_all(stdin)
        .expect("failed to write hook fixture");
    child
        .wait_with_output()
        .expect("failed to wait for icg hook")
}

#[test]
fn layer_one_malformed_stdin_fails_open() {
    let fixture = fs::read("tests/fixtures/malformed-stdin.json").unwrap();
    let output = run_hook(&fixture, None);

    assert!(
        output.status.success(),
        "malformed stdin must allow; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn layer_one_corrupt_rule_pack_fails_open() {
    let fixture = fs::read("tests/fixtures/corrupt-rule-pack.json").unwrap();
    let input = br#"{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/foo"}}"#;
    let output = run_hook(
        input,
        Some(Path::new("tests/fixtures/corrupt-rule-pack.json")),
    );

    assert!(
        output.status.success(),
        "corrupt rule pack must allow; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.is_empty());
}
