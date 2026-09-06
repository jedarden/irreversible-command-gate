//! A heredoc body is literal text, not shell to be lexed.
//!
//! `lex_shell_commands` had no `<<` handling at all, so an apostrophe inside a
//! heredoc body opened a quote that never closed. The remainder of the command
//! collapsed into one unterminated word and no executable token was emitted
//! after it -- and since command-mode packs are dispatched by executable
//! basename, every one of them was skipped for the rest of the command.
//!
//! Six of the ten shipped packs dispatch that way (openbao, git, docker,
//! beads, misc, tmux), which is every Critical destructive rule. 2.6% of real
//! agent traffic hits the shape: ordinary prose with a contraction, written to
//! a file, in the same tool call as a real command.

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

#[test]
fn apostrophe_in_a_heredoc_body_does_not_hide_a_later_command() {
    let command =
        "cat <<'EOF' > /tmp/note\nopenssl-sys's build script\nEOF\nbao kv destroy secret/prod/db";
    assert!(
        denied(command),
        "an apostrophe in a heredoc body must not stop the openbao pack from \
         seeing the destroy that follows it"
    );
}

#[test]
fn double_quote_in_a_heredoc_body_does_not_hide_a_later_command() {
    let command =
        "cat <<'EOF' > /tmp/note\nhe said \"hello\"\nEOF\ndocker system prune --all --volumes";
    assert!(
        denied(command),
        "a double quote in a heredoc body must not hide the prune"
    );
}

#[test]
fn both_quote_kinds_in_a_heredoc_body_do_not_hide_a_later_command() {
    let command = "cat <<'EOF' > /tmp/n\nit's a \"test\"\nEOF\ndocker system prune --all --volumes";
    assert!(
        denied(command),
        "mixed quotes in a heredoc body must not hide the prune"
    );
}

#[test]
fn unquoted_and_dash_forms_behave_the_same() {
    for opener in ["<<EOF", "<<\"EOF\"", "<<-EOF"] {
        let command = format!("cat {opener} > /tmp/n\nit's fine\nEOF\nbao kv destroy secret/x");
        assert!(denied(&command), "{opener} body must not hide the destroy");
    }
}

/// The body is literal: a line that merely *contains* the terminator is not
/// the terminator, and text inside the body must not be dispatched as a
/// command of its own.
#[test]
fn the_body_ends_only_at_a_line_equal_to_the_terminator() {
    let command =
        "cat <<'EOF' > /tmp/n\nEOF is mentioned here\nnot EOF either\nEOF\nbao kv destroy secret/x";
    assert!(
        denied(command),
        "the destroy after the real terminator must be seen"
    );
}

/// A heredoc body must not be able to *manufacture* a denial either -- text
/// being written to a file is not a command being run.
#[test]
fn a_command_named_only_inside_a_heredoc_body_is_not_evaluated() {
    let command = "cat <<'EOF' > /tmp/notes.md\nNever run: bao kv destroy secret/prod\nEOF";
    assert!(
        !denied(command),
        "writing the words `bao kv destroy` into a file is documentation, not \
         a destructive invocation"
    );
}

/// An unterminated heredoc is malformed shell. Consuming to end-of-input is
/// the conservative reading; it must not panic or mis-attribute.
#[test]
fn an_unterminated_heredoc_does_not_panic() {
    let command = "cat <<'EOF' > /tmp/n\nit's unterminated\nbao kv destroy secret/x";
    let _ = engine().evaluate_command(&CommandSource::Hook(command.to_string()));
}

/// `<<<` is a herestring, not a heredoc: nothing after it should be swallowed.
#[test]
fn a_herestring_is_not_a_heredoc() {
    let command = "grep x <<< \"it's here\" ; bao kv destroy secret/x";
    assert!(
        denied(command),
        "a herestring must not swallow the rest of the line"
    );
}

/// The real corpus shape: a multi-line commit message written through a
/// quoted heredoc inside `$( )`, whose body contains both quote kinds.
///
/// The security properties hold -- see the two tests below. What does not yet
/// work is `git-commit-without-pathspec` matching, because the lexer does not
/// recurse into `$( )`: the first double quote in the body closes the outer
/// word and the rule is handed a fragment. Tracked as irrevers-3e313b79.
#[test]
#[ignore = "lexer does not recurse into $( ); tracked as irrevers-3e313b79"]
fn a_requoted_commit_message_still_reaches_the_pathspec_rule() {
    let command = "git commit -q -m \"$(cat <<'EOF'\nfix: openssl-sys's script says \"Could not find\"\nEOF\n)\"";
    assert!(denied(command));
}

/// The part that actually matters: a message whose body *mentions* a
/// destructive command is prose being written to a file, not an invocation.
#[test]
fn prose_inside_a_commit_message_is_not_an_invocation() {
    let command = "git commit src/a.rs -m \"$(cat <<'EOF'\ndocs: explain why \"bao kv destroy\" is denied\nEOF\n)\"";
    assert!(
        !denied(command),
        "documenting a destructive command must not be treated as running it"
    );
}

/// And a real destructive command after such a message must still be caught,
/// even though the message truncates the segment.
#[test]
fn a_real_command_after_a_truncating_message_is_still_caught() {
    let command = "git commit src/a.rs -m \"$(cat <<'EOF'\nmsg with \"quotes\" inside\nEOF\n)\" && bao kv destroy secret/x";
    assert!(
        denied(command),
        "the lexer must recover and still see the destroy"
    );
}
