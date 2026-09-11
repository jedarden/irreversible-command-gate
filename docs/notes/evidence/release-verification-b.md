# Closing evidence — release-verification closed beads, batch 2/5

Evidence for the eight beads in the "Release verification / release process /
distribution integrity" section of
`docs/notes/closed-beads-release-verification-inventory.md` (irrevers-c52de1f2,
child of irrevers-622aae24). Method: `bead show <id>` for each bead's own
description/notes, cross-referenced with `git log --grep`, `git log --diff-filter`,
`git log -S`, and per-file history in this repo, plus `git log` in a local
`~/declarative-config` checkout for the one bead whose artifact lives there.
Live cluster check run 2026-09-10 against iad-ci. Evidence gathering only — no
plan edits.

Audited 2026-09-11 (irrevers-8176471b): every commit/test/line reference below
was re-resolved and verified against the current tree, live iad-ci, and the
`~/declarative-config` history. All references resolve; one clarification was
added under irrevers-f59f9313 (two implementing commits carry subjects about
other bundled work — quoted below so `git log` readers aren't misled). No
evidence was found needing retraction.

All eight beads were confirmed **Closed** at time of writing, so every entry
below is closing evidence, not a status correction. Timestamps are UTC unless
a `-0400` offset is shown (commit timestamps are recorded in the repo's local
zone); bead timestamps are UTC.

Re-audited 2026-09-11 (irrevers-61ad3e1e): every commit SHA (13 in this repo,
4 in `~/declarative-config`), every named test, every line reference, and the
close-timestamp arithmetic below were re-resolved against git history, the
current tree, and `bead show`. All verify. Four diffstat counts were restated
precisely — they had quoted total-churn numbers as insertions: `+78` →
`+72 −6` under irrevers-2cb3dbd2, `−130` → `+8 −122` under
irrevers-96594031, and `+731`/`+283`/`+119` → `+642 −89`/`+236 −47`/
`+66 −53` under irrevers-075634b8. No citation required retraction.

---

## irrevers-2cb3dbd2 — Gate the actual modular release packs in icg-ci instead of static fixtures

**Verifiable.** Implementing commit `6eaeb70` (2026-08-26 00:03:26 -0400 =
2026-08-26T04:03:26Z) landed **10 seconds** before the bead closed
(2026-08-26T04:03:36Z). It added exactly what the description demanded:

- `tests/release_gate_integrity_tests.rs` (284 lines, new) with the
  mutation-a-real-pack integration tests: `mutating_real_pack_causes_coverage_gate_to_fail`,
  `widening_safe_pattern_in_real_pack_causes_coverage_gate_to_fail`, plus the
  manifest-verification pair `pack_manifest_provides_cryptographic_verification`
  and `mutating_pack_after_manifest_causes_verification_failure`.
- +110 lines in `tests/icg_ci_integration_tests.rs`.
- 78 changed lines (+72 −6) in the icg-ci workflow template switching the release stage from
  static fixtures to the real bytes: `build-pack --pack-dir packs`,
  `pack-manifest --pack-dir packs`, `coverage-diff` and `redos-check` over the
  released packs, with the archive produced from the already-verified bytes.
  That comment — "This now gates the ACTUAL packs being released, not static
  fixtures" — is still present in
  `containers/argo-guarded-builder/icg-ci-guarded-workflowtemplate.yml`
  (commands at lines 117, 121, 156, 162, 202–224 today), so the gate has not
  regressed since.

The handoff note's warnings (duplicate `pack_manifest` import, missing
`std::fs`, a failing `git-force-push` regression case) were resolved before
`6eaeb70`; the four-asset green releases from v0.1.1 onward (batch-1 evidence
for irrevers-84b36e47) show the stage runs green end-to-end, and `9df67da`
(2026-09-10, "test: green the full hook-detection surface for CI") keeps the
surface green.

Caveat: the bead's description also asked for deny-regression generation over
every released pack; the template's regression stage notes it "skip[s]
regression checks but still verify[s] the current packs" for prior-release
packs, so that one sub-clause is partially satisfied (coverage, manifest and
ReDoS gates fully run; deny-regression runs on current packs).

## irrevers-c87a3c50 — Base self-updater and trust pointer (icg update)

**No verifiable evidence found** for this bead itself — its closure predates
any implementation, and no commit is attributable to it.

The bead was created at 2026-08-14T14:37:10.287Z and updated (closed) at
2026-08-14T14:37:11.518Z — one second later, with no notes and no assignee.
That is during the bead-forge→bead-rs migration rehydration: `0941686`
("migrate: rehydrate the bead workspace from bead-forge to bead-rs") was
committed at 2026-08-14 10:37:13 -0400 = 14:37:13Z, two seconds after this
bead's close timestamp. At closure the repo contained **no self-updater or
trust-pointer code at all** — the repo's first feature commits (`97b8eba`,
trust pointer) landed ~12.5 hours later, and `src/update.rs` did not exist
until `d1e2b38`.

