use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};

// Realistic, non-placeholder-shaped credential vectors so the guarded
// patterns are exercised the way a live leak would look. Every line that
// carries one ends in a gitleaks:allow marker: the coexisting
// org-rule-guard.py Write check (and this project's own secrets pack,
// once installed) excuses deliberate fixtures marked that way.
const GITHUB_TOKEN: &str = "ghp_Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56"; // gitleaks:allow
const GITHUB_PAT: &str = "github_pat_11AA22bb33CC44dd55EE66ff77GG88hh99II00jj"; // gitleaks:allow
const AWS_ACCESS_KEY_ID: &str = "AKIAZQ9XBNT4W2PLRJ7H"; // gitleaks:allow
const SLACK_TOKEN: &str = "xoxb-924AbcDef123GhiJkl456Mno"; // gitleaks:allow
const ANTHROPIC_API_KEY: &str = "sk-ant-Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56"; // gitleaks:allow
const PEM_HEADER: &str = "-----BEGIN RSA PRIVATE KEY-----"; // gitleaks:allow

fn load_secrets_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/secrets.json").expect("secrets pack should load"))
        .expect("secrets pack should validate");
    engine
}

fn assert_denied_by(engine: &Engine, command: &str, expected_pattern: &str) {
    let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
    match result {
        CheckResult::Denied {
            ref pack_id,
            ref pattern_id,
            ..
        } => {
            assert_eq!(pack_id, "secrets", "for {command:?}");
            assert_eq!(pattern_id, expected_pattern, "for {command:?}");
        }
        other => {
            panic!("expected {command:?} to be denied by secrets/{expected_pattern}, got {other:?}")
        }
    }
}

fn assert_allowed(engine: &Engine, command: &str) {
    let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
    assert!(
        matches!(result, CheckResult::Allowed),
        "expected {command:?} to be allowed, got {result:?}"
    );
}

#[test]
fn manifest_declares_unconditional_tier_one_deny_rules() {
    let pack = load_pack("packs/secrets.json").expect("secrets pack should load");

    assert_eq!(pack.id, "secrets");
    // The whole design point of this pack: empty tool_keywords makes the
    // engine scan the entire raw command string unconditionally instead of
    // basename-dispatching on a guarded executable (Architecture, plan.md).
    assert!(pack.tool_keywords.is_empty());
    assert!(pack.applies_to.is_empty());
    assert_eq!(pack.safe_patterns.len(), 4);
    assert_eq!(pack.guarded_patterns.len(), 6);

    let expected_rules = [
        "github-token",
        "github-fine-grained-pat",
        "aws-access-key-id",
        "slack-token",
        "anthropic-api-key",
        "pem-private-key-header",
    ];
    for (rule, expected_id) in pack.guarded_patterns.iter().zip(expected_rules) {
        assert_eq!(rule.id, expected_id);
        assert!(rule.enabled);
        assert_eq!(rule.tier, Tier::Tier1);
        assert_eq!(rule.severity, Severity::Critical);
        // A leaked credential exposes data but destroys nothing -- the
        // destructive flag is for coverage-diff regression detection of
        // destructive-operation rules, which this is not.
        assert!(!rule.destructive);
        assert_eq!(rule.redirect.channel, Channel::Deny);
        assert!(rule.redirect.rewrite_template.is_none());
        assert!(matches!(rule.check, Check::CommandRegex { .. }));
    }
}

