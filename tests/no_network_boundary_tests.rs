//! No-network evaluation boundary tests (bead: irrevers-d95bad06).
//!
//! AGENTS.md rule 3 and the plan's architecture section make a precise
//! claim: evaluation is deterministic and performs **no network I/O**, with
//! one deliberate, scoped exception — the stale-remote-head lookup that runs
//! a single `git ls-remote` before an ordinary (non-force) `git push`,
//! allowed only because the push itself is already a network operation.
//!
//! Nothing in the tree enforced that claim before this suite; a module could
//! grow a fetch and every document would keep saying "no network". These
//! tests lock the boundary from three independent directions:
//!
//! 1. **Behavioral, whole process.** The real `icg check` binary runs with
//!    every process it could conceivably reach the network through shadowed
//!    by a spy shim that records the attempt and forwards to the real
//!    binary. Ordinary command and content evaluation must produce their
//!    verdicts with an empty spy log: nothing was spawned, so nothing could
//!    have touched the network. (A child process is this codebase's only
//!    route from evaluation to the network; the structural test pins that,
//!    too.)
//! 2. **The exception, exactly.** A push against a repo whose upstream is
//!    current is allowed, and the spy log then holds exactly the three git
//!    invocations of the stale-remote-head lookup — two local `rev-parse`s
//!    and one `ls-remote`. After the remote advances, the same push is
//!    denied by `git-stale-remote-head-push` with the same three
//!    invocations. A non-push command in the same repo — upstream
//!    configured and all — must spawn nothing: the lookup is gated on the
//!    push, not on the repository.
//! 3. **Structural.** The engine's only process spawns live inside
//!    `check_remote_head_stale`; that function's only caller is the
//!    `push_requires_current_remote_head` predicate; that predicate is
//!    gated on `git push` without `--force`; and across the crate, process
//!    spawns and in-process network APIs may appear only on the allowlisted
//!    non-evaluation surfaces (`documented_commands`' backup inspection, the
//!    wrapper's exec of the real binary, alerting/update/health_server). A
//!    new network capability has to edit this suite to exist.
//!
//! When the host has a working `strace`, a fourth test runs the same
//! batteries under `strace -f -e trace=network` and asserts the kernel saw
//! no internet-protocol socket at all — direct evidence rather than
//! inference. Skipped where strace or ptrace is unavailable; the other
//! layers do not depend on it.

use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn packs_dir() -> PathBuf {
    Path::new(ROOT).join("packs")
}

/// One input per verdict, every rule shipped in `packs/`: allow, warning
/// (secret read to stdout), rewrite (force-push de-escalation), deny.
const COMMAND_BATTERY: &[(&str, &str)] = &[
    ("git status", "ALLOW:"),
    ("bao kv get secret/app/db", "WARNING:"),
    ("git push --force origin main", "REWRITE:"),
    ("bao kv destroy secret/app/db", "DENIED by icg"),
];

/// Content mode is the Write/Edit/apply_patch path — file contents, not
/// commands. One allow and one deny from the shipped content packs.
const CONTENT_BATTERY: &[(&str, &str)] = &[
    ("meeting notes body\n", "ALLOW:"),
    ("image: ronaldraygun/armor:latest\n", "DENIED by icg"),
];

/// The stale-remote-head lookup's exact spawn sequence, as `check` performs
/// it for `git push origin main`: resolve the upstream, read the tracked
/// SHA, then the one live query. Nothing else may appear.
const STALE_LOOKUP_SPAWNS: [&str; 3] = [
    "git rev-parse --abbrev-ref --symbolic-full-name @{u}",
    "git rev-parse origin/main",
    "git ls-remote --heads origin main",
];

/// Every binary evaluation could conceivably reach the network through,
/// plus the generic shells. Each is shadowed by a spy shim during a run.
const SPAWNABLE: &[&str] = &[
    "git",
    "sh",
    "bash",
    "curl",
    "wget",
    "ssh",
    "nc",
    "ncat",
    "socat",
    "scp",
    "rsync",
    "ftp",
    "bao",
    "vault",
    "docker",
    "kubectl",
    "helm",
    "aws",
    "gcloud",
    "git-remote-http",
    "git-remote-https",
];

/// True when a PATH candidate is one of ICG's own wrapper symlinks rather
/// than the tool it shadows.
///
/// The release CI image (`argo-guarded-builder`) installs ICG as a PATH
/// wrapper by symlinking `git`, `cargo`, `npm` and friends in
/// `/usr/local/bin` to the `icg` binary, and puts that directory first in
/// PATH -- the self-referential protection the release workflow exists to
/// publish. Canonicalizing the candidate follows the symlink, so a wrapper
/// resolves to a file named `icg`.
fn is_icg_wrapper(candidate: &Path) -> bool {
    std::fs::canonicalize(candidate)
        .ok()
        .and_then(|resolved| {
            resolved
                .file_name()
                .map(|name| name == std::ffi::OsStr::new("icg"))
        })
        .unwrap_or(false)
}

