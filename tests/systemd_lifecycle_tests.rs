//! Behavioral coverage for `systemd/install.sh` and `systemd/uninstall.sh`.
//!
//! `systemd_consistency_tests.rs` keeps the drift *check* honest; this file
//! keeps the install/uninstall pair honest. The incident behind the
//! scaffolding (`irrevers-46f2b741`) was a lifecycle failure — a unit
//! installed as a copy outliving its script — so the lifecycle scripts
//! deserve the same scrutiny as the check: install must link (never copy),
//! must enable only units that ask, must refuse to clobber anything that is
//! not its own symlink, and must refuse to report success over drift;
//! uninstall must remove exactly our symlinks — including the orphaned kind
//! left behind when a commit deleted the tracked unit — and nothing else.
//!
//! Everything runs against fixture directories with a fake `systemctl` on
//! PATH, so no test touches the real `~/.config/systemd/user` and the suite
//! works on hosts with no systemd at all. The scripts are copied into the
//! fixture because `install.sh` deliberately pairs the tracked-unit
//! directory with its own location — that pairing is the invariant under
//! test, not something to route around with an override.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Env {
    root: PathBuf,
    /// The scripts' own directory: plays the role of the repo's `systemd/`.
    sysdir: PathBuf,
    /// Plays the role of `~/.config/systemd/user`.
    host: PathBuf,
    bin: PathBuf,
    shim_log: PathBuf,
}

