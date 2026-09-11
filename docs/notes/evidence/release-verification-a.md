# Closing evidence — release-verification closed beads, batch 1/5

Evidence for the eight beads in the "Release verification / release process /
distribution integrity" section of
`docs/notes/closed-beads-release-verification-inventory.md` (irrevers-c52de1f2,
child of irrevers-622aae24). Method: `bead show <id>` for each bead's own
description/notes, cross-referenced with `git log --grep`, `git log -S`, and
per-file history in this repo. Live GitHub release checks run 2026-09-10 via
`gh release view -R jedarden/irreversible-command-gate`. Evidence gathering
only — no plan edits.

Audited 2026-09-11 (irrevers-8176471b): every commit/tag/test/line reference
below was re-resolved and verified against the current tree, live GitHub
releases, and live iad-ci. Two corrections and two refreshes were applied in
place: the c2fcfd9 timing under irrevers-e77615c8 (an earlier revision misread
its `-0400` commit offset as UTC and said "~4h"; the true gap is ~30 seconds),
the canary-icg wording under irrevers-6de781f4 (one doc-comment mention does
exist in `src/trust_pointer.rs`), a live pullability re-check added under
irrevers-340ae322, and the release-count refresh under irrevers-eff8909f.

All eight beads were confirmed **Closed** at time of writing, so every entry
below is closing evidence, not a status correction.

---

## irrevers-84b36e47 — Verify icg-ci produces a real, complete GitHub release

