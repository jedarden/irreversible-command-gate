//! Guards for the agent-scoped PATH-wrapper fallback deployment
//! (`scripts/deploy-path-wrappers.sh` + `scripts/icg-harness-env.sh`,
//! documented in `docs/operators/path-wrapper-fallback.md`).
//!
//! The fallback exists for harnesses with no blocking native PreToolUse
//! hook: pack `tool_keywords` become root-owned argv[0] symlinks under
//! `/usr/local/libexec/icg-wrappers`, and a launch wrapper prepends that
//! directory to `PATH` for the harness process tree only. These tests keep
//! the deployment honest about the parts that have broken or been assumed
//! before:
//!
//! - the wrapper directory is root-owned and never lands in a login shell;
//! - install/canary/verify/remove actually work end to end against a staged
//!   tree (every default path is overridable precisely for this);
//! - the canary denial probe uses a real *deny* rule -- `git push --force`
//!   is a rewrite (`updated_input`), so a force-push canary would silently
//!   test the wrong channel;
//! - removal is idempotent and preserves entries it does not own,
//!   including dangling symlinks an `[ -e ]` count would skip;
//! - the documented blind spots are real: absolute-path invocation and
//!   `ICG_DISABLED=1` both bypass the wrapper, and `kubectl` is never
//!   shadowed.
//!
//! A PATH wrapper is a fallback, not a native hook; the doc-disclaimer
//! tests below pin that language so coverage is never overstated.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The checkout under test.
///
/// Runtime cwd first, baked `CARGO_MANIFEST_DIR` only as a fallback: this
/// box's global cargo config points at a shared target dir, and a reused
/// test binary from another extraction must not read *that* tree's scripts
/// and packs (the stale-path failure that reopened irrevers-f231c112).
fn checkout() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() && cwd.join("packs").is_dir() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn deploy_script() -> PathBuf {
    checkout().join("scripts/deploy-path-wrappers.sh")
}

fn launcher_source() -> PathBuf {
    checkout().join("scripts/icg-harness-env.sh")
}

fn packs_dir() -> PathBuf {
    checkout().join("packs")
}

fn icg_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_icg"))
}

/// A staged deployment tree: overridable wrapper dir, launcher location,
/// and the real icg test binary, so the whole script runs unprivileged.
struct Stage {
    root: PathBuf,
}

impl Stage {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "icg-path-wrapper-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("bin")).expect("stage bin dir");
        Self { root }
    }

    fn wrapper_dir(&self) -> PathBuf {
        self.root.join("wrappers")
    }

    fn launcher(&self) -> PathBuf {
        self.root.join("bin").join("icg-harness-env")
    }

    fn deploy(&self, command: &str) -> Output {
        Command::new(deploy_script())
            .arg(command)
            .arg("--wrapper-dir")
            .arg(self.wrapper_dir())
            .arg("--icg")
            .arg(icg_binary())
            .arg("--pack-dir")
            .arg(packs_dir())
            .arg("--launcher")
            .arg(self.launcher())
            .env_remove("ICG_DISABLED")
            .env_remove("ICG_PRACTICE")
            .output()
            .expect("deploy script should run")
    }

    /// Run a command through the staged `git` wrapper symlink, in a
    /// directory that is not a git repository, with the staged pack
    /// directory loaded -- the same shape the canary probes use.
    fn wrapped_git(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        let cwd = self.root.join("neutral-cwd");
        fs::create_dir_all(&cwd).expect("neutral cwd");
        let mut command = Command::new(self.wrapper_dir().join("git"));
        command
            .args(args)
            .current_dir(&cwd)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.wrapper_dir().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("ICG_RULE_PACK", packs_dir())
            .env_remove("ICG_PRACTICE")
            .env_remove("ICG_DISABLED");
        for (key, value) in extra_env {
            command.env(key, value);
        }
        command.output().expect("wrapped git should run")
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn combined(output: &Output) -> String {
    format!("{}{}", stdout_of(output), stderr_of(output))
}

fn code_of(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

/// Resolve the real git binary the way a shell would, WITHOUT the wrapper
/// directory -- the absolute path an agent could invoke directly.
fn real_git_binary() -> PathBuf {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let candidate = Path::new(dir).join("git");
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if fs::metadata(&candidate)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
                {
                    return candidate;
                }
            }
            #[cfg(not(unix))]
            {
                return candidate;
            }
        }
    }
    panic!("no git binary in PATH; the fallback tests need one to shadow");
}

