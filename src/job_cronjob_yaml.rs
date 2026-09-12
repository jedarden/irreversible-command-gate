//! Pure content predicate for detecting `kind: Job` / `kind: CronJob` YAML.
//!
//! This is the content half of the Job/CronJob write guard absorbed from
//! `org-rule-guard.py` rule 2 (parent: irrevers-efe57f54): ArgoCD cannot
//! manage Job and CronJob manifests idempotently and their pods are not
//! ArgoCD-owned, so a Write/Edit or Codex `apply_patch` that introduces one
//! should be stopped before it lands. Like `src/github_workflows.rs` it does
//! no filesystem or environment I/O and no network I/O -- it is a lexical
//! scan over the content string a Write/Edit tool call or normalized
//! `apply_patch` hunk supplies, so it is safe to call from a hook that must
//! not add I/O to the fast path. The seam takes `&str`, so raw non-UTF-8
//! bytes are the caller's concern: converting them lossily is safe, and the
//! replacement characters simply never spell `kind` or `Job`, so corrupted
//! input declines rather than matches.
//!
//! The scan is line-anchored, not a YAML parse. A line trips the guard only
//! when its mapping key is exactly `kind` (any case, with or without quotes)
//! and its value is exactly `Job` or `CronJob` (any case, bare or quoted) up
//! to end of line or a trailing comment. That anchor is what keeps the
//! named false positives quiet: `kind: Jobber` is a different scalar, a
//! comment line is not a declaration, `description: "kind: Job"` has a
//! different key, and a line *inside* a block scalar (`data:\n  script: |`)
//! is a string literal, not a document kind. Block scalars are tracked by
//! indentation; quoted multi-line scalars are not, which is a documented,
//! accepted limitation -- the guard is a backstop for an honest agent, not a
//! YAML validator. Malformed or partial YAML never panics: every helper is
//! total over `&str` and simply declines to match what it cannot read.

/// The redirect shown when a `kind: Job` / `kind: CronJob` declaration is
/// denied.
///
/// Per the project's redirect policy (`docs/notes/redirect-not-just-block.md`)
/// a denial must be actionable, not just a block: the reason states *why* the
/// manifest is refused and *what to write instead* concretely enough that the
/// next step needs no research.
pub const BLOCKED_REASON: &str = "kind: Job / kind: CronJob manifests are blocked: ArgoCD \
    cannot manage them idempotently and their pods are not ArgoCD-owned, so they are never \
    pruned and hold resource reservations indefinitely. Instead of the Job/CronJob, write a \
    Deployment with an internal scheduling loop for recurring work, or an Argo WorkflowTemplate \
    for genuinely one-shot work; Argo WorkflowTemplates live in the declarative-config \
    repository under k8s/iad-ci/argo-workflows/.";

/// Pack attribution for the denial the engine emits when this guard trips.
///
/// The guard is built into the hook front-end (like `github-workflows`), not
/// loaded from a pack file, so these constants are how a downstream step
/// recognizes *this* guard among all denials the engine can emit. Pinned as
/// literals in the tests so an accidental rename fails a test instead of
/// silently breaking the denial's consumers.
pub const PACK_ID: &str = "job-cronjob-yaml";

/// Pattern attribution paired with [`PACK_ID`].
pub const PATTERN_ID: &str = "kind-job-cronjob";

/// Content fixtures that must trip the guard. This module's tests drive the
/// table through [`is_job_or_cronjob_yaml`], and the guard's wider coverage
/// (the engine's `evaluate_content` tests, and any hook-level integration
/// suite) reuses the same table rather than redefining its own spellings, so
/// a fixture added here is proven at every layer. Entries are commented with
/// the spelling each one exercises. Whitespace runs are written explicitly
/// (`\t`, doubled spaces) so the variant each entry proves stays visible.
pub const GUARDED_CONTENTS: &[&str] = &[
    // plainest Job manifest form
    "apiVersion: batch/v1\nkind: Job\nmetadata:\n  name: migrate\n",
    // the CronJob form
    "apiVersion: batch/v1\nkind: CronJob\nmetadata:\n  name: nightly\n",
    // case variant on the key, doubled space after the colon
    "Kind:  Job\n",
    // case variant on the value
    "kind: JoB\n",
    // everything uppercase
    "KIND: CRONJOB\n",
    // tab indentation and a tab after the colon
    "\tkind:\tJob\n",
    // extra spaces on both sides of the colon
    "kind :   Job\n",
    // double-quoted value
    "kind: \"Job\"\n",
    // single-quoted value
    "kind: 'CronJob'\n",
    // trailing comment after the value
    "kind: Job  # one-shot backfill\n",
    // CRLF line endings (a Windows-authored manifest)
    "kind: Job\r\nmetadata:\r\n  name: win\r\n",
    // the second document of a multi-document stream
    "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cm\n---\napiVersion: batch/v1\nkind: CronJob\nmetadata:\n  name: sweep\n",
    // document-start marker carrying the mapping inline
    "--- kind: Job\n",
];

