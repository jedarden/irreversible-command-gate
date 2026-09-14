//! Characterization of `$( )` command substitution in `lex_shell_commands`.
//!
//! The lexer treats `$(` outside a single-quoted word as a nested command
//! context (child irrevers-c6f6c7b8's implementation of the approach settled
//! in irrevers-f7201bd7's characterization): the interior lexes as commands
//! with its own quote and heredoc state, and the matching `)` pops back to
//! the surrounding word.
//!
//! * Top level: the interior commands dispatch under their own basenames --
//!   `cat`, not `$(cat` -- and heredocs, quotes, and separators inside the
//!   substitution behave normally.
//! * Inside a double-quoted word the substitution still runs, so its interior
//!   commands dispatch too, and the surrounding word survives as one argv
//!   word. That is what lets the corpus shape
//!   `git commit -m "$(cat <<'EOF' ... )"` reach the anchored
//!   `git-commit-without-pathspec` regex whole again.
//! * Inside a single-quoted word it stays literal text.
//! * Backticks get no substitution treatment. That is an explicit deferral,
//!   not an oversight.
//!
//! The fix approach is settled in
//! [`docs/notes/command-substitution-lexing.md`](../docs/notes/command-substitution-lexing.md):
//! a nested-context stack in the lexer, not a relaxation of the anchored
//! `git-commit-without-pathspec` regex.

use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::load_pack;
use std::path::Path;

fn engine() -> Engine {
    let mut engine = Engine::new();
    let packs = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
    let mut entries: Vec<_> = std::fs::read_dir(&packs)
        .expect("packs/ readable")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    entries.sort();
    for path in entries {
        engine
            .load_pack(load_pack(&path).expect("pack loads"))
            .expect("pack loads");
    }
    engine
}

fn denied(command: &str) -> bool {
    matches!(
        engine().evaluate_command(&CommandSource::Hook(command.to_string())),
        CheckResult::Denied { .. }
    )
}

/// Executable basenames the lexer surfaces for pack dispatch. Token-level
/// characterization does not need the packs loaded, only the engine's
/// segmentation.
fn executables(command: &str) -> Vec<String> {
    Engine::new().get_executables(&CommandSource::Hook(command.to_string()))
}

/// Top level is the context that half-works. `<<` inside `$( )` is still a
/// heredoc redirection -- the lexer is not inside a quoted word there -- so
/// the body is consumed as literal text and lexing resumes after the
/// terminator. A command after the body, with either quote kind inside the
/// body, still reaches pack dispatch.
#[test]
fn top_level_substitution_still_honors_the_heredoc_inside_it() {
    let command =
        "$(cat <<'EOF' > /tmp/n\nthe body's \"quoted\" text\nEOF\nbao kv destroy secret/x)";
    assert!(
        denied(command),
        "a heredoc body inside a top-level $( ) must be consumed, not lexed -- \
         quotes in it must not hide the destroy after the terminator"
    );
}

/// `;` and `&&` between words that began inside a top-level `$( )` are still
/// command boundaries, so inner commands surface with a clean executable.
/// The first one used to glue to its `$(`; the not-glued tests below pin
/// that it now dispatches as itself.
#[test]
fn a_separator_inside_a_top_level_substitution_surfaces_later_inner_commands() {
    for separator in ["&&", ";"] {
        let command = format!("$(echo start {separator} bao kv destroy secret/x)");
        assert!(
            denied(&command),
            "an inner command after {separator} inside a top-level $( ) must \
             still reach the openbao pack"
        );
    }
}

/// The guard the future fix must not break: with no `$( )` present, operators
/// inside a double-quoted word are data. Nested-context handling must key on
/// the substitution opener, never on quotes alone.
#[test]
fn a_plain_double_quoted_word_keeps_operators_as_data() {
    let command = "echo \"run; bao kv destroy secret/x && ls\"";
    assert!(!denied(command), "a quoted operand is not a command list");
    assert_eq!(
        executables(command),
        vec!["echo".to_string()],
        "nothing inside a plain double-quoted word may dispatch as a command"
    );
}

/// `$` is not special to the lexer, so inside single quotes `$( ... )` is
/// literal text like any other. That is correct shell semantics and must
/// survive the nested-context fix.
#[test]
fn substitution_text_inside_a_single_quoted_word_stays_literal() {
    let command = "echo '$(bao kv destroy secret/x)'";
    assert!(
        !denied(command),
        "single-quoted substitution text is a string operand, not an invocation"
    );
    assert_eq!(
        executables(command),
        vec!["echo".to_string()],
        "no executable token may come out of single-quoted text"
    );
}

/// An unterminated `$(` is malformed shell. However the lexer recovers, it
/// must not panic -- in a quoted word, bare, or with a pending heredoc body.
#[test]
fn an_unterminated_substitution_does_not_panic() {
    for command in [
        "echo \"$(cat foo",
        "$(cat foo",
        "$(cat <<'EOF'\nunterminated",
    ] {
        let _ = executables(command);
        let _ = engine().evaluate_command(&CommandSource::Hook(command.to_string()));
    }
}