/// Resolve `name` against this test process's own PATH, before any shimming,
/// skipping ICG's own wrapper symlinks.
///
/// Skipping them is not cosmetic -- without it this suite deadlocks in the
/// guarded builder, and only there. `real_binary("git")` returns
/// `/usr/local/bin/git`, which is a symlink to `icg`, so the spy shim execs
/// the wrapper; the wrapper resolves the real `git` by walking PATH while
/// skipping *its own* symlink, finds this run's spy shim (prepended to PATH
/// by `shimmed_path`), and execs that -- which execs the wrapper again. The
/// two shims trade the call forever, the binary produces no output, and the
/// CI pod sits until its deadline. On any ordinary host `/usr/local/bin/git`
/// does not exist, the genuine `git` is found, and the bug is invisible;
/// this is the same "skip my own symlink" rule the wrapper itself applies.
fn real_binary(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .as_deref()
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file() && !is_icg_wrapper(candidate))
}

/// A directory of spy shims plus the log they append to. Every spawable name
/// becomes a script that records `name args...` and then execs the real
/// binary, so a run behaves exactly as it would unshimmed. Names with no
/// real binary on the host still get a shim — log and exit 127 — so a spawn
/// attempt shows up in the log even where the tool is not installed.
struct SpyRun {
    _shims: TempDir,
    log: PathBuf,
}

impl SpyRun {
    fn start() -> SpyRun {
        let shims = TempDir::new().expect("spy shim directory should be created");
        let log = shims.path().join("spawn.log");
        for name in SPAWNABLE {
            let record = format!("printf '{name} %s\\n' \"$*\" >> {}\n", quote(&log));
            let body = match real_binary(name) {
                Some(real) => format!("#!/bin/sh\n{record}exec {} \"$@\"\n", real.display()),
                None => format!("#!/bin/sh\n{record}exit 127\n"),
            };
            fs::write(shims.path().join(name), body).expect("spy shim should be written");
            // Without the execute bit the engine's spawn fails outright and
            // the predicate fails open — an ALLOW that masquerades as a
            // passing boundary. The shims must actually run.
            #[cfg(unix)]
            fs::set_permissions(shims.path().join(name), fs::Permissions::from_mode(0o755))
                .expect("spy shim should be made executable");
        }
        SpyRun { _shims: shims, log }
    }

    /// Run the real `icg check` binary with the shims first on PATH and the
    /// operational sinks named into a scratch directory, from `cwd`.
    fn check(&self, args: &[&str], cwd: &Path) -> Output {
        let scratch = TempDir::new().expect("operational scratch directory");
        base_command(["check", "--pack"], cwd)
            .arg(packs_dir())
            .args(args)
            .env("PATH", shimmed_path(self._shims.path()))
            .env("ICG_DENIAL_LOG", scratch.path().join("denials.jsonl"))
            .env("ICG_STATE_PATH", scratch.path().join("state.json"))
            .env("ICG_TELEMETRY_PATH", scratch.path().join("telemetry"))
            .env("ICG_HEALTH_PATH", scratch.path().join("health"))
            .output()
            .expect("icg check should run")
    }

