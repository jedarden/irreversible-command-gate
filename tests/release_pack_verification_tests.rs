//! Guards for `scripts/verify-release-packs`, the post-publish pack-asset
//! integrity gate landed with bead `irrevers-bfbdf8f4`.
//!
//! v0.1.71 became Latest with a nested `packs/<name>.json` archive -- the
//! exact defect irrevers-64f7633e had already fixed in the icg-ci packager --
//! because the release was hand-published around CI while the icg-ci mutex
//! queue was starved. Fixing the pipeline cannot close a bypass around the
//! pipeline: hosts consume the published asset, so the published asset is
//! what has to be checked. The script runs the updater's own acceptance
//! contract (src/update.rs validate_root_directory_entry +
//! archive_pack_filename, the size caps, pack-manifest --verify) against a
//! release's assets; these tests run its offline `--check-archive` mode
//! against tar fixtures covering every direction the updater can reject.
//! The network path (gh download, git tag byte-identity) is deliberately
//! not exercised here -- rust-verify has no GitHub credentials -- the same
//! split as tests/doc_asset_check_tests.rs.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Reused test binaries bake `CARGO_MANIFEST_DIR` of a dead extraction (see
/// documentation_consistency_tests::audited_checkout); prefer the runtime
/// cwd, which cargo sets to the package root of the tree under test.
fn repo_root() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script_path() -> PathBuf {
    repo_root().join("scripts").join("verify-release-packs")
}

fn icg_bin() -> &'static str {
    env!("CARGO_BIN_EXE_icg")
}

fn run_script(args: &[&Path]) -> (i32, String) {
    let out = Command::new(script_path())
        .args(args)
        .output()
        .expect("scripts/verify-release-packs should run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

/// A minimal well-formed pack manifest -- the same shape the icg-ci
/// template's first-release fallback emits, so pack-manifest parses it.
fn pack_json(id: &str) -> String {
    format!(
        "{{\"id\":\"{id}\",\"tool_keywords\":[],\"applies_to\":[],\
         \"safe_patterns\":[],\"guarded_patterns\":[]}}\n"
    )
}

/// A temp tree with a `packs/` directory plus an empty `packs/sub/` for the
/// stray-directory-entry fixtures.
struct PackFixture {
    dir: tempfile::TempDir,
}

impl PackFixture {
    fn new() -> PackFixture {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("packs").join("sub")).expect("packs dir");
        PackFixture { dir }
    }

    fn packs_dir(&self) -> PathBuf {
        self.dir.path().join("packs")
    }

    fn write_pack(&self, id: &str) -> PathBuf {
        self.write_file(&format!("{id}.json"), pack_json(id).as_bytes())
    }

    fn write_file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.packs_dir().join(relative);
        fs::write(&path, contents).expect("write pack file");
        path
    }

    /// `tar -czf out <members>` from `from`, each member relative to `from`.
    fn archive(&self, out_name: &str, from: &Path, members: &[&Path]) -> PathBuf {
        let out = self.dir.path().join(out_name);
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&out)
            .args(members)
            .current_dir(from)
            .status()
            .expect("tar should run");
        assert!(status.success(), "tar fixture build failed for {out_name}");
        out
    }

    /// The exact command the fixed icg-ci packager runs: root-level *.json
    /// members only, from inside the packs directory.
    fn root_level_archive(&self, ids: &[&str]) -> PathBuf {
        let mut members: Vec<PathBuf> = ids
            .iter()
            .map(|id| PathBuf::from(format!("./{id}.json")))
            .collect();
        members.sort();
        let refs: Vec<&Path> = members.iter().map(|p| p.as_path()).collect();
        self.archive("root-level.tgz", &self.packs_dir(), &refs)
    }

    fn check(&self, archive: &Path) -> (i32, String) {
        run_script(&[
            Path::new("--check-archive"),
            archive,
            Path::new("--pack-manifest-bin"),
            Path::new(icg_bin()),
        ])
    }
}

fn manifest_for(packs_dir: &Path, out: &Path) {
    let status = Command::new(icg_bin())
        .args(["pack-manifest", "--pack-dir"])
        .arg(packs_dir)
        .args(["--output"])
        .arg(out)
        .status()
        .expect("pack-manifest should run");
    assert!(status.success(), "pack-manifest fixture generation failed");
}

#[test]
fn gate_script_is_executable() {
    let mode = fs::metadata(script_path())
        .expect("scripts/verify-release-packs should exist")
        .permissions()
        .mode();
    assert!(
        mode & 0o111 != 0,
        "scripts/verify-release-packs must be executable; the release runbooks invoke it directly"
    );
}

