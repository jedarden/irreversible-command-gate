//! Shared recognition of ICG hook commands in harness configuration files.
//!
//! Both config installers ([`cursor_hooks`](super::cursor_hooks) for
//! `.cursor/hooks.json`, [`gemini_hooks`](super::gemini_hooks) for
//! `.gemini/settings.json`) decide ownership the same way: a hook entry is
//! ICG-owned when its `command` invokes the icg binary in hook mode for
//! that harness. The predicate and the shell-word machinery it needs live
//! here once, so the two installers cannot drift apart in what they
//! recognize — a drift would mean one installer duplicates entries the
//! other's output or fails to clean up its own.

use std::path::Path;

/// Is this `command` an entry the installer for `harness` owns?
///
/// Owned means: the command **starts with** a word that resolves to the
/// icg binary (its file name is `icg`), the word sequence includes the
/// `hook` subcommand, and it declares `--harness <harness>`. Starting with
/// the binary is the load-bearing part: every entry an installer writes
/// does, so the installer's own output is always recognized again
/// (idempotence), while a foreign command that merely mentions `icg hook
/// --harness <harness>` mid-line — as `echo`'s arguments, say — is
/// preserved untouched. Words are split the way a shell would, so a quoted
/// path — including one with spaces, which is exactly what [`shell_quote`]
/// writes — still reads as ICG-owned.
pub fn is_icg_hook_command_for(command: &str, harness: &str) -> bool {
    let tokens = shell_split(command);

    let Some(first) = tokens.first() else {
        return false;
    };
    let invokes_icg = Path::new(first)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "icg" || name == "icg.exe");
    let runs_hook_mode = tokens.iter().any(|token| token == "hook");
    let declares_harness = tokens
        .windows(2)
        .any(|pair| pair[0] == "--harness" && pair[1] == harness);

    invokes_icg && runs_hook_mode && declares_harness
}

/// Split a hook `command` into shell words, honoring the quoting a POSIX
/// shell (and the harnesses' own hook execution) would: single quotes
/// protect everything — with the `'\''` idiom folding back into a literal
/// quote — double quotes protect everything but `\` escapes, an unquoted
/// `\` escapes the next character, and unquoted whitespace separates
/// words. This is recognition, not execution: unknown constructs degrade
/// to literal characters, which keeps ownership detection conservative
/// rather than clever.
pub fn shell_split(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') | None => break,
                        Some(literal) => current.push(literal),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') | None => break,
                        Some('\\') => match chars.next() {
                            Some(escaped) => current.push(escaped),
                            None => break,
                        },
                        Some(literal) => current.push(literal),
                    }
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                    in_word = true;
                }
            }
            whitespace if whitespace.is_whitespace() => {
                if in_word {
                    tokens.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            other => {
                current.push(other);
                in_word = true;
            }
        }
    }
    if in_word {
        tokens.push(current);
    }
    tokens
}

/// Quote a path for inclusion in a hook `command` string. Paths made of
/// shell-safe characters (the common case: `/usr/local/bin/icg`) pass
/// through unquoted; anything else gets single quotes with embedded-quote
/// escaping.
pub fn shell_quote(text: &str) -> String {
    if text.is_empty() {
        return "''".to_string();
    }
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
    {
        return text.to_string();
    }
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_requires_binary_hook_mode_and_the_named_harness() {
        assert!(is_icg_hook_command_for(
            "/usr/local/bin/icg hook --harness cursor",
            "cursor"
        ));
        assert!(is_icg_hook_command_for(
            "icg hook --harness gemini-cli --rule-pack /etc/icg/packs",
            "gemini-cli"
        ));
        assert!(is_icg_hook_command_for(
            "'/opt/my tools/icg' hook --harness cursor",
            "cursor"
        ));

        // Not icg, not hook mode, or the other harness's name: not ours.
        assert!(!is_icg_hook_command_for(
            "echo icg hook --harness cursor",
            "cursor"
        ));
        assert!(!is_icg_hook_command_for(
            "/usr/local/bin/icg hook",
            "cursor"
        ));
        assert!(!is_icg_hook_command_for(
            "/usr/local/bin/icg hook --harness cursor",
            "gemini-cli"
        ));
        assert!(!is_icg_hook_command_for(
            "/usr/local/bin/icg hook --harness gemini-cli",
            "cursor"
        ));
        assert!(!is_icg_hook_command_for(
            "/usr/local/bin/icg coverage --list",
            "cursor"
        ));
        assert!(!is_icg_hook_command_for(
            "icg-hook --harness cursor",
            "cursor"
        ));
        assert!(!is_icg_hook_command_for("", "cursor"));
    }

    #[test]
    fn shell_quoted_paths_survive_a_recognition_round_trip() {
        // Whatever path an installer records, the predicate must recognize
        // it again — including the quoting styles shell_quote produces —
        // or a re-run would duplicate entries.
        for path in [
            "/usr/local/bin/icg",
            "/opt/my tools/icg",
            "/opt/it's/icg",
            "'/weird leading quote/icg",
        ] {
            let quoted = shell_quote(path);
            let command = format!("{quoted} hook --harness gemini-cli");
            assert!(
                is_icg_hook_command_for(&command, "gemini-cli"),
                "{command:?} (from path {path:?}) must read as ICG-owned"
            );
        }

        // And the split itself is honest about what a shell would see.
        assert_eq!(
            shell_split("'/opt/my tools/icg' hook --harness gemini-cli"),
            vec!["/opt/my tools/icg", "hook", "--harness", "gemini-cli"]
        );
        assert_eq!(
            shell_quote("/opt/it's/icg"),
            "'/opt/it'\\''s/icg'",
            "embedded single quotes use the '\\'' idiom"
        );
    }
}