/// False-positive fixtures that must never trip the guard, shared the same
/// way as [`GUARDED_CONTENTS`]. From irrevers-b559d088: longer words sharing
/// the prefix, comments, plain prose that merely mentions the phrase,
/// unrelated fields whose value merely contains the phrase, other workload
/// kinds, block-scalar string literals, and malformed or partial YAML.
pub const UNGUARDED_CONTENTS: &[&str] = &[
    // longer scalars sharing the `Job` prefix are different values
    "kind: Jobber\n",
    "kind: Jobs\n",
    "kind: CronJobset\n",
    // a value with trailing prose is a different scalar
    "kind: Job template\n",
    // other workload kinds
    "kind: Deployment\n",
    "kind: Pod\n",
    // a comment is not a declaration
    "# kind: Job\n",
    "apiVersion: v1\n# kind: CronJob\nkind: ConfigMap\n",
    // plain non-manifest prose that merely mentions the phrase
    "the kind: Job controller supersedes hand-rolled schedules\n",
    "notes: run a kind: CronJob nightly to sweep stale bundles\n",
    // an unrelated field whose value merely contains the phrase
    "description: \"kind: Job\"\n",
    "annotations:\n  note: kind: Job\n",
    "docs: https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/\n",
    // `kind` as the prefix of a longer key
    "kindliness: Job\n",
    // no whitespace after the colon is one plain scalar, not a mapping
    "kind:Job\n",
    // a sequence item is not the document kind
    "- kind: Job\n",
    // block scalars embed manifests as string literals
    "data:\n  seed-job.sh: |\n    #!/bin/sh\n    cat <<'EOF'\n    kind: Job\n    EOF\n",
    "manifests:\n  - |\n    kind: CronJob\n",
    // malformed and partial YAML must be declined, never panicked on
    "",
    ": :\n",
    "kind:\n",
    "kind: \"Job\n",
    "kind: Job#not-a-comment\n",
    "kind: Job,\n",
    "\u{0}\u{7}\n",
];

/// Structured outcome of checking a Write/Edit target's content against the
/// Job/CronJob guard.
///
/// This mirrors [`crate::github_workflows::Detection`]: a non-match is always
/// the explicit [`Detection::NoMatch`] variant, and a match carries both the
/// exact trimmed line that triggered it (`matched_line`) and the
/// human-readable `reason` -- a redirect-message step never needs to re-run
/// the scan to explain the denial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// The content declares `kind: Job` or `kind: CronJob`.
    Matched {
        /// The trimmed offending line, exactly as it appears in the content
        /// written to the file, so a redirect message can quote it back.
        matched_line: String,
        /// Human-readable redirect ([`BLOCKED_REASON`]).
        reason: String,
    },
    /// The content does not declare a Job or CronJob (or the path is not a
    /// YAML file the guard scopes itself to).
    NoMatch,
}

impl Detection {
    /// Does this detection represent a match?
    pub fn is_match(&self) -> bool {
        matches!(self, Detection::Matched { .. })
    }
}

