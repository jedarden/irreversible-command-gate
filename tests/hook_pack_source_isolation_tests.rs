//! The hook enforces the installed chain only — even from inside a checkout.
//!
//! Operator commands have a separate source-resolution chain that may inspect
//! an explicit or checkout source; the hook must not. The deployment guide
//! states the contract outright ("It never reads a repository checkout") and
//! AGENTS.md calls `ICG_INSTALLED_PACK_DIR` "an operator-command-only override
//! the hook never reads" — but the labeling and drift tests
//! (`pack_source_drift_tests.rs`) exercise operator front-ends only, so the
//! hook half of that contract lived in prose alone. A hook that started
//! consulting the working directory would silently enforce unreviewed
//! checkout packs in every guarded session — the exact failure the pack
//! source labeling exists to make visible.
//!
//! These tests run the real hook binary from a staged checkout that carries
//! its own `packs/`, and pin what it denies: the installed chain's packs,
//! and nothing else. Every hook run isolates its health, telemetry and
//! denial-log sinks the same way `hook_pack_directory_tests.rs` does.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use tempfile::TempDir;

const INSTALLED_OVERRIDE: &str = "ICG_INSTALLED_PACK_DIR";

/// A minimal valid pack, self-contained so shipped-pack drift cannot move
/// what the fixtures below mean. The keyword derives from the id, so a
/// command can be written against it: `<id>ctl destroy`. Identical to the
/// generator in `pack_source_drift_tests.rs` — the two files pin the two
/// halves of one contract, and their fixtures should not drift apart.
fn pack_json(id: &str) -> String {
    pack_json_with_keyword(id, &format!("{id}ctl"))
}

