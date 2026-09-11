# Final inventory — closing evidence for the 41 closed release-verification and fail-closed-harness beads

Compiled deliverable for **irrevers-2c3b4637** ("Write inventory of closed
release-verification and fail-closed-harness beads with evidence"), assembled
by its final split child **irrevers-f7a52307** from the three prior children:

- **irrevers-622aae24** → enumeration: `docs/notes/closed-beads-release-verification-inventory.md` (41 rows, three sections)
- **irrevers-c52de1f2** → evidence: `docs/notes/evidence/consolidated-evidence-summary.md` + five per-category files (`release-verification-{a,b}.md`, `fail-closed-harness-{a,b}.md`, `ci-gate-test-harness.md`)
- **irrevers-fa1b03f9** → weak-evidence flags: `docs/notes/evidence/closing-evidence-flags.md`

Written 2026-09-11 at repo HEAD `e6771f0`. This file is the one-stop record
the parent's acceptance criteria ask for: every relevant closed bead with its
ID, title, close date, a summary of the concrete closing evidence, and an
explicit flag wherever that evidence is weak or unverifiable. Per-bead
detail (full commit lists, test line numbers, verification method) lives in
the source files above; nothing here contradicts them. **No PRs exist
anywhere in the inventory** — this repo works directly on `main`, so commits
are the unit of record. `docs/plan/plan.md` was not edited by any step in
this chain (its last touch, `d26a83a` 2026-09-11, belongs to the separate
workflows-guard documentation bead irrevers-a39bdf35).

Close dates are the bead's `updated_at` at the close instant, captured by the
enumeration pass on 2026-09-10 (bead-rs tracks no separate close timestamp).
They are the authoritative close-instant record: live `updated_at` has since
drifted on all 41 beads because the short-form evidence summaries were
appended to their notes on 2026-09-11T08:34Z, so a fresh `bead show Updated:`
no longer reads as the close date.

Status tags, per the consolidated summary's legend:

- **GIT-VERIFIED** — implementing/verifying commits exist, resolve at HEAD, and match the bead's scope (VIA SUCCESSORS/VIA CHILDREN = split/bookkeeping close delivered under other IDs).
- **SELF-REPORTED** — closing evidence is the bead's own notes (manual verification or an evidence-only deliverable); real when recorded, not re-checkable from git.
- **NO VERIFIABLE EVIDENCE** — collected explicitly; nothing omitted.

Flag tags, per the flags file:

- **FLAG T1** — no verifiable closing evidence at all.
- **FLAG T2** — self-reported only; close-time evidence not re-checkable (sometimes no longer re-checkable from the live systems it described either).
- **FLAG T3** — work real and verified at HEAD, but the closure record itself is a timing/bookkeeping event rather than a completion record.
- **solid** / **solid (caveat)** — evidence verified at HEAD; a caveat is scope/verification-shape, not missing evidence.

---

## Section A — Release verification / release process / distribution integrity (16 beads)

### irrevers-84b36e47 — Verify icg-ci produces a real, complete GitHub release
Close **2026-09-06**. **GIT-VERIFIED.** Release v0.1.1 published 25 s before
close, non-draft with all four assets (`icg`, `icg-packs.tar.gz`,
`pack-manifest.json`, `rule-pack.json`); reproduces on every later release.
Unblocking commits `3399989`, `c5d391b`, `eec8e73`; only ID-citing commit
`9350f19` is an empty CI trigger. **solid (caveat)** — the green Argo
Workflow object is TTL-reaped; the surviving GitHub release object is the
evidence.

### irrevers-e77615c8 — icg-ci: publish the rule-pack artifact as a release asset
Close **2026-08-24**. **GIT-VERIFIED.** `2c541ec` added the `build-pack`
command and the release-upload step in the workflowtemplate; `c2fcfd9` ~30 s
before close. `rule-pack.json` is a live release asset since v0.1.1,
consumed by the coverage-diff gate (template line 130). **solid.**

### irrevers-37eb1100 — Release-cutting runbook
Close **2026-08-15**. **GIT-VERIFIED.** `436bdce` added
`docs/runbooks/release-cutting.md` (+108), exactly matching the close notes;
maintained since (`1b6f6a6`). **solid.**

### irrevers-340ae322 — Prove ronaldraygun/argo-guarded-builder:0.1.0 is published and pullable by icg-ci
Close **2026-08-30**. **SELF-REPORTED.** Manual `docker manifest inspect`
(digest `sha256:dd3a46c3…98ef5`) plus `icg-ci-rg9n7` pod states; the pods
are unrecoverable (podGC OnPodCompletion) and the registry now 401s
anonymous manifest requests — neither half re-verifiable today. Indirect
corroboration only: `VERSION` and both `image:` pins read 0.1.0; fresh 0.1.0
pulls re-observed (`icg-ci-czdkx`, 2026-09-11). **FLAG T2.**

### irrevers-e2bb8fbf — Add --channel to icg trust (canary rollout)
Close **2026-08-30**. **GIT-VERIFIED.** `c7e9df5` is the exact close-note
claim: `src/main.rs` help hint plus `tests/maintenance_tasks_tests.rs` (+134)
with `maintenance_scenario_trust_channel_roundtrip` (:450 at HEAD). Later
touches `890429f`, `0fb164d`. **solid.**

### irrevers-6de781f4 — Canary rollout via NEEDLE --identifier
Close **2026-08-15**. **GIT-VERIFIED — mechanism only.** `90a9653` added
channel support (`TrustPointer::for_channel`, `src/update.rs`,
`src/main.rs`); end-to-end round-trip later via `c7e9df5`. The operational
half — an actually launched `canary-icg` worker — has no verifiable evidence
beyond a doc comment (`src/trust_pointer.rs:102`). **FLAG T3** (half the
closure unsubstantiated).

### irrevers-b6579270 — Per-release deny-rate telemetry and rolling baseline
Close **2026-08-21**. **GIT-VERIFIED.** `5b4d5df` (59 s before close;
`src/engine.rs`, `src/state_store.rs`, `tests/release_telemetry_tests.rs`) +
`721f3d9` (baseline/deviation wired to poison-pill rollback). Tests at HEAD:
`engine_persists_per_release_evaluation_and_deny_counts` (:19),
`engine_telemetry_feeds_poison_pill_rollback` (:47). **solid.**

### irrevers-eff8909f — Write inventory of shipped releases v0.1.0–v0.1.6 with dates and commits
Close **2026-09-10**. **SELF-REPORTED (evidence-only bead, corroborated).**
Deliverable is the tag table in its own notes — no carrying commit; the
content verifies independently (all seven tags exist at the recorded SHAs,
v0.1.0 `f0fe556` … v0.1.6 `aab687d`). Two recorded problems: the "zero
GitHub Releases" line was wrong for the mirror even at close, and the stated
range is far stale (v0.1.37 Latest at the flags pass). **FLAG T2.**

### irrevers-2cb3dbd2 — Gate the actual modular release packs in icg-ci instead of static fixtures
Close **2026-08-26**. **GIT-VERIFIED.** `6eaeb70` 10 s before close:
`tests/release_gate_integrity_tests.rs` (+284 — four named
mutate/verify/widen tests), `tests/icg_ci_integration_tests.rs` (+110), and
the template switch to real pack bytes (commands at template lines
117/121/156/162/202–224); follow-up `b8aed51`. **solid (caveat)** — one
sub-clause explicitly skipped: deny-regression generation runs on current
packs, not prior-release packs.

### irrevers-c87a3c50 — Base self-updater and trust pointer (icg update)
Close **2026-08-14**. **NO VERIFIABLE EVIDENCE.** Closed **1 second after
creation**, no notes, no assignee — a bead-forge→bead-rs rehydration
artifact (rehydration commit `0941686` landed 2 s later). No implementing
commit attributable; the repo verifiably contained none of the described
code at close (tip `200bb7e`; `src/trust_pointer.rs` first appears ~9.5 h
later in `97b8eba`). Its scope landed under successors irrevers-5fdc2e13 and
irrevers-f59f9313 — treat this closure as void bookkeeping and reconcile the
scope against those two. Never reopened despite the identical false-closure
shape `943b3ca` documented for siblings. **FLAG T1.**

### irrevers-5fdc2e13 — Trust pointer mechanism
Close **2026-08-15**. **GIT-VERIFIED.** `97b8eba` 14 s before close:
`src/trust_pointer.rs` (+254; atomic write-then-rename store) + the
`icg trust` CLI; all six close-note tests still at HEAD. Carried note (not a
flag): the shipped default path was agent-writable until `d1e2b38` fixed it
next day. **solid.**

### irrevers-f59f9313 — icg update: self-updater command
Close **2026-08-15**. **GIT-VERIFIED — late-landing.** The implementing pair
`9c6951b` (`Commands::Update`) and `d1e2b38` (`src/update.rs` +317) landed
**~21–54 min after close**, bundled under unrelated subjects; tests followed
in `287b866`, superseded by `409ca42`. The close notes were an unanchored
completion claim. **FLAG T3** (close predates work; no hash, no test names
in the closure record).

### irrevers-96594031 — Migrate the shipped trust-pointer and rule-pack artifact paths off agent-writable locations
Close **2026-08-26**. **GIT-VERIFIED.** Both close-note commits verify:
`d1e2b38` (trust pointer → root-owned `/etc/icg/trust-pointer.json`) and
`a03a7e6` (runtime state → `/var/cache/icg/`, `dirs` dependency removed).
Scoped check present as described: `verify_artifact_directory_security()`
(`src/trust_pointer.rs:127`, introduced `4d5c1a5`). **solid.**

### irrevers-ca79d63a — Deploy the binary, rule-pack artifact and trust pointer outside the guarded agent's writable filesystem
Close **2026-08-23**. **GIT-VERIFIED.** `a03a7e6` (2 min before close)
matches the close notes line for line: the three named runtime-state files
moved to `/var/cache/icg/`; `/etc/icg/` placement had landed via `d1e2b38`.
Duplicate-claim question resolved: `a03a7e6` belongs here, `d1e2b38` to
irrevers-96594031. **solid.**

### irrevers-e00a5381 — icg-ci Argo WorkflowTemplate
Close **2026-08-15**. **GIT-VERIFIED (cross-repo).** declarative-config
`122623ae` created the template (+107) 29 s before close; buildout
`0f3f5faf` (deny-regression), `d11a6472` (coverage-diff), `b2c2a0e5` (tag
push) — all four resolve in `~/declarative-config`; the `icg-ci` template is
live on iad-ci. Correctly no evidence in this repo. **solid.**

### irrevers-075634b8 — Make icg update atomically deploy the complete modular production pack directory
Close **2026-08-26**. **GIT-VERIFIED.** `409ca42` 10 s before close:
`src/update.rs` +642 −89 (modular-archive selection, per-pack validation,
traversal/symlink rejection, atomic swap with rollback); both demanded
end-to-end tests at HEAD
(`trusted_release_update_replaces_complete_pack_directory_and_preserves_enforcement`
:330, `malformed_release_archive_cannot_partially_deploy_or_escape_the_pack_root`
:436). **solid.**

**Section A: 16 = 13 git-verified + 2 self-reported + 1 no-evidence. Flags: T1 ×1
(c87a3c50), T2 ×2 (340ae322, eff8909f), T3 ×2 (6de781f4, f59f9313).**

---

## Section B — Fail-closed harness / policy enforcement behavior (20 beads)

### irrevers-8d2d4a73 — Engine: unconditional fail-open on parse failure or exception
Close **2026-08-15**. **GIT-VERIFIED.** `48acfc1` 14 s before close: every
check path in `src/engine.rs`/`src/main.rs` wrapped; both acceptance
fixtures wired as `tests/fail_open_tests.rs`
`layer_one_malformed_stdin_fails_open` and
`layer_one_corrupt_rule_pack_fails_open` — at HEAD. **solid.**

### irrevers-aab3854c — Design fail-closed transition state machine and graduation criteria
Close **2026-08-16**. **GIT-VERIFIED.** `16f84c1` added
`docs/design/fail-closed-transition.md` (+342) covering every criterion
(state machine, per-state fleet behavior, graduation, transition triggers,
poison-pill integration, durable state/crash recovery, emergency rollback).
Docs only, matching the design-only scope. **solid.**

### irrevers-cd3f4c44 — Graduated fail-open to fail-closed policy for guard crashes
Close **2026-08-21**. **GIT-VERIFIED.** `bb362fb` 16 s before close: created
`src/fail_closed.rs` (800 lines, durable PolicyStore) +
`tests/fail_closed_policy_tests.rs` (+167). Full scoped chain
`bb362fb`/`17971b7`/`3f0f00d`/`859e19e`;
`DEFAULT_GRADUATION_THRESHOLD = 3` resolves the "threshold TBD" placeholder.
**solid.**

### irrevers-8a24ad8d — Implement fail-open baseline and fail-closed enforcement modes
Close **2026-08-21**. **GIT-VERIFIED.** `17971b7` 15 s before close: wired
the configured PolicyMode into `src/engine.rs`/`src/health.rs` and added
`tests/fail_closed_runtime_tests.rs` (+132) with the acceptance matrix
(`recovered_guard_crash_denies_in_fail_closed_mode` :240,
`lifecycle_reports_recovered_crash_once` :298). **solid.**

### irrevers-019c36d3 — Implement fail-closed policy transition mechanism
Close **2026-08-21**. **GIT-VERIFIED — strongest record in the section.**
The bead's own notes name all four implementing commits (`bb362fb`,
`17971b7`, `3f0f00d`, `859e19e`) and the exact verification commands; all
four resolve with the claimed subjects, `859e19e` 3 min before close.
**solid.**

