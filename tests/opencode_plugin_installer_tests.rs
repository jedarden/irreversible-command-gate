//! End-to-end tests for `icg install-opencode-plugin` (src/opencode_plugin.rs).
//!
//! The module's unit tests pin the embedded artifact's shape; the node
//! suite in `opencode-plugin/test/` pins the plugin's gate semantics. These
//! drive the shipped binary instead, so the acceptance criteria hold for
//! the command an operator actually types: a fresh install into a missing
//! plugin directory; idempotence (a re-run is a no-op, byte-identical);
//! a stale ICG copy replaced with a one-shot `.icg-backup`; a foreign file
//! refused in both directions (never clobbered on install, never removed
//! on uninstall); the `--file` / `--project-dir` / global-default path
//! derivation under explicit `XDG_CONFIG_HOME` and `HOME`; and the
//! guarantee that nothing but the plugin file (and its one backup) is
//! written — OpenCode's own configuration files are never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::tempdir;

/// The installed `icg` binary under test.
fn icg(args: &[&str]) -> Output {
    icg_with_env(args, &[])
}

/// Run the binary with extra `KEY=VALUE` environment overrides.
fn icg_with_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("icg should run")
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} should succeed (exit {:?}):\n{}{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn assert_failure(output: &Output, context: &str) {
    assert!(
        !output.status.success(),
        "{context} should fail, but exited 0:\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}

/// The plugin file as this workspace ships it — the installer's embedded
/// bytes must be exactly these.
fn shipped_plugin() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("opencode-plugin/icg.ts");
    fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "shipped plugin at {} must be readable: {error}",
            path.display()
        )
    })
}

#[test]
fn fresh_install_deploys_the_shipped_plugin_bytes() {
    let dir = tempdir().expect("tempdir");
    let target = dir.path().join("plugin/icg.ts");

    let output = icg(&[
        "install-opencode-plugin",
        "--file",
        target.to_str().unwrap(),
    ]);
    assert_success(&output, "fresh install");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("created"),
        "fresh install reports created: {stdout}"
    );

    let deployed = fs::read_to_string(&target).expect("deployed plugin readable");
    assert_eq!(
        deployed,
        shipped_plugin(),
        "deployed bytes are the shipped bytes"
    );
    // The pinned registration facts, on the bytes an operator actually got:
    assert!(
        deployed.contains("@icg-opencode-plugin v1"),
        "content marker present"
    );
    assert!(
        deployed.contains("\"tool.execute.before\""),
        "pinned 1.18.29 gate hook"
    );
    assert!(
        deployed.contains("/usr/local/bin/icg"),
        "the root-owned absolute icg path, never a PATH-dependent invocation"
    );
    assert!(
        deployed.contains("\"hook\", \"--harness\", \"open-code\""),
        "the adapter invocation is pinned to the spelling every shipped icg \
         accepts (0.1.62 takes only the derived kebab-case; later builds keep \
         it as an alias)"
    );
    assert!(
        target.parent().unwrap().is_dir(),
        "missing plugin directory created"
    );
}

#[test]
fn reinstall_is_a_byte_level_no_op() {
    let dir = tempdir().expect("tempdir");
    let target = dir.path().join("icg.ts");
    assert_success(
        &icg(&[
            "install-opencode-plugin",
            "--file",
            target.to_str().unwrap(),
        ]),
        "first install",
    );

    let before = fs::read(&target).expect("target readable");
    let metadata_before = fs::metadata(&target)
        .expect("target metadata")
        .modified()
        .unwrap();

    let output = icg(&[
        "install-opencode-plugin",
        "--file",
        target.to_str().unwrap(),
    ]);
    assert_success(&output, "reinstall");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("already up to date"),
        "reinstall reports already current: {}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert_eq!(
        fs::read(&target).expect("target readable"),
        before,
        "bytes untouched"
    );
    assert_eq!(
        fs::metadata(&target)
            .expect("target metadata")
            .modified()
            .unwrap(),
        metadata_before,
        "a no-op does not even rewrite the file"
    );
    assert!(
        !dir.path().join("icg.ts.icg-backup").exists(),
        "no backup for a no-op"
    );
}

