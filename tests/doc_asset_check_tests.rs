//! Guards for the README asset-reference gate, landed with bead
//! `irrevers-c869b114`.
//!
//! The demo GIF went stale once (irrevers-8c3bab0e) and nothing in the
//! verification path would have caught a missing, emptied, or wrong-typed
//! README asset either -- nor a truncated one, whose magic is fine and
//! whose body is gone (irrevers-dab6b05d): cargo test looks at code and
//! prose, never at what
//! the README *points at*, so a broken reference ships as a dead image with
//! every gate green. `scripts/check-doc-assets` closes that and
//! scripts/definition-of-done.sh runs it directly — but rust-verify (this
//! suite) never executes repo scripts, so without these tests CI would
//! carry the gate nowhere. The fixture tests run the script against temp
//! trees covering each failure direction; the real-repo test pins that the
//! committed README still passes and that the parser still finds references
//! at all, since a regex rotted into matching nothing would otherwise pass
//! vacuously.

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
    repo_root().join("scripts").join("check-doc-assets")
}

fn run_script(args: &[&Path]) -> (i32, String) {
    let out = Command::new(script_path())
        .args(args)
        .output()
        .expect("scripts/check-doc-assets should run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

/// A temp tree with a README whose relative references resolve inside it,
/// the way a markdown renderer would resolve them.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new(readme: &str) -> Fixture {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("README.md"), readme).expect("write README");
        Fixture { dir }
    }

    fn asset(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.path().join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("asset dir");
        fs::write(&path, bytes).expect("write asset");
        path
    }

    fn readme_path(&self) -> PathBuf {
        self.dir.path().join("README.md")
    }
}

/// A minimal well-formed GIF: magic, logical screen descriptor, an image
/// separator, and the 0x3B trailer the gate requires as the final byte.
const GIF89A: &[u8] = b"GIF89a\x01\x00\x01\x00\x00\xff,;";

#[test]
fn gate_script_is_executable() {
    let mode = fs::metadata(script_path())
        .expect("scripts/check-doc-assets should exist")
        .permissions()
        .mode();
    assert!(
        mode & 0o111 != 0,
        "scripts/check-doc-assets must be executable; definition-of-done.sh runs it directly"
    );
}

#[test]
fn real_readme_passes_and_the_parser_still_finds_references() {
    let (code, out) = run_script(&[]);
    assert_eq!(code, 0, "the committed README should pass the gate:\n{out}");
    // "…: 19 local asset reference(s), all resolve" — the count is load-bearing.
    // A parser that matches nothing must not read as health, so assert the
    // number is real rather than merely that the script exited 0.
    let count = out.lines().find_map(|line| {
        line.split(" local asset reference")
            .next()
            .and_then(|head| head.rsplit(' ').next())
            .and_then(|n| n.parse::<usize>().ok())
    });
    assert!(
        count.is_some_and(|n| n >= 10),
        "the committed README should yield a double-digit local reference \
         count; got {count:?} — the parser has probably rotted:\n{out}"
    );
}

#[test]
fn gif_without_a_trailer_fails_as_truncated() {
    // The shape the magic check was blind to (irrevers-dab6b05d): a
    // capture killed mid-run leaves a well-magic'd head and no 0x3B
    // trailer. Presence, size, and type all pass; only the final byte
    // knows.
    let f = Fixture::new("# t\n\n![demo](assets/demo.gif)\n");
    f.asset("assets/demo.gif", b"GIF89a\x01\x00\x01\x00\x00\xff,");

    let (code, out) = run_script(&[&f.readme_path()]);
    assert_eq!(code, 1, "a trailer-less GIF must fail the gate:\n{out}");
    assert!(
        out.contains("truncated"),
        "should name the failure truncation:\n{out}"
    );
    assert!(
        out.contains("GIF trailer"),
        "should say what the final byte should have been:\n{out}"
    );
}

#[test]
fn missing_asset_fails_with_the_reference_named() {
    let f = Fixture::new("# t\n\n![demo](assets/demo.gif)\n\n[ok](assets/healthy.md)\n");
    f.asset("assets/healthy.md", b"fine");

    let (code, out) = run_script(&[&f.readme_path()]);
    assert_eq!(code, 1, "a missing asset must fail the gate:\n{out}");
    assert!(
        out.contains("assets/demo.gif"),
        "should name the reference:\n{out}"
    );
    assert!(out.contains("missing"), "should say what is wrong:\n{out}");
    assert!(
        !out.contains("assets/healthy.md"),
        "must not flag the healthy reference:\n{out}"
    );
}