### irrevers-fffef435 — Integrate with poison-pill mechanism for automatic graduation
Close **2026-08-21**. **GIT-VERIFIED.** `3f0f00d` 16 s before close:
`src/fail_closed.rs` +213 −21 and
`operator_force_graduate_and_force_revert_are_durable` (:214); read-only
integration from `bb362fb` (`policy_reconciles_unique_clean_releases_and_graduates`
:50, `poison_pill_resets_open_policy_without_editing_telemetry` :109).
**solid.**

### irrevers-0f49129d — Poison-pill auto-rollback (original bundled)
Close **2026-08-15**. **GIT-VERIFIED VIA SUCCESSORS (split close).** Closed
the same second successor irrevers-ff4f17da was created; **no commit or note
lands under its own ID**. Measurement half delivered via irrevers-b6579270
(`5b4d5df`, `721f3d9`), reaction half via irrevers-ff4f17da (`d653ade`,
`src/rollback.rs`) — the bundled scope is fully delivered and verifiable,
but this closure is a bookkeeping event, not a completion claim. **FLAG T3.**

### irrevers-ff4f17da — Poison-pill auto-rollback: revert the trust pointer on a deny-rate spike
Close **2026-08-21**. **GIT-VERIFIED.** `d653ade` 20 s before close:
`src/rollback.rs` (+371, `check_and_rollback`) + rollback runbook updates;
five conservative-trigger tests inline at HEAD (`:212`, `:294`, `:322`,
`:341`, `:362`). **solid.**