// ---------------------------------------------------------------------------
// script shape
// ---------------------------------------------------------------------------

#[test]
fn fallback_scripts_are_valid_and_executable() {
    for (path, shell) in [(deploy_script(), "bash"), (launcher_source(), "sh")] {
        let syntax = Command::new(shell)
            .arg("-n")
            .arg(&path)
            .output()
            .unwrap_or_else(|e| panic!("{shell} should run: {e}"));
        assert!(
            syntax.status.success(),
            "{} has a syntax error:\n{}",
            path.display(),
            String::from_utf8_lossy(&syntax.stderr)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)
                .unwrap_or_else(|e| panic!("{} should exist: {e}", path.display()))
                .permissions()
                .mode();
            assert!(
                mode & 0o111 != 0,
                "{} must be executable; it is meant to be run by hand on a host",
                path.display()
            );
        }
    }
}

#[test]
fn deploy_script_targets_the_root_owned_libexec_directory() {
    let script = fs::read_to_string(deploy_script()).expect("deploy script should be readable");
    assert!(
        script.contains("WRAPPER_DIR=\"/usr/local/libexec/icg-wrappers\""),
        "the default wrapper directory must be /usr/local/libexec/icg-wrappers, \
         not a user-writable location like /usr/local/bin"
    );
    assert!(
        script.contains("install -d -o root -g root -m 0755"),
        "the wrapper directory must be created root-owned: a wrapper the \
         guarded agent can rewrite is not a guard"
    );
}

/// PATH prepending must be launch-scoped. Neither script may touch a login
/// shell file, and the launcher must exec rather than export into a parent.
#[test]
fn fallback_never_writes_a_login_shell_and_launcher_execs() {
    let deploy = fs::read_to_string(deploy_script()).expect("deploy script should be readable");
    let launcher = fs::read_to_string(launcher_source()).expect("launcher should be readable");
    for script in [&deploy, &launcher] {
        for rc in [".bashrc", ".profile", ".zshrc", ".zshenv", "/etc/profile"] {
            assert!(
                !script.contains(rc),
                "the PATH-wrapper fallback must never write {rc}; the wrapper \
                 directory belongs in a harness launch environment only"
            );
        }
    }
    assert!(
        launcher.contains("exec \"$@\""),
        "icg-harness-env must exec the harness command; a launcher that runs \
         it as a child and exits would leak the modified PATH into nothing \
         and confuse process supervision"
    );
}

// ---------------------------------------------------------------------------
// doc claims the tests pin
// ---------------------------------------------------------------------------

#[test]
fn fallback_doc_exists_and_disclaims_hook_equivalence() {
    let doc = fs::read_to_string(checkout().join("docs/operators/path-wrapper-fallback.md"))
        .expect("docs/operators/path-wrapper-fallback.md should exist");
    assert!(
        doc.contains("not equivalent to a native pre-tool hook"),
        "the fallback doc must state plainly that wrapper coverage is not \
         equivalent to a native pre-tool hook"
    );
    for blind_spot in [
        "Absolute-path invocation",
        "Structured edits",
        "MCP and library calls",
        "ICG_DISABLED",
        "kubectl",
    ] {
        assert!(
            doc.contains(blind_spot),
            "the fallback doc must document the {blind_spot} blind spot"
        );
    }
    let index = fs::read_to_string(checkout().join("docs/operators/README.md"))
        .expect("operators README should be readable");
    assert!(
        index.contains("path-wrapper-fallback.md"),
        "docs/operators/README.md should link the fallback guide"
    );
}

// ---------------------------------------------------------------------------
// staged deployment lifecycle
// ---------------------------------------------------------------------------

