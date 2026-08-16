# Irreversible Command Gate (icg) — operator documentation

`icg` is a per-invocation guard for local AI coding and automation agents. It
reads a harness tool call, evaluates the configured rule pack, and returns a
deny, rewrite, warning, or allow decision before the call executes.

## Start here

- New installation or deployment: [Installation and deployment guide](deployment-guide.md)
- Installation and hook failures: [Troubleshooting](troubleshooting.md)
- Moving from the existing Python hook: [Migration guide](migration-from-org-rule-guard.md)
- Interpreting a denial: [Deny-message guide](deny-messages.md)

## Capability status in the current tree

The native `icg hook` command is the supported integration path. It accepts
PreToolUse JSON for:

- `Bash` command-mode calls;
- Claude-style `Write` and `Edit` content calls; and
- Codex `apply_patch` calls, including multi-file patches.

The hidden `icg wrapper` subcommand is still a parser scaffold. It does not
execute the real shadowed binary or enforce a rule pack, so PATH symlinks must
not be deployed from this tree. Cloud-hosted agent sessions are not covered by
a binary installed on this host.

## Operator commands

The executable's current top-level command families are:

```text
icg hook
icg status
icg trust show|set|check
icg update
icg health status|reset|mark-start|mark-clean-exit|record-crash
icg telemetry status|reset|configure
icg regression-suite <manifest>
icg coverage-diff <previous> <current>
icg new-pack <name>
```

Run `icg <command> --help` on the installed binary for the exact options.
The guide intentionally does not document commands that are not present in
the current CLI, such as `icg check`, `icg audit`, or updater dry-run and
rollback flags.

## Deployment model

Production artifacts belong in administrator-controlled locations:

```text
/usr/local/bin/icg
/etc/icg/rule-pack.json
/etc/icg/trust-pointer.json
/etc/icg/last-update-check.json
```

The binary and policy artifacts should be root-owned and not writable by the
guarded agent. Hook telemetry is auxiliary state under `/var/cache/icg` and
must not be confused with the trusted rule-pack or release pointer.

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