### irrevers-3fc4bdde — Apply the documented ICG_DISABLED emergency bypass to hook and PATH-wrapper enforcement
Close **2026-08-26**. **GIT-VERIFIED.** `20808e9` 18 s before close (compile
fix `9ec6848` 1 min earlier): added `src/emergency_bypass.rs`, routed Hook
and the argv[0] wrapper through the bypass; named tests in
`tests/emergency_response_tests.rs`; `#[ignore]` stripped from
`emergency_scenario_3_bypass_guard_with_disabled_flag` (:68). **solid.**

### irrevers-f891f555 — Make the fail-closed policy read path lock-free for guarded invocations
Close **2026-09-08**. **GIT-VERIFIED.** `0a5faa9` names the bead ID in its
subject, 18 s before close: `PolicyStore::load` no longer creates/writes the
`.lock`; graduation reconcile moved to `icg policy reconcile`;
`tests/fail_closed_runtime_tests.rs` +170 −5
(`hook_invocation_leaves_administrator_owned_policy_untouched`,
`operator_policy_commands_manage_the_durable_policy`). **solid.**

### irrevers-edb5c4ca — Stop reconciling the fail-closed policy from the hook and wrapper paths
Close **2026-09-08**. **GIT-VERIFIED.** Same shared changeset `0a5faa9`
(closed 36 s later): both required call-site comments at HEAD naming
`icg policy reconcile`; reconcile only at `src/main.rs:2539`; same 0555
negative control and green `cargo test` in the commit body. **solid.**