#[test]
fn staged_install_verify_and_idempotency() {
    let stage = Stage::new("install");

    let install = stage.deploy("install");
    assert_eq!(
        code_of(&install),
        0,
        "staged install should succeed:\n{}",
        combined(&install)
    );

    let git_link = stage.wrapper_dir().join("git");
    assert!(git_link.is_symlink(), "git must be wrapped");
    let target = fs::read_link(&git_link).expect("git wrapper should be a readable symlink");
    // current_exe() canonicalizes (on this box /build is a symlink to
    // /data/build), so compare canonical forms, not strings.
    let canonical = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    assert_eq!(
        canonical(&target),
        canonical(&icg_binary()),
        "wrappers must point at the icg binary the deploy was told to use"
    );
    assert!(
        stage
            .wrapper_dir()
            .join("kubectl")
            .symlink_metadata()
            .is_err(),
        "kubectl must never be shadowed: it is intentionally outside the packs"
    );
    assert!(
        stage.launcher().is_file(),
        "the launch wrapper should be installed"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(stage.launcher()).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "the installed launcher must be executable"
        );
    }

    // Idempotent: a second install over the deployment succeeds and leaves
    // the same link set in place.
    let again = stage.deploy("install");
    assert_eq!(
        code_of(&again),
        0,
        "second install should succeed:\n{}",
        combined(&again)
    );
    assert!(
        git_link.is_symlink(),
        "git wrapper should survive reinstall"
    );

    let verify = stage.deploy("verify");
    assert_eq!(
        code_of(&verify),
        0,
        "verify should pass on a fresh staged install:\n{}",
        combined(&verify)
    );
    assert!(
        combined(&verify).contains("kubectl is not shadowed"),
        "verify should confirm kubectl is not shadowed"
    );

    let status = stage.deploy("status");
    assert_eq!(
        code_of(&status),
        0,
        "status should be read-only and succeed"
    );
}

#[test]
fn staged_install_refuses_foreign_entries_and_empty_packs() {
    let stage = Stage::new("refuse");
    let install = stage.deploy("install");
    assert_eq!(code_of(&install), 0, "setup install should succeed");

    fs::write(stage.wrapper_dir().join("foreign"), b"x").unwrap();
    let refused = stage.deploy("install");
    assert_ne!(
        code_of(&refused),
        0,
        "install must refuse a wrapper directory that holds a foreign entry"
    );
    assert!(
        stderr_of(&refused).contains("refusing to install"),
        "the refusal should say what it is refusing:\n{}",
        combined(&refused)
    );
    fs::remove_file(stage.wrapper_dir().join("foreign")).unwrap();

    // An empty pack set would deploy a directory that guards nothing.
    let empty_packs = stage.root.join("empty-packs");
    fs::create_dir_all(&empty_packs).unwrap();
    let empty = Command::new(deploy_script())
        .arg("install")
        .arg("--wrapper-dir")
        .arg(stage.wrapper_dir())
        .arg("--icg")
        .arg(icg_binary())
        .arg("--pack-dir")
        .arg(&empty_packs)
        .arg("--launcher")
        .arg(stage.launcher())
        .output()
        .expect("deploy script should run");
    assert_ne!(
        code_of(&empty),
        0,
        "install must refuse an empty pack directory"
    );
    assert!(
        combined(&empty).contains("no rule packs"),
        "the empty-pack refusal should explain itself:\n{}",
        combined(&empty)
    );
}

/// The five canaries through the real icg binary: reach-through exec,
/// enforced denial, practice pass-through, missing-target refusal, and
/// recursion refusal. This is the acceptance gate for enforcement.
#[test]
fn staged_canaries_pass() {
    let stage = Stage::new("canary");
    let install = stage.deploy("install");
    assert_eq!(code_of(&install), 0, "setup install should succeed");

    let canary = stage.deploy("canary");
    assert_eq!(
        code_of(&canary),
        0,
        "all canaries should pass before enforcement:\n{}",
        combined(&canary)
    );
    assert!(
        combined(&canary).contains("All canaries passed"),
        "canary output should say so explicitly"
    );
    // Self-cleaning: only the pack-derived symlinks remain.
    assert!(
        stage
            .wrapper_dir()
            .join("icgwrapcanary")
            .symlink_metadata()
            .is_err(),
        "the canary tool symlink should be gone after the run"
    );
}

