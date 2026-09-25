//! Guards for the `systemd/` scaffolding, landed with bead `irrevers-ccb71837`.
//!
//! `092e82c` deleted a unit's script from the repo while the installed copy
//! of the unit lived on in `~/.config/systemd/user/`, and its timer woke every
//! five minutes to fail `203/EXEC` for a week. The scaffolding makes that
//! drift mechanically detectable; these tests keep the scaffolding honest:
//! the scripts must parse, must stay executable, the README must state the
//! same-commit invariant, and `check-consistency.sh` must actually flag every
//! drift direction (against fixtures — the CI runner has no
//! `~/.config/systemd/user`, which is why the script takes `--repo-only` and
//! directory overrides): a tracked unit pointing at a missing path, a host
//! unit executing a missing repo path, a dangling link into the repo, and —
//! the direction that closes the loop on `irrevers-833f9353` — a tracked
//! unit installed as anything other than install.sh's symlink to the
//! tracked file, which stays invisible to the path checks for as long as
//! every path it references still exists.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn systemd_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("systemd")
}

fn script_path(name: &str) -> PathBuf {
    systemd_dir().join(name)
}

fn scripts() -> Vec<&'static str> {
    vec!["install.sh", "uninstall.sh", "check-consistency.sh"]
}

#[test]
fn systemd_scripts_are_valid_bash() {
    for name in scripts() {
        let output = Command::new("bash")
            .args(["-n", script_path(name).to_str().unwrap()])
            .output()
            .expect("bash should run");
        assert!(
            output.status.success(),
            "systemd/{name} has a syntax error:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn systemd_scripts_are_executable() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in scripts() {
            let mode = fs::metadata(script_path(name))
                .unwrap_or_else(|e| panic!("systemd/{name} should exist: {e}"))
                .permissions()
                .mode();
            assert!(
                mode & 0o111 != 0,
                "systemd/{name} must be executable; it is meant to be run by hand on a host"
            );
        }
    }
}

/// The invariant is the whole point of the scaffolding; if the README stops
/// stating it, the scaffolding becomes three scripts nobody knows the rule
/// behind.
#[test]
fn systemd_readme_states_the_same_commit_invariant() {
    let readme = fs::read_to_string(systemd_dir().join("README.md"))
        .expect("systemd/README.md should be readable");
    assert!(
        readme.contains("in the same commit"),
        "systemd/README.md must state that deleting a script requires deleting its \
         tracked unit in the same commit"
    );
    assert!(
        readme.contains("uninstall.sh"),
        "systemd/README.md must say that hosts need uninstall.sh run after such a commit"
    );
    assert!(
        readme.contains("symlink"),
        "systemd/README.md must require units to be installed as symlinks, not copies"
    );
}