### irrevers-9eb4de16 — Add regression tests for a root-owned policy directory on the hook path
Close **2026-09-08**. **GIT-VERIFIED.** Same changeset `0a5faa9` (closed 1 s
later): `hook_invocation_leaves_administrator_owned_policy_untouched` (:328,
0555 policy dir, stderr asserted clean) and
`operator_policy_commands_manage_the_durable_policy` (:486); refinement
`c6dcc3c`. **solid (caveat)** — the "fails without the fix" criterion rests
on the commit body's manual negative control (v0.1.3 warns), not a recorded
pre-fix test run.

### irrevers-93baa29a — Verify icg policy status and reconcile as root on an installed host layout
Close **2026-09-08**. **SELF-REPORTED (verification-only bead — the expected
shape).** No commit was in scope; the entire evidence is the 2026-09-07
manual root-run notes (`policy status` exit 0; `policy reconcile` exit 0,
idempotent, writes nothing; scratch-store exercises both directions; live
`/etc/icg` silent vs uid-1000 scratch reproducing the ownership warning).
Nothing independently checkable — a reviewer must trust the notes or re-run.
**FLAG T2.**

### irrevers-3e6c6fde — Hook and wrapper guarded paths take the fail-closed policy lock via crash recovery
Close **2026-09-08**. **GIT-VERIFIED — strongest record in the section.**
Fix `0e669f2` names the bead ID, ~2m51s before close: guarded crash recovery
records evidence via `StateStore::record_guard_crash` (new `ICG_STATE_PATH`
override keeping tests off `/var/cache/icg`); only `icg policy reconcile`
converts the counter to a poison-pill event. Regression tests at HEAD
(`tests/fail_closed_runtime_tests.rs:406`, `:151`); commit body: all three
crash tests fail against the previous behavior. **solid.**

