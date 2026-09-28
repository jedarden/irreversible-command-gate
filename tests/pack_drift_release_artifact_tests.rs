//! `pack-drift` comparisons against the extracted shape of a release asset.
//!
//! `icg update` extracts `icg-packs.tar.gz` into a staging directory with
//! every manifest at that directory's root. These tests deliberately build
//! the reference side in that same root-level layout rather than using the
//! checkout's `packs/` directory, so the drift contract is exercised against
//! what an update actually activates.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn pack_json(id: &str) -> String {
    let keyword = format!("{id}ctl");
    serde_json::to_string_pretty(&serde_json::json!({
        "id": id,
        "tool_keywords": [keyword],
        "guarded_patterns": [{
            "id": format!("{id}-destroy"),
            "enabled": true,
            "type": "command_regex",
            "regex": format!("^{keyword} destroy"),
            "tier": "tier1",
            "severity": "High",
            "explanation": format!("{id} destroy cannot be undone"),
            "destructive": true,
            "redirect": {
                "channel": "deny",
                "reason_template": format!("run {keyword} destroy --dry-run first")
            }
        }]
    }))
    .expect("fixture pack should serialize")
}

struct ReleaseArtifactFixture {
    _dir: TempDir,
    installed: PathBuf,
    release: PathBuf,
}

impl ReleaseArtifactFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let installed = dir.path().join("installed");
        let release = dir.path().join("release-packs");
        fs::create_dir(&installed).expect("installed directory");
        fs::create_dir(&release).expect("release artifact directory");
        Self {
            _dir: dir,
            installed,
            release,
        }
    }

    fn write_installed(&self, id: &str) -> PathBuf {
        write_pack(&self.installed, id)
    }

    /// Write directly at the root, matching the updater's extracted staging
    /// directory for a root-level `icg-packs.tar.gz` release asset.
    fn write_release(&self, id: &str) -> PathBuf {
        write_pack(&self.release, id)
    }

    fn run(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_icg"))
            .args([
                "pack-drift",
                "--installed",
                self.installed.to_str().expect("installed path is UTF-8"),
                "--reference",
                self.release.to_str().expect("release path is UTF-8"),
            ])
            .current_dir(self._dir.path())
            .env_remove("ICG_PACK_DIR")
            .env_remove("ICG_RULE_PACK")
            .env_remove("ICG_INSTALLED_PACK_DIR")
            .output()
            .expect("icg should run")
    }
}

fn write_pack(dir: &Path, id: &str) -> PathBuf {
    let path = dir.join(format!("{id}.json"));
    fs::write(&path, pack_json(id)).expect("fixture pack should write");
    path
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be UTF-8")
}

#[test]
fn pack_drift_accepts_an_identical_extracted_release_artifact() {
    let fixture = ReleaseArtifactFixture::new();
    fixture.write_installed("alpha");
    fixture.write_release("alpha");

    let output = fixture.run();

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(fixture.release.join("alpha.json").is_file());
    assert!(stdout(&output).contains("OK: no drift — 1 pack(s) byte-identical"));
}

#[test]
fn pack_drift_reports_a_release_pack_missing_from_installed() {
    let fixture = ReleaseArtifactFixture::new();
    fixture.write_installed("alpha");
    fixture.write_release("alpha");
    fixture.write_release("beta");

    let output = fixture.run();

    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("MISSING FROM INSTALLED: beta"));
    assert!(stdout(&output).contains("DRIFT: 1 difference(s)"));
}

#[test]
fn pack_drift_reports_an_installed_pack_extra_to_the_release() {
    let fixture = ReleaseArtifactFixture::new();
    fixture.write_installed("alpha");
    fixture.write_installed("stray");
    fixture.write_release("alpha");

    let output = fixture.run();

    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("NOT IN REFERENCE: stray"));
    assert!(stdout(&output).contains("DRIFT: 1 difference(s)"));
}

#[test]
fn pack_drift_reports_changed_bytes_in_a_root_level_release_artifact() {
    let fixture = ReleaseArtifactFixture::new();
    fixture.write_installed("alpha");
    let release_pack = fixture.write_release("alpha");
    let mut changed: Value =
        serde_json::from_str(&fs::read_to_string(&release_pack).expect("release pack readable"))
            .expect("release pack should be JSON");
    changed["guarded_patterns"]
        .as_array_mut()
        .expect("guarded_patterns array")
        .push(serde_json::json!({
            "id": "alpha-wipe",
            "enabled": true,
            "type": "command_regex",
            "regex": "^alphactl wipe",
            "tier": "tier1",
            "severity": "High",
            "explanation": "alphactl wipe cannot be undone",
            "destructive": true,
            "redirect": {
                "channel": "deny",
                "reason_template": "run alphactl wipe --dry-run first"
            }
        }));
    fs::write(
        &release_pack,
        serde_json::to_string_pretty(&changed).expect("changed pack should serialize"),
    )
    .expect("changed release pack should write");

    let output = fixture.run();

    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(text.contains("CHANGED: alpha"), "{text}");
    assert!(text.contains("(1 guarded patterns) vs reference"), "{text}");
    assert!(text.contains("(2 guarded patterns)"), "{text}");
}

#[test]
fn pack_drift_reports_unreadable_installed_and_release_packs() {
    let fixture = ReleaseArtifactFixture::new();
    fixture.write_installed("alpha");
    fixture.write_release("alpha");
    let broken_installed = fixture.installed.join("broken.json");
    let broken_release = fixture.release.join("broken.json");
    fs::write(&broken_installed, "{ not valid json").expect("broken installed pack");
    fs::write(&broken_release, "[]").expect("malformed release pack");

    let output = fixture.run();

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains(&format!("UNREADABLE: {} (", broken_installed.display())),
        "{text}"
    );
    assert!(
        text.contains(&format!("UNREADABLE: {} (", broken_release.display())),
        "{text}"
    );
    assert!(text.contains("DRIFT: 2 difference(s)"), "{text}");
}

#[test]
fn pack_drift_rejects_a_malformed_installed_layout() {
    let fixture = ReleaseArtifactFixture::new();
    let nested = fixture.installed.join("packs");
    fs::create_dir(&nested).expect("nested installed directory");
    write_pack(&nested, "alpha");
    fixture.write_release("alpha");

    let output = fixture.run();

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("no .json packs found in the installed location"));
    assert!(stdout(&output).is_empty());
}

#[test]
fn pack_drift_rejects_a_malformed_release_layout() {
    let fixture = ReleaseArtifactFixture::new();
    fixture.write_installed("alpha");
    let nested = fixture.release.join("packs");
    fs::create_dir(&nested).expect("nested release directory");
    write_pack(&nested, "alpha");

    let output = fixture.run();

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("no .json packs found in the reference location"));
    assert!(stdout(&output).is_empty());
}
