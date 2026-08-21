# Practice mode

Practice mode exercises the complete rule-pack check path while leaving every
guarded operation executable. A denial is reported as what **would have been
denied**; it is not converted into a hook `deny` response or a wrapper failure.
Use it only for a deliberate observation window, then remove it and verify
that enforcing mode is active.

## Hook front-end

Pass `--practice` in the configured hook command:

```text
icg hook --practice --rule-pack /etc/icg/rule-pack.json
```

For each `PreToolUse` check, the response contains a top-level Codex
`systemMessage` with the persistent `ICG PRACTICE MODE ACTIVE` banner. When a
rule would deny the call, that message also contains `WOULD DENY` and the
normal pack, pattern, and reason details. The hook still returns
`permissionDecision: "allow"`; it does not use `additionalContext` for this
near-miss report because Codex does not reliably surface that field.

The flag can also be enabled for a configured hook with `ICG_PRACTICE=1`.
Accepted true values are `1`, `true`, `yes`, and `on`.

## PATH-wrapper front-end

The PATH wrapper cannot safely reinterpret a tool's own `--practice` argument,
so enable practice mode in its environment:

```bash
ICG_PRACTICE=1 git status
ICG_PRACTICE=1 vault kv destroy secret/example
```

The wrapper writes the active banner to stderr on every guarded check. A
would-be denial is also written directly to stderr, while the real binary is
executed with its original arguments. Stdout remains the real tool's stdout.

For a direct wrapper invocation, the equivalent explicit flag is available:

```text
icg wrapper --practice git status
```

## Returning to enforcement

Unset `ICG_PRACTICE`, remove `--practice` from the hook configuration, and run
a known guarded check. It must return a hook `deny` or a non-zero wrapper
failure, and no practice banner should appear.