#[test]
fn stale_icg_copy_is_replaced_with_a_oneshot_backup() {
    let dir = tempdir().expect("tempdir");
    let target = dir.path().join("icg.ts");
    // A realistic stale copy: the marker still names it ICG-owned, but a
    // hand-edit drifted the stall cap from the shipped bytes.
    let stale = shipped_plugin().replacen("10_000", "30_000", 1);
    assert_ne!(stale, shipped_plugin(), "the drift must change the bytes");
    fs::write(&target, &stale).expect("stale copy written");

    let output = icg(&[
        "install-opencode-plugin",
        "--file",
        target.to_str().unwrap(),
    ]);
    assert_success(&output, "update over stale ICG copy");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("updated"),
        "stale copy reports updated: {stdout}"
    );
    assert!(
        stdout.contains(".icg-backup"),
        "the backup is announced: {stdout}"
    );

    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        shipped_plugin(),
        "target replaced"
    );
    let backup = dir.path().join("icg.ts.icg-backup");
    assert_eq!(
        fs::read_to_string(&backup).unwrap(),
        stale,
        "backup holds the prior bytes"
    );

    // The backup is one-shot: a second update (another divergent ICG copy)
    // replaces the target again but leaves the FIRST backup intact.
    let second_stale = shipped_plugin().replacen("10_000", "45_000", 1);
    fs::write(&target, &second_stale).expect("second stale copy written");
    assert_success(
        &icg(&[
            "install-opencode-plugin",
            "--file",
            target.to_str().unwrap(),
        ]),
        "second update",
    );
    assert_eq!(
        fs::read_to_string(&backup).unwrap(),
        stale,
        "the first backup is never overwritten"
    );
}

#[test]
fn foreign_plugin_is_refused_on_install_and_uninstall() {
    let dir = tempdir().expect("tempdir");
    let target = dir.path().join("icg.ts");
    let foreign = "// someone else's OpenCode plugin\nexport default async () => ({});\n";
    fs::write(&target, foreign).expect("foreign plugin written");

    let install = icg(&[
        "install-opencode-plugin",
        "--file",
        target.to_str().unwrap(),
    ]);
    assert_failure(&install, "install over a foreign plugin");
    assert!(
        String::from_utf8_lossy(&install.stderr).contains("foreign plugin"),
        "the refusal names the reason: {}",
        String::from_utf8_lossy(&install.stderr),
    );
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        foreign,
        "foreign file untouched"
    );

    let uninstall = icg(&[
        "install-opencode-plugin",
        "--file",
        target.to_str().unwrap(),
        "--uninstall",
    ]);
    assert_failure(&uninstall, "uninstall of a foreign plugin");
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        foreign,
        "foreign file still untouched"
    );
}

#[test]
fn uninstall_removes_only_marker_bearing_targets() {
    let dir = tempdir().expect("tempdir");
    let target = dir.path().join("icg.ts");
    assert_success(
        &icg(&[
            "install-opencode-plugin",
            "--file",
            target.to_str().unwrap(),
        ]),
        "install",
    );

    let output = icg(&[
        "install-opencode-plugin",
        "--file",
        target.to_str().unwrap(),
        "--uninstall",
    ]);
    assert_success(&output, "uninstall");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Removed"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!target.exists(), "the plugin file is gone");

    // Uninstalling an absent target is a clean no-op, not an error.
    let again = icg(&[
        "install-opencode-plugin",
        "--file",
        target.to_str().unwrap(),
        "--uninstall",
    ]);
    assert_success(&again, "uninstall when not installed");
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("not installed"),
        "{}",
        String::from_utf8_lossy(&again.stdout),
    );
}

#[test]
fn project_dir_derives_the_dot_opencode_plugin_path() {
    let dir = tempdir().expect("tempdir");
    let output = icg(&[
        "install-opencode-plugin",
        "--project-dir",
        dir.path().to_str().unwrap(),
    ]);
    assert_success(&output, "project-dir install");
    let target = dir.path().join(".opencode/plugin/icg.ts");
    assert!(target.exists(), "the project channel target is written");
    assert_eq!(fs::read_to_string(&target).unwrap(), shipped_plugin());
}