/// Backticks are command substitution too, but the lexer treats `` ` `` as an
/// ordinary word character: the inner words surface as operands of `echo`
/// and no inner executable dispatches. That is the *deferred* decision, not
/// an accident -- see docs/notes/command-substitution-lexing.md. If backticks
/// ever get nested-context treatment, this test must be flipped alongside
/// that change, not silently left behind.
#[test]
fn backtick_substitution_is_explicitly_deferred() {
    let command = "echo `bao kv destroy secret/x`";
    assert!(
        !denied(command),
        "backticks do not open a nested context today (deferred -- see the \
         notes file before changing this)"
    );
    assert_eq!(
        executables(command),
        vec!["echo".to_string()],
        "backtick content must not dispatch as commands while deferred"
    );
}

/// The corpus shape's security property (parent bead irrevers-3e313b79
/// verified both directions): prose naming a destructive command inside a
/// requoted message is documentation and must never manufacture an
/// *invocation* -- no `bao` executable token, no openbao denial.
///
/// What changed with the nested-context fix is the commit shape around the
/// prose. Before nesting, the body's first `"` fragmented the message and
/// `git commit -m "$(cat ...)"` reached the packs as fragments that no
/// anchored rule could match. Now the message arrives as one argv word, so
/// `git-commit-without-pathspec` sees the pathspec-less commit the shell
/// would actually run and denies it -- on the commit's own shape, never on
/// the words inside the message. That recovery is the coverage the parent
/// bead filed this lexer defect for.
#[test]
fn requoted_prose_denies_on_the_commit_shape_never_on_the_prose() {
    let command = "git commit -m \"$(cat <<'EOF'\ndocs: why \"bao kv destroy\" is denied\nEOF\n)\"";
    let result = engine().evaluate_command(&CommandSource::Hook(command.to_string()));
    let CheckResult::Denied {
        reason,
        pack_id,
        pattern_id,
        ..
    } = result
    else {
        panic!(
            "a pathspec-less commit whose message comes from a substitution \
             must reach git-commit-without-pathspec whole"
        );
    };
    assert_eq!(
        pattern_id, "git-commit-without-pathspec",
        "the denial must come from the commit shape; reason: {reason}"
    );
    assert_eq!(
        pack_id, "git",
        "the prose must not route the denial through any other pack"
    );
    assert!(
        !executables(command).iter().any(|e| e == "bao"),
        "the message must not leak a bao executable token"
    );
}

/// The same prose shape WITH a pathspec stays allowed: the redirect the
/// pathspec rule exists to give has already been followed.
#[test]
fn requoted_prose_with_a_pathspec_stays_allowed() {
    let command =
        "git commit src/a.rs -m \"$(cat <<'EOF'\ndocs: why \"bao kv destroy\" is denied\nEOF\n)\"";
    assert!(
        !denied(command),
        "a commit that names its paths is not a pathspec-less commit, \
         whatever its message contains"
    );
    assert!(
        !executables(command).iter().any(|e| e == "bao"),
        "the message must not leak a bao executable token"
    );
}

/// The opening `$(` used to glue onto the first inner word, dispatching it as
/// executable `$(cat` and matching no pack. With a nested context the inner
/// command surfaces as itself.
#[test]
fn top_level_substitution_first_inner_command_is_not_glued() {
    let command = "$(cat <<'EOF' > /tmp/n\nbody\nEOF\necho done)";
    assert!(
        executables(command).contains(&"cat".to_string()),
        "the inner cat must dispatch as `cat`, not `$(cat`"
    );
}

/// Same glue, denial-level: before nesting, the *first* inner command after a
/// separator was the only one hidden (`$(bao` matches no pack keyword). It
/// must reach the openbao pack like its siblings after the next separator.
#[test]
fn a_first_inner_command_after_a_separator_reaches_dispatch() {
    assert!(denied("$(bao kv destroy secret/x; echo done)"));
}

/// The corpus shape from the parent bead. Before nesting, the first `"` inside
/// the body closed the outer word and the message arrived as three fragmented
/// argv words after `-m`. The nested context delivers it as one word; the
/// denial-level consequence (`git-commit-without-pathspec` matching again) is
/// pinned in tests/heredoc_lexing_tests.rs.
#[test]
fn a_nested_double_quote_does_not_close_the_outer_word() {
    let command = "git commit -q -m \"$(cat <<'EOF'\nfix: openssl-sys's script says \"Could not find\"\nEOF\n)\"";
    let token = Engine::new()
        .segment_command(&CommandSource::Hook(command.to_string()))
        .into_iter()
        .find(|token| token.executable == "git")
        .expect("a git token must be produced");
    let message_index = token
        .args
        .iter()
        .position(|arg| arg == "-m")
        .expect("-m must be an argument");
    assert_eq!(
        token.args.len(),
        message_index + 2,
        "the message must arrive as exactly one argv word, not fragments"
    );
}

/// Inside a double-quoted word the substitution is still run by the shell, so
/// its interior commands must dispatch even though the surrounding quotes
/// make the substitution an operand.
#[test]
fn a_substitution_inside_double_quotes_surfaces_inner_commands() {
    assert!(denied("echo \"$(bao kv destroy secret/x && ls)\""));
}
