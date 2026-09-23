# PreToolUse response schema

This note defines the response contract emitted by the hook front-end for
Claude Code and the local Codex CLI. The contract intentionally uses the
intersection of the two harnesses: `allow` and `deny` decisions, plus the
optional `updatedInput` and `additionalContext` fields. The hook writes one
JSON object to stdout and writes diagnostics only to stderr.

**What a harness accepts is not what a harness acts on.** The schema below is
the wire both harnesses parse; each harness's runtime then decides which of
those fields have effect, and those decisions are *not* uniform. The star
example is the rewrite channel: `updatedInput` is generic contract language,
but Codex's runtime honors `permissionDecision: "deny"` alone — it rejects
the `allow` decision that `updatedInput` is defined to ride on — so a
rewrite response that the schema would call well-formed is dead on arrival
there. ICG degrades per harness instead of sending responses a runtime will
drop; the capability matrix and degradation rules live in
[`harness-adapter-contract.md`](harness-adapter-contract.md) §5–§6, and the
per-harness behavior is summarized under "Harness expectations" below.

## Common envelope

Every structured response has this envelope:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow"
  }
}
```

The accepted fields inside `hookSpecificOutput` are:

| Field | Type | Meaning |
| --- | --- | --- |
| `hookEventName` | the literal string `PreToolUse` | Identifies the hook event. |
| `permissionDecision` | `allow` or `deny` in the shared profile | Controls whether the pending tool call may proceed. |
| `permissionDecisionReason` | non-empty string when present | Explains the decision. For a deny, this is required and is returned to the model as the denial reason. |
| `updatedInput` | JSON object | Complete replacement for the pending tool's input arguments. It is not a patch and must not be nested under another `input` field. |
| `additionalContext` | string | Non-blocking context for the model. It does not change the tool arguments. |

Codex also accepts the top-level `systemMessage` common output field for a
user-visible warning. Practice mode uses that field for its per-check active
banner and would-be-denial report; it deliberately does not put that report
in `additionalContext`.

The common response constraints are:

1. `hookEventName` is always `PreToolUse`.
2. `permissionDecision: "deny"` requires a non-empty
   `permissionDecisionReason` and must not include `updatedInput`.
3. An `updatedInput` response requires `permissionDecision: "allow"`.
   `updatedInput` replaces the whole tool-input object, so unchanged fields
   must be copied into it. Note that this precondition makes the field
   harness-specific in practice: a runtime that refuses `allow` refuses
   every `updatedInput` response, whatever the schema says (Codex does
   exactly that — see "Harness expectations").
4. An `additionalContext` response leaves the original input unchanged. It
   may be combined with `updatedInput` when the rewrite also needs an
   explanation.
5. The shared profile does not use `ask` or `defer`. Claude supports both,
   while Codex parses `ask` but reports it as unsupported, has no `defer`
   in its current decision enum, and — the part that constrains this
   profile — rejects `allow` itself (see "Harness expectations").

## Deny versus updatedInput

These are different channels, not two ways to express the same result:

| Channel | Decision field | Input effect | Harness action |
| --- | --- | --- | --- |
| Deny | `permissionDecision: "deny"` | No replacement input | Do not run the tool; return `permissionDecisionReason` to the model. |
| Updated input | `permissionDecision: "allow"` | Replace the complete `tool_input` with `updatedInput` | Run the tool using the replacement arguments. |

`permissionDecision` answers “may this call execute?” `updatedInput` answers
“which arguments should execute?” A rewrite is therefore not represented by a
`rewrite` string, a top-level `verdict`, or a deny plus a suggested command.
Returning deny with a suggested `updatedInput` would be contradictory and is
outside this contract.

When a harness cannot hear the grant half of this contract — Codex's runtime
refusing `allow` — the rewrite collapses into the other channel: the front
end returns a plain deny carrying the rewrite's reason rather than let the
matched input run unreplaced. That is degradation of one verdict into the
surviving channel, not a third response kind; every such rule is stated per
adapter in [`harness-adapter-contract.md`](harness-adapter-contract.md) §5.

## Exact updatedInput structure

The canonical rewrite response is:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "updatedInput": {
      "<tool-input-field>": "<replacement-value>"
    }
  }
}
```