#[test]
fn staged_remove_is_idempotent_and_preserves_unrelated_entries() {
    let stage = Stage::new("remove");
    let install = stage.deploy("install");
    assert_eq!(code_of(&install), 0, "setup install should succeed");
    let git_link = stage.wrapper_dir().join("git");
    assert!(git_link.is_symlink());

    // Counted BEFORE the unrelated entries are added: the report covers
    // exactly the icg-pointing wrappers, not everything in the directory.
    let ours_before = fs::read_dir(stage.wrapper_dir())
        .expect("wrapper dir should list")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_symlink())
        .count();
    assert!(
        ours_before > 0,
        "staged install should have produced wrapper symlinks"
    );

    // Unrelated entries: a regular file, and a DANGLING symlink -- the
    // case an `[ -e ]`-based count skips even though it still occupies
    // the directory (and would make rmdir fail).
    fs::write(stage.wrapper_dir().join("plainfile"), b"x").unwrap();
    std::os::unix::fs::symlink(
        stage.root.join("definitely-not-a-target"),
        stage.wrapper_dir().join("not-ours"),
    )
    .unwrap();
    // An unrelated launcher must not be clobbered by removal either.
    fs::write(stage.launcher(), b"#!/bin/sh\nexit 42\n").unwrap();

    let remove = stage.deploy("remove");
    assert_eq!(
        code_of(&remove),
        0,
        "remove should succeed with unrelated entries present:\n{}",
        combined(&remove)
    );
    assert!(
        combined(&remove).contains(&format!("removed {ours_before} wrapper symlink(s)")),
        "remove should report the wrappers it (via icg uninstall) removed, \
         not just its own sweep:\n{}",
        combined(&remove)
    );
    assert!(
        !git_link.symlink_metadata().is_ok(),
        "our git wrapper should be gone"
    );
    assert!(
        stage.wrapper_dir().join("plainfile").is_file(),
        "an unrelated regular file must be preserved"
    );
    assert!(
        stage.wrapper_dir().join("not-ours").is_symlink(),
        "an unrelated (dangling) symlink must be preserved"
    );
    assert!(
        stage.wrapper_dir().is_dir(),
        "a non-empty wrapper directory should be left in place, not forced"
    );
    assert!(
        stage.launcher().is_file(),
        "a launcher that differs from the checked-in source is not ours to \
         remove"
    );

    // Idempotent: a second remove is a clean no-op, and still does not
    // touch the unrelated launcher.
    let again = stage.deploy("remove");
    assert_eq!(code_of(&again), 0, "second remove should succeed");
    assert!(
        stage.launcher().is_file(),
        "second remove must not touch the unrelated launcher either"
    );
    assert!(
        stage.wrapper_dir().join("plainfile").is_file(),
        "second remove must still preserve unrelated entries"
    );
}

// ---------------------------------------------------------------------------
// launcher scoping
// ---------------------------------------------------------------------------