/// Check a Write/Edit target against the Job/CronJob guard and return a
/// structured [`Detection`].
///
/// The guard is scoped to `.yaml`/`.yml` paths (case-insensitive), the same
/// scope `org-rule-guard.py` applies to its rule 2: prose, markdown, and
/// source files that merely *mention* `kind: Job` stay writable. Only the
/// new content is judged -- an Edit that fixes an existing Job (`kind: Job`
/// in the old text, `kind: Deployment` in the new) must stay allowed.
pub fn detect(file_path: &str, content: &str) -> Detection {
    if !is_yaml_path(file_path) {
        return Detection::NoMatch;
    }
    match find_kind_declaration(content) {
        Some(matched_line) => Detection::Matched {
            matched_line,
            reason: BLOCKED_REASON.to_string(),
        },
        None => Detection::NoMatch,
    }
}

/// Does `content` declare `kind: Job` or `kind: CronJob` anywhere, including
/// in a later document of a multi-document stream?
///
/// The single source of truth for the matching logic; [`detect`] wraps it
/// with the YAML path scope.
pub fn is_job_or_cronjob_yaml(content: &str) -> bool {
    find_kind_declaration(content).is_some()
}

/// Is `path` a YAML file the guard scopes itself to? Case-insensitive on the
/// extension, matching how `.YAML` on a case-insensitive filesystem is still
/// a manifest a cluster would load.
fn is_yaml_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".yaml") || lower.ends_with(".yml")
}

/// Scan `content` line by line for a `kind: Job` / `kind: CronJob` mapping
/// entry.
///
/// Returns the first offending line, trimmed. Tracks block scalars by
/// indentation so string-literal content inside `|` / `>` blocks is skipped:
/// once a line introduces a block scalar, every following line that is blank
/// or more indented is scalar content until a line dedents back to the
/// introducer's indentation or shallower. Partial or malformed input simply
/// fails to match; nothing here panics.
fn find_kind_declaration(content: &str) -> Option<String> {
    // Indentation width of the block-scalar introducer we are inside, if any.
    let mut block_scalar_indent: Option<usize> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim_end();
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        let body = &line[indent..];

        // Inside a block scalar: blank and more-indented lines are scalar
        // content, never mapping entries.
        if let Some(introducer_indent) = block_scalar_indent {
            if body.is_empty() || indent > introducer_indent {
                continue;
            }
            block_scalar_indent = None;
        }

        let Some(body) = document_body(body) else {
            continue;
        };

        if is_job_or_cronjob_declaration(body) {
            return Some(body.to_string());
        }

        if introduces_block_scalar(body) {
            block_scalar_indent = Some(indent);
        }
    }

    None
}

/// Classify one already-dedented line into the body worth parsing, or `None`
/// for lines that can never carry a mapping entry: blanks, comments, and
/// document markers. A `---` document-start with content on the same line
/// yields that content; a bare `---` or a `...` document-end yields `None`.
fn document_body(body: &str) -> Option<&str> {
    if body.is_empty() || body.starts_with('#') {
        return None;
    }

    let rest = match body.strip_prefix("---") {
        // `---` exactly, or a longer run of dashes, is not a marker with
        // inline content unless whitespace follows.
        Some(rest) if rest.is_empty() || rest.starts_with([' ', '\t']) => rest.trim_start(),
        Some(_) => body,
        None => body,
    };

    if rest.is_empty() || rest.starts_with('#') {
        return None;
    }
    // A `...` document-end may carry nothing but a comment.
    if rest
        .strip_prefix("...")
        .is_some_and(|tail| tail.is_empty() || tail.starts_with([' ', '\t']))
    {
        return None;
    }

    Some(rest)
}

/// Does this dedented line declare `kind: Job` or `kind: CronJob`?
fn is_job_or_cronjob_declaration(body: &str) -> bool {
    match mapping_value_after_kind_key(body) {
        Some(after_colon) => is_job_or_cronjob_value(after_colon),
        None => false,
    }
}