`updatedInput` is the complete replacement object. The object must use the
same argument names and value types as the incoming `tool_input`; do not
convert the harness's input naming convention while rewriting it. For a Bash
call, for example, preserve an incoming `description`, `timeout`, or
`run_in_background` field even when only `command` changes:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "updatedInput": {
      "command": "git push --force-with-lease origin main",
      "description": "Push reviewed changes",
      "timeout": 120000,
      "run_in_background": false
    }
  }
}
```

For content tools, substitution happens inside the replacement object. A
Claude Code `Write` rewrite replaces `content` while retaining `file_path`
and any other input fields. An `Edit` rewrite replaces `new_string` while
retaining `file_path` and `old_string`. The field spelling must match the
incoming Claude Code payload (`file_path`, `old_string`, and `new_string`);
older camelCase fixtures are not a reason to add a second field to the
replacement object.

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "updatedInput": {
      "file_path": "deploy/app.yaml",
      "content": "image: app:1.2.3\nstorageClassName: sata\n"
    }
  }
}
```

For Codex `apply_patch`, the replacement is the complete patch argument
object, and `command` must be a string containing the replacement patch:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "updatedInput": {
      "command": "*** Begin Patch\n*** Update File: deploy/app.yaml\n@@\n-image: app:latest\n+image: app:1.2.3\n*** End Patch"
    }
  }
}
```

This `command` requirement is what keeps the replacement object
harness-neutral: the complete-replacement shape is the same object Claude
Code's Bash tool and Codex's Bash/`apply_patch` tools both expect, so a
rewrite built for one harness is structurally valid for the other. Whether
the *other* harness will act on it is a separate question — the answer per
harness is under "Harness expectations" below. Codex in particular never
gets that far today: its runtime refuses the `allow` decision the rewrite
rides on, so ICG degrades the rewrite to a deny there and the replacement
object is never sent.

## Harness expectations

The same envelope meets two very different runtimes. What each harness
actually acts on today:

| Field | Claude Code | Codex CLI (pinned 0.154.0) |
| --- | --- | --- |
| `permissionDecision: "deny"` | honored; reason sent to the model | **honored — the only decision the runtime acts on**; reason required |
| `permissionDecision: "allow"` | honored | **rejected** (`unsupported permissionDecision:allow`) |
| `updatedInput` (rewrite) | honored; whole-object replacement | **unreachable** — its required precondition, an accepted `allow`, is itself rejected |
| `additionalContext` (warning) | honored | parsed, **not acted on** yet |
| `hookEventName` | ignored | required |

The consequence for rewrites: on Claude Code a Rewrite verdict runs the
replacement arguments; on Codex the same verdict degrades to a deny
carrying the rewrite's reason, because granting Codex a modified command is
not a response Codex can hear. ICG's renderer performs that degradation
from declared per-adapter capabilities — it never sends a field it knows
the runtime will refuse or drop ([`harness-adapter-contract.md`](harness-adapter-contract.md)
§5).

### Claude Code

Claude Code reads the structured response from `hookSpecificOutput` for a
`PreToolUse` hook. `permissionDecision: "deny"` prevents the call and sends
`permissionDecisionReason` to Claude. `permissionDecision: "allow"` permits
the call, and `updatedInput` replaces the entire tool-input object before
execution. Therefore, a Claude response must include every input field that
the tool still needs, not only the field being changed.

Claude Code also supports other `PreToolUse` decisions, including `ask` and
`defer`, but they are deliberately outside this cross-harness contract.

### Codex CLI

The target is the local Codex CLI's synchronous `PreToolUse` command hook,
not a cloud-hosted Codex task. Codex reads the same `hookSpecificOutput`
envelope — `hookEventName` is required on the response — but **its runtime
honors `permissionDecision: "deny"` alone**. Verified against the shipped
Codex CLI 0.154.0 binary, which carries these rejections:

```
PreToolUse hook returned unsupported permissionDecision:allow
PreToolUse hook returned unsupported permissionDecision:ask
PreToolUse hook returned updatedInput without permissionDecision:allow
PreToolUse hook returned permissionDecision:deny without a non-empty permissionDecisionReason
```

So the earlier reading of this schema — "Codex accepts a rewrite when it
carries `permissionDecision: "allow"`" — described the *schema*, not the
*runtime*: the runtime refuses `allow` itself, which takes every
`updatedInput` response with it. Each response kind therefore lands as
follows:

- **Deny** is the one decision Codex executes, and it requires a non-empty
  `permissionDecisionReason`. This is the channel the guard's protection
  rides on.
- **Allow** is never emitted as a decision. ICG omits the field entirely —
  an absent decision leaves the call to Codex's own permission flow, which
  is the same outcome without the per-call hook error.
- **Rewrite** is unreachable, so ICG degrades it to a deny carrying the
  rewrite's reason: the matched command must not run unreplaced, and deny
  is the only channel Codex acts on.
- **Warning** sends `additionalContext` (Codex's schema accepts the field)
  with no decision field, accepting that Codex ignores the text today; the
  day Codex honors it, the text is already on the wire.

Validating a Codex response against the published schema is therefore not
sufficient for this harness: the runtime is stricter than its schema, and
the gap is silent apart from a per-call hook error. The runtime pin and the
matrix that keeps CI honest about it are in
[`harness-adapter-contract.md`](harness-adapter-contract.md) §6.2.

Codex may accept an older top-level `{"decision":"block","reason":"..."}`
deny shape, but the hook front-end must not emit it. The nested
`hookSpecificOutput` shape is the portable form and is also the form Claude
Code expects. Codex parses several future or compatibility fields, including
`ask`, but does not currently support them as executable `PreToolUse`
decisions; emitting them causes the hook run to be reported as failed.

## Examples of each response channel

### Deny

The tool is not run and no content substitution is offered:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "Destructive command blocked by policy."
  }
}
```