#[test]
fn global_default_follows_xdg_config_home_then_home() {
    let xdg = tempdir().expect("xdg tempdir");
    let output = icg_with_env(
        &["install-opencode-plugin"],
        &[
            ("XDG_CONFIG_HOME", xdg.path().to_str().unwrap()),
            ("HOME", "/nonexistent-icg-test"),
        ],
    );
    assert_success(&output, "XDG_CONFIG_HOME install");
    let target = xdg.path().join("opencode/plugin/icg.ts");
    assert!(
        target.exists(),
        "the global channel target lands under XDG_CONFIG_HOME"
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), shipped_plugin());

    // Without XDG_CONFIG_HOME the resolution falls back to $HOME/.config.
    let home = tempdir().expect("home tempdir");
    let output = icg_with_env(
        &["install-opencode-plugin"],
        &[
            ("XDG_CONFIG_HOME", ""),
            ("HOME", home.path().to_str().unwrap()),
        ],
    );
    assert_success(&output, "HOME fallback install");
    let target = home.path().join(".config/opencode/plugin/icg.ts");
    assert!(
        target.exists(),
        "the global channel target lands under HOME/.config"
    );
}

#[test]
fn install_touches_nothing_but_the_plugin_file() {
    // The "does not modify OpenCode's permission configuration" guarantee,
    // at the filesystem level: every config file OpenCode could resolve in
    // the target's vicinity must survive byte-identical.
    let dir = tempdir().expect("tempdir");
    let plugin_dir = dir.path().join("plugin");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");

    let config = "{ \"$schema\": \"https://opencode.ai/config.json\",\n  \"permission\": { \"bash\": \"allow\" }\n}\n";
    let configc = "// jsonc commentary\n{ \"small_model\": \"beep\" }\n";
    let package = "{ \"dependencies\": { \"@opencode-ai/plugin\": \"1.18.29\" } }\n";
    for (name, contents) in [
        ("opencode.json", config),
        ("opencode.jsonc", configc),
        ("package.json", package),
    ] {
        fs::write(dir.path().join(name), contents).expect("config written");
    }
    let neighbor = plugin_dir.join("someone-else.ts");
    fs::write(&neighbor, "// neighbor plugin\n").expect("neighbor written");

    let output = icg(&[
        "install-opencode-plugin",
        "--file",
        plugin_dir.join("icg.ts").to_str().unwrap(),
    ]);
    assert_success(&output, "install beside config files");

    assert_eq!(
        fs::read_to_string(dir.path().join("opencode.json")).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("opencode.jsonc")).unwrap(),
        configc
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        package
    );
    assert_eq!(
        fs::read_to_string(&neighbor).unwrap(),
        "// neighbor plugin\n"
    );
    // Exactly one new file, no stray siblings beyond the neighbor:
    let entries: Vec<PathBuf> = fs::read_dir(&plugin_dir)
        .expect("plugin dir readable")
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    assert_eq!(
        entries.len(),
        2,
        "only icg.ts added beside the neighbor: {entries:?}"
    );
}

#[test]
fn global_uninstall_is_driven_by_the_derived_target_not_a_hardcoded_path() {
    // A guard for the operator path: the default target under a redirected
    // XDG_CONFIG_HOME uninstalls cleanly, proving the command is driven by
    // its derived target (and, on a developer box, that nobody needs to
    // point it at the real ~/.config to exercise removal).
    let xdg = tempdir().expect("xdg tempdir");
    let env = [
        ("XDG_CONFIG_HOME", xdg.path().to_str().unwrap()),
        ("HOME", "/nonexistent-icg-test"),
    ];
    assert_success(
        &icg_with_env(&["install-opencode-plugin"], &env),
        "install into scratch XDG",
    );

    let output = icg_with_env(&["install-opencode-plugin", "--uninstall"], &env);
    assert_success(&output, "uninstall from scratch XDG");
    assert!(
        !xdg.path().join("opencode/plugin/icg.ts").exists(),
        "the derived global target is removed"
    );
}