### irrevers-50077acb — Set explicit root-owned 0755 modes on the icg trust directories in argo-guarded-builder
Close **2026-09-08**. **GIT-VERIFIED.** `c38b0cd` ~16 min before close,
Dockerfile-only (+40 −2 — no test files by nature): `root:root`/`0755` on
`/etc/icg`, `/etc/icg/packs`, `/etc/icg/overrides`, `/var/cache/icg`;
build-time assertion RUN fails the image build on any non-root-owned or
world-writable trust dir — the standing check. **solid.**

### irrevers-ffdc924b — Add guard health tracking and crash monitoring infrastructure
Close **2026-08-21**. **GIT-VERIFIED.** `a525523` 13 s before close, +921
lines (`src/health.rs`, `src/health_server.rs`, `src/metrics.rs`,
telemetry); every acceptance item traceable, six inline tests still at HEAD
(stale-run-marker crash detection, clean-exit clearing, signal/OOM
classification, cgroup OOM evidence, metrics round-trip). **solid.**

### irrevers-9007792b — Operational monitoring and alerting infrastructure
Close **2026-08-21**. **GIT-VERIFIED.** `a750033` 10 s before close, +1268
lines: Grafana dashboard, Prometheus alerts + scrape config, promtail
config, monitor loop `run_monitor`, `collect_snapshot`
(`src/monitoring.rs:145`), Prometheus exposition; inline tests at HEAD
(:470, :545, `src/denial_log.rs:1153`). **solid.**

### irrevers-0d710c9a — Write activation documentation and operational runbooks
Close **2026-08-21**. **GIT-VERIFIED.** `859e19e` 16 s before close, +2452
lines across ten docs (`docs/operators/fail-closed-mode.md`,
`docs/operators/training-manual.md`, `docs/onboarding-guide.md`, plus
rollback/incident-response/troubleshooting/architecture updates) — all
present at HEAD. Docs only, consistent with scope. **solid.**