#[test]
fn zero_byte_asset_fails() {
    let f = Fixture::new("# t\n\n![demo](assets/demo.gif)\n");
    f.asset("assets/demo.gif", b"");

    let (code, out) = run_script(&[&f.readme_path()]);
    assert_eq!(code, 1, "a zero-byte asset must fail the gate:\n{out}");
    assert!(
        out.contains("assets/demo.gif"),
        "should name the reference:\n{out}"
    );
    assert!(out.contains("empty"), "should say what is wrong:\n{out}");
}

#[test]
fn wrong_type_asset_fails() {
    // PNG bytes under a .gif name: present, non-empty, and exactly the
    // wrong thing — the shape a rename or a bad export produces.
    let f = Fixture::new("# t\n\n![demo](assets/demo.gif)\n");
    f.asset("assets/demo.gif", b"\x89PNG\r\n\x1a\nrest of a png");

    let (code, out) = run_script(&[&f.readme_path()]);
    assert_eq!(code, 1, "a wrong-typed asset must fail the gate:\n{out}");
    assert!(
        out.contains("assets/demo.gif"),
        "should name the reference:\n{out}"
    );
    assert!(
        out.contains("wrong type"),
        "should say what is wrong:\n{out}"
    );
    assert!(
        out.contains("a GIF image"),
        "should name what the extension declared:\n{out}"
    );
}

#[test]
fn well_formed_fixture_passes_including_html_img_and_anchors() {
    let f = Fixture::new(
        "# t\n\n\
         ![demo](assets/demo.gif)\n\n\
         <img src=\"assets/flow.svg\" alt=\"flow\">\n\n\
         [script](assets/demo.sh)\n\n\
         [section](assets/README.md#heading) — fragment stripped\n\n\
         [external](https://example.com/x.png) and [anchor](#t) are skipped\n",
    );
    f.asset("assets/demo.gif", GIF89A);
    f.asset(
        "assets/flow.svg",
        b"<?xml version=\"1.0\"?><svg xmlns=\"\"></svg>",
    );
    f.asset("assets/demo.sh", b"#!/bin/sh\necho hi\n");
    f.asset("assets/README.md", b"# docs\n");

    let (code, out) = run_script(&[&f.readme_path()]);
    assert_eq!(
        code, 0,
        "every shape this repo's README actually uses must pass:\n{out}"
    );
}

/// An empty (0-byte) SVG is the zero-byte case, not the type case; and a
/// GIF-sized file whose payload is text is the wrong-type case. Both
/// directions on one media type pin that the checks are ordered
/// missing -> empty -> type.
#[test]
fn empty_then_wrong_type_are_distinct_failures() {
    let empty = Fixture::new("# t\n\n![s](assets/flow.svg)\n");
    empty.asset("assets/flow.svg", b"");
    let (code, out) = run_script(&[&empty.readme_path()]);
    assert_eq!(code, 1);
    assert!(
        out.contains("empty"),
        "0-byte svg should read as empty:\n{out}"
    );

    let typed = Fixture::new("# t\n\n![s](assets/flow.svg)\n");
    typed.asset("assets/flow.svg", b"just some text, no svg element");
    let (code, out) = run_script(&[&typed.readme_path()]);
    assert_eq!(code, 1);
    assert!(
        out.contains("wrong type"),
        "text under .svg should read as wrong type:\n{out}"
    );
}

/// A README whose references are all external or same-document parses no
/// local reference at all. That must fail, not pass: a regex rotted into
/// matching nothing would otherwise green-light every future README.
#[test]
fn vacuous_pass_is_rejected() {
    let f = Fixture::new("# t\n\n[ext](https://example.com/a.png) [anchor](#t)\n");

    let (code, out) = run_script(&[&f.readme_path()]);
    assert_eq!(
        code, 1,
        "parsing no local reference must fail, not pass vacuously:\n{out}"
    );
    assert!(
        out.contains("no local reference"),
        "should say why it failed:\n{out}"
    );
}

#[test]
fn missing_scanned_file_is_a_usage_error_not_a_reference_failure() {
    let nowhere = tempfile::tempdir().expect("temp dir");
    let (code, _out) = run_script(&[&nowhere.path().join("nope.md")]);
    assert_eq!(code, 2, "a vanished scanned file is usage, exit 2");
}
