# Force-push `updatedInput` validation

This is the end-to-end scenario for the hook's command-rewrite path. It uses
the same `PreToolUse` executable that Claude Code and the local Codex CLI call,
but sends each harness's input spelling to verify the shared response contract.

## Scenario

The incoming Bash calls are intentionally unsafe:

```text
git push --force origin main
git push -f origin main
git push --force-with-lease origin main
```

The focused test pack matches each force-push flag and configures the
`updated_input` redirect as:

```json
{
  "channel": "updated_input",
  "reason_template": "Stripped --force/-f/--force-with-lease flags because force-pushing can overwrite remote history and lose commits; a normal push is safer.",
  "rewrite_template": "{command_without_force}"
}
```

Run the validation with:

```bash
cargo test --test force_push_updated_input_tests
```

## Expected response for both harnesses

Both payload spellings must produce the same structured response:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "updatedInput": {
      "command": "git push origin main",
      "description": "Push reviewed changes",
      "timeout": 120000,
      "run_in_background": false
    },
    "additionalContext": "Stripped --force/-f/--force-with-lease flags because force-pushing can overwrite remote history and lose commits; a normal push is safer. [pack=git-force-push-updated-input, pattern=strip-force-push-flags]"
  }
}
```

The replacement is a complete tool-input object, so fields unrelated to the
command remain present. The reserved rewrite marker removes only the
force-push option, preserving the remote, refspec, and other push arguments.
The rewritten command contains none of `--force`, `-f`, or
`--force-with-lease`. The explanation says both what was removed and why it
is unsafe.

The explanation is emitted as `additionalContext`, not
`permissionDecisionReason`: this is an allow-with-replacement response. A
true deny uses `permissionDecision: "deny"` and
`permissionDecisionReason`, and must not include `updatedInput`; combining a
deny reason with a replacement input would be contradictory for either
harness.

The integration test covers these two input envelopes:

- Claude Code-style `toolName` / `toolInput` (camelCase).
- Codex CLI-style `tool_name` / `tool_input` (snake_case).

Both are normalized by the hook and receive the same `hookSpecificOutput`
envelope required by the shared Claude/Codex profile.
