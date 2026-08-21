# Integration-scenario coverage gap analysis

The fixtures and tests in `tests/fixtures/integration-scenarios/` and
`tests/integration_scenarios.rs` validate the three integration scenarios in
`docs/examples/README.md`.

## Scenario 10: `org-rule-guard.py` migration

The test suite compares the documented org hook inventory with the loaded icg
packs in three categories:

| Category | Covered by the tests | Expected owner |
| --- | --- | --- |
| `image: ...:latest` | deny and pinned-image allow probes | Both hooks during coexistence |
| `.github/workflows/` writes | deny in the org-hook contract; allow in icg | `org-rule-guard.py` |
| `kind: Job` manifests | deny in the org-hook contract; allow in icg | `org-rule-guard.py` |
| mutating `kubectl` commands | deny in the org-hook contract; allow in icg | `org-rule-guard.py` |
| OpenBao/Vault destructive commands | allow in the org-hook contract; deny by icg | icg-only coverage |

When `/home/coding/.claude/hooks/org-rule-guard.py` exists, the integration
test also executes that hook for every probe and compares its live decision to
the fixture. The fixture assertions remain portable when the host hook is not
installed. This makes the migration boundary explicit: overlapping rules must
produce compatible decisions, while org-only rules must remain enabled until
an equivalent icg pack exists.

Remaining migration gaps are the org hook's workflow, Kubernetes workload-kind,
mutating-kubectl, and file-write credential-value rules. The current icg pack
set adds broader command coverage, including OpenBao, Git, Docker, tmux, beads,
and deprecated bead CLI protection, but does not claim to absorb those org-only
content and kubectl rules.

## Scenario 11: multi-harness support

The fixture has four probes: Claude Code Bash and Write payloads use the
camelCase keys (`toolName`, `toolInput`, `filePath`), while Codex CLI Bash and
`apply_patch` payloads use snake_case keys (`tool_name`, `tool_input`,
`file_path` where applicable). Each probe is exercised through:

1. the library parser and `InputSource` conversion;
2. `icg check --stdin --harness ...`; and
3. the native `icg hook` JSON response envelope.

The current implementation has one shared adapter and one shared response
shape. The main maintenance gap is external: Codex hook availability and its
configuration surface can change independently of icg. These tests therefore
pin the accepted wire aliases and require both formats to continue reaching
the same rule-pack decision.

## Scenario 12: repository overrides

The end-to-end test covers request creation, human approval, release-bound TOML
installation, exact repository and release verification, an in-scope allow,
an out-of-scope deny, listing, and expiry rejection. It also checks that a
safe semver image remains allowed without an exemption.

The override system intentionally does not support an unscoped or permanent
bypass. The artifact must be `overrides/<repository>.toml`, must name an exact
release reference, must reference an existing deny rule, and must be fresh.
The current gap is operational rather than an untested decision: production
deployment still needs the release pipeline and repository review process to
publish these artifacts. The fixture tests the local contract that those
systems must satisfy.

## Verification command

Run the focused suite with:

```text
cargo test --test integration_scenarios
```

Run all project tests with:

```text
cargo test
```