The work the bead describes did eventually land, but under the two split
children this bead's own description points at: irrevers-5fdc2e13 (trust
pointer, `97b8eba`) and irrevers-f59f9313 (self-updater, `d1e2b38` + `9c6951b`)
— see those entries. The only artifact that existed at close was the design
doc `docs/notes/self-update-and-release-gating.md` (added `32aa6bd`,
2026-08-13). This closure is a migration-era bookkeeping event, not evidence
of shipped work; it has the same false-closure shape commit `943b3ca`
(2026-08-22) documented for sibling beads, but was never reopened.

## irrevers-5fdc2e13 — Trust pointer mechanism

**Verifiable.** The bead's own notes name the commit, and it checks out:
`97b8eba` (2026-08-14 23:04:19 -0400 = 2026-08-15T03:04:19Z) landed **14
seconds** before the bead closed (2026-08-15T03:04:33Z). "feat(trust-pointer):
implement Layer 4 minimal form trust pointer mechanism" added
`src/trust_pointer.rs` (254 lines) with `TrustPointer` (trusted_ref /
updated_at / justification), `TrustPointerStore` with atomic
write-then-rename persistence, and the `icg trust` CLI in `src/main.rs`.

Tests: the bead's notes claim 6/6 passing at close; `src/trust_pointer.rs`
today carries those six (`test_trust_pointer_create`,
`test_trust_pointer_with_justification`, `test_store_save_and_load`,
`test_store_get_trusted_ref`, `test_store_is_trusted`, `test_atomic_write`)
plus two channel tests (`test_for_channel_path`, `test_channel_isolation`)
added later by `90a9653` (canary channels, 2026-08-15).

Caveat carried by history, not hidden by it: the shipped default path was
then `XDG_CONFIG_HOME/icg/trust-pointer.json` (agent-writable); the
root-owned `/etc/icg/` location came the next morning in `d1e2b38` — that
defect is what irrevers-96594031 was created (and closed) to cover.

## irrevers-f59f9313 — icg update: self-updater command

**Verifiable, with a timing caveat.** The implementation matches the notes'
description exactly, but the push landed shortly *after* the close timestamp:

- `9c6951b` (2026-08-14 23:47:30 -0400 = 2026-08-15T03:47:30Z; subject
  "feat(engine): implement content-mode input acquisition" — the commit
  bundles engine and updater work, so match on the `src/main.rs` delta, not
  the subject) wired the CLI: `Commands::Update` in `src/main.rs` (GitHub
  Releases check per the trust pointer, `UpdateConfig`). Its tree references
  `update::*` while not yet containing `src/update.rs` — the code existed
  uncommitted and went in broken at that boundary.
- `d1e2b38` (2026-08-15 00:21:04 -0400 = 2026-08-15T04:21:04Z; subject
  "docs(plan): decide install path, ownership, and mode for guard artifacts"
  — likewise a bundled commit) added
  `src/update.rs` (317 lines): one-shot GitHub Releases API check, download of
  the rule-pack artifact per the trust pointer, atomic write-then-rename
  replacement, no persistent process — the per-invocation architecture the
  description specifies. It also flipped the trust-pointer default to
  `/etc/icg/`.

The bead closed at 2026-08-15T03:26:55Z — 21 minutes before `9c6951b` and 54
minutes before `d1e2b38`. So the closure was made on work-in-progress that
was pushed immediately afterward; the described behavior is real and in
history, but the close timestamp is ~1 hour ahead of the earliest verifiable
commit.

