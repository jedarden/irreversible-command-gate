//! Characterization of `$( )` command substitution in `lex_shell_commands`.
//!
//! The lexer has no `$( )` awareness: `$` and `(` are ordinary word
//! characters, so what happens depends entirely on the quoting context the
//! substitution appears in. This suite pins each context's present behavior;
//! expectations that are not true yet ship `#[ignore]` with a reference to
//! irrevers-f7201bd7 so a later child can un-ignore them.
//!
//! * Top level: the words inside `$( )` are lexed more or less normally --
//!   a `<<` heredoc there is honored and its body consumed, and `;`/`&&`
//!   still act as command boundaries -- but the opening `$(` glues onto the
//!   first inner word, so that one command dispatches as executable
//!   `$(cat`/`$(bao` and matches no pack.
//! * Inside a double-quoted word the whole substitution is one inert blob
//!   until a `"` inside the nested context closes the outer word, which is
//!   what fragments the corpus shape `git commit -m "$(cat <<'EOF' ... )"`.
//! * Inside a single-quoted word it stays literal text.
//! * Backticks get no substitution treatment either. That is an explicit
//!   deferral, not an oversight.
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
/// command boundaries, so every inner command *after the first* surfaces
/// with a clean executable. The first one does not -- see the ignored test
/// below.
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

/// The corpus shape's security properties hold today (parent bead
/// irrevers-3e313b79 verified both directions): prose naming a destructive
/// command inside a requoted message is documentation, and it must not
/// manufacture an invocation even while the message fragments.
#[test]
fn requoted_prose_does_not_become_an_invocation() {
    let command = "git commit -m \"$(cat <<'EOF'\ndocs: why \"bao kv destroy\" is denied\nEOF\n)\"";
    assert!(
        !denied(command),
        "writing the words `bao kv destroy` into a commit message is prose, \
         not a destructive invocation"
    );
    assert!(
        !executables(command).iter().any(|e| e == "bao"),
        "the fragmented message must not leak a bao executable token"
    );
}

/// The opening `$(` is glued onto the first inner word, so the inner `cat`
/// dispatches as executable `$(cat` and matches no pack. Once the lexer
/// treats `$( )` as a nested context, the inner command must surface as
/// itself.
#[test]
#[ignore = "opening $( glues into the first inner word ($(cat)); tracked as irrevers-f7201bd7"]
fn top_level_substitution_first_inner_command_is_not_glued() {
    let command = "$(cat <<'EOF' > /tmp/n\nbody\nEOF\necho done)";
    assert!(
        executables(command).contains(&"cat".to_string()),
        "the inner cat must dispatch as `cat`, not `$(cat`"
    );
}

/// Same glue, denial-level: the *first* inner command after a separator is
/// today the only one hidden (`$(bao` matches no pack keyword). It must
/// reach the openbao pack like its siblings after the next separator do.
#[test]
#[ignore = "first inner word after $( is glued ($(bao); tracked as irrevers-f7201bd7"]
fn a_first_inner_command_after_a_separator_reaches_dispatch() {
    assert!(denied("$(bao kv destroy secret/x; echo done)"));
}

/// The corpus shape from the parent bead. Today the first `"` inside the
/// body closes the outer word, so the message arrives as three fragmented
/// argv words after `-m`. The nested-context fix must deliver it as one
/// word; the denial-level consequence (`git-commit-without-pathspec`
/// matching again) is pinned as the ignored test in
/// tests/heredoc_lexing_tests.rs.
#[test]
#[ignore = "a double quote inside the nested context closes the outer word; tracked as irrevers-f7201bd7"]
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

/// Inside a double-quoted word nothing inside `$( )` is lexed today, so
/// inner commands never dispatch. With a nested context they must, even
/// though the surrounding quotes make the substitution an operand.
#[test]
#[ignore = "quoted $( ) content is one inert blob today; tracked as irrevers-f7201bd7"]
fn a_substitution_inside_double_quotes_surfaces_inner_commands() {
    assert!(denied("echo \"$(bao kv destroy secret/x && ls)\""));
}