### irrevers-1517a263 — cargo test writes into the production denial log on an instrumented host
Close **2026-09-07**. **GIT-VERIFIED.** `0c062e3` names the bead ID, ~1m41s
before close: `denial_log::operational_log_path()` resolves every
operational write's sink and refuses the `/var/cache/icg` default to
test-driven callers; `tests/denial_log_pollution_guard_tests.rs` (+241 —
`an_in_process_denial_never_reaches_the_live_log` :185). Commit body and
notes record the negative control and the production-log archive. **solid.**

### irrevers-0aa08f4e — Routine run_started lifecycle telemetry prints to stderr on every guarded invocation
Close **2026-09-08**. **GIT-VERIFIED.** `c6dcc3c` names the bead ID, 16 s
before close: `HealthStore::start_run` no longer prints on the healthy path;
the fault/crash emitters that must stay are held by
`a_recovered_crash_still_announces_itself_on_stderr`
(`tests/fail_closed_runtime_tests.rs:553`). Commit body: zero-byte stderr
against live `/etc/icg` vs two lines on deployed v0.1.3; 60 suites green,
clippy clean. **solid.**

**Section B: 20 = 19 git-verified (one via successors) + 1 self-reported +
0 no-evidence. Flags: T2 ×1 (93baa29a), T3 ×1 (0f49129d).**

---

## Section C — CI gate / test-harness infrastructure (5 beads)

### irrevers-b4b37bf0 — Layer 1: regression-suite CI gate
Close **2026-08-15**. **GIT-VERIFIED VIA CHILDREN (split-bookkeeping close).**
At the close instant no regression-suite code existed — the parent closed as
its scope split. Children delivered same day: irrevers-7684fa60 → `0970190`
(`src/regression.rs` +614, `regression-suite` CLI, tests) and
irrevers-69594753 → declarative-config `0f3f5faf` (build-failing
deny-regression step). The gate still runs in CI (template line 149, over
real pack bytes since `6eaeb70`). **FLAG T3.**

### irrevers-b0a453b2 — Layer 1: verify the regression-suite gate actually fails the build
Close **2026-08-15**. **SELF-REPORTED.** The mutation experiment (deny →
`additional_context` on `vault-kv-destroy` → `cargo test` 101, gate exit 1;
restore → green) exists in **no commit**. Recorded staleness: `e1aab5a`
(2026-09-06) deliberately narrowed that exact edge — the same flip now exits
0 with a reasoned skip, and enforcement moved to the pinned-corpus
invariants at HEAD (`tests/regression_suite_scope_tests.rs:139`, `:83`,
`:43`; `tests/icg_ci_integration_tests.rs:43`). What it verified was real
when performed but is no longer the behavior at HEAD. **FLAG T2.**

### irrevers-f61efd80 — Layer 1: coverage-diff CI gate
Close **2026-08-15**. **GIT-VERIFIED — closure-timing caveats.** `97b8eba`
created `src/coverage.rs`, the `coverage-diff` CLI, tests and three fixtures
~1 h before close — but the described report-format/justification scope
landed **~9.6 h after close** (`48d5a60`); CI wiring declarative-config
`d11a6472`; the gate still runs at template line 156. Close notes carried
only a terminology reconciliation. **FLAG T3** (close predates the full
described scope).

### irrevers-29a9131c — Layer 1: verify the coverage-diff gate actually blocks an unjustified change
Close **2026-08-15**. **SELF-REPORTED.** The block-then-pass experiment
(no `--justification` → exit 2 `regressions_detected`; with justification →
exit 0) lives only in the close notes; independently re-reproduced
identically 2026-09-10/11. Real-pack regression detection is pinned at HEAD
by `tests/release_gate_integrity_tests.rs`; fixture inconsistencies fixed by
`c096a1e`. Residual gap: a pure redirect-channel flip is not flagged by
coverage-diff — channel enforcement rests on the pinned corpus. **FLAG T2.**