Test coverage followed in `287b866` (2026-08-16, "test: add icg-ci end-to-end
integration coverage", `tests/icg_ci_integration_tests.rs`) and was later
superseded by the archive-deploying updater of irrevers-075634b8 (`409ca42`).

## irrevers-96594031 — Migrate the shipped trust-pointer and rule-pack artifact paths off agent-writable locations

**Verifiable.** The bead closed at 2026-08-26T01:39:15Z with notes stating
"migration already completed in prior commits"; both cited commits verify:

1. `d1e2b38` (2026-08-15 00:21:04 -0400): trust pointer moved from
   `XDG_CONFIG_HOME/icg/trust-pointer.json` to `/etc/icg/trust-pointer.json`
   (root-owned), together with the install-path/ownership/mode decision
   recorded in `docs/plan/plan.md` (the decision the sibling bead
   irrevers-ca79d63a scoped).
2. `a03a7e6` (2026-08-22 21:14:14 -0400): "fix(deployment): use root-owned
   system paths for all artifacts" — runtime state
   (`denial_log.rs`, `health.rs`, `state_store.rs`) moved from user-writable
   `dirs::state_dir()`/`dirs::cache_dir()` to `/var/cache/icg/`, and the
   `dirs` dependency was removed (Cargo.toml, Cargo.lock +8 −122).

The scoped check exists as described: `verify_artifact_directory_security()`
(`src/trust_pointer.rs:127`, introduced `4d5c1a5`, 2026-08-15) fails on a
directory that is not root-owned, is world-writable, or is writable by the
current (non-root) user. Current test surface around that check:
`installation_tests.rs` exercises the installed `/etc/icg` layout
(`installation_scenario_2_verify_rule_packs_load`,
`installation_scenario_4_dangerous_command_denied`), and the
`secure_tempdir()` helpers in `tests/fail_closed_runtime_tests.rs` and
`tests/maintenance_tasks_tests.rs` encode the check's invariants (0700, not
world-writable) so the rest of the suite can run beside it.

## irrevers-ca79d63a — Deploy the binary, rule-pack artifact and trust pointer outside the guarded agent's writable filesystem

**Verifiable.** Commit `a03a7e6` (2026-08-22 21:14:14 -0400 =
2026-08-23T01:14:14Z) landed **2 minutes** before the bead closed
(2026-08-23T01:16:06Z) and matches the close notes line for line: exactly the
three named files (`denial_log.rs`, `health.rs`, `state_store.rs`) moved to
`/var/cache/icg/`, and the `dirs` dependency was removed. Security-critical
artifacts (rule-pack, trust pointer) were already at root-owned `/etc/icg/`
via `d1e2b38` (2026-08-15), which is the same commit that recorded the
deploy-location decision (install path, ownership, mode) in `docs/plan/plan.md`
— the decision this bead said was missing.

Cross-reference, not part of this closure: deployment-shape hardening
continued after close — `c38b0cd` (2026-09-08, "pin root-owned 0755 modes on
the icg trust directories") and the mode/detection fixes around it belong to
irrevers-50077acb and irrevers-f839b213, which are outside this batch.

## irrevers-e00a5381 — icg-ci Argo WorkflowTemplate

**Verifiable.** The artifact lives in declarative-config (as the description
required), not this repo: `~/declarative-config` commit `122623ae`
("feat(iad-ci): add icg-ci Argo WorkflowTemplate", 2026-08-14 22:01:48 -0400 =
2026-08-15T02:01:48Z) landed **29 seconds** before the bead closed
(2026-08-15T02:02:17Z), at `k8s/iad-ci/argo-workflows/icg-ci-workflowtemplate.yml`.

Same-week buildout on the same file: `0f3f5faf` (2026-08-15 09:27 -0400, deny
regression suite stage), `d11a6472` (2026-08-15 09:44 -0400, coverage-diff
gate stage), `b2c2a0e5` (2026-08-16, push release tags to Forgejo before
creating the GitHub release).

Live check 2026-09-10: `kubectl --server=http://traefik-iad-ci:8001 get
workflowtemplates -n argo-workflows` lists `icg-ci` (alongside
`icg-guarded-builder`) — the template exists and is applied on the cluster
today. The Argo-not-GitHub-Actions constraint is honored; this repo has no
`.github/workflows/`.

## irrevers-075634b8 — Make icg update atomically deploy the complete modular production pack directory

**Verifiable.** Implementing commit `409ca42` (2026-08-25 22:55:03 -0400 =
2026-08-26T02:55:03Z) landed **10 seconds** before the bead closed
(2026-08-26T02:55:13Z). "feat(update): atomically deploy modular pack
archives" changed `src/update.rs` by 731 lines (+642 −89) — modular-archive selection from
the trusted release, archive-layout and per-pack validation before
activation, traversal/symlink hazard rejection, atomic swap of the whole
root-owned pack directory with rollback preservation, channel-specific
directories — and wired it through `src/main.rs`, exactly the scope the
description enumerates.

The description's two demanded end-to-end tests are present in
`tests/icg_ci_integration_tests.rs` (+236 −47 in the same commit):
`trusted_release_update_replaces_complete_pack_directory_and_preserves_enforcement`
(line 330 — enforcement of secrets/image-tag/storage-class after update) and
`malformed_release_archive_cannot_partially_deploy_or_escape_the_pack_root`
(line 436 — a malformed archive cannot partially deploy or escape the pack
root). Operator docs were updated in the same commit
(`docs/operators/deployment-guide.md` +66 −53, `docs/runbooks/rule-pack-updates.md`,
`docs/notes/self-update-and-release-gating.md`), closing the "deployment docs
say the production directory remains manual" gap the description called out.