**Verifiable.** The done-when condition still holds against the live registry:
`gh release view v0.1.1 -R jedarden/irreversible-command-gate` shows
`isDraft=false` with all four expected assets — `icg`, `icg-packs.tar.gz`,
`pack-manifest.json`, `rule-pack.json`. The release was published
2026-09-06T13:06:02Z, **25 seconds before** the bead's close timestamp
(2026-09-06T13:06:27Z) — the bead closed on the first complete release icg-ci
ever produced (all prior runs had failed; see the bead's 2026-08-26 note on
workflow `icg-ci-manual-mnwqh` failing at cargo test). The unblocking commits
are in history: `3399989` (2026-08-26, "test(ci): isolate status from installed
trust state", `tests/installation_tests.rs`), `c5d391b` (2026-08-26, "fix(ci):
unblock rule-pack release gates"), `eec8e73` (2026-08-27, "fix(test): isolate
status tests from the CI image's /etc/icg state"). v0.1.2 and later releases
carry the same four assets, confirming the result reproduces.

Caveat: the Argo Workflow object for the green run is no longer recoverable —
iad-ci retains only 5 icg-ci workflows and none from Sep 5–7 (TTL-reaped), so
the workflow name for the v0.1.1 run cannot be cited. The surviving GitHub
release object is the evidence.

## irrevers-e77615c8 — icg-ci: publish the rule-pack artifact as a release asset

**Verifiable.** Implementing commit `2c541ec` (2026-08-22, "feat(icg-ci):
publish rule-pack artifact as release asset") added the `build-pack` command
(`src/main.rs`, `src/rule_pack.rs`) and the release-upload step in
`containers/argo-guarded-builder/icg-ci-guarded-workflowtemplate.yml`. Follow-up
`c2fcfd9` (2026-08-24, "fix(rule-pack): exclude unconditional packs from merged
artifact", `src/rule_pack.rs`) landed ~30 seconds before the bead closed
(2026-08-24 00:26:39 -0400 = 2026-08-24T04:26:39Z; close 2026-08-24T04:27:11Z
— an earlier revision of this file misread the `-0400` offset as UTC and said
~4h). Live check 2026-09-10: `rule-pack.json` is present as an
asset on releases v0.1.1 and v0.1.2, and the workflow itself consumes the asset
across releases — the coverage-diff gate runs `gh release download
$PREVIOUS_VERSION --pattern rule-pack.json` (workflowtemplate line 130), which
succeeds on every run.

## irrevers-37eb1100 — Release-cutting runbook

**Verifiable.** Commit `436bdce` (2026-08-15, "docs: add release-cutting
runbook") added `docs/runbooks/release-cutting.md` (108 lines) and linked it
from `docs/notes/self-update-and-release-gating.md` — exactly matching the
close notes. The runbook is still present (7.5KB) and was deliberately revised
by the v0.1.2 release commit `1b6f6a6` ("icg-ci template cuts releases itself
now"), i.e. it has been maintained, not just written.

## irrevers-340ae322 — Prove ronaldraygun/argo-guarded-builder:0.1.0 is published and pullable by icg-ci

**Verifiable (manual verification recorded on the bead; no commits reference
the ID).** Close notes (2026-08-30T01:56Z, ten minutes after creation) record
both done-when criteria satisfied: `docker manifest inspect
ronaldraygun/argo-guarded-builder:0.1.0` succeeded with manifest digest
`sha256:dd3a46c3f85c1d6f52e55b4ad9a62cc06d6341b5055fdd5eb3e8bdd883d98ef5`, and
workflow `icg-ci-rg9n7`'s `codex-hook-compatibility` pods reached `Running`
(not ImagePullBackOff) while using the pinned image. Live re-check 2026-09-11
(this audit): pod `icg-ci-f8crj-build-and-release-30640588` (created
2026-09-11T00:06:52Z, iad-ci) is `Running` on
`ronaldraygun/argo-guarded-builder:0.1.0` — the same tag still published and
successfully pulled by icg-ci today. (The registry now answers anonymous
manifest requests with 401 — the repo is not anonymously pullable — so
in-cluster pulls remain the only observable pullability proof from here.)
In-repo corroboration:
`containers/argo-guarded-builder/VERSION` pins the version that the
`icg-ci-guarded-workflowtemplate.yml` image references resolve against (0.1.0
at verification time; since bumped in a separate version-bump bead).

## irrevers-e2bb8fbf — Add --channel to icg trust to match icg update (canary rollout)

**Verifiable.** Commit `c7e9df5` (2026-08-30, "feat: add --channel flag
visibility in icg trust help and round-trip test") is the exact close-note
claim: `src/main.rs` (+2, help hint making `--channel` visible in `icg trust
--help`) and `tests/maintenance_tasks_tests.rs` (+134) adding
`maintenance_scenario_trust_channel_roundtrip` (present at line 450 of the
current tree, covering `icg trust set --channel canary` → `icg update --channel
canary` plus channel isolation). Pre-existing channel coverage it built on:
`maintenance_scenario_trust_channel_support`
(`tests/maintenance_tasks_tests.rs:404`), inline `test_for_channel_path` /
`test_channel_isolation` (`src/trust_pointer.rs`), and
`update_channels_get_isolated_default_paths` (`src/update.rs:967`). Closed
2026-08-30T12:19Z.

## irrevers-6de781f4 — Canary rollout via NEEDLE --identifier

**Partially verifiable.** The mechanism half is evidenced: `90a9653`
(2026-08-15, "feat(canary): implement canary rollout via channel identifiers")
added channel support across `src/trust_pointer.rs` (+80, including
`TrustPointer::for_channel` and the inline `test_for_channel_path` /
`test_channel_isolation` unit tests), `src/update.rs`, and `src/main.rs` —
same-day as the close (2026-08-15T05:16Z). No commit message references the
bead ID and the bead's notes are empty, so the linkage is by date and scope
match. The end-to-end round-trip test arrived later, via irrevers-e2bb8fbf
(`c7e9df5`). **The operational half has no verifiable evidence found**: the
only in-repo mention of a launched `canary-icg` NEEDLE worker (the bead's
stated roll-out step) is a doc comment in `src/trust_pointer.rs` describing
the intended mechanism ("// Canary channel worker (launched via NEEDLE
--identifier canary-icg)") — no docs, manifests, or runbook record one
actually being launched.

## irrevers-b6579270 — Per-release deny-rate telemetry and rolling baseline

**Verifiable.** Two implementing commits, neither of which cites the bead ID in
its message (bead notes are empty; linkage is by scope and timing — the bead
closed 2026-08-21T00:54Z, 59 seconds after the first commit landed):

- `5b4d5df` (2026-08-20 20:53 -0400 = 2026-08-21T00:53Z, "feat: persist
  per-release deny-rate telemetry") — `src/engine.rs`, `src/state_store.rs`
  (per-invocation persistence via the state store), plus
  `tests/release_telemetry_tests.rs` and plan updates.
- `721f3d9` (2026-08-20 22:16 -0400, "feat: integrate rolling telemetry with
  auto-rollback") — wired the baseline/deviation to the poison-pill rollback
  consumer (`src/main.rs` + tests).

Test evidence in the current tree:
`engine_persists_per_release_evaluation_and_deny_counts` and
`engine_telemetry_feeds_poison_pill_rollback`
(`tests/release_telemetry_tests.rs:19` and `:47`).

## irrevers-eff8909f — Write inventory of shipped releases v0.1.0–v0.1.6 with dates and commits

**Verifiable.** Evidence-only bead (acceptance allowed "comment or file
reference"; no plan edits, per its own criteria). The complete findings
artifact — all 7 tags v0.1.0–v0.1.6 with commit SHAs, dates, one-line summaries,
and the reproducible source commands — is recorded verbatim in the bead's notes
(closed 2026-09-10T10:30Z). Spot-checked against local git: all seven tags
exist, are lightweight, and point at the recorded SHAs (e.g. v0.1.0 →
`f0fe556`, v0.1.2 → `1b6f6a6`, v0.1.6 → `aab687d`). Two nuances for downstream
reconciliation (refreshed by the 2026-09-11 audit): (1) tags/releases beyond
the bead's stated range now exist — `v0.1.7`–`v0.1.9` tags and releases
`v0.1.7`–`v0.1.10` as of 2026-09-11; (2) the bead's "zero GitHub Releases
exist" line was true only against origin (Forgejo `git.ardenone.com`, no
GitHub host configured there) — the GitHub mirror does have releases v0.1.1
through v0.1.10, verified live 2026-09-11 (v0.1.1–v0.1.7 verified in the
original 2026-09-10 pass).