fn pack_json_with_keyword(id: &str, keyword: &str) -> String {
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

fn write_pack(dir: &Path, id: &str) {
    fs::write(dir.join(format!("{id}.json")), pack_json(id)).expect("fixture pack should write");
}

fn write_pack_with_keyword(dir: &Path, id: &str, keyword: &str) {
    fs::write(
        dir.join(format!("{id}.json")),
        pack_json_with_keyword(id, keyword),
    )
    .expect("fixture pack should write");
}

/// A staged world: an "installed" trust directory with `installed_ids`, a
/// checkout whose `packs/` carries `checkout_ids`, and the hook to run
/// against them from inside that checkout.
struct Staged {
    _dir: TempDir,
    installed: PathBuf,
    checkout: PathBuf,
    support: PathBuf,
}

fn stage(installed_ids: &[&str], checkout_ids: &[&str]) -> Staged {
    let dir = TempDir::new().expect("temporary directory");
    let installed = dir.path().join("installed");
    let checkout = dir.path().join("checkout");
    let repository = checkout.join("packs");
    let support = dir.path().join("support");
    fs::create_dir(&installed).expect("installed directory");
    fs::create_dir(&checkout).expect("checkout directory");
    fs::create_dir(&repository).expect("checkout packs directory");
    fs::create_dir(&support).expect("support directory");
    for id in installed_ids {
        write_pack(&installed, id);
    }
    for id in checkout_ids {
        write_pack(&repository, id);
    }
    Staged {
        _dir: dir,
        installed,
        checkout,
        support,
    }
}

impl Staged {
    /// Run `icg hook` inside the staged checkout with `rule_pack` as the
    /// hook's chain — through `ICG_RULE_PACK` (the production shape) or the
    /// `--rule-pack` flag — and a Bash command on stdin.
    fn run_hook(&self, rule_pack: &Path, via_env: bool, command: &str) -> Value {
        self.spawn_hook(rule_pack, via_env, &[], command)
    }

    /// Run the hook with an extra environment mapping staged alongside the
    /// chain — the shape an operator-command override would leak through.
    fn run_hook_with_extra_env(
        &self,
        rule_pack: &Path,
        extra: &[(&str, &Path)],
        command: &str,
    ) -> Value {
        self.spawn_hook(rule_pack, true, extra, command)
    }

    /// Run the hook with every competing source staged at once. The explicit
    /// hook argument and `ICG_RULE_PACK` are hook inputs; the other two
    /// variables belong only to operator source resolution and must never
    /// change this result.
    fn run_hook_with_sources(
        &self,
        explicit_rule_pack: Option<&Path>,
        rule_pack_env: Option<&Path>,
        operator_pack_env: Option<&Path>,
        installed_override: Option<&Path>,
        command: &str,
    ) -> Value {
        let mut hook = Command::new(env!("CARGO_BIN_EXE_icg"));
        hook.arg("hook");
        if let Some(path) = explicit_rule_pack {
            hook.args([
                "--rule-pack",
                path.to_str().expect("pack path should be UTF-8"),
            ]);
        }
        self.configure_hook_command(&mut hook);
        hook.env_remove("ICG_RULE_PACK");
        hook.env_remove("ICG_PACK_DIR");
        hook.env_remove(INSTALLED_OVERRIDE);
        if let Some(path) = rule_pack_env {
            hook.env(
                "ICG_RULE_PACK",
                path.to_str().expect("pack path should be UTF-8"),
            );
        }
        if let Some(path) = operator_pack_env {
            hook.env(
                "ICG_PACK_DIR",
                path.to_str().expect("pack path should be UTF-8"),
            );
        }
        if let Some(path) = installed_override {
            hook.env(
                INSTALLED_OVERRIDE,
                path.to_str().expect("pack path should be UTF-8"),
            );
        }
        self.finish_hook(&mut hook, command)
    }

    fn spawn_hook(
        &self,
        rule_pack: &Path,
        via_env: bool,
        extra: &[(&str, &Path)],
        command: &str,
    ) -> Value {
        let rule_pack = rule_pack.to_str().expect("pack path should be valid UTF-8");
        let mut hook = Command::new(env!("CARGO_BIN_EXE_icg"));
        hook.arg("hook");
        if via_env {
            hook.env("ICG_RULE_PACK", rule_pack);
        } else {
            hook.args(["--rule-pack", rule_pack]);
        }
        self.configure_hook_command(&mut hook);
        // The staged world must decide the outcome, not the host's: drop
        // both operator-side overrides unless a test stages them itself.
        hook.env_remove("ICG_PACK_DIR");
        hook.env_remove(INSTALLED_OVERRIDE);
        for (key, value) in extra {
            hook.env(key, value);
        }
        hook.current_dir(&self.checkout);
        self.finish_hook(&mut hook, command)
    }

    fn configure_hook_command(&self, hook: &mut Command) {
        // Test-spawned hooks record their side effects into the staged
        // support directory, never the host's live sinks.
        hook.env("ICG_HEALTH_PATH", self.support.join("health.json"));
        hook.env("ICG_TELEMETRY_PATH", self.support.join("telemetry.json"));
        hook.env("ICG_DENIAL_LOG", self.support.join("denials.jsonl"));
        hook.current_dir(&self.checkout);
        hook.stdin(Stdio::piped());
        hook.stdout(Stdio::piped());
        hook.stderr(Stdio::piped());
    }

    fn finish_hook(&self, hook: &mut Command, command: &str) -> Value {
        let mut child = hook.spawn().expect("hook process should start");
        let payload = json!({
            "tool_name": "Bash",
            "tool_input": {"command": command},
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
}

fn decision(response: &Value) -> Option<&str> {
    response["hookSpecificOutput"]["permissionDecision"].as_str()
}

fn reason(response: &Value) -> String {
    response["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The hook denied the command, and attributes the denial to the given pack
/// and pattern — the attribution the denial log and coverage reports key on.
fn assert_denied_by(response: &Value, pack_id: &str, pattern_id: &str) {
    assert_eq!(
        decision(response),
        Some("deny"),
        "the command should be denied: {response}"
    );
    let denial_reason = reason(response);
    assert!(
        denial_reason.contains(&format!("pack={pack_id}")),
        "the denial should name the pack: {denial_reason}"
    );
    assert!(
        denial_reason.contains(&format!("pattern={pattern_id}")),
        "the denial should name the pattern: {denial_reason}"
    );
}

/// The hook raised no veto against the command, and nothing in the response
/// attributes one to the given pack. Allow verdicts may omit
/// `permissionDecision` entirely — an absent decision is a valid no-veto —
/// so the field's absence counts as not denied.
fn assert_not_denied_by(response: &Value, pack_id: &str) {
    assert_ne!(
        decision(response),
        Some("deny"),
        "a checkout-only pack must never deny: {response}"
    );
    assert!(
        !response.to_string().contains(&format!("pack={pack_id}")),
        "no denial may be attributed to the checkout pack: {response}"
    );
}

/// The fixture pack denies when the hook is pointed straight at it. Without
/// this control, the "never denied" assertions below would pass vacuously —
/// a malformed fixture never matches anything, and a hook that enforced
/// nothing would look isolated.
#[test]
fn the_fixture_pack_denies_when_the_hook_is_pointed_at_it() {
    let staged = stage(&[], &["checkout"]);
    let response = staged.run_hook(
        &staged.checkout.join("packs"),
        false,
        "checkoutctl destroy all",
    );
    assert_denied_by(&response, "checkout", "checkout-destroy");
}

/// A hook running inside a checkout enforces the installed chain it was
/// configured with: each staged installed pack denies its own command.
#[test]
fn a_hook_inside_a_checkout_enforces_the_installed_chain() {
    let staged = stage(&["alpha", "beta"], &["alpha", "beta", "checkout"]);
    let alpha = staged.run_hook(&staged.installed, true, "alphactl destroy everything");
    assert_denied_by(&alpha, "alpha", "alpha-destroy");
    let beta = staged.run_hook(&staged.installed, true, "betactl destroy everything");
    assert_denied_by(&beta, "beta", "beta-destroy");
}

/// The incident shape, pinned on the hook side: the working directory
/// carries a pack the installed set lacks — a source the operator commands
/// may inspect — and the hook still does not enforce it.
/// If the hook ever starts unioning the checkout's `packs/`, this goes red.
#[test]
fn a_hook_inside_a_checkout_never_enforces_a_checkout_only_pack() {
    let staged = stage(&["alpha"], &["checkout"]);
    let response = staged.run_hook(&staged.installed, true, "checkoutctl destroy everything");
    assert_not_denied_by(&response, "checkout");
    // The hook did load the installed chain — the same run's installed pack
    // still denies, so the silence about the checkout pack is resolution,
    // not a hook that loaded nothing at all.
    let installed = staged.run_hook(&staged.installed, true, "alphactl destroy everything");
    assert_denied_by(&installed, "alpha", "alpha-destroy");
}

/// `ICG_INSTALLED_PACK_DIR` stages the installed side for operator commands;
/// the hook's chain is `ICG_RULE_PACK` and the trust directory alone. An
/// override directory staged into the hook's environment must not add a
/// single enforceable pack.
#[test]
fn the_operator_pack_override_never_reaches_the_hook() {
    let dir = TempDir::new().expect("temporary directory");
    let operator_only = dir.path().join("operator-only");
    fs::create_dir(&operator_only).expect("operator-only directory");
    write_pack(&operator_only, "victima");

    let staged = stage(&["alpha"], &[]);
    let response = staged.run_hook_with_extra_env(
        &staged.installed,
        &[
            (INSTALLED_OVERRIDE, operator_only.as_path()),
            ("ICG_PACK_DIR", operator_only.as_path()),
        ],
        "victimactl destroy everything",
    );
    assert_not_denied_by(&response, "victima");
    let installed = staged.run_hook(&staged.installed, true, "alphactl destroy everything");
    assert_denied_by(&installed, "alpha", "alpha-destroy");
}

/// The hook's source precedence is deterministic even when every competing
/// source carries the same pack id with a different policy. An explicit
/// `--rule-pack` wins over `ICG_RULE_PACK`; without the explicit argument,
/// `ICG_RULE_PACK` wins. In both cases neither the operator-only environment
/// nor the checkout can add or replace a hook rule.
#[test]
fn hook_precedence_is_explicit_then_rule_pack_env_and_never_operator_or_checkout() {
    let staged = stage(&[], &[]);
    let environment = staged._dir.path().join("environment");
    let operator = staged._dir.path().join("operator");
    let installed_override = staged._dir.path().join("installed-override");
    fs::create_dir(&environment).expect("environment directory should exist");
    fs::create_dir(&operator).expect("operator directory should exist");
    fs::create_dir(&installed_override).expect("installed override should exist");

    // Every source claims the same pack id, but only its source-specific
    // command can match. This catches both accidental source unioning and a
    // lower-priority copy silently replacing the selected pack.
    write_pack_with_keyword(&staged.installed, "collision", "installed-source");
    write_pack_with_keyword(
        &staged.checkout.join("packs"),
        "collision",
        "checkout-source",
    );
    write_pack_with_keyword(&environment, "collision", "environment-source");
    write_pack_with_keyword(&operator, "collision", "operator-source");
    write_pack_with_keyword(&installed_override, "collision", "override-source");

    let candidates = [
        ("installed", "installed-source", "collision"),
        ("checkout", "checkout-source", "collision"),
        ("environment", "environment-source", "collision"),
        ("operator", "operator-source", "collision"),
        ("installed override", "override-source", "collision"),
    ];

    // The explicit hook argument is authoritative, even when all other
    // environment paths point at conflicting copies of the same pack.
    for (source, keyword, pack_id) in candidates {
        let response = staged.run_hook_with_sources(
            Some(&staged.installed),
            Some(&environment),
            Some(&operator),
            Some(&installed_override),
            &format!("{keyword} destroy everything"),
        );
        if source == "installed" {
            assert_denied_by(&response, pack_id, "collision-destroy");
        } else {
            assert_not_denied_by(&response, pack_id);
        }
    }

    // With no explicit argument, the hook environment is authoritative. The
    // operator's ICG_PACK_DIR and ICG_INSTALLED_PACK_DIR seams, plus the
    // checkout's packs/, remain invisible to hook evaluation.
    for (source, keyword, pack_id) in candidates {
        let response = staged.run_hook_with_sources(
            None,
            Some(&environment),
            Some(&operator),
            Some(&installed_override),
            &format!("{keyword} destroy everything"),
        );
        if source == "environment" {
            assert_denied_by(&response, pack_id, "collision-destroy");
        } else {
            assert_not_denied_by(&response, pack_id);
        }
    }
}