/// If `body` starts with a `kind` mapping key, return everything after the
/// `:`, else `None`.
///
/// The key may be quoted (`"kind": "Job"` is JSON-flavored YAML). The whole
/// key must be exactly `kind` (case-insensitive): `kindliness: Job` reads as
/// the key `kindliness`, and `kind:Job` with no whitespace after the colon
/// is one plain *scalar* in YAML, not a mapping entry, so neither matches.
/// Whitespace is allowed on either side of the colon (`kind : Job` is a
/// mapping entry per the YAML plain-scalar rules, because a colon followed
/// by whitespace always separates). For a *plain* key the separator colon
/// must therefore be followed by whitespace or end of line; a quoted key has
/// no such requirement (`"kind":"Job"` is a valid JSON-flavored mapping).
fn mapping_value_after_kind_key(body: &str) -> Option<&str> {
    if body.starts_with('"') || body.starts_with('\'') {
        let quote = body.as_bytes()[0] as char;
        let quoted = &body[1..];
        let (key, rest) = quoted.split_once(quote)?;
        if !key.eq_ignore_ascii_case("kind") {
            return None;
        }
        rest.trim_start_matches([' ', '\t']).strip_prefix(':')
    } else {
        let key_len = body.chars().take_while(|c| c.is_ascii_alphabetic()).count();
        if key_len == 0 {
            return None;
        }
        let (key, rest) = body.split_at(key_len);
        if !key.eq_ignore_ascii_case("kind") {
            return None;
        }
        let after_colon = rest.trim_start_matches([' ', '\t']).strip_prefix(':')?;
        // Plain-key rule: `:` separates only when followed by whitespace or
        // end of line. `kind:Job` never splits -- it is one plain scalar.
        if after_colon.is_empty() || after_colon.starts_with([' ', '\t']) {
            Some(after_colon)
        } else {
            None
        }
    }
}

/// Is the text after a `kind:` colon exactly the scalar `Job` or `CronJob`?
///
/// The value may be quoted, in which case it must close on the same line and
/// only a comment or whitespace may follow. A bare value must end at end of
/// line, whitespace, or a comment: `Jobber`, `Job#x` (no whitespace before
/// `#`, so YAML reads it into the scalar), and `Job,` are all different
/// scalars. Tags, anchors, aliases, and flow collections are declined by the
/// alphabetic-prefix check -- a plain `Job` never starts with `!`, `&`, `*`,
/// `{`, or `[`.
fn is_job_or_cronjob_value(after_colon: &str) -> bool {
    let value = after_colon.trim_start_matches([' ', '\t']);
    if value.is_empty() {
        return false;
    }

    if value.starts_with('"') || value.starts_with('\'') {
        let quote = value.as_bytes()[0] as char;
        let quoted = &value[1..];
        let Some((inner, rest)) = quoted.split_once(quote) else {
            // Unterminated quote: malformed input, never a match.
            return false;
        };
        let trailing = rest.trim_start_matches([' ', '\t']);
        return is_job_or_cronjob_scalar(inner)
            && (trailing.is_empty() || trailing.starts_with('#'));
    }

    let scalar_len = value
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .count();
    if scalar_len == 0 {
        return false;
    }
    let (scalar, rest) = value.split_at(scalar_len);
    if !is_job_or_cronjob_scalar(scalar) {
        return false;
    }
    // The scalar must end here: end of line, or horizontal whitespace with
    // at most a trailing comment. `Jobber`, `Job#x` and `Job,` stop here.
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return false;
    }
    let trailing = rest.trim_start_matches([' ', '\t']);
    trailing.is_empty() || trailing.starts_with('#')
}

/// Is `scalar` (case-insensitively) one of the two banned manifest kinds?
fn is_job_or_cronjob_scalar(scalar: &str) -> bool {
    scalar.eq_ignore_ascii_case("job") || scalar.eq_ignore_ascii_case("cronjob")
}

/// Does this dedented line introduce a block scalar (`|` / `>` with optional
/// chomping and indentation indicators)?
///
/// Both shapes YAML allows are recognized: a mapping entry
/// (`seed-job.sh: |`) and a sequence item owning the scalar directly
/// (`- |`). This is only ever used to *skip* subsequent more-indented lines,
/// never to match one, so a false positive here costs a missed match on
/// malformed input -- never a wrong denial.
fn introduces_block_scalar(body: &str) -> bool {
    let item = strip_sequence_indicator(body);
    if is_block_scalar_header(item) {
        return true;
    }
    mapping_value_is_block_header(item)
}

/// Strip one `- ` sequence-item indicator, if the line has one.
fn strip_sequence_indicator(body: &str) -> &str {
    match body.strip_prefix('-') {
        Some(rest) if rest.is_empty() || rest.starts_with([' ', '\t']) => {
            rest.trim_start_matches([' ', '\t'])
        }
        _ => body,
    }
}