#[test]
fn the_fixed_packager_output_passes() {
    let fixture = PackFixture::new();
    fixture.write_pack("alpha");
    fixture.write_pack("beta");
    let archive = fixture.root_level_archive(&["alpha", "beta"]);

    let (code, out) = fixture.check(&archive);
    assert_eq!(code, 0, "the fixed packager's layout must pass:\n{out}");
    // The count is load-bearing: a layout check that matched nothing would
    // otherwise read as health.
    assert!(
        out.contains("2 root-level JSON pack(s)"),
        "summary must report the real member count:\n{out}"
    );
}

#[test]
fn the_historical_nested_layout_is_rejected() {
    let fixture = PackFixture::new();
    fixture.write_pack("alpha");
    // `tar -C <checkout> ... packs` -- the command that shipped every
    // release up to v0.1.68 and hand-published v0.1.71.
    let archive = fixture.archive("nested.tgz", fixture.dir.path(), &[Path::new("packs")]);

    let (code, out) = fixture.check(&archive);
    assert_eq!(code, 1);
    assert!(
        out.contains("nested"),
        "a nested archive must be named as nested:\n{out}"
    );
    assert!(
        out.contains("validate_root_directory_entry"),
        "the rejection should point at the updater contract it mirrors:\n{out}"
    );
}

#[test]
fn a_non_json_member_is_rejected() {
    let fixture = PackFixture::new();
    fixture.write_pack("alpha");
    fixture.write_file("coverage-justifications.md", b"approval prose\n");
    let archive = fixture.archive(
        "non-json.tgz",
        &fixture.packs_dir(),
        &[
            Path::new("./alpha.json"),
            Path::new("./coverage-justifications.md"),
        ],
    );

    let (code, out) = fixture.check(&archive);
    assert_eq!(code, 1);
    assert!(
        out.contains("is not a JSON manifest"),
        "a root-level non-JSON member is just as rejected as a nested one:\n{out}"
    );
}

#[test]
fn a_stray_directory_entry_is_rejected() {
    let fixture = PackFixture::new();
    fixture.write_pack("alpha");
    let archive = fixture.archive(
        "stray-dir.tgz",
        &fixture.packs_dir(),
        &[Path::new("./alpha.json"), Path::new("./sub")],
    );

    let (code, out) = fixture.check(&archive);
    assert_eq!(code, 1);
    assert!(
        out.contains("directory entry") && out.contains("nested"),
        "a non-root directory entry must be rejected even when every file member is root-level:\n{out}"
    );
}

#[test]
fn a_duplicate_member_is_rejected() {
    let fixture = PackFixture::new();
    fixture.write_pack("alpha");
    let archive = fixture.archive(
        "duplicate.tgz",
        &fixture.packs_dir(),
        &[Path::new("./alpha.json"), Path::new("./alpha.json")],
    );

    let (code, out) = fixture.check(&archive);
    assert_eq!(code, 1);
    assert!(
        out.contains("duplicate file"),
        "a duplicated manifest must be rejected:\n{out}"
    );
}

#[test]
fn an_archive_without_manifests_is_rejected() {
    let fixture = PackFixture::new();
    let archive = fixture.archive("empty.tgz", &fixture.packs_dir(), &[Path::new("./sub")]);

    let (code, out) = fixture.check(&archive);
    assert_eq!(code, 1);
    assert!(
        !out.contains("OK "),
        "a directory-entry-only archive must not read as health:\n{out}"
    );
    // The lone './sub' member trips the nested-entry rejection before the
    // count cap is reached; either layout rejection is the gate working, so
    // pin the property, not which message fired.
    assert!(
        out.contains("nested") || out.contains("no JSON manifests"),
        "the walk must name a layout rejection for a manifest-free archive:\n{out}"
    );
}

#[test]
fn a_symlink_member_is_rejected() {
    let fixture = PackFixture::new();
    let alpha = fixture.write_pack("alpha");
    let link = fixture.packs_dir().join("link.json");
    std::os::unix::fs::symlink(&alpha, &link).expect("symlink fixture");
    let archive = fixture.archive(
        "symlink.tgz",
        &fixture.packs_dir(),
        &[Path::new("./link.json")],
    );

    let (code, out) = fixture.check(&archive);
    assert_eq!(code, 1);
    assert!(
        out.contains("not a regular file"),
        "links and special files are forbidden by the updater and by this gate:\n{out}"
    );
}