#[test]
fn launcher_scopes_path_and_practice_to_the_child_process_tree() {
    let stage = Stage::new("launcher");
    let install = stage.deploy("install");
    assert_eq!(code_of(&install), 0, "setup install should succeed");

    let probe = stage.root.join("probe.sh");
    fs::write(
        &probe,
        "#!/bin/sh\nprintf 'first=%s\\n' \"${PATH%%:*}\"\nprintf 'practice=%s\\n' \"${ICG_PRACTICE:-unset}\"\nprintf 'carried=%s\\n' \"$ICG_LAUNCHER_TEST_VAR\"\n",
    )
    .unwrap();
    fs::set_permissions(&probe, {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&probe).unwrap().permissions();
        perms.set_mode(0o755);
        perms
    })
    .unwrap();

    let practice = Command::new(stage.launcher())
        .arg("--practice")
        .arg(&probe)
        .env("ICG_WRAPPER_DIR", stage.wrapper_dir())
        .env("ICG_LAUNCHER_TEST_VAR", "kept")
        .env_remove("ICG_PRACTICE")
        .output()
        .expect("launcher should run");
    assert_eq!(
        code_of(&practice),
        0,
        "practice launch should succeed:\n{}",
        combined(&practice)
    );
    assert!(
        stderr_of(&practice).contains("PRACTICE mode"),
        "practice launch must announce that it is not enforcement"
    );
    let out = stdout_of(&practice);
    assert!(
        out.contains(&format!("first={}", stage.wrapper_dir().display())),
        "the child's PATH must start with the wrapper directory:\n{out}"
    );
    assert!(
        out.contains("practice=1"),
        "--practice must export ICG_PRACTICE=1 to the whole process tree"
    );
    assert!(
        out.contains("carried=kept"),
        "unrelated launch environment must be preserved"
    );

    let enforcing = Command::new(stage.launcher())
        .arg(&probe)
        .env("ICG_WRAPPER_DIR", stage.wrapper_dir())
        .env("ICG_LAUNCHER_TEST_VAR", "kept")
        .env_remove("ICG_PRACTICE")
        .output()
        .expect("launcher should run");
    assert_eq!(code_of(&enforcing), 0, "enforcing launch should succeed");
    assert!(
        stdout_of(&enforcing).contains("practice=unset"),
        "without --practice the child must NOT be in practice mode"
    );

    // The parent (this test process) PATH never contained the wrapper dir.
    let own = std::env::var("PATH").unwrap();
    assert!(
        !own.split(':')
            .any(|d| d == stage.wrapper_dir().to_str().unwrap()),
        "the launcher must not be able to mutate its parent's PATH"
    );
}

#[test]
fn launcher_refuses_to_launch_an_unguarded_tree() {
    let stage = Stage::new("refuse-launch");

    let missing = Command::new(launcher_source())
        .arg("true")
        .env("ICG_WRAPPER_DIR", stage.root.join("absent"))
        .output()
        .expect("launcher should run");
    assert_ne!(code_of(&missing), 0, "missing wrapper dir must refuse");
    assert!(
        stderr_of(&missing).contains("does not exist"),
        "refusal should name the problem"
    );

    let empty = stage.root.join("empty");
    fs::create_dir_all(&empty).unwrap();
    let empty_out = Command::new(launcher_source())
        .arg("true")
        .env("ICG_WRAPPER_DIR", &empty)
        .output()
        .expect("launcher should run");
    assert_ne!(
        code_of(&empty_out),
        0,
        "a wrapper directory with no symlinks guards nothing and must refuse"
    );
    assert!(
        stderr_of(&empty_out).contains("no wrapper symlinks"),
        "refusal should name the problem"
    );
}

#[test]
fn launcher_warns_when_the_guard_is_disabled_at_launch() {
    let stage = Stage::new("disabled");
    let install = stage.deploy("install");
    assert_eq!(code_of(&install), 0, "setup install should succeed");

    let out = Command::new(stage.launcher())
        .arg("true")
        .env("ICG_WRAPPER_DIR", stage.wrapper_dir())
        .env("ICG_DISABLED", "1")
        .output()
        .expect("launcher should run");
    assert_eq!(
        code_of(&out),
        0,
        "ICG_DISABLED is a documented escape hatch"
    );
    assert!(
        stderr_of(&out).contains("ICG_DISABLED"),
        "the launcher must warn when the guard will stand down"
    );
}

// ---------------------------------------------------------------------------
// blind spots, exercised mechanically
// ---------------------------------------------------------------------------

