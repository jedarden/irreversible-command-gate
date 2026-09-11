# The `.github/workflows` detection seam

Where the workflows-path guard detects a protected Write/Edit target, what
structured fields its denial carries, and what the redirect-message step is
expected to consume. Written as the handoff note for that step.

## Where detection happens

The single source of truth for the matching logic is
`is_github_workflows_path` in `src/github_workflows.rs`, together with the
shared fixture tables `GUARDED_PATHS` / `UNGUARDED_PATHS` that both that
module's unit tests and the hook-level integration tests iterate. The
structured wrapper around it is `github_workflows::detect(path) -> Detection`,
which returns `Detection::Matched { matched_path, reason }` or
`Detection::NoMatch`.

The only caller that matters for Claude Code Write/Edit events is
`Engine::evaluate_content_inner` in `src/engine.rs`: it calls `detect` on the
source's file path *before any pack dispatch*, so a workflows-path match
denies even against an empty rule pack, with pack attribution
`github-workflows` / pattern `github-workflows-protected`. The `icg hook`
front-end reaches it via stdin JSON → `input_source_from_pre_tool_use` →
`evaluate_content`; `tests/github_workflows_hook_integration_tests.rs` drives
that exact path through the compiled binary.

## What the deny payload carries

In-process — the shape a code-level redirect step actually receives — a
workflows denial is:

```rust
CheckResult::Denied {
    reason,                     // Detection::Matched's `reason`, verbatim
    pack_id: "github-workflows",
    pattern_id: "github-workflows-protected",
    matched_path: Some(path),   // the exact caller-supplied path, not canonicalized
}
```

`pack_id` / `pattern_id` are how a downstream step recognizes *this* guard
among all denials the engine can emit. `matched_path` is the exact string the
tool call targeted — `Detection::Matched` documents that it is deliberately
not canonicalized, so a redirect message can quote back what was asked for.

At the hook boundary, `render_hook_response` in `src/main.rs` renders that
same denial as:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "<reason> [pack=github-workflows, pattern=github-workflows-protected, path=<matched_path>]"
  }
}
```

The two structured fields survive into the wire format only inside this
`permissionDecisionReason` prose. The `path=` segment comes from the
denial's own `matched_path` (`file=` is the caller-context fallback for
denials that carry none) — see `denial_path_segment` in `src/main.rs`. The
guard denies rather than rewrites, so the response carries no `updatedInput`.

One place the fields deliberately do *not* reach: the operational denial log
drops `matched_path` and records the target from the input source itself
(`record_operational_denial` in `src/denial_log.rs`). The audit log is not
this seam.

## What the redirect-message step must consume

Consume the structured fields from `CheckResult::Denied` — `matched_path` and
`reason`, recognized via `pack_id == "github-workflows"` — and do not:

- **re-run the matcher** on the input path (`is_github_workflows_path` or
  `detect` a second time). The parent acceptance criterion for this guard is
  "no duplicated matching logic"; a second call site risks drifting from the
  decision actually being emitted, and the denial already carries the answer.
- **parse `permissionDecisionReason`** to recover the fields. That string is
  a display format (`… [pack=…, pattern=…, path=…]`); coupling a redirect to
  it means a cosmetic rewording breaks the redirect silently.

The redirect *text* itself — the Argo Workflows / `declarative-config`
pointer a workflows denial should carry — is the project-wide policy recorded
in [redirect-not-just-block.md](redirect-not-just-block.md); that note also
fixes the channel (deny + actionable `permissionDecisionReason`, not
`updatedInput`, since the sanctioned alternative is a different operation,
not a safer spelling of the same one).