#[test]
fn credential_values_are_denied_regardless_of_executable() {
    let engine = load_secrets_engine();

    // The canonical gap this pack closes: no guarded executable to
    // basename-match, so only the unconditional whole-command scan can
    // ever see these.
    assert_denied_by(
        &engine,
        &format!(r#"echo "{GITHUB_TOKEN}" >> notes.md"#),
        "github-token",
    );
    assert_denied_by(
        &engine,
        &format!(r#"printf '%s' "{AWS_ACCESS_KEY_ID}" > /tmp/aws.env"#),
        "aws-access-key-id",
    );
    assert_denied_by(
        &engine,
        &format!(r#"curl -sS -H "Authorization: token {GITHUB_TOKEN}" https://api.example/..."#),
        "github-token",
    );

    // Environment assignments, exports, and multi-segment pipelines: the
    // scan is over the raw command string, not tokenized dispatch.
    assert_denied_by(
        &engine,
        &format!(r#"ANTHROPIC_API_KEY={ANTHROPIC_API_KEY} curl https://api.example/..."#),
        "anthropic-api-key",
    );
    assert_denied_by(
        &engine,
        &format!(r#"export SLACK_TOKEN={SLACK_TOKEN}"#),
        "slack-token",
    );
    assert_denied_by(
        &engine,
        &format!(r#"echo hi && echo "{GITHUB_TOKEN}" > /tmp/f"#),
        "github-token",
    );

    // A command whose executable another pack guards (git) still gets the
    // secrets scan; secrets does not depend on any pack's dispatch.
    assert_denied_by(
        &engine,
        &format!(r#"git commit -m "rotate {SLACK_TOKEN}""#),
        "slack-token",
    );
}

#[test]
fn every_guarded_credential_shape_fires_on_its_own_pattern() {
    let engine = load_secrets_engine();

    let vectors: [(&str, &str); 5] = [
        (GITHUB_TOKEN, "github-token"),
        (GITHUB_PAT, "github-fine-grained-pat"),
        (AWS_ACCESS_KEY_ID, "aws-access-key-id"),
        (SLACK_TOKEN, "slack-token"),
        (ANTHROPIC_API_KEY, "anthropic-api-key"),
    ];
    for (value, pattern_id) in vectors {
        assert_denied_by(&engine, &format!(r#"echo "{value}" >> /tmp/f"#), pattern_id);
    }
}

#[test]
fn pem_private_key_in_a_heredoc_is_denied() {
    let engine = load_secrets_engine();

    let command =
        format!("cat > /tmp/key.pem <<'EOF'\n{PEM_HEADER}\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\nEOF");
    assert_denied_by(&engine, &command, "pem-private-key-header");

    // Bare header echoed into a variable, no heredoc at all.
    assert_denied_by(
        &engine,
        &format!(r#"KEY="{PEM_HEADER}" && echo done"#),
        "pem-private-key-header",
    );
}

#[test]
fn placeholder_shaped_fixtures_are_allowed() {
    let engine = load_secrets_engine();

    // Documentation stand-ins: the value's body carries a placeholder word
    // (mirrors org-rule-guard.py's PLACEHOLDER machinery) ...
    assert_allowed(
        &engine,
        "echo ghp_examplexxxxxxxxxxxxxxxxxxxxxx >> notes.md",
    );
    assert_allowed(&engine, "export AWS_KEY=AKIAIOSFODNN7EXAMPLE");
    assert_allowed(
        &engine,
        "curl -H \"Authorization: token github_pat_placeholder_value_00000000000000000000\"",
    );

    // ... or is a run of a single filler character (the real-world fixture
    // case org-rule-guard.py names: ghp_ + 40 literal x's). The word-list
    // safe pattern covers x-runs; the all-zero pattern covers 0-runs.
    assert_allowed(
        &engine,
        "echo ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx >> notes.md",
    );
    assert_allowed(
        &engine,
        "echo ghp_00000000000000000000000000000000 >> notes.md",
    );
    assert_allowed(
        &engine,
        "echo sk-ant-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx >> notes.md",
    );

    // PEM with a placeholder marker inside the 200-char window after the
    // header -- this repo's <app>-secret.yml.template convention.
    assert_allowed(
        &engine,
        "cat > key.pem <<'EOF'\n-----BEGIN PRIVATE KEY-----\nYOUR_KEY_HERE\nEOF",
    );

    // The explicit escape hatch, same marker org-rule-guard.py honors.
    assert_allowed(
        &engine,
        &format!(r#"echo "{GITHUB_TOKEN}" >> notes.md # gitleaks:allow"#),
    );

    // Ordinary commands with no credential shape at all.
    assert_allowed(&engine, "echo hello > notes.md");
    assert_allowed(&engine, "cargo test --workspace");
}

#[test]
fn safe_pattern_skip_is_pack_wide() {
    let engine = load_secrets_engine();

    // Documented coarseness vs org-rule-guard.py: safe patterns skip the
    // whole pack for the command, not just the matched value, so a command
    // carrying BOTH a placeholder fixture and a real token is allowed here
    // where org-rule-guard.py's per-match exemption would still deny the
    // real one. Accepted for the honest-agent threat model (mixing a real
    // credential into a fixture line is not a leak vector an honest agent
    // produces) and covered during coexistence by the per-match hook.
    let command =
        format!(r#"echo "fixture ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx real {GITHUB_TOKEN}""#);
    assert_allowed(&engine, &command);
}

#[test]
fn denial_reasons_do_not_echo_the_matched_value() {
    let engine = load_secrets_engine();

    let vectors = [
        GITHUB_TOKEN,
        GITHUB_PAT,
        AWS_ACCESS_KEY_ID,
        SLACK_TOKEN,
        ANTHROPIC_API_KEY,
        PEM_HEADER,
    ];
    for value in vectors {
        let result =
            engine.evaluate_command(&CommandSource::Hook(format!(r#"echo "{value}" > /tmp/f"#)));
        match result {
            CheckResult::Denied { ref reason, .. } => {
                assert!(
                    !reason.contains(value),
                    "denial reason must not repeat the credential value for {value:?}"
                );
            }
            other => panic!("expected a denial for a {value:?}-shaped command, got {other:?}"),
        }
    }
}

#[test]
fn wrapper_argv_carrying_a_token_is_also_denied() {
    // The hook front-end is the coverage claim (the wrapper never sees
    // commands for binaries it does not shadow), but when a shadowed
    // binary IS invoked with a credential in argv, the joined-argv scan
    // fires too -- incidental bonus coverage, not the design driver.
    let engine = load_secrets_engine();

    let argv = [
        "curl".to_string(),
        "-H".to_string(),
        format!("Authorization: token {GITHUB_TOKEN}"),
    ];
    let result = engine.evaluate_command(&engine.read_from_argv(argv.to_vec()));
    assert!(
        matches!(
            result,
            CheckResult::Denied {
                ref pack_id,
                ref pattern_id,
                ..
            } if pack_id == "secrets" && pattern_id == "github-token"
        ),
        "expected wrapper-argv credential to be denied, got {result:?}"
    );
}

#[test]
fn secrets_coexists_with_keyword_dispatched_packs() {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/misc.json").expect("misc pack should load"))
        .expect("misc pack should validate");
    engine
        .load_pack(load_pack("packs/secrets.json").expect("secrets pack should load"))
        .expect("secrets pack should validate");

    // Keyword dispatch still works alongside the unconditional scan ...
    assert!(matches!(
        engine.evaluate_command(&CommandSource::Hook("needle cleanup".to_string())),
        CheckResult::Denied { ref pack_id, .. } if pack_id == "misc"
    ));
    assert!(matches!(
        engine.evaluate_command(&CommandSource::Hook("needle status".to_string())),
        CheckResult::Allowed
    ));

    // ... and the unconditional scan is unaffected by the other pack.
    assert_denied_by(
        &engine,
        &format!(r#"echo "{SLACK_TOKEN}" >> /tmp/f"#),
        "slack-token",
    );
}