### irrevers-ed77224f — End-to-end integration testing for icg-ci workflow
Close **2026-08-16**. **GIT-VERIFIED.** `287b866` 17 s before close:
`tests/icg_ci_integration_tests.rs` (+397) —
`icg_ci_release_candidate_runs_both_layer_one_gates_and_emits_layer_two_report`
(:78); the self-update area later hardened by `409ca42` (:330, :436); the
shipped template pinned by `ci_workflow_gates_actual_pack_bytes_not_fixtures`
(:609). **solid (caveat)** — area (1), driving the actual Argo API, is
operational-only evidence (green four-asset releases), not an in-repo test.

**Section C: 5 = 3 git-verified + 2 self-reported + 0 no-evidence. Flags:
T2 ×2 (b0a453b2, 29a9131c), T3 ×2 (b4b37bf0, f61efd80).**

---

## Explicit weak/unverifiable-evidence flags (11 of 41, for downstream scrutiny)

### Tier 1 — no verifiable closing evidence at all (1)
- **irrevers-c87a3c50** — 1-second rehydration-artifact close; no code
  existed at close; scope lives under successors irrevers-5fdc2e13 /
  irrevers-f59f9313. Treat as void bookkeeping; reconcile scope against the
  successors.

### Tier 2 — self-reported only; close-time evidence not re-checkable (5)
- **irrevers-340ae322** — podGC + registry 401: neither half of the
  publish/pull claim re-verifiable today; git-side corroboration only.
- **irrevers-eff8909f** — tag table in notes only; content verifies but the
  "zero GitHub Releases" line was wrong for the mirror and the range is stale.
- **irrevers-93baa29a** — manual-run notes only (expected shape for a
  verification-only bead, but nothing independently checkable).
- **irrevers-b0a453b2** — notes-only mutation experiment; `e1aab5a`
  superseded the exact behavior it verified — re-anchor to the pinned-corpus
  invariants before relying on it.
- **irrevers-29a9131c** — notes-only experiment, mitigated by independent
  re-reproduction + pinned tests; residual redirect-channel-flip gap stands.

### Tier 3 — work verified at HEAD, closure record defective (5)
- **irrevers-0f49129d** — split/bookkeeping close; no evidence under its own
  ID (delivered via b6579270 + ff4f17da).
- **irrevers-b4b37bf0** — split/bookkeeping close; no code existed at the
  close instant (delivered same day by children).
- **irrevers-f59f9313** — close predates its implementing commits by
  ~21–54 min; closure record was an unanchored completion claim.
- **irrevers-f61efd80** — close predates the described report-format/
  justification scope by ~9.6 h.
- **irrevers-6de781f4** — operational half (launched `canary-icg` worker)
  unsubstantiated; mechanism half git-verified.

### Confirmed solid (26 of 41)
22 clean (all Section B entries except 93baa29a/0f49129d, plus e77615c8,
37eb1100, e2bb8fbf, b6579270, 5fdc2e13, 96594031, ca79d63a, e00a5381,
075634b8) and 4 with recorded caveats that do not weaken the closure:
**irrevers-84b36e47**, **irrevers-2cb3dbd2**, **irrevers-9eb4de16**,
**irrevers-ed77224f** (caveats stated inline above).

Count check: 1 (T1) + 5 (T2) + 5 (T3) + 26 solid = **41**, matching the
inventory row for row. Classification check: 35 GIT-VERIFIED + 5
SELF-REPORTED + 1 NO VERIFIABLE EVIDENCE = **41** (16+20+5 by section).
PR-based verification: not applicable to any entry (no PRs exist in this
repo's flow).

---

## Compilation-time spot checks (2026-09-11, HEAD `e6771f0`)

This compile did not repeat the source passes' verification (recorded on
irrevers-c52de1f2, irrevers-f58a53ca, irrevers-fa1b03f9); it re-confirmed:

- All 41 inventory beads still `Closed` via `bead show` at compile time.
- 42 cited commit hashes (a sample spanning every section, plus both tag
  SHAs v0.1.0/v0.1.6) resolve via `git cat-file -e` at HEAD.
- The per-bead entries above are faithful condensations of the consolidated
  summary; flags match `closing-evidence-flags.md` tier for tier.
- `docs/plan/plan.md` untouched by this chain (see header note on `d26a83a`).
