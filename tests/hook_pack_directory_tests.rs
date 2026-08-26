use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;

const GITHUB_TOKEN: &str = "ghp_Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56"; // gitleaks:allow

fn run_hook(
    pack_directory: &Path,
    configure_with_environment: bool,
    tool_name: &str,
    tool_input: Value,
) -> Value {
    let support = tempdir().expect("hook support directory should exist");
    let pack_directory = pack_directory
        .to_str()
        .expect("pack directory should be valid UTF-8");
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command.arg("hook");
    if configure_with_environment {
        command.env("ICG_RULE_PACK", pack_directory);
    } else {
        command.args(["--rule-pack", pack_directory]);
    }

    let mut child = command
        .env("ICG_HEALTH_PATH", support.path().join("health.json"))
        .env("ICG_TELEMETRY_PATH", support.path().join("telemetry.json"))
        .env("ICG_DENIAL_LOG", support.path().join("denials.jsonl"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");

    let payload = json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
    });
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(payload.to_string().as_bytes())
        .expect("hook input should be written");

    let output = child
        .wait_with_output()
        .expect("hook process should finish");
    assert!(
        output.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("hook stdout should be one JSON object")
}

fn assert_denied_by(response: Value, pack_id: &str, pattern_id: &str) {
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

#[test]
fn hook_loads_unconditional_and_content_packs_from_a_directory() {
    // This is the installed production shape. A merged rule-pack.json omits
    // these empty-keyword packs because merging would lose their dispatch
    // semantics, so the hook must load each JSON manifest independently.
    let packs = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");

    let secret = run_hook(
        &packs,
        false,
        "Bash",
        json!({"command": format!("echo {GITHUB_TOKEN} > /tmp/token")}),
    );
    assert_denied_by(secret, "secrets", "github-token");

    let image = run_hook(
        &packs,
        false,
        "Write",
        json!({"filePath": "deploy/app.yaml", "content": "image: nginx:latest\n"}),
    );
    assert_denied_by(image, "image-tag", "image-tag-latest");

    let storage = run_hook(
        &packs,
        false,
        "Write",
        json!({"filePath": "deploy/pvc.yaml", "content": "storageClassName: ssd-large\n"}),
    );
    assert_denied_by(storage, "storage-class", "storage-class-ssd");

    // The Argo and container deployment shapes configure the same directory
    // through ICG_RULE_PACK rather than adding an argument to the hook.
    let configured = run_hook(
        &packs,
        true,
        "Bash",
        json!({"command": format!("echo {GITHUB_TOKEN} > /tmp/token")}),
    );
    assert_denied_by(configured, "secrets", "github-token");
}
