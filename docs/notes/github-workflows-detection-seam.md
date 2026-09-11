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

Codex `apply_patch` events reach the same `detect` call. `normalize_apply_patch`
extracts one content source per file the patch touches — every `*** Add File:` /
`*** Update File:` / `*** Delete File:` header, and both ends of a `*** Move to:`
(the source path stays its own entry, so moving a workflow file to an unguarded
path cannot smuggle it past the guard). A single-file patch evaluates as one
`Content`; a multi-file patch becomes a `ContentBatch` that
`evaluate_content_batch` runs file-by-file through the same
`evaluate_content_inner`, so a guarded path anywhere in the patch denies with the
same payload. Parsing is defensive: a truncated patch (Begin marker seen, End
marker lost) is salvaged as far as it parsed and its headers are still checked,
while input with no Begin marker or no file header stays `InvalidInput` — the
hook front-end treats that as unmatched and allows, which is its fail-open path.

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
That exact rendered string is pinned for both Write and Edit by
`assert_hook_deny` in `tests/github_workflows_hook_integration_tests.rs`
(and the `CheckResult` fields by
`evaluate_content_denies_every_guarded_path_form_for_both_tools` beside it),
so a change to either shape fails a test rather than drifting silently.

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

The channel is fixed by [redirect-not-just-block.md](redirect-not-just-block.md):
deny + actionable `permissionDecisionReason`, not `updatedInput`, since the
sanctioned alternative is a different operation, not a safer spelling of the
same one.

## Where the redirect text lives (implemented)

The redirect message is `PROTECTED_REASON` in `src/github_workflows.rs`,
carried verbatim as the denial's `reason` and rendered as the
`permissionDecisionReason` prefix at both integration points — Claude Code
Write/Edit (`InputSource::Content`) and Codex `apply_patch`
(`InputSource::ContentBatch` and single-file `Content`) — because both funnel
through the same `evaluate_content_inner` detection call. It states why the
write is blocked (workflow definitions grant arbitrary CI privileges) and what
to do instead: a human maintainer making the change in a reviewed pull
request, with CI-pipeline changes landing in the `declarative-config`
repository where the Argo Workflows templates live
(`k8s/iad-ci/argo-workflows/`). That wording is pinned by
`protected_reason_is_an_actionable_redirect_not_just_a_block` (structure:
why + instead + reviewed channel) and by the exact-string hooks in
`assert_hook_deny` / `assert_workflows_denial` (verbatim carry-through).

## Fail-open boundary (implemented)

The whole predicate+detection pipeline sits behind one top-level fail-open
boundary, `Engine::input_source_from_pre_tool_use_fail_open` in
`src/engine.rs`, which the `icg hook` front-end uses instead of calling
`input_source_from_pre_tool_use` directly. Any in-process failure — a
structured `InvalidInput` from the patch parser, or a panic from the parser
or matcher on input they did not anticipate — collapses to `None`, the value
the hook's None branch renders as a plain allow. (The stdin read and the
evaluation stages have their own `catch_unwind` boundaries:
`read_pre_tool_use_payload_from_stdin` and `evaluate_content` /
`evaluate_content_batch`, the latter honoring an operator's fail-closed
policy.) The boundary's fault handling is pinned by
`fail_open_boundary_turns_an_injected_pipeline_fault_into_an_allow` (injected
panicking stub fails open, healthy stage passes through) and
`fail_open_conversion_allows_unparseable_patches_and_keeps_real_detections`
(unparseable input allows; a well-formed guarded patch through the same
boundary still denies); the end-to-end Err variant at the binary boundary is
`hook_fails_open_on_unparseable_patch_input`.
