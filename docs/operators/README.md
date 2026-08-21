# Irreversible Command Gate (icg) — operator documentation

`icg` is a per-invocation guard for local AI coding and automation agents. It
reads a harness tool call, evaluates the configured rule pack, and returns a
deny, rewrite, warning, or allow decision before the call executes.

## Start here

- New installation or deployment: [Installation and deployment guide](deployment-guide.md)
- Fail-closed activation and operations: [Fail-Closed mode guide](fail-closed-mode.md)
- Installation and hook failures: [Troubleshooting](troubleshooting.md)
- Observing would-be denials safely: [Practice mode](practice-mode.md)
- Moving from the existing Python hook: [Migration guide](migration-from-org-rule-guard.md)
- Interpreting a denial: [Deny-message guide](deny-messages.md)

## Capability status in the current tree

The native `icg hook` command is the supported integration path. It accepts
PreToolUse JSON for:

- `Bash` command-mode calls;
- Claude-style `Write` and `Edit` content calls; and
- Codex `apply_patch` calls, including multi-file patches.

The hidden `icg wrapper` subcommand is used by Unix PATH symlinks. It evaluates
the command, denies matching rules, and `exec`s the real binary found later in
`PATH`. It is still not a complete security boundary for absolute-path
invocations or direct library calls. Cloud-hosted agent sessions are not
covered by a binary installed on this host.

## Operator commands

The executable's current top-level command families are:

```text
icg hook
icg status
icg trust show|set|check
icg update
icg health status|reset|mark-start|mark-clean-exit|record-crash
icg telemetry status|reset|configure
icg policy status|reconcile|configure|demote|force-graduate|force-revert
icg check --command|--stdin|--file
icg explain --pattern|--denial
icg coverage --list
icg bug-report --output <path>
icg backup create|verify
icg override create|approve|list
icg regression-suite <manifest>
icg coverage-diff <previous> <current>
icg new-pack <name>
```

Run `icg <command> --help` on the installed binary for the exact options.
The diagnostic commands operate on explicit local rule-pack paths when no
installed pack is available. Repository overrides remain release-bound: an
approval without an exact trusted release reference records the review but does
not create an active bypass artifact.

## Deployment model

Production artifacts belong in administrator-controlled locations:

```text
/usr/local/bin/icg
/etc/icg/rule-pack.json
/etc/icg/trust-pointer.json
/etc/icg/last-update-check.json
/etc/icg/fail-closed-policy.json
```

The binary and policy artifacts should be root-owned and not writable by the
guarded agent. Hook telemetry is auxiliary state under `/var/cache/icg` and
must not be confused with the trusted rule-pack or release pointer.

The fail-closed policy starts in Fail-Open. `icg policy reconcile` consumes the
same per-release deny-rate and rollback state used by poison-pill auto-rollback;
each uniquely observed release can advance the persisted clean streak once.
The default graduation threshold is three releases and can be changed with
`icg policy configure --threshold N`, which restarts qualification. A poison
pill resets open-mode qualification, while a poison pill after graduation does
not silently demote Fail-Closed. Use `icg policy demote --reason "..."` only as
an authenticated, out-of-band emergency action; requalification is then
required. `icg policy force-graduate --reason "..."` and
`icg policy force-revert --reason "..."` are similarly authenticated manual
controls. Every clean-release observation, poison-pill reset, graduation, and
manual override is retained as bounded structured event telemetry in the
policy snapshot and exposed by `icg policy status`.

For activation prerequisites, configuration defaults, monitoring, emergency
demotion, and troubleshooting, use the [Fail-Closed mode guide](fail-closed-mode.md).
In particular, `ICG_FAIL_CLOSED=false` does not demote a durable Fail-Closed
policy; use `icg policy force-revert --reason "..."` from an administrator
control path during an incident.

## Rule packs and release safety

Rule packs are release data, not ad-hoc local bypass files. Before adopting a
new pack, run the fixed deny-regression suite and `coverage-diff/v1`, complete
the required human review, and advance the exact trust pointer. See the
[release-cutting runbook](../runbooks/release-cutting.md) and
[per-repository override contract](../notes/per-repo-overrides.md).

## Design and developer references

- [Project README](../../README.md)
- [Implementation plan](../plan/plan.md)
- [Fail-closed transition design](../design/fail-closed-transition.md)
- [Multi-harness integration notes](../notes/multi-harness-integration.md)
- [Existing enforcement coverage](../notes/existing-enforcement-infrastructure.md)