    /// What spawned, in order. No log at all means nothing was ever spawned.
    fn spawns(&self) -> Vec<String> {
        match fs::read_to_string(&self.log) {
            Ok(text) => text.lines().map(str::to_string).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Single-quote a path for use inside a `/bin/sh` script. Test paths never
/// contain a quote, but keep the shim honest rather than lucky.
fn quote(path: &Path) -> String {
    format!("'{}'", path.display())
}

fn shimmed_path(shims: &Path) -> OsString {
    let mut dirs = vec![shims.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(dirs).expect("PATH should join")
}

/// A bare `icg check` invocation over the shipped packs: exits 0 for every
/// verdict (parse stdout), with fail-closed policy variables cleared so the
/// run sees the shipped fail-open default regardless of the host's env.
fn base_command(const_args: [&str; 2], cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(const_args)
        .current_dir(cwd)
        .env_remove("ICG_FAIL_CLOSED")
        .env_remove("ICG_FAIL_CLOSED_POLICY");
    command
}

fn first_line(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Ordinary command evaluation — all four verdicts, shipped rules only —
/// must reach its verdict without spawning anything at all.
#[test]
fn ordinary_command_evaluation_spawns_nothing() {
    let manifest = Path::new(ROOT);
    for &(command, expected_prefix) in COMMAND_BATTERY {
        let spy = SpyRun::start();
        let output = spy.check(&["--command", command], manifest);
        assert!(
            output.status.success(),
            "`icg check` must stay successful for {command:?}: {:?}",
            output.status
        );
        let first = first_line(&output);
        assert!(
            first.starts_with(expected_prefix),
            "command {command:?} should be {expected_prefix:?} but was {first:?}"
        );
        let spawns = spy.spawns();
        assert!(
            spawns.is_empty(),
            "ordinary evaluation of {command:?} must not spawn anything, but saw {spawns:?}"
        );
    }
}

/// Content evaluation — the Write/Edit path — must reach its verdict
/// without spawning anything at all.
#[test]
fn content_evaluation_spawns_nothing() {
    let manifest = Path::new(ROOT);
    for &(content, expected_prefix) in CONTENT_BATTERY {
        let spy = SpyRun::start();
        let files = TempDir::new().expect("content scratch directory");
        let file = files.path().join("edit-target.yaml");
        fs::write(&file, content).expect("content file should be written");
        let path = file.to_str().expect("content path should be utf-8");

        let output = spy.check(&["--file", path], manifest);
        assert!(
            output.status.success(),
            "`icg check` must stay successful for content {:?}: {:?}",
            content,
            output.status
        );
        let first = first_line(&output);
        assert!(
            first.starts_with(expected_prefix),
            "content {content:?} should be {expected_prefix:?} but was {first:?}"
        );
        let spawns = spy.spawns();
        assert!(
            spawns.is_empty(),
            "content evaluation must not spawn anything, but saw {spawns:?}"
        );
    }
}

/// The one exception: the stale-remote-head lookup, and only for pushes.
#[test]
fn stale_remote_head_lookup_is_the_only_spawn_and_only_pushes_reach_it() {
    let workspace = TempDir::new().expect("git workspace");
    let root = workspace.path();
    let repo = root.join("repo");
    let remote = root.join("remote.git");
    let clone = root.join("clone");

    git(root, &["init", "-q", "-b", "main", "repo"]);
    configure_identity(&repo);
    write_commit(&repo, "seed.txt");
    git(root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);

    // Up-to-date upstream: the push is allowed, and reaching that verdict
    // costs exactly the lookup — two local rev-args and one live query.
    let spy = SpyRun::start();
    let output = spy.check(&["--command", "git push origin main"], &repo);
    let first = first_line(&output);
    assert!(
        first.starts_with("ALLOW:"),
        "an up-to-date push should be allowed but was {first:?}"
    );
    assert_eq!(
        spy.spawns(),
        STALE_LOOKUP_SPAWNS,
        "an up-to-date push must run exactly the stale-remote-head lookup"
    );

    // Advance the remote from a second clone; the local repo is now stale
    // and the same push is denied by the same lookup, still three spawns.
    git(root, &["clone", "-q", remote.to_str().unwrap(), "clone"]);
    configure_identity(&clone);
    write_commit(&clone, "ahead.txt");
    git(&clone, &["push", "-q", "origin", "main"]);

    let spy = SpyRun::start();
    let output = spy.check(&["--command", "git push origin main"], &repo);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        first_line(&output).starts_with("DENIED by icg"),
        "a stale push should be denied but was {stdout:?}"
    );
    assert!(
        stdout.contains("git-stale-remote-head-push"),
        "the denial should name the stale-remote-head rule: {stdout:?}"
    );
    assert_eq!(
        spy.spawns(),
        STALE_LOOKUP_SPAWNS,
        "a stale push must run exactly the stale-remote-head lookup, nothing more"
    );

    // Same repository, same configured upstream, non-push command: the
    // lookup is gated on the push, not on the repository's shape.
    let spy = SpyRun::start();
    let output = spy.check(&["--command", "git status"], &repo);
    let first = first_line(&output);
    assert!(
        first.starts_with("ALLOW:"),
        "git status should stay quiet but was {first:?}"
    );
    let spawns = spy.spawns();
    assert!(
        spawns.is_empty(),
        "a non-push command must not reach the lookup, but saw {spawns:?}"
    );
}

fn git(working_dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(working_dir)
        .status()
        .expect("git should run");
    assert!(status.success(), "git {args:?} should succeed");
}

fn configure_identity(repo: &Path) {
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Boundary Test"]);
}

fn write_commit(repo: &Path, name: &str) {
    fs::write(repo.join(name), "content\n").expect("commit file should be written");
    git(repo, &["add", name]);
    git(repo, &["commit", "-q", "-m", name, "--", name]);
}

// --- Structural layer ------------------------------------------------------

/// Byte offset of the start of the line holding a top-level item signature.
fn line_start(source: &str, signature: &str) -> usize {
    let at = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature:?} should exist in the source"));
    source[..at].rfind('\n').map_or(0, |newline| newline + 1)
}

/// Byte offset of the next top-level item after the one whose signature line
/// starts at `from` — the end of that item's body for confinement checks.
/// Top-level means column 0; indented `use`/`fn`/attribute lines inside a
/// body never match.
fn next_top_level_item(source: &str, from: usize) -> usize {
    const PREFIXES: [&str; 16] = [
        "pub fn ",
        "pub(crate) fn ",
        "fn ",
        "#[",
        "impl ",
        "struct ",
        "enum ",
        "mod ",
        "const ",
        "static ",
        "type ",
        "use ",
        "pub struct",
        "pub enum",
        "pub mod",
        "pub const",
    ];
    let mut offset = from;
    for line in source[from..].lines() {
        if offset > from && PREFIXES.iter().any(|prefix| line.starts_with(prefix)) {
            return offset;
        }
        offset += line.len() + 1;
    }
    source.len()
}

/// Every process spawn in the engine must live inside
/// `check_remote_head_stale`, and that function must have exactly one
/// caller: the push-gated predicate arm. This is the standing guard behind
/// "the engine does no network I/O" — a second caller, a widened gate, or a
/// spawn anywhere else in the engine fails here.
#[test]
fn the_engine_spawns_nothing_outside_the_stale_remote_head_lookup() {
    let engine = fs::read_to_string(Path::new(ROOT).join("src/engine.rs"))
        .expect("engine.rs should be readable");

    // Every mention of the one live network query — the call and its doc
    // comment — lives inside the lookup function; nothing anywhere else.
    let lookup_start = line_start(&engine, "pub fn check_remote_head_stale");
    let lookup_end = next_top_level_item(&engine, lookup_start);
    let lookup = &engine[lookup_start..lookup_end];
    assert!(
        lookup.contains("ls-remote"),
        "the ls-remote call must live inside check_remote_head_stale"
    );
    assert_eq!(
        engine.matches("ls-remote").count(),
        lookup.matches("ls-remote").count(),
        "git ls-remote must not appear anywhere outside check_remote_head_stale"
    );

    // Confinement: no evaluation code outside the lookup spawns processes.
    for (at, _) in engine.match_indices("Command::new") {
        assert!(
            at >= lookup_start && at < lookup_end,
            "Command::new outside check_remote_head_stale — evaluation must not spawn processes"
        );
    }

    // Exactly one caller, sitting inside the push-gated predicate arm.
    let sites: Vec<usize> = engine
        .match_indices("check_remote_head_stale(")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        sites.len(),
        2,
        "check_remote_head_stale must be defined once and called once — a second caller would widen the network exception"
    );
    let call = sites
        .into_iter()
        .find(|at| !(*at >= lookup_start && *at < lookup_end))
        .expect("one call site outside the definition");
    let arm = engine
        .find("\"push_requires_current_remote_head\"")
        .expect("the push predicate arm must exist");
    let fallback = engine[arm..]
        .find("\n            _ =>")
        .map_or(engine.len(), |at| arm + at);
    assert!(
        arm < call && call < fallback,
        "the lookup's only call must sit inside the push_requires_current_remote_head arm"
    );

    // The gate must be push-only and must keep force pushes on the Tier 1
    // rewrite path, which needs no network at all.
    let gate = &engine[arm..call];
    assert!(
        gate.contains("command.contains(\"git push\")"),
        "the lookup must be gated on push commands"
    );
    assert!(
        gate.contains("--force"),
        "force pushes must stay outside the lookup, on the rewrite path"
    );
}

/// The crate-wide boundary: process spawns and in-process network APIs may
/// exist only on the allowlisted, non-evaluation surfaces. Anything new has
/// to be added here — and justify itself in review — before it can run.
#[test]
fn process_and_network_capability_stays_on_the_allowlisted_surfaces() {
    const SPAWN_ALLOWLIST: &[&str] = &["engine.rs", "documented_commands.rs", "main.rs"];
    const NETWORK_ALLOWLIST: &[&str] = &["alerting.rs", "update.rs", "health_server.rs"];
    const NETWORK_MARKERS: &[&str] = &[
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "UnixStream",
        "ToSocketAddrs",
        "reqwest::",
        "hyper::",
        "tokio::net",
    ];

    let src = Path::new(ROOT).join("src");
    for entry in fs::read_dir(&src).expect("src directory should be readable") {
        let path = entry.expect("src entry should be readable").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).expect("source file should be readable");

        if SPAWN_ALLOWLIST.contains(&name.as_str()) {
            if name == "documented_commands.rs" {
                // tar inspection for `icg backup`. run_backup dispatches to
                // its private helpers create_backup/verify_backup, where the
                // spawns actually live, so the allowed region is run_backup's
                // whole subtree: run_backup through the next `pub` item.
                let backup_start = line_start(&text, "pub fn run_backup");
                let backup_end = text[backup_start..]
                    .find("\npub ")
                    .map_or(text.len(), |at| backup_start + at + 1);
                for (at, _) in text.match_indices("Command::new") {
                    assert!(
                        at >= backup_start && at < backup_end,
                        "documented_commands.rs must only spawn inside run_backup"
                    );
                }
            }
            if name == "main.rs" {
                // The wrapper may only exec the real binary it resolved.
                for (at, _) in text.match_indices("Command::new") {
                    assert!(
                        text[at..].starts_with("Command::new(&real_binary)"),
                        "main.rs may only exec the resolved real binary, saw: {}",
                        text[at..].split('{').next().unwrap_or("").trim()
                    );
                }
            }
        } else {
            assert_eq!(
                text.matches("Command::new").count(),
                0,
                "{name} spawns a process but is not on the spawn allowlist — \
                 if this is an evaluation surface, the no-network boundary \
                 does not hold"
            );
        }

        let network_hits: Vec<&str> = NETWORK_MARKERS
            .iter()
            .copied()
            .filter(|marker| text.contains(marker))
            .collect();
        if !NETWORK_ALLOWLIST.contains(&name.as_str()) {
            assert!(
                network_hits.is_empty(),
                "{name} references network APIs {network_hits:?} but is not on \
                 the network allowlist — if this is an evaluation surface, the \
                 no-network boundary does not hold"
            );
        }
    }
}

/// Kernel-level proof, where strace works: the same batteries produce their
/// verdicts with the kernel seeing no internet-protocol socket at all.
/// Skipped where strace or ptrace is unavailable — the behavioral and
/// structural layers carry the boundary there.
#[test]
fn the_kernel_sees_no_inet_socket_during_evaluation_when_strace_works() {
    let probe = TempDir::new().expect("strace probe directory");
    let usable = Command::new("strace")
        .args(["-f", "-o"])
        .arg(probe.path().join("probe.log"))
        .arg("true")
        .output();
    let Ok(usable) = usable else {
        eprintln!("strace is not installed; skipping the kernel-level layer");
        return;
    };
    if !usable.status.success() {
        eprintln!("ptrace is unavailable; skipping the kernel-level layer");
        return;
    }

    let manifest = Path::new(ROOT);
    let traces = TempDir::new().expect("trace directory");
    let mut joined = String::new();
    let mut trace = |args: &[&str], label: &str| {
        let log = traces.path().join(label);
        let output = Command::new("strace")
            .args(["-f", "-e", "trace=network", "-o"])
            .arg(&log)
            // The same argv base_command would run, one layer down.
            .arg(env!("CARGO_BIN_EXE_icg"))
            .args(["check", "--pack"])
            .arg(packs_dir())
            .args(args)
            .current_dir(manifest)
            .env_remove("ICG_FAIL_CLOSED")
            .env_remove("ICG_FAIL_CLOSED_POLICY")
            .env("ICG_DENIAL_LOG", traces.path().join("denials.jsonl"))
            .env("ICG_STATE_PATH", traces.path().join("state.json"))
            .env("ICG_TELEMETRY_PATH", traces.path().join("telemetry"))
            .env("ICG_HEALTH_PATH", traces.path().join("health"))
            .output()
            .expect("traced icg check should run");
        assert!(
            output.status.success(),
            "traced `icg check` must stay successful for {label}: {:?}",
            output.status
        );
        joined.push_str(
            &fs::read_to_string(&log)
                .unwrap_or_else(|_| panic!("strace should have written {label}")),
        );
    };

    for &(command, _) in COMMAND_BATTERY {
        trace(&["--command", command], "command-case");
    }
    for &(content, _) in CONTENT_BATTERY {
        let files = TempDir::new().expect("content scratch directory");
        let file = files.path().join("edit-target.yaml");
        fs::write(&file, content).expect("content file should be written");
        let path = file.to_str().expect("content path should be utf-8");
        trace(&["--file", path], "content-case");
    }

    assert!(
        !joined.contains("AF_INET"),
        "evaluation must not create an internet-protocol socket; \
         strace saw:\n{joined}"
    );
}
