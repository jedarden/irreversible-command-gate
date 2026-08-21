# PreToolUse response schema

This note defines the response contract emitted by the hook front-end for
Claude Code and the local Codex CLI. The contract intentionally uses the
intersection of the two harnesses: `allow` and `deny` decisions, plus the
optional `updatedInput` and `additionalContext` fields. The hook writes one
JSON object to stdout and writes diagnostics only to stderr.

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
   must be copied into it.
4. An `additionalContext` response leaves the original input unchanged. It
   may be combined with `updatedInput` when the rewrite also needs an
   explanation.
5. The shared profile does not use `ask` or `defer`. Claude supports both,
   while Codex currently parses `ask` but reports it as unsupported and does
   not include `defer` in its current decision enum.

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

This `command` requirement is important for the shared adapter: Codex
accepts `updatedInput` for Bash and `apply_patch` only when the replacement
object contains a string `command`. Claude Code's Bash tool uses the same
shape, so command rewrites are portable between the two harnesses.

## Harness expectations

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
envelope. For a deny it requires a non-empty
`permissionDecisionReason`. For a rewrite it requires
`permissionDecision: "allow"` and rejects `updatedInput` unless it is
present with that decision. For Bash and `apply_patch`, the replacement's
`command` member must be a string; for MCP and other local function tools,
the replacement object is the tool's argument object.

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

### Future additionalContext-only response

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

The adapter may add `additionalContext` to a deny or rewrite response when a
human/model-facing explanation is useful, but it must never use
`additionalContext` as a substitute for `permissionDecision: "deny"`.

## Sources and maintenance note

The harness contracts are changing, especially in Codex. Re-check the
installed harness documentation before adding a new response field:

- [Claude Code hooks: PreToolUse decision control](https://code.claude.com/docs/en/hooks#pretooluse-decision-control)
- [Codex CLI hooks: PreToolUse](https://developers.openai.com/codex/hooks#pretooluse)
- [Codex generated PreToolUse input schema](https://github.com/openai/codex/blob/main/codex-rs/hooks/schema/generated/pre-tool-use.command.input.schema.json)
