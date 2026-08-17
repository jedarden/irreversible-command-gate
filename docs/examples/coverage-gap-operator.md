# Operator scenario coverage-gap audit

Audit date: 2026-08-17

This audit covers the five operator scenarios in
[`docs/examples/README.md`](README.md). `✓` means the scenario has a stable
fixture, its documented `icg` commands are executable, and expected behavior is
asserted by the fixture-backed integration suite.

## Summary

| Scenario | Fixture | Scenario test source | Documented commands/output | Overall |
|---|---|---|---|---|
| 1. First-time Installation | ✓ `installation.json`, `installation-packs/` | ✓ `tests/operator_scenarios.rs` | ✓ version, health, hook, stdin allow/deny | ✓ Covered |
| 2. Daily Operations | ✓ `daily-operations.json` | ✓ `tests/operator_scenarios.rs` | ✓ tables, summaries, JSON, `export-denial` | ✓ Covered |
| 3. Handling Denials | ✓ `handling-denials.json`, `handling-denials-pack.json` | ✓ `tests/operator_scenarios.rs` | ✓ denial format, redirect, explain, safe alternatives | ✓ Covered |
| 4. Emergency Response | ✓ `emergency-response.json` | ✓ `tests/operator_scenarios.rs` | ✓ health, `ICG_DISABLED`, restore, export | ✓ Covered |
| 5. Maintenance Tasks | ✓ `maintenance.json`, `installation-packs/` | ✓ `tests/operator_scenarios.rs` | ✓ verbose health, trends, update check, backup | ✓ Covered |

All five operator scenarios are now covered (`✓`). The integration suite uses
fixture-path environment overrides only to make the documented commands
deterministic and isolated from host state; the command arguments remain the
ones shown in `docs/examples/README.md`.

## Scenario details

### Scenario 1: First-time Installation — ✓ covered

`installation.json` and the three fixture packs supply the documented Vault,
Git, and image-tag installation inventory. The test validates `--version`,
`health --check-packs`, `health --check-hooks`, `health --verbose`, the
documented stdin denial fields, and the safe allow path. A missing hook
configuration is also asserted as a failure path.

### Scenario 2: Daily Operations — ✓ covered

`daily-operations.json` is consumed as the denial log. The test validates the
one-hour denial table, seven-day pattern summary, JSON output, the documented
`export-denial den-abc123` report, and an unknown denial failure. External
commands such as `jq`, `cat`, `grep`, and `gh` remain outside this CLI audit.

### Scenario 3: Handling Denials — ✓ covered

The Vault pack fixture drives the documented denial block and
`explain --pattern vault-kv-destroy` output. The test asserts severity,
explanation, redirect, successful safe alternatives, and a missing-pattern
failure. The external Vault commands are represented by their corresponding
safe `icg check --command` allow assertions.

### Scenario 4: Emergency Response — ✓ covered

`emergency-response.json` supplies the incident fields, bypass command, denial
ID, and restored-health output. The test writes and reads a temporary incident
record, checks recent-denial health, verifies the one-command bypass warning,
confirms that protection is restored after the environment is removed, and
exports the denial for follow-up.

### Scenario 5: Maintenance Tasks — ✓ covered

`maintenance.json` supplies the trend, update, and backup output oracles. The
test validates verbose health, the 30-day denial trend,
`update --check-only`, backup creation and verification, and corrupt-archive
failure handling using temporary state that is removed by the test fixture.

## Verification

The focused command for the complete operator coverage is:

```text
cargo test --test operator_scenarios
```

The suite contains five tests covering all five scenarios and asserts both
success and failure paths for stateful operations.
