//! Guards for `install.sh`.
//!
//! The installer's whole reason to exist is that `icg hook` fails open: with
//! no readable pack directory it answers `{"permissionDecision":"allow"}` and
//! exits 0, so a half-finished install is indistinguishable from a working
//! one. These tests keep the parts that make it more than a copy command --
//! the self-test, the empty-pack refusal, and `--force` on wrapper removal.

use std::fs;
use std::path::Path;
use std::process::Command;

fn script() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh"))
        .expect("install.sh should be readable")
}

#[test]
fn install_script_is_valid_bash() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let output = Command::new("bash")
        .args(["-n", path.to_str().unwrap()])
        .output()
        .expect("bash should run");
    assert!(
        output.status.success(),
        "install.sh has a syntax error:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn install_script_is_executable() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
        let mode = fs::metadata(&path)
            .expect("install.sh exists")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "install.sh must be executable; curl | bash works either way but \
             `./install.sh` from a clone does not"
        );
    }
}

/// The installer must prove enforcement, not assume it.
#[test]
fn install_script_self_tests_the_hook_before_reporting_success() {
    let s = script();
    assert!(
        s.contains("probe_hook"),
        "install.sh should probe the hook rather than trusting the file copy"
    );
    assert!(
        s.contains(r#"*'"permissionDecision":"deny"'*"#),
        "install.sh must assert a destructive command is DENIED through the hook"
    );
    assert!(
        s.contains(r#"*'"permissionDecision":"allow"'*"#),
        "install.sh must assert an ordinary command is ALLOWED through the hook \
         -- a guard that blocks everything gets switched off"
    );
    assert!(
        s.contains("SELF-TEST FAILED"),
        "a failed probe must be an install failure, not a warning"
    );
}

/// Installing a binary with no packs yields a silent no-op guard.
#[test]
fn install_script_refuses_an_empty_pack_set() {
    let s = script();
    assert!(
        s.contains("no rule packs found in"),
        "install.sh must refuse when the pack source is empty"
    );
    assert!(
        s.contains("silently allows everything -- refusing"),
        "the refusal should say why an empty pack set is worse than no install"
    );
}

/// `icg install --uninstall` prompts on stdin without --force: in a script it
/// reads EOF, prints "Uninstall cancelled" and exits 0, leaving the symlinks
/// behind with no error. This bit the installer once.
#[test]
fn install_script_forces_wrapper_removal_and_verifies_it() {
    let s = script();
    let uninstall_line = s
        .lines()
        .find(|line| line.contains("install --dir \"$WRAPPER_DIR\" --uninstall"))
        .expect("install.sh should remove wrapper symlinks on --uninstall");
    assert!(
        uninstall_line.contains("--force"),
        "wrapper removal needs --force; without it the prompt reads EOF and \
         silently cancels: {uninstall_line}"
    );
    assert!(
        s.contains("still point at $BIN in"),
        "install.sh should verify the symlinks are actually gone rather than \
         trusting the exit code"
    );
}

/// Everything the guard owns must be root-owned, per the deployment model.
#[test]
fn install_script_installs_root_owned() {
    let s = script();
    for target in ["\"$BIN\"", "\"$PACK_DIR/\""] {
        assert!(
            s.contains("-o root -g root"),
            "install.sh must install {target} root-owned: a guard the agent can \
             rewrite is not a guard"
        );
    }
    assert!(
        !s.contains("chmod 777") && !s.contains("-m 0777"),
        "nothing the installer creates may be world-writable"
    );
}

/// `--practice` must register a hook that actually cannot block.
///
/// Introducing a guard to a live fleet non-enforcing is the whole reason the
/// flag exists; a `--practice` install that registered the enforcing command
/// would block 48 agents on lab the moment they restarted.
#[test]
fn practice_mode_registers_a_non_enforcing_hook() {
    let s = script();
    assert!(
        s.contains(r#"HOOK_COMMAND="$BIN hook --practice""#),
        "--practice must register `icg hook --practice`, not the enforcing form"
    );
    assert!(
        s.contains("it will block NOTHING"),
        "the installer should say plainly that a practice install enforces nothing"
    );
    // The self-test deliberately probes the ENFORCING path: a practice
    // deployment still needs proof the guard is capable of denying.
    assert!(
        s.contains(r#"| ICG_RULE_PACK="$PACK_DIR" "$BIN" hook 2>/dev/null"#),
        "the self-test must probe `icg hook` without --practice, so a practice \
         install still proves the guard can deny"
    );
}
