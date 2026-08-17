# Operator scenario coverage-gap audit

Audit date: 2026-08-17

This audit covers the five operator scenarios in
[`docs/examples/README.md`](README.md). A fixture means a stable, scenario-specific
file under `tests/fixtures/`; a test source file or a JSON document created inside
a test's temporary directory does not count as a fixture. `✓` means the scenario
has a fixture, its documented `icg` commands are implemented, and its expected
behavior is asserted. `⚠` means partial coverage. `✗` means the primary workflow
is not executable/testable.

## Summary

| Scenario | Fixture in `tests/fixtures/` | Scenario test source | Documented `icg` commands | Expected output testable | Overall |
|---|---|---|---|---|---|
| 1. First-time Installation | ✗ None | ⚠ `tests/installation_tests.rs` | ⚠ `check --stdin` works; `--version`, health flags, and documented packs do not line up with the CLI/repository | ⚠ Smoke assertions exist, but the documented JSON and health output are not asserted | ⚠ Partial |
| 2. Daily Operations | ✓ `daily_operations_denials.json` (not consumed by a test) | ✗ No scenario test source | ✗ Denial-history/status filters and `export-denial` are absent | ✗ No test reads the fixture or exercises the output | ✗ Missing |
| 3. Handling Denials | ✗ None | ⚠ `tests/handling_denials_tests.rs` | ⚠ `explain --pattern` works with a supplied pack; the documented Vault pattern/pack is unavailable | ⚠ Related denial/allow assertions exist, but not the documented Vault output | ⚠ Partial |
| 4. Emergency Response | ✗ None | ⚠ `tests/emergency_response_tests.rs` (does not compile; two tests are ignored) | ✗ `status --health`, `ICG_DISABLED=1`, and `export-denial` are not implemented | ✗ Only the incident-record text is tested; the bypass and denial-history outputs are not | ✗ Missing |
| 5. Maintenance Tasks | ✗ None | ⚠ `tests/maintenance_tasks_tests.rs` | ⚠ Backup create/verify works; health `--verbose`, denial trends, and update `--check-only` do not | ⚠ Smoke tests cover adjacent commands, not the documented output | ⚠ Partial |

No operator scenario is fully covered (`✓`).

## Scenario details

### Scenario 1: First-time Installation — ⚠ partial

Reference: `docs/examples/README.md:27-135`.

- Fixture: no `tests/fixtures/installation*` or equivalent scenario fixture exists.
- Test source: `tests/installation_tests.rs` exercises version, pack loading, check,
  health, status, explain, coverage, and stdin smoke paths. It does not load a
  fixture. In the focused run, 7 tests passed and 2 failed: `--version` did not
  succeed and the coverage-output assertion did not match the actual output.
- Commands:
  - `icg check --stdin` and `--harness` are accepted, but the repository contains
    `image-tag`, `storage-class`, and `beads` packs, not the documented Vault and
    Git packs.
  - `icg --version` is rejected by the current CLI.
  - `icg health --check-packs`, `icg health --check-hooks`, and
    `icg health --verbose` are rejected. The implemented health entry point is
    `icg health status`.
- Expected output: the scenario's structured JSON (including `severity` and
  `telemetryId`) is not the output produced by the human-facing check command,
  which prints `ALLOW`, `DENIED`, and pack/pattern lines. Existing assertions only
  check broad substrings, so they do not lock down the documented contract.

### Scenario 2: Daily Operations — ✗ missing

Reference: `docs/examples/README.md:139-209`.

- Fixture: `tests/fixtures/daily_operations_denials.json` exists and contains the
  three denial records shown by the scenario, but `rg` finds no test that loads or
  validates it.
- Test source: there is no `daily_operations` scenario test file.
- Commands: all three `icg status --denials ...` forms are rejected because
  `status` currently accepts only trust-pointer path/channel options. The
  documented `icg export-denial den-abc123` command is also not a CLI command.
  `jq`, `cat`, `grep`, and `gh` are external commands and are outside this CLI
  audit.
- Expected output: the existing fixture is not an executable oracle. There is no
  test for the tabular denial list, pattern summary, JSON export, or report
  export, so the scenario cannot currently be run or output-validated.

### Scenario 3: Handling Denials — ⚠ partial

Reference: `docs/examples/README.md:213-270`.

- Fixture: no scenario-specific fixture exists. The tests construct temporary
  packs inline instead.