#[test]
fn an_oversized_member_is_rejected() {
    let fixture = PackFixture::new();
    // 4 MiB + 1 byte of junk: over MAX_PACK_BYTES without needing to parse.
    let big = vec![b'x'; 4 * 1024 * 1024 + 1];
    fixture.write_file("big.json", &big);
    let archive = fixture.root_level_archive(&["big"]);

    let (code, out) = fixture.check(&archive);
    assert_eq!(code, 1);
    assert!(
        out.contains("exceeds 4194304 bytes"),
        "per-manifest size cap must mirror src/update.rs MAX_PACK_BYTES:\n{out}"
    );
}

#[test]
fn more_than_max_pack_count_is_rejected() {
    let fixture = PackFixture::new();
    let mut ids: Vec<String> = Vec::new();
    for i in 0..257 {
        let id = format!("pack{i:03}");
        fixture.write_pack(&id);
        ids.push(id);
    }
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let archive = fixture.root_level_archive(&id_refs);

    let (code, out) = fixture.check(&archive);
    assert_eq!(code, 1);
    assert!(
        out.contains("more than 256 manifests"),
        "member cap must mirror src/update.rs MAX_PACK_COUNT:\n{out}"
    );
}

#[test]
fn manifest_verify_accepts_the_gated_bytes() {
    let fixture = PackFixture::new();
    fixture.write_pack("alpha");
    fixture.write_pack("beta");
    let manifest = fixture.dir.path().join("pack-manifest.json");
    manifest_for(&fixture.packs_dir(), &manifest);
    let archive = fixture.root_level_archive(&["alpha", "beta"]);

    let (code, out) = run_script(&[
        Path::new("--check-archive"),
        archive.as_path(),
        Path::new("--manifest"),
        manifest.as_path(),
        Path::new("--pack-manifest-bin"),
        Path::new(icg_bin()),
    ]);
    assert_eq!(code, 0, "the archive holds exactly the gated bytes:\n{out}");
    assert!(
        out.contains("matches manifest"),
        "the verify step's own confirmation should surface:\n{out}"
    );
}

#[test]
fn a_tampered_member_fails_manifest_verify() {
    let fixture = PackFixture::new();
    fixture.write_pack("alpha");
    fixture.write_pack("beta");
    let manifest = fixture.dir.path().join("pack-manifest.json");
    manifest_for(&fixture.packs_dir(), &manifest);

    // Flip one byte after the manifest was generated, then repackage.
    let alpha = fixture.packs_dir().join("alpha.json");
    let mut body = fs::read_to_string(&alpha).expect("read alpha");
    body = body.replace("\"alpha\"", "\"alphA\"");
    fs::write(&alpha, body).expect("tamper alpha");
    let archive = fixture.root_level_archive(&["alpha", "beta"]);

    let (code, out) = run_script(&[
        Path::new("--check-archive"),
        archive.as_path(),
        Path::new("--manifest"),
        manifest.as_path(),
        Path::new("--pack-manifest-bin"),
        Path::new(icg_bin()),
    ]);
    assert_eq!(code, 1);
    assert!(
        out.contains("mismatched SHA-256"),
        "a member that differs from the gated bytes must fail on its hash:\n{out}"
    );
}

#[test]
fn an_unmanifested_pack_fails_manifest_verify() {
    let fixture = PackFixture::new();
    fixture.write_pack("alpha");
    let manifest = fixture.dir.path().join("pack-manifest.json");
    manifest_for(&fixture.packs_dir(), &manifest);

    // The archive ships a second pack the manifest never indexed.
    fixture.write_pack("beta");
    let archive = fixture.root_level_archive(&["alpha", "beta"]);

    let (code, out) = run_script(&[
        Path::new("--check-archive"),
        archive.as_path(),
        Path::new("--manifest"),
        manifest.as_path(),
        Path::new("--pack-manifest-bin"),
        Path::new(icg_bin()),
    ]);
    assert_eq!(code, 1);
    assert!(
        out.contains("not in the manifest"),
        "an archive may not ship a pack the release manifest does not know:\n{out}"
    );
}

#[test]
fn usage_errors_exit_two() {
    let (code, out) = run_script(&[]);
    assert_eq!(code, 2, "no arguments at all is a usage error:\n{out}");

    let (code, out) = run_script(&[Path::new("--nonsense")]);
    assert_eq!(code, 2, "an unknown option is a usage error:\n{out}");

    let (code, out) = run_script(&[
        Path::new("--check-archive"),
        Path::new("/nonexistent/icg-packs.tar.gz"),
    ]);
    assert_eq!(
        code, 1,
        "a named archive that does not exist is a check failure, not a usage error:\n{out}"
    );
    assert!(
        out.contains("archive not found"),
        "the failure should name the problem:\n{out}"
    );
}