fn unique_root(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "icg-systemd-lifecycle-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

impl Env {
    fn new(tag: &str) -> Env {
        let root = unique_root(tag);
        let sysdir = root.join("systemd");
        fs::create_dir_all(&sysdir).expect("create systemd dir");
        fs::create_dir_all(root.join("scripts")).expect("create scripts dir");
        // The script every fixture unit executes; its existence is what makes
        // the tree repo-side consistent for the install self-check.
        fs::write(root.join("scripts/job.sh"), "#!/bin/bash\n").expect("write job.sh");

        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("systemd");
        for script in ["install.sh", "uninstall.sh", "check-consistency.sh"] {
            fs::copy(source.join(script), sysdir.join(script))
                .unwrap_or_else(|e| panic!("copy {script}: {e}"));
            let mode = fs::metadata(sysdir.join(script))
                .expect("copied script exists")
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "{script} must stay executable");
        }

        // Fake systemctl: records every invocation, succeeds at everything.
        // `systemctl --user enable` on a fixture unit would otherwise fail on
        // hosts where the unit is not in the real search path — and worse,
        // succeed in a way no test could see. The shim makes both visible.
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("create bin dir");
        let shim = bin.join("systemctl");
        fs::write(
            &shim,
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" >>\"$ICG_SHIM_LOG\"\nexit 0\n",
        )
        .expect("write systemctl shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("shim +x");

        let env = Env {
            host: root.join("host-unit-dir"),
            shim_log: root.join("systemctl.log"),
            root,
            sysdir,
            bin,
        };
        // `install.sh` creates the host dir itself; `uninstall.sh` tolerates
        // it missing. Tests that need it sooner call `ensure_host`.
        env
    }

    fn ensure_host(&self) -> &Path {
        fs::create_dir_all(&self.host).expect("create host dir");
        &self.host
    }

    fn unit(&self, name: &str, with_install_section: bool) -> PathBuf {
        let mut body = format!(
            "[Unit]\nDescription=fixture {name}\n\n[Service]\n\
             WorkingDirectory={}\nExecStart=/usr/bin/env bash {}/scripts/job.sh\n",
            self.root.display(),
            self.root.display()
        );
        if with_install_section {
            body.push_str("\n[Install]\nWantedBy=default.target\n");
        }
        let path = self.sysdir.join(name);
        fs::write(&path, body).expect("write unit");
        path
    }

    /// A regular file at the destination: the copied-unit shape this repo
    /// used to leave behind (the retired icg-frontier-consistency.service).
    fn host_plain_file(&self, name: &str) -> PathBuf {
        self.ensure_host();
        let path = self.host.join(name);
        fs::write(&path, "[Service]\nExecStart=/opt/old-copy\n").expect("write plain file");
        path
    }

    /// A symlink at the destination pointing somewhere that is not us.
    fn host_foreign_symlink(&self, name: &str, target: &Path) -> PathBuf {
        self.ensure_host();
        let path = self.host.join(name);
        std::os::unix::fs::symlink(target, &path).expect("symlink foreign");
        path
    }

    fn run(&self, script: &str, args: &[&str]) -> Output {
        let path_var = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(self.sysdir.join(script))
            .args(args)
            .env("PATH", path_var)
            .env("ICG_HOST_UNIT_DIR", &self.host)
            .env("ICG_REPO_ROOT", &self.root)
            .env("ICG_SHIM_LOG", &self.shim_log)
            .output()
            .unwrap_or_else(|e| panic!("{script} should run: {e}"))
    }

    fn text(out: &Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    fn assert_ok(&self, out: &Output, what: &str) -> String {
        let text = Self::text(out);
        assert!(
            out.status.success(),
            "{what} should succeed (exit {:?}):\n{text}",
            out.status.code()
        );
        text
    }

    /// Every `systemctl` invocation the shim saw, in order.
    fn shim_calls(&self) -> Vec<String> {
        match fs::read_to_string(&self.shim_log) {
            Ok(text) => text.lines().map(str::to_owned).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn host_names(&self) -> Vec<String> {
        match fs::read_dir(self.ensure_host()) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn install_symlinks_tracked_units_and_reloads_the_daemon() {
    let e = Env::new("install-links");
    e.unit("a-first.service", false);
    e.unit("b-second.timer", true);

    let out = e.run("install.sh", &[]);
    let text = e.assert_ok(&out, "install");

    let link = e.ensure_host().join("a-first.service");
    let target = std::fs::read_link(&link).expect("dest must be a symlink");
    assert_eq!(target, e.sysdir.join("a-first.service"), "link target");
    assert!(
        !link.symlink_metadata().expect("link").is_file(),
        "the destination must never be a copy — a copy is the failure mode"
    );
    assert!(
        text.contains("b-second.timer"),
        "both units reported:\n{text}"
    );

    let calls = e.shim_calls();
    assert!(
        calls.iter().any(|c| c.contains("daemon-reload")),
        "install must daemon-reload, saw: {calls:?}"
    );
}

#[test]
fn install_enables_only_units_that_ask_for_it() {
    let e = Env::new("install-enable");
    e.unit("a-no-install.service", false);
    e.unit("b-wants-enable.service", true);

    let out = e.run("install.sh", &[]);
    e.assert_ok(&out, "install");

    let calls = e.shim_calls();
    let enables: Vec<_> = calls.iter().filter(|c| c.contains("enable")).collect();
    assert_eq!(
        enables.len(),
        1,
        "exactly one unit has [Install]; enable calls: {calls:?}"
    );
    assert!(
        enables[0].contains("b-wants-enable.service"),
        "the [Install] unit is the one enabled: {calls:?}"
    );
}

#[test]
fn install_now_starts_the_enabled_unit() {
    let e = Env::new("install-now");
    e.unit("a-no-install.service", false);
    e.unit("b-wants-enable.service", true);

    let out = e.run("install.sh", &["--now"]);
    e.assert_ok(&out, "install --now");

    let calls = e.shim_calls();
    let enables: Vec<_> = calls.iter().filter(|c| c.contains("enable")).collect();
    assert_eq!(enables.len(), 1, "calls: {calls:?}");
    assert!(
        enables[0].contains("--now"),
        "--now must reach the enable call: {calls:?}"
    );
}

#[test]
fn install_is_idempotent_for_its_own_symlinks() {
    let e = Env::new("install-idempotent");
    e.unit("a-first.service", true);

    e.assert_ok(&e.run("install.sh", &[]), "first install");
    let text = e.assert_ok(&e.run("install.sh", &[]), "second install");

    assert!(
        text.contains("already linked"),
        "the second run must recognize its own symlink:\n{text}"
    );
    let link = e.ensure_host().join("a-first.service");
    assert_eq!(
        std::fs::read_link(&link).expect("link"),
        e.sysdir.join("a-first.service"),
        "the link is unchanged by the second run"
    );
}

#[test]
fn install_refuses_to_clobber_a_foreign_symlink() {
    let e = Env::new("install-foreign-link");
    e.unit("a-first.service", false);
    let foreign = e.host_foreign_symlink("a-first.service", Path::new("/opt/someone-else.service"));

    let out = e.run("install.sh", &[]);
    let text = Env::text(&out);

    assert_eq!(
        out.status.code(),
        Some(1),
        "must refuse, not replace:\n{text}"
    );
    assert!(
        text.contains("refusing"),
        "the refusal should say why:\n{text}"
    );
    assert_eq!(
        std::fs::read_link(&foreign).expect("foreign link"),
        Path::new("/opt/someone-else.service"),
        "the foreign symlink must be left exactly as found"
    );
}

#[test]
fn install_refuses_to_clobber_a_regular_file() {
    let e = Env::new("install-plain-file");
    e.unit("a-first.service", false);
    let plain = e.host_plain_file("a-first.service");

    let out = e.run("install.sh", &[]);
    let text = Env::text(&out);

    assert_eq!(out.status.code(), Some(1), "must refuse:\n{text}");
    assert!(text.contains("regular file"), "name the problem:\n{text}");
    assert!(
        plain
            .symlink_metadata()
            .expect("plain file survives")
            .is_file(),
        "the copied unit must be surfaced, not silently replaced"
    );
}

#[test]
fn install_dry_run_creates_nothing() {
    let e = Env::new("install-dry-run");
    e.unit("a-first.service", true);

    let out = e.run("install.sh", &["--dry-run"]);
    let text = e.assert_ok(&out, "dry run");

    assert!(
        text.contains("would run: ln -s"),
        "dry run should show the link it would create:\n{text}"
    );
    assert!(
        e.host_names().is_empty(),
        "dry run must create nothing in the host dir"
    );
    assert!(
        e.shim_calls().is_empty(),
        "dry run must not reach systemctl"
    );
}

#[test]
fn install_with_no_tracked_units_is_a_no_op() {
    let e = Env::new("install-empty");

    let out = e.run("install.sh", &[]);
    let text = e.assert_ok(&out, "empty install");

    assert!(
        text.contains("nothing to install"),
        "say so instead of failing:\n{text}"
    );
}

/// Install ends by running the consistency check; a tree that is already
/// drifting must not get an "installed and consistent" verdict.
#[test]
fn install_refuses_to_report_success_over_drift() {
    let e = Env::new("install-drift");
    let path = e.unit("a-dangling.service", false);
    fs::write(
        &path,
        format!(
            "[Service]\nExecStart={}/scripts/vanished.sh\n",
            e.root.display()
        ),
    )
    .expect("write dangling unit");

    let out = e.run("install.sh", &[]);
    let text = Env::text(&out);

    assert_ne!(
        out.status.code(),
        Some(0),
        "install must fail when the self-check finds drift:\n{text}"
    );
    assert!(
        !text.contains("and consistent"),
        "no success line over a drifted tree:\n{text}"
    );
    assert!(
        text.contains("vanished.sh"),
        "the underlying drift is named:\n{text}"
    );
}

#[test]
fn uninstall_removes_our_symlinks_and_leaves_everything_else() {
    let e = Env::new("uninstall-ours");
    e.unit("a-first.service", false);
    e.unit("b-second.service", false);
    e.assert_ok(&e.run("install.sh", &[]), "install");

    let foreign = e.host_foreign_symlink("elsewhere.service", Path::new("/opt/other.service"));
    let plain = e.host_plain_file("plain.service");

    let out = e.run("uninstall.sh", &[]);
    let text = e.assert_ok(&out, "uninstall");

    assert!(
        e.host.join("a-first.service").symlink_metadata().is_err(),
        "our symlink must be removed"
    );
    assert!(
        e.host.join("b-second.service").symlink_metadata().is_err(),
        "our symlink must be removed"
    );
    assert_eq!(
        std::fs::read_link(&foreign).expect("foreign survives"),
        Path::new("/opt/other.service"),
        "a symlink that is not ours must survive"
    );
    assert!(
        plain.symlink_metadata().expect("plain survives").is_file(),
        "a regular file must never be deleted"
    );
    // Neither file is ours and neither collides with a tracked unit name, so
    // uninstall has nothing to say about them: the real host unit directory
    // holds dozens of other projects' units, and commenting on those would be
    // noise. Residue is check-consistency.sh's job, by repo-reference.
    assert!(
        !text.contains("plain.service") && !text.contains("elsewhere.service"),
        "untracked host files are not ours to comment on:\n{text}"
    );

    let calls = e.shim_calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("disable") && c.contains("a-first.service")),
        "our units are disabled before removal: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("daemon-reload")),
        "systemd is told to drop the units: {calls:?}"
    );
}

/// The one plain file uninstall does name: one sitting at a tracked unit's
/// destination, which is exactly the residue install.sh refuses to clobber.
/// It is reported for hand removal and counted, never deleted.
#[test]
fn uninstall_names_a_plain_file_at_a_tracked_units_destination() {
    let e = Env::new("uninstall-plain-tracked");
    e.unit("a-first.service", false);
    let plain = e.host_plain_file("a-first.service");

    let out = e.run("uninstall.sh", &[]);
    let text = e.assert_ok(&out, "uninstall");

    assert!(
        plain.symlink_metadata().expect("plain survives").is_file(),
        "the copied unit is never deleted:\n{text}"
    );
    assert!(
        text.contains("regular file"),
        "the copied unit is named for hand removal:\n{text}"
    );
    assert!(
        text.contains("a-first.service"),
        "the colliding tracked unit is named:\n{text}"
    );
    assert!(
        text.contains("skipped 1"),
        "the summary counts what it refused to touch:\n{text}"
    );
}

/// The case the scaffolding originally missed: a commit deletes a tracked
/// unit, a host that has not uninstalled yet holds the now-dangling symlink,
/// and uninstall must still take that link — it is ours by construction.
#[test]
fn uninstall_removes_orphans_whose_tracked_unit_was_deleted() {
    let e = Env::new("uninstall-orphan");
    e.unit("a-first.service", false);
    let doomed = e.unit("b-doomed.service", false);
    e.assert_ok(&e.run("install.sh", &[]), "install");

    // The deleting commit: the tracked unit goes away repo-side.
    fs::remove_file(&doomed).expect("delete tracked unit");

    let out = e.run("uninstall.sh", &[]);
    let text = e.assert_ok(&out, "uninstall");

    assert!(
        e.host.join("b-doomed.service").symlink_metadata().is_err(),
        "the orphaned symlink itself must be removed even though its unit is gone\n{text}"
    );
    assert!(
        e.host.join("a-first.service").symlink_metadata().is_err(),
        "the still-tracked link is removed as usual"
    );
    assert!(
        text.contains("orphaned"),
        "the orphan removal is reported as such:\n{text}"
    );
}

/// The 092e82c shape taken to the end: every tracked unit is already gone
/// from the repo, and a host is left holding dangling links. Uninstall used
/// to exit "nothing to uninstall" here, leaving the exact drift that fired
/// 203/EXEC for a week.
#[test]
fn uninstall_removes_orphans_even_with_no_tracked_units() {
    let e = Env::new("uninstall-all-orphaned");
    e.ensure_host();
    std::os::unix::fs::symlink(e.sysdir.join("gone.service"), e.host.join("gone.service"))
        .expect("dangling link");

    let out = e.run("uninstall.sh", &[]);
    let text = e.assert_ok(&out, "uninstall with no tracked units");

    assert!(
        e.host.join("gone.service").symlink_metadata().is_err(),
        "the dangling link itself must go (exists() is false for a dangling \
         link, so the link's own metadata is what proves removal):\n{text}"
    );
    assert!(
        text.contains("no units tracked"),
        "the empty tracked set is explained, not an early exit:\n{text}"
    );
}

#[test]
fn uninstall_dry_run_removes_nothing() {
    let e = Env::new("uninstall-dry-run");
    e.unit("a-first.service", false);
    e.assert_ok(&e.run("install.sh", &[]), "install");
    e.ensure_host();
    std::os::unix::fs::symlink(e.sysdir.join("gone.service"), e.host.join("gone.service"))
        .expect("dangling link");

    let out = e.run("uninstall.sh", &["--dry-run"]);
    let text = e.assert_ok(&out, "dry run");

    assert!(e.host.join("a-first.service").exists(), "link kept");
    assert!(
        e.host.join("gone.service").symlink_metadata().is_ok(),
        "the dangling link itself is kept — exists() follows the link and is \
         false for a dangling one, so only the link's own metadata proves the \
         dry run left it"
    );
    assert!(
        text.contains("dry run complete"),
        "dry run says it changed nothing:\n{text}"
    );
}

/// Deleting a unit is only half the cleanup: systemd must be told, or the
/// unit lingers in the manager until the next reload. Both scripts reload.
#[test]
fn uninstall_reloads_the_daemon_after_removal() {
    let e = Env::new("uninstall-reload");
    e.unit("a-first.service", true);
    e.assert_ok(&e.run("install.sh", &[]), "install");
    e.shim_calls(); // discard install-time calls

    e.assert_ok(&e.run("uninstall.sh", &[]), "uninstall");
    let calls = e.shim_calls();
    assert!(
        calls.iter().any(|c| c.contains("daemon-reload")),
        "uninstall must daemon-reload, saw: {calls:?}"
    );
}