### Updated input

The tool is allowed with a complete replacement for its arguments. The
optional `additionalContext` explains the rewrite without changing the
decision:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "updatedInput": {
      "command": "git push --force-with-lease origin main",
      "description": "Push reviewed changes"
    },
    "additionalContext": "The unsafe --force form was changed to --force-with-lease."
  }
}
```

### additionalContext-only response (warning)

This is non-blocking advisory context. The original tool input runs
unchanged:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "additionalContext": "Review the target worktree before continuing."
  }
}
```

The examples in this section show the full-capability shape, which is what
Claude Code acts on. Codex acts on less: there the same warning renders
without the `allow` decision (the runtime rejects it) and the
`additionalContext` text rides along unacted-on. The recorded per-harness
renderings for every verdict — allow, warning, rewrite, deny — are the
golden fixtures in `tests/fixtures/adapter/` (`claude-code-*` and
`codex-cli-*` pairs), compared byte-for-byte against the real binary by
`tests/adapter_contract_tests.rs`.

The adapter may add `additionalContext` to a deny or rewrite response when a
human/model-facing explanation is useful, but it must never use
`additionalContext` as a substitute for `permissionDecision: "deny"`.

## Sources and maintenance note

The harness contracts are changing, especially in Codex. Re-check the
installed harness documentation before adding a new response field — and
verify against the *installed runtime*, not the published schema alone: the
Codex schema still advertises `allow` and `updatedInput` that its runtime
refuses, which is how this note once described rewrites as portable that
Codex had already stopped accepting. `tests/codex_compat_matrix_tests.rs`
pins `icg-ci`'s compatibility matrix to the same runtime pin
(`CODEX_RUNTIME_PIN` in `src/adapter.rs`) that the rejections above were
verified against.

- [Claude Code hooks: PreToolUse decision control](https://code.claude.com/docs/en/hooks#pretooluse-decision-control)
- [Codex CLI hooks: PreToolUse](https://developers.openai.com/codex/hooks#pretooluse)
- [Codex generated PreToolUse input schema](https://github.com/openai/codex/blob/main/codex-rs/hooks/schema/generated/pre-tool-use.command.input.schema.json)