fn run_check(
    repo_root: &Path,
    unit_dir: &Path,
    host_dir: Option<&Path>,
    repo_only: bool,
) -> (i32, String) {
    let mut cmd = Command::new(script_path("check-consistency.sh"));
    cmd.env("ICG_REPO_ROOT", repo_root)
        .env("ICG_SYSTEMD_DIR", unit_dir);
    if let Some(host) = host_dir {
        cmd.env("ICG_HOST_UNIT_DIR", host);
    }
    if repo_only {
        cmd.arg("--repo-only");
    }
    let out = cmd.output().expect("check-consistency.sh should run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

/// A temp tree shaped like a repo: scripts/ and systemd/ under one root.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "icg-systemd-check-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("scripts")).expect("scripts dir");
        fs::create_dir_all(root.join("systemd")).expect("systemd dir");
        fs::create_dir_all(root.join("host")).expect("host dir");
        Fixture { root }
    }

    fn script(&self, name: &str, contents: &str) -> PathBuf {
        let p = self.root.join("scripts").join(name);
        fs::write(&p, contents).expect("write script");
        p
    }

    /// `extra` prefixes the ExecStart line (e.g. `/usr/bin/env bash `).
    fn tracked_unit(&self, name: &str, exec_start: &str) {
        fs::write(
            self.root.join("systemd").join(name),
            format!("[Service]\nExecStart={exec_start}\n"),
        )
        .expect("write tracked unit");
    }

    fn host_unit(&self, name: &str, exec_start: &str) {
        fs::write(
            self.root.join("host").join(name),
            format!("[Service]\nExecStart={exec_start}\n"),
        )
        .expect("write host unit");
    }

    /// `target` may or may not exist — dangling links are a shape under test.
    #[cfg(unix)]
    fn host_symlink(&self, name: &str, target: &Path) {
        std::os::unix::fs::symlink(target, self.root.join("host").join(name))
            .expect("symlink host unit");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn check_passes_when_both_sides_are_consistent() {
    let f = Fixture::new();
    f.script("good.sh", "#!/bin/bash\n");
    f.tracked_unit(
        "good.service",
        &format!("{}/scripts/good.sh", f.root.display()),
    );
    f.host_unit(
        "hostgood.service",
        &format!("{}/scripts/good.sh", f.root.display()),
    );
    // A unit executing paths outside this repo is another project's business.
    f.host_unit("foreign.service", "/opt/someone-elses/tool --run");

    let (code, out) = run_check(
        &f.root,
        &f.root.join("systemd"),
        Some(&f.root.join("host")),
        false,
    );
    assert_eq!(code, 0, "consistent fixture should pass, got:\n{out}");
}

#[test]
fn check_flags_tracked_unit_pointing_at_a_missing_path() {
    let f = Fixture::new();
    f.script("good.sh", "#!/bin/bash\n");
    f.tracked_unit(
        "good.service",
        &format!("{}/scripts/good.sh", f.root.display()),
    );
    f.tracked_unit(
        "bad.service",
        &format!("{}/scripts/gone.sh", f.root.display()),
    );

    let (code, out) = run_check(&f.root, &f.root.join("systemd"), None, true);
    assert_eq!(code, 1, "dangling tracked unit should fail, got:\n{out}");
    assert!(out.contains("bad.service"), "should name the unit:\n{out}");
    assert!(
        out.contains("gone.sh"),
        "should name the missing path:\n{out}"
    );
    assert!(
        !out.contains("good.service"),
        "must not flag the healthy unit:\n{out}"
    );
}

/// The original incident's unit ran `/usr/bin/env bash <repo script>` — the
/// path is the third token, not the first. Token-splitting must catch it.
#[test]
fn check_flags_env_interpreter_forms() {
    let f = Fixture::new();
    f.tracked_unit(
        "env-unit.service",
        &format!("/usr/bin/env bash {}/scripts/vanished.sh", f.root.display()),
    );

    let (code, out) = run_check(&f.root, &f.root.join("systemd"), None, true);
    assert_eq!(
        code, 1,
        "env-interpreter form should be flagged, got:\n{out}"
    );
    assert!(
        out.contains("vanished.sh"),
        "should name the missing script:\n{out}"
    );
}

/// The exact shape of the original bead: a host unit (not tracked here, a
/// pre-scaffolding leftover) executing a repo path that no longer exists.
#[test]
fn check_flags_host_unit_executing_a_missing_repo_path() {
    let f = Fixture::new();
    f.script("there.sh", "#!/bin/bash\n");
    f.host_unit(
        "leftover.service",
        &format!("{}/scripts/there.sh", f.root.display()),
    );
    // Drop the script after the unit references it: repo-side deletion.
    fs::remove_file(f.root.join("scripts").join("there.sh")).expect("drop script");

    let (code, out) = run_check(
        &f.root,
        &f.root.join("systemd"),
        Some(&f.root.join("host")),
        false,
    );
    assert_eq!(code, 1, "host-side orphan should fail, got:\n{out}");
    assert!(
        out.contains("leftover.service"),
        "should name the host unit:\n{out}"
    );
    assert!(
        out.contains("203/EXEC"),
        "the remediation should say what failure this prevents:\n{out}"
    );
}

/// A repo-side unit deletion leaves a dangling symlink on any host where
/// uninstall.sh was never run. That is drift too.
#[test]
fn check_flags_dangling_symlinks_into_the_repo() {
    let f = Fixture::new();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        f.root.join("systemd").join("deleted.service"),
        f.root.join("host").join("deleted.service"),
    )
    .expect("symlink");

    let (code, out) = run_check(
        &f.root,
        &f.root.join("systemd"),
        Some(&f.root.join("host")),
        false,
    );
    assert_eq!(code, 1, "dangling symlink should fail, got:\n{out}");
    assert!(
        out.contains("deleted.service"),
        "should name the unit:\n{out}"
    );
    assert!(
        out.contains("daemon-reload"),
        "remediation should include daemon-reload:\n{out}"
    );
}

/// A tracked unit installed on the host as a plain-file COPY. Every path it
/// references still exists, so direction (b) has nothing to say — only the
/// tracked -> host pairing check can catch it, and catching it here (while
/// the script still lives) is the whole point: this is the silent half of
/// the original incident, one script-deletion away from a week of 203/EXEC.
#[test]
fn check_flags_plain_file_copy_of_a_tracked_unit() {
    let f = Fixture::new();
    f.script("pair.sh", "#!/bin/bash\n");
    f.tracked_unit(
        "pair.service",
        &format!("{}/scripts/pair.sh", f.root.display()),
    );
    f.host_unit(
        "pair.service",
        &format!("{}/scripts/pair.sh", f.root.display()),
    );

    let (code, out) = run_check(
        &f.root,
        &f.root.join("systemd"),
        Some(&f.root.join("host")),
        false,
    );
    assert_eq!(code, 1, "a copy of a tracked unit is drift, got:\n{out}");
    assert!(out.contains("pair.service"), "should name the unit:\n{out}");
    assert!(
        out.contains("regular file"),
        "should say what the host file is:\n{out}"
    );
    assert!(
        out.contains("install.sh"),
        "remediation should be to reinstall as a symlink:\n{out}"
    );
}

/// A host file at a tracked unit's destination that symlinks somewhere else
/// — another checkout, a renamed file, someone else's unit. install.sh is
/// the only creator of links to tracked units and links to the tracked file
/// itself, so anything else at that destination is drift.
#[test]
fn check_flags_installed_unit_symlinking_elsewhere() {
    let f = Fixture::new();
    f.script("pair.sh", "#!/bin/bash\n");
    f.tracked_unit(
        "pair.service",
        &format!("{}/scripts/pair.sh", f.root.display()),
    );
    f.host_symlink("pair.service", &f.root.join("elsewhere.service"));

    let (code, out) = run_check(
        &f.root,
        &f.root.join("systemd"),
        Some(&f.root.join("host")),
        false,
    );
    assert_eq!(
        code, 1,
        "a foreign symlink at our destination is drift:\n{out}"
    );
    assert!(out.contains("pair.service"), "should name the unit:\n{out}");
    assert!(
        out.contains("elsewhere.service"),
        "should say where it actually points:\n{out}"
    );
}

/// The shape install.sh produces — every tracked unit present on the host as
/// a symlink to the tracked file — passes the full host scan, and a tracked
/// unit this host never installed is not a violation.
#[test]
fn check_passes_when_installed_units_are_symlinks_to_tracked_units() {
    let f = Fixture::new();
    f.script("pair.sh", "#!/bin/bash\n");
    f.script("spare.sh", "#!/bin/bash\n");
    f.tracked_unit(
        "linked.service",
        &format!("{}/scripts/pair.sh", f.root.display()),
    );
    f.tracked_unit(
        "not-installed-here.service",
        &format!("{}/scripts/spare.sh", f.root.display()),
    );
    f.host_symlink(
        "linked.service",
        &f.root.join("systemd").join("linked.service"),
    );

    let (code, out) = run_check(
        &f.root,
        &f.root.join("systemd"),
        Some(&f.root.join("host")),
        false,
    );
    assert_eq!(
        code, 0,
        "install.sh's own output must satisfy the full check, got:\n{out}"
    );
}

/// The CI form: no host unit dir, `--repo-only`. It must pass on a tree that
/// is consistent repo-side — including the real repository itself, which
/// currently tracks zero units.
#[test]
fn check_repo_only_passes_on_the_real_repository() {
    let (code, out) = run_check(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &systemd_dir(),
        None,
        true,
    );
    assert_eq!(
        code, 0,
        "the real repo should be repo-side consistent, got:\n{out}"
    );
}

/// The DoD is where the host-side half of the check is actually wired. CI's
/// fixture suites prove the detector; only a gate run on a real host can see
/// real drift, because the orphan/copy scan needs `~/.config/systemd/user`.
/// A refactor that quietly downgraded the gate back to `--repo-only` would
/// silence the repo's only always-on host-drift enforcement, so pin the
/// wiring: the full scan is the default, `--repo-only` survives only as the
/// loud extraction form, and the guard that chooses between them honors the
/// same `ICG_HOST_UNIT_DIR` override the scripts themselves do.
#[test]
fn definition_of_done_wires_the_host_side_systemd_gate() {
    let dod = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join("definition-of-done.sh"),
    )
    .expect("scripts/definition-of-done.sh should exist");
    assert!(
        dod.contains("run systemd/check-consistency.sh\n"),
        "the DoD must run the FULL consistency check — host scan included, \
         not just the repo-side half:\n{dod}"
    );
    assert!(
        dod.contains("--repo-only"),
        "the extraction form (--repo-only) must stay reachable for trees the \
         host did not link — that is the guard, not a downgrade:\n{dod}"
    );
    assert!(
        dod.contains("ICG_HOST_UNIT_DIR"),
        "the host-vs-extraction guard must honor ICG_HOST_UNIT_DIR, the same \
         override install.sh, uninstall.sh and the check itself take:\n{dod}"
    );
}