- Test source: `tests/handling_denials_tests.rs` covers denial indicators,
  pack/pattern references, `explain --pattern`, safe/unsafe content examples,
  and repeated denials. In the focused run, 9 of 10 tests passed; the redirect
  test's temporary pack is malformed (`check.type` is missing), so that test
  cannot validate its expected output.
- Commands: `icg explain --pattern` is implemented when a matching pack is
  supplied. The documented `vault-kv-destroy` lookup cannot succeed against the
  repository's available packs because no Vault pack is present. The `vault kv
  patch` and `vault kv get` commands are external Vault commands, not `icg`
  commands, and are not executed by the scenario test.
- Expected output: related denial output is testable at a smoke level, but the
  exact documented Vault denial block and secret metadata output are not tied to
  a fixture or asserted. The current check output also does not emit the
  documented structured fields such as severity and telemetry ID.

### Scenario 4: Emergency Response — ✗ missing

Reference: `docs/examples/README.md:274-343`.

- Fixture: no emergency-record or denial-history fixture exists.
- Test source: `tests/emergency_response_tests.rs` contains an incident-record
  test and smoke tests, but the file currently fails to compile because it passes
  `Cow<str>` directly to `fs::write`. The `ICG_DISABLED` bypass test and denial
  history test are both marked `#[ignore]` as unimplemented.
- Commands: `icg status --health`, `icg health --check-packs`, and
  `icg export-denial` are rejected or absent. `ICG_DISABLED=1` has no
  implementation. The implemented health form is `icg health status`; plain
  `icg health` requires a subcommand.
- Expected output: only the shell-created incident record is checked in the
  test source. The documented bypass warning/success, recent-denial status, and
  report export have no passing executable assertion.

### Scenario 5: Maintenance Tasks — ⚠ partial

Reference: `docs/examples/README.md:347-410`.

- Fixture: no maintenance scenario fixture exists. The release-manifest fixtures
  in `tests/fixtures/` belong to coverage-diff tests, not this operator workflow.
- Test source: `tests/maintenance_tasks_tests.rs` covers health status, generic
  status, trust pointers, backup help, and several adjacent maintenance tools.
  In the focused run, 13 of 15 tests passed; coverage-list output and trust
  channel setup failed. No test consumes an operator fixture.
- Commands:
  - `icg backup create --output ...` and `icg backup verify ...` are implemented.
  - `icg health --verbose` is not implemented; use of `icg health status` is a
    different command contract.
  - `icg status --denials --trend --since 30d` is not implemented.
  - `icg update --check-only` is rejected; `icg update` has no `--check-only`
    option and performs the update flow.
- Expected output: backup verification has an executable output path, and basic
  health/status output has smoke assertions. The documented health inventory,
  denial trend table, and update-availability output are not fixture-backed or
  asserted.

## Implementation and test work needed

1. Decide whether the examples or the CLI are authoritative, then align the
   installation contract: version reporting, pack names, health pack/hook checks,
   verbose health output, and the check response schema.
2. Implement and test denial history queries: `--denials`, time windows,
   pattern summaries, trends, JSON formatting, and `export-denial`. Make the
   daily-operations fixture an input to those tests rather than an unused file.
3. Add a real Vault/Git rule-pack fixture or rewrite Scenarios 1 and 3 to use
   packs shipped in this repository. Add exact assertions for denial fields,
   redirects, and safe alternatives.
4. Define a safe, auditable emergency procedure. Either implement a supported
   bypass with passing tests and an emergency fixture, or remove the
   `ICG_DISABLED` instructions; repair the compile error and unignore only tests
   for implemented behavior.
5. Implement `health --verbose`, denial trends, and `update --check-only`, or
   revise Scenario 5 to use the supported commands. Add a maintenance fixture
   for stable health/trend/update output and test backup creation plus
   verification end to end.

## Verification notes

The audit used the current working tree, including unrelated in-progress changes
that were not modified. The exact documented CLI invocations were also probed
against the built binary; unsupported options returned Clap exit status 2. The
focused test commands were:

```text
cargo test --test installation_tests -- --nocapture
cargo test --test handling_denials_tests -- --nocapture
cargo test --test maintenance_tasks_tests -- --nocapture
cargo test --test emergency_response_tests -- --nocapture
```

The last command stops at the test-compilation error described above.