/// The pair that IS the absolute-path blind spot: the same would-be-denied
/// command is denied through PATH resolution and untouched when invoked by
/// absolute path.
#[test]
fn absolute_path_invocation_bypasses_the_wrapper() {
    let stage = Stage::new("abs");
    let install = stage.deploy("install");
    assert_eq!(code_of(&install), 0, "setup install should succeed");

    // Positive control: through the wrapper, a pathspec-less commit is
    // denied before exec.
    let wrapped = stage.wrapped_git(&["commit", "-m", "icg test"], &[]);
    assert_ne!(code_of(&wrapped), 0, "wrapped call should be denied");
    assert!(
        stderr_of(&wrapped).contains("command denied"),
        "expected an icg denial through the wrapper"
    );

    // Blind spot: the same command by absolute path never reaches icg.
    let cwd = stage.root.join("neutral-cwd");
    fs::create_dir_all(&cwd).unwrap();
    let direct = Command::new(real_git_binary())
        .args(["commit", "-m", "icg test"])
        .current_dir(&cwd)
        .env(
            "PATH",
            format!(
                "{}:{}",
                stage.wrapper_dir().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("ICG_RULE_PACK", packs_dir())
        .output()
        .expect("git should run");
    let all = combined(&direct);
    assert!(
        !all.contains("command denied"),
        "an absolute-path invocation must bypass the wrapper entirely"
    );
    assert!(
        all.contains("not a git repository"),
        "the real git must have run (its own error is the proof):\n{all}"
    );
}

/// `ICG_DISABLED=1` is the documented emergency bypass: the wrapper stands
/// down loudly and the real binary runs without rule evaluation.
#[test]
fn icg_disabled_bypasses_enforcement_loudly() {
    let stage = Stage::new("disabled-bypass");
    let install = stage.deploy("install");
    assert_eq!(code_of(&install), 0, "setup install should succeed");

    let bypassed = stage.wrapped_git(&["commit", "-m", "icg test"], &[("ICG_DISABLED", "1")]);
    let all = combined(&bypassed);
    assert!(
        !all.contains("command denied"),
        "ICG_DISABLED=1 must stand the guard down"
    );
    assert!(
        all.contains("ICG_DISABLED"),
        "the bypass must announce itself"
    );
    assert!(
        all.contains("not a git repository"),
        "the real git must still run (bypass, not break):\n{all}"
    );
}

/// A wrapper with no real binary later in PATH refuses explicitly instead
/// of recursing into itself, and a second icg symlink is skipped rather
/// than execed -- wrapper recursion is impossible by construction.
#[test]
fn wrapper_refuses_missing_targets_and_never_recurses() {
    let stage = Stage::new("recursion");
    let install = stage.deploy("install");
    assert_eq!(code_of(&install), 0, "setup install should succeed");

    let cwd = stage.root.join("neutral-cwd");
    fs::create_dir_all(&cwd).unwrap();

    // PATH contains only the wrapper directory: no real git exists later.
    let lonely = Command::new(stage.wrapper_dir().join("git"))
        .args(["--version"])
        .current_dir(&cwd)
        .env("PATH", stage.wrapper_dir().as_os_str())
        .env("ICG_RULE_PACK", packs_dir())
        .env_remove("ICG_PRACTICE")
        .output()
        .expect("wrapper should run");
    assert_ne!(code_of(&lonely), 0, "a missing target must not succeed");
    assert!(
        stderr_of(&lonely).contains("could not find the real"),
        "the refusal must be an explicit icg error, not a bare 127"
    );

    // A second icg symlink later in PATH is skipped, not execed; with no
    // real binary anywhere the result is the same explicit refusal -- and
    // crucially NOT a recursion or a hang.
    let second = stage.root.join("second-icg");
    fs::create_dir_all(&second).unwrap();
    std::os::unix::fs::symlink(icg_binary(), second.join("git")).unwrap();
    let skipped = Command::new(stage.wrapper_dir().join("git"))
        .args(["--version"])
        .current_dir(&cwd)
        .env(
            "PATH",
            format!("{}:{}", stage.wrapper_dir().display(), second.display()),
        )
        .env("ICG_RULE_PACK", packs_dir())
        .env_remove("ICG_PRACTICE")
        .output()
        .expect("wrapper should run");
    assert_ne!(code_of(&skipped), 0);
    assert!(
        stderr_of(&skipped).contains("could not find the real"),
        "a second icg symlink must be skipped, leaving the same explicit \
         refusal rather than a recursive exec loop"
    );
}