/// The operator-facing runbook is what this scaffolding previously lacked:
/// three scripts and a design README, but no procedure to follow at the
/// moment a host drifts. It must exist, cover the scripts it runbook-izes,
/// state the symlink requirement, and stay reachable from the documentation
/// map — otherwise the next operator rediscovers all of it from the journal.
#[test]
fn systemd_runbook_exists_and_is_linked_from_the_documentation_map() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runbook = fs::read_to_string(root.join("docs/runbooks/systemd-unit-lifecycle.md"))
        .expect("docs/runbooks/systemd-unit-lifecycle.md should exist");
    for covered in [
        "systemd/install.sh",
        "systemd/uninstall.sh",
        "systemd/check-consistency.sh",
    ] {
        assert!(
            runbook.contains(covered),
            "the systemd runbook must cover {covered}:\n{runbook}"
        );
    }
    assert!(
        runbook.contains("symlink"),
        "the systemd runbook must state the symlink requirement — units are \
         installed as symlinks, never copies:\n{runbook}"
    );
    let map = fs::read_to_string(root.join("docs/README.md"))
        .expect("docs/README.md — the documentation map — should exist");
    assert!(
        map.contains("runbooks/systemd-unit-lifecycle.md"),
        "the documentation map must link the systemd runbook, or no operator \
         will find it when a gate goes red:\n{map}"
    );
}