/// Is `s` exactly a block-scalar header: `|` or `>` plus optional indicator
/// characters, then end of line or a comment?
fn is_block_scalar_header(s: &str) -> bool {
    let rest = match s.strip_prefix('|').or_else(|| s.strip_prefix('>')) {
        Some(rest) => rest,
        None => return false,
    };
    let indicator_len = rest
        .chars()
        .take_while(|c| matches!(c, '-' | '+' | '1'..='9'))
        .count();
    let trailing = rest[indicator_len..].trim_start_matches([' ', '\t']);
    trailing.is_empty() || trailing.starts_with('#')
}

/// Is the mapping value on this line a block-scalar header?
fn mapping_value_is_block_header(body: &str) -> bool {
    let Some((key, value)) = body.split_once(':') else {
        return false;
    };
    // The separator must be the YAML mapping forms: `:` at end of line or
    // followed by whitespace.
    if !(value.is_empty() || value.starts_with([' ', '\t'])) {
        return false;
    }
    let _ = key;
    is_block_scalar_header(value.trim_start_matches([' ', '\t']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shared_guarded_fixture_matches() {
        for content in GUARDED_CONTENTS {
            assert!(
                is_job_or_cronjob_yaml(content),
                "expected {content:?} to be guarded"
            );
        }
    }

    #[test]
    fn every_shared_unguarded_fixture_is_ignored() {
        for content in UNGUARDED_CONTENTS {
            assert!(
                !is_job_or_cronjob_yaml(content),
                "expected {content:?} to stay writable"
            );
        }
    }

    /// The acceptance cases named on the parent bead, each with its own
    /// assertion so a regression names the exact spelling that broke.
    mod acceptance_variants {
        use super::*;

        #[test]
        fn case_variants_on_key_and_value() {
            assert!(is_job_or_cronjob_yaml("Kind:  Job\n"));
            assert!(is_job_or_cronjob_yaml("kind: job\n"));
            assert!(is_job_or_cronjob_yaml("KIND: cronjob\n"));
            assert!(is_job_or_cronjob_yaml("kind: JOB\n"));
        }

        #[test]
        fn whitespace_variants() {
            assert!(is_job_or_cronjob_yaml("kind: Job")); // no trailing newline
            assert!(is_job_or_cronjob_yaml("kind:   Job\n"));
            assert!(is_job_or_cronjob_yaml("kind:\tJob\n"));
            assert!(is_job_or_cronjob_yaml("\tkind: Job\n"));
            assert!(is_job_or_cronjob_yaml("kind : Job\n"));
            assert!(is_job_or_cronjob_yaml("  kind: Job\n"));
        }

        #[test]
        fn quoted_values() {
            assert!(is_job_or_cronjob_yaml("kind: \"Job\"\n"));
            assert!(is_job_or_cronjob_yaml("kind: 'Job'\n"));
            assert!(is_job_or_cronjob_yaml("kind: \"CronJob\"\n"));
            assert!(is_job_or_cronjob_yaml("kind: \"Job\"  # quoted\n"));
        }

        #[test]
        fn quoted_keys() {
            assert!(is_job_or_cronjob_yaml("\"kind\": Job\n"));
            assert!(is_job_or_cronjob_yaml("'kind': 'CronJob'\n"));
        }

        #[test]
        fn multi_document_streams() {
            let job_second = "apiVersion: v1\nkind: ConfigMap\n---\nkind: Job\n";
            assert!(is_job_or_cronjob_yaml(job_second));
            let job_third = "---\nkind: ConfigMap\n---\nkind: Deployment\n---\nkind: CronJob\n";
            assert!(is_job_or_cronjob_yaml(job_third));
            // A document end between two documents does not hide either.
            assert!(is_job_or_cronjob_yaml(
                "kind: ConfigMap\n...\n---\nkind: Job\n"
            ));
        }

        #[test]
        fn named_false_positives_stay_writable() {
            // The value is a different scalar.
            assert!(!is_job_or_cronjob_yaml("kind: Jobber\n"));
            // A comment is not a declaration.
            assert!(!is_job_or_cronjob_yaml("# kind: Job\n"));
            // An unrelated field's value contains the phrase.
            assert!(!is_job_or_cronjob_yaml("note: \"kind: Job\"\n"));
        }

        #[test]
        fn block_scalar_string_literals_stay_writable() {
            // A ConfigMap embedding a script that *mentions* a Job manifest.
            let config_map = "data:\n  seed.sh: |\n    kind: Job\n    echo done\n";
            assert!(!is_job_or_cronjob_yaml(config_map));
            // A sequence item owning a block scalar directly.
            let sequence = "manifests:\n  - |\n    kind: CronJob\n";
            assert!(!is_job_or_cronjob_yaml(sequence));
            // A dedent back out of the scalar restores normal parsing: the
            // document after the scalar is still judged.
            let after_scalar = "data:\n  script: |\n    kind: Job\nkind: Deployment\n";
            assert!(!is_job_or_cronjob_yaml(after_scalar));
            let job_after_scalar = "data:\n  script: |\n    x: 1\nkind: Job\n";
            assert!(is_job_or_cronjob_yaml(job_after_scalar));
        }

        #[test]
        fn scalar_boundaries_reject_lookalikes() {
            // No whitespace before `#` means YAML folds it into the scalar.
            assert!(!is_job_or_cronjob_yaml("kind: Job#x\n"));
            // Trailing punctuation is part of the scalar.
            assert!(!is_job_or_cronjob_yaml("kind: Job,\n"));
            // Trailing prose is a different scalar.
            assert!(!is_job_or_cronjob_yaml("kind: Job template\n"));
            // Text after a closing quote is malformed YAML, not a match.
            assert!(!is_job_or_cronjob_yaml("kind: 'Job'x\n"));
        }

        /// Malformed and partial YAML must be declined without panicking.
        /// The battery includes interior NULs, control characters, deeply
        /// indented runs, unterminated quotes, and bare document markers --
        /// every helper is total over `&str`, and this proves it on the
        /// inputs most likely to be fed to the hook by a broken generator.
        #[test]
        fn malformed_input_never_panics() {
            for malformed in [
                "",
                "\n",
                ":",
                "::",
                ": :\n",
                "kind",
                "kind:",
                "kind: :",
                "kind: \"Job\n",
                "kind: 'Job\n",
                "\"kind\": Job\n",
                "\"kind\": \"Job\n",
                "---- kind: Job\n",
                "---",
                "...",
                "--- --- ---\n",
                "\u{0}\u{7}\n",
                "kind: Job\n\u{0}kind: CronJob\n",
                &format!("{}kind: Job\n", " ".repeat(10_000)),
                &"\t".repeat(10_000),
                "kind: Job".repeat(1_000).as_str(),
            ] {
                let _ = is_job_or_cronjob_yaml(malformed);
                let _ = find_kind_declaration(malformed);
            }
        }

        /// A deep block scalar with blank lines inside must skip the blanks
        /// without ending the scalar early.
        #[test]
        fn blank_lines_inside_a_block_scalar_stay_inside_it() {
            let with_blanks = "data:\n  script: |\n    start\n\n    kind: Job\n\n  other: 1\n";
            assert!(!is_job_or_cronjob_yaml(with_blanks));
        }

        /// A comment at column zero ends the block scalar (it dedents) and,
        /// being a comment, still never matches.
        #[test]
        fn dedented_comment_ends_the_scalar_and_stays_a_comment() {
            let content = "data:\n  script: |\n    kind: Job\n# kind: CronJob\n";
            assert!(!is_job_or_cronjob_yaml(content));
        }
    }

    #[test]
    fn detect_is_scoped_to_yaml_paths() {
        let job = "kind: Job\n";
        for path in [
            "k8s/job.yaml",
            "k8s/job.yml",
            "K8s/JOB.YAML",
            "a/b/cronjob.Yml",
        ] {
            assert!(
                detect(path, job).is_match(),
                "expected {path} to be guarded"
            );
        }
        for path in [
            "docs/jobs.md",
            "README",
            "src/job.rs",
            "job.txt",
            "job.yaml.j2",
            "job",
            "",
        ] {
            assert_eq!(
                detect(path, job),
                Detection::NoMatch,
                "{path} must stay writable"
            );
        }
    }

    #[test]
    fn detect_returns_matched_with_line_and_reason() {
        let content = "apiVersion: batch/v1\nkind: Job\nmetadata:\n  name: migrate\n";
        match detect("k8s/job.yaml", content) {
            Detection::Matched {
                matched_line,
                reason,
            } => {
                assert_eq!(matched_line, "kind: Job");
                assert_eq!(reason, BLOCKED_REASON);
                assert!(!reason.is_empty());
            }
            Detection::NoMatch => panic!("expected Matched for a Job manifest"),
        }
    }

    #[test]
    fn detect_is_match_reflects_variant() {
        assert!(detect("k8s/job.yaml", "kind: CronJob\n").is_match());
        assert!(!detect("k8s/job.yaml", "kind: Deployment\n").is_match());
    }

    /// Plain prose inside a `.yaml` path stays writable: the path scope only
    /// gates *which* files are judged, it never lowers the bar for what
    /// counts as a declaration once a file is.
    #[test]
    fn detect_ignores_prose_in_a_yaml_path() {
        for prose in [
            "the kind: Job controller supersedes hand-rolled schedules\n",
            "notes: run a kind: CronJob nightly to sweep stale bundles\n",
            "# kind: Job\n",
        ] {
            assert_eq!(
                detect("runbook.yaml", prose),
                Detection::NoMatch,
                "prose must stay writable: {prose:?}"
            );
        }
    }

    /// Every accepted spelling yields a `Detection::Matched` carrying the
    /// offending line as written -- the scan must not rewrite what a
    /// redirect message quotes back.
    #[test]
    fn detect_quotes_the_offending_line_for_every_variant() {
        for (content, expected_line) in [
            ("kind: Job\n", "kind: Job"),
            ("Kind:  Job\n", "Kind:  Job"),
            ("kind :   CronJob\n", "kind :   CronJob"),
            ("--- kind: Job\n", "kind: Job"),
        ] {
            match detect("deploy/x.yaml", content) {
                Detection::Matched { matched_line, .. } => {
                    assert_eq!(matched_line, expected_line);
                }
                Detection::NoMatch => panic!("expected Matched for {content:?}"),
            }
        }
    }

    /// The denial reason is a redirect, not just a block: it must state why
    /// the manifest is banned and name the sanctioned alternatives
    /// concretely enough that the next step needs no research -- the same
    /// bar `github_workflows::PROTECTED_REASON` is held to.
    #[test]
    fn blocked_reason_is_an_actionable_redirect_not_just_a_block() {
        // Why the write is blocked.
        assert!(
            BLOCKED_REASON.contains("blocked"),
            "reason should state that the manifest is blocked: {BLOCKED_REASON}"
        );
        assert!(
            BLOCKED_REASON.contains("never be pruned") || BLOCKED_REASON.contains("never pruned"),
            "reason should state the ArgoCD consequence: {BLOCKED_REASON}"
        );
        // What to do instead.
        assert!(
            BLOCKED_REASON.contains("Instead"),
            "reason should mark the alternative with \"Instead\": {BLOCKED_REASON}"
        );
        assert!(
            BLOCKED_REASON.contains("Deployment"),
            "reason should name the recurring-work alternative: {BLOCKED_REASON}"
        );
        assert!(
            BLOCKED_REASON.contains("WorkflowTemplate"),
            "reason should name the one-shot alternative: {BLOCKED_REASON}"
        );
        assert!(
            BLOCKED_REASON.contains("declarative-config"),
            "reason should point at where workflow templates live: {BLOCKED_REASON}"
        );
    }

    /// A file that is only *mentioned* as a YAML path elsewhere never
    /// matters: the guard judges the content of the path being written, and
    /// path scoping is the only channel `detect` reads the path through.
    /// Scoping is a pure suffix test on the extension -- a trailing slash
    /// names a directory (never a Write/Edit target) and does not extend the
    /// scope to it.
    #[test]
    fn path_scoping_is_extension_only() {
        // A trailing slash names a directory, not a Write/Edit target.
        assert_eq!(detect("jobs.yaml/", "kind: Job\n"), Detection::NoMatch);
        // Query strings and fragments in a path-like string do not count.
        assert_eq!(detect("job.yaml?x=1", "kind: Job\n"), Detection::NoMatch);
    }
}
