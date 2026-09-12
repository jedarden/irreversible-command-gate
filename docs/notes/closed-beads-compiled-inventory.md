# Compiled inventory — closed release-verification and fail-closed-harness beads (42)

Single compiled deliverable for **irrevers-d0a73a21**: every closed bead in the
release-verification / fail-closed-harness inventory with its ID, title, close
date, a one-line evidence summary, and its evidence-strength rating, plus an
explicit weak/unverifiable-evidence section. Compiled 2026-09-12 at repo HEAD
`e669d14`.

This document **reconciles** the prior artifacts; it does not replace their
detail:

- **Enumeration** — `docs/notes/closed-beads-release-verification-inventory.md`
  (second pass, irrevers-5dfe499e): the 42 IDs, titles, and close dates below
  are its rows, verbatim. Close dates are the last `closed` event per bead in
  `.beads/checkpoint/forensic.jsonl`, not `updated_at` (mass-drifted by the
  2026-09-11T08:34Z evidence pass — see that file's method section).
- **Per-bead evidence** — `docs/notes/evidence/second-pass-closing-evidence.md`
  (irrevers-4bc81065): fresh extraction for irrevers-49dbb095 and an
  independent re-verification of the inherited 41 at HEAD `c4385ac`. Full
  commit lists, test line numbers, and verification method for the 41
  inherited entries live in `docs/notes/evidence/final-inventory.md` (first
  pass, irrevers-2c3b4637/irrevers-f7a52307) and remain valid — zero
  `src/`/`tests/`/`containers/` drift since (only version bumps intervened).
- **Weak-evidence flags** — **irrevers-fdc1cd74** (closed 2026-09-12T00:18Z),
  the flagging child this bead depends on: every bead rated
  solid / partial / weak-unverifiable after independent spot-checks at HEAD
  `028055d` (52/52 cited in-repo SHAs resolve; 7/7 tags; 4/4 cross-repo
  declarative-config SHAs; ID-linkage greps re-run; every flagged close_reason
  re-read from the forensic log). Its ratings are authoritative here.

Evidence summaries below are one-line condensations; where a full entry lives
in `final-inventory.md` (sections A/B/C) or second-pass §1, nothing in this
file contradicts it. No PRs exist anywhere in the inventory — this repo works
directly on `main`, so commits are the unit of record.

## Legend

Classification (from the close-note + git/test record):

- **GIT** — implementing/verifying commits exist, resolve at HEAD, and match
  scope (VIA = delivered under successor/child IDs; still git-verified, but
  the closure itself was a bookkeeping event).
- **SELF** — closing evidence is the bead's own notes (manual verification or
  an evidence-only deliverable); real when recorded, not re-checkable from git.
- **NONE** — no verifiable closing evidence at all.

Rating (irrevers-fdc1cd74; first-pass tier in parentheses where it maps):

- **solid** — evidence verified at HEAD. *solid (caveat)* records a
  scope/verification-shape caveat that does not weaken the closure.
- **partial** (≈ T3, plus eff8909f from T2 — see reconciliation) — substance
  real and verifiable, but the closure record itself is defective: split/
  bookkeeping close, close predating the work, or a half-unsubstantiated scope.
- **weak-unverifiable** (≈ T1/T2) — closing evidence is not re-checkable from
  the repo (or never existed); trust-the-notes, re-run, or treat as void.

## Section A — Release verification / release process / distribution integrity (16)

| ID | Title | Close (UTC) | Evidence summary | Class | Rating |
|---|---|---|---|---|---|
| irrevers-84b36e47 | Verify icg-ci produces a real, complete GitHub release | 2026-09-06 13:06 | Release v0.1.1 published non-draft, all four assets, 25 s before close; reproduces on every later release; unblocks `3399989`/`c5d391b`/`eec8e73` (ID-citing `9350f19` is an empty CI trigger) | GIT | solid (caveat: Argo run TTL-reaped; the release object is the evidence) |
| irrevers-e77615c8 | icg-ci: publish the rule-pack artifact as a release asset | 2026-08-24 04:27 | `2c541ec` build-pack + upload step (`c2fcfd9` 30 s before close, cites ID); `rule-pack.json` live asset since v0.1.1, consumed by the coverage-diff gate | GIT | solid |
| irrevers-37eb1100 | Release-cutting runbook | 2026-08-15 13:57 | `436bdce` added `docs/runbooks/release-cutting.md` +108, matching close notes; maintained since (`1b6f6a6`) | GIT | solid |
| irrevers-340ae322 | Prove ronaldraygun/argo-guarded-builder:0.1.0 is published and pullable by icg-ci | 2026-08-30 01:56 | Manual `docker manifest inspect` (registry digest) + CI pod states — pods podGC-deleted, registry 401s anonymous manifests: neither half re-checkable today; git-side 0.1.0 pins + fresh-pull re-observation only | SELF | **weak-unverifiable (T2)** |
| irrevers-e2bb8fbf | Add --channel to icg trust to match icg update (canary rollout) | 2026-08-30 12:19 | `c7e9df5` is the exact close-note claim: `--channel` help hint + `maintenance_scenario_trust_channel_roundtrip` (tests/maintenance_tasks_tests.rs:450 at HEAD); later `890429f`/`0fb164d` | GIT | solid |
| irrevers-6de781f4 | Canary rollout via NEEDLE --identifier | 2026-08-15 05:16 | Mechanism git-verified (`90a9653` cites ID: `TrustPointer::for_channel`; round-trip via `c7e9df5`); the launched-`canary-icg`-worker half rests on a doc comment only (src/trust_pointer.rs:102) | GIT | **partial (T3)** |
| irrevers-b6579270 | Per-release deny-rate telemetry and rolling baseline | 2026-08-21 00:54 | `5b4d5df` 59 s before close (engine/state_store/release_telemetry tests) + `721f3d9` wires baseline/deviation to poison-pill rollback; tests at HEAD (tests/release_telemetry_tests.rs:19/:47) | GIT | solid |
| irrevers-eff8909f | Write inventory of shipped releases v0.1.0–v0.1.6 with dates and commits | 2026-09-10 10:30 | Tag table in its own notes, no carrying commit; substance re-verifies (all 7 tags resolve at recorded SHAs `f0fe556`…`aab687d`) but two claims were wrong/stale at close ("zero GitHub Releases" false for the mirror; range since stale) | SELF | **partial** (moved up from T2 — see reconciliation) |
| irrevers-2cb3dbd2 | Gate the actual modular release packs in icg-ci instead of static fixtures | 2026-08-26 04:03 | `6eaeb70` 10 s before close: release_gate_integrity_tests +284, icg_ci_integration_tests +110, template switched to real pack bytes; follow-up `b8aed51` | GIT | solid (caveat: prior-release-pack regression-gen sub-clause explicitly skipped at close) |
| irrevers-c87a3c50 | Base self-updater and trust pointer (icg update) | 2026-08-14 14:37 | Closed 1 s after creation, supersede-only close reason, no commit/test; repo verifiably held none of the described code at close (tip `200bb7e`; trust_pointer.rs first appears `97b8eba` ~9.5 h later); scope lives under successors 5fdc2e13/f59f9313 | NONE | **weak-unverifiable (T1)** — void rehydration bookkeeping |
| irrevers-5fdc2e13 | Trust pointer mechanism | 2026-08-15 03:04 | `97b8eba` 14 s before close: src/trust_pointer.rs +254 (atomic write-then-rename) + `icg trust` CLI; all six close-note tests at HEAD (default path agent-writable until `d1e2b38` next day — carried note) | GIT | solid |
| irrevers-f59f9313 | icg update: self-updater command | 2026-08-15 03:26 | Implementing pair `9c6951b`/`d1e2b38` landed ~21–54 min **after** close under unrelated subjects; close notes an unanchored completion claim; tests via `287b866`→`409ca42` | GIT | **partial (T3)** |
| irrevers-96594031 | Migrate the shipped trust-pointer and rule-pack artifact paths off agent-writable locations | 2026-08-26 01:39 | `d1e2b38` (trust pointer → root-owned /etc/icg/trust-pointer.json) + `a03a7e6` (state → /var/cache/icg); scoped check `verify_artifact_directory_security()` (src/trust_pointer.rs:127, `4d5c1a5`) | GIT | solid |
| irrevers-ca79d63a | Deploy the binary, rule-pack artifact and trust pointer outside the guarded agent's writable filesystem | 2026-08-23 01:16 | `a03a7e6` 2 min before close, matching close notes line for line (three runtime-state files → /var/cache/icg; /etc/icg placement via `d1e2b38`) | GIT | solid |
| irrevers-e00a5381 | icg-ci Argo WorkflowTemplate | 2026-08-15 02:02 | declarative-config `122623ae` 29 s before close; buildout `0f3f5faf`/`d11a6472`/`b2c2a0e5` — all four cross-repo SHAs re-resolved; template live on iad-ci | GIT | solid (cross-repo) |
| irrevers-075634b8 | Make icg update atomically deploy the complete modular production pack directory | 2026-08-26 02:55 | `409ca42` 10 s before close: src/update.rs +642 (modular archive, per-pack validation, traversal/symlink rejection, atomic swap w/ rollback); both e2e tests at HEAD (:330/:436) | GIT | solid |

## Section B — Fail-closed harness / policy enforcement behavior (21)

| ID | Title | Close (UTC) | Evidence summary | Class | Rating |
|---|---|---|---|---|---|
| irrevers-8d2d4a73 | Engine: unconditional fail-open on parse failure or exception | 2026-08-15 13:22 | `48acfc1` 14 s before close wraps every check path; both acceptance fixtures at HEAD (tests/fail_open_tests.rs:34/:46) | GIT | solid |
| irrevers-aab3854c | Design fail-closed transition state machine and graduation criteria | 2026-08-16 03:03 | `16f84c1` docs/design/fail-closed-transition.md +342 covering every criterion; docs-only, matching design-only scope | GIT | solid |
| irrevers-cd3f4c44 | Graduated fail-open to fail-closed policy for guard crashes | 2026-08-21 01:25 | `bb362fb` 16 s before close: src/fail_closed.rs (800-line durable PolicyStore) + tests +167; full chain `bb362fb`/`17971b7`/`3f0f00d`/`859e19e`; `DEFAULT_GRADUATION_THRESHOLD = 3` | GIT | solid |
| irrevers-8a24ad8d | Implement fail-open baseline and fail-closed enforcement modes | 2026-08-21 02:44 | `17971b7` 15 s before close wires PolicyMode into engine/health + fail_closed_runtime_tests +132 (acceptance matrix :240/:298) | GIT | solid |
| irrevers-019c36d3 | Implement fail-closed policy transition mechanism | 2026-08-21 03:08 | Close note names all four implementing commits and the exact verification commands; all resolve, `859e19e` 3 min before close | GIT | solid |
| irrevers-fffef435 | Integrate with poison-pill mechanism for automatic graduation | 2026-08-21 02:53 | `3f0f00d` 16 s before close (+213 −21; `operator_force_graduate_and_force_revert_are_durable` :214) + `bb362fb` read-only integration tests (:50/:109) | GIT | solid |
| irrevers-0f49129d | Poison-pill auto-rollback (original bundled) | 2026-08-15 03:11 | Closed the same second successor ff4f17da was created; **nothing under its own ID**; scope fully delivered via b6579270 (measurement) + ff4f17da (reaction) | GIT VIA | **partial (T3)** — bookkeeping close |
| irrevers-ff4f17da | Poison-pill auto-rollback: revert the trust pointer on a deny-rate spike | 2026-08-21 01:05 | `d653ade` 20 s before close: src/rollback.rs +371 (`check_and_rollback`) + runbook updates; five conservative-trigger tests at HEAD | GIT | solid |
| irrevers-3fc4bdde | Apply the documented ICG_DISABLED emergency bypass to hook and PATH-wrapper enforcement | 2026-08-26 02:19 | `20808e9` 18 s before close (+compile fix `9ec6848`): src/emergency_bypass.rs routed through Hook and argv[0] wrapper; `emergency_scenario_3_bypass_guard_with_disabled_flag` un-ignored (tests/emergency_response_tests.rs:68) | GIT | solid |
| irrevers-f891f555 | Make the fail-closed policy read path lock-free for guarded invocations | 2026-09-08 02:06 | `0a5faa9` cites the ID, 18 s before close: lock-free `PolicyStore::load`, graduation reconcile moved to `icg policy reconcile`; +170 −5 tests | GIT | solid |
| irrevers-edb5c4ca | Stop reconciling the fail-closed policy from the hook and wrapper paths | 2026-09-08 02:07 | Same changeset `0a5faa9` (closed 36 s later): both call-site comments at HEAD naming `icg policy reconcile`; reconcile only at src/main.rs:2539 | GIT | solid |
| irrevers-9eb4de16 | Add regression tests for a root-owned policy directory on the hook path | 2026-09-08 02:07 | Same changeset `0a5faa9` (1 s later): tests :328/:486; refinement `c6dcc3c` | GIT | solid (caveat: "fails without the fix" rests on the commit body's manual negative control, not a recorded pre-fix test) |
| irrevers-93baa29a | Verify icg policy status and reconcile as root on an installed host layout | 2026-09-08 03:21 | Entire evidence is 2026-09-07 manual root-run notes on the installed host; nothing in the repo can reproduce a host-level root run — trust-the-notes or re-run (expected shape for a verification-only bead, still unverifiable) | SELF | **weak-unverifiable (T2)** |
| irrevers-3e6c6fde | Hook and wrapper guarded paths take the fail-closed policy lock via crash recovery | 2026-09-08 05:37 | `0e669f2` cites the ID, ~2m51s before close: `StateStore::record_guard_crash` + `ICG_STATE_PATH` override; only reconcile converts to poison-pill; tests :406/:151; negative control in commit body | GIT | solid |
| irrevers-50077acb | Set explicit root-owned 0755 modes on the icg trust directories in argo-guarded-builder | 2026-09-08 05:42 | `c38b0cd` ~16 min before close, Dockerfile-only: root:root 0755 on /etc/icg{,/packs,/overrides} + /var/cache/icg; build-time assertion RUN fails the image build on any bad mode — the standing check | GIT | solid |
| irrevers-ffdc924b | Add guard health tracking and crash monitoring infrastructure | 2026-08-21 02:33 | `a525523` 13 s before close, +921 (health.rs, health_server.rs, metrics.rs, telemetry); every acceptance item traceable; six inline tests at HEAD | GIT | solid |
| irrevers-9007792b | Operational monitoring and alerting infrastructure | 2026-08-21 03:28 | `a750033` 10 s before close, +1268 (Grafana dashboard, Prometheus alerts/scrape, promtail, monitor loop); tests :470/:545, src/denial_log.rs:1153 | GIT | solid |
| irrevers-0d710c9a | Write activation documentation and operational runbooks | 2026-08-21 03:06 | `859e19e` 16 s before close, +2452 across ten docs, all present at HEAD | GIT | solid |
| irrevers-1517a263 | cargo test writes into the production denial log on an instrumented host | 2026-09-07 20:31 | `0c062e3` cites the ID, ~1m41s before close: `operational_log_path()` refuses the /var/cache/icg default for test-driven callers; denial_log_pollution_guard_tests +241 (:185) | GIT | solid |
| irrevers-0aa08f4e | Routine run_started lifecycle telemetry prints to stderr on every guarded invocation | 2026-09-08 02:10 | `c6dcc3c` cites the ID, 16 s before close: `start_run` silent on the healthy path; `a_recovered_crash_still_announces_itself_on_stderr` (:553); zero-byte-stderr negative control in commit body | GIT | solid |
| **irrevers-49dbb095** | Wire fail-open boundary around the hook predicate pipeline | 2026-09-11 17:31 | `b5b0f28` 66 min before close: `fail_open_boundary` (src/engine.rs:1200/:1229, wired :1185) collapses `InvalidInput`/panics to allow; `catch_unwind` around stdin read + evaluate_content/_batch (operator policy still honored); tests at HEAD engine.rs:4318/:4349, github_workflows_hook_integration_tests.rs:364, fail_open_tests.rs:34/:46 (13-test count precise at close; 19 at HEAD is post-close growth) | GIT | solid (caveat: commit does not cite the ID — linkage rests on the close note; commit bundles the excluded redirect-message scope, f0e0f9db) |

## Section C — CI gate / test-harness infrastructure (5)

| ID | Title | Close (UTC) | Evidence summary | Class | Rating |
|---|---|---|---|---|---|
| irrevers-b4b37bf0 | Layer 1: regression-suite CI gate | 2026-08-15 03:40 | No gate code existed at the close instant — parent closed as its scope split; children delivered same day (`0970190` src/regression.rs +614; declarative-config `0f3f5faf` build-failing step); gate re-confirmed live | GIT VIA | **partial (T3)** — bookkeeping close |
| irrevers-b0a453b2 | Layer 1: verify the regression-suite gate actually fails the build | 2026-08-15 13:33 | The mutation experiment exists in **no commit**; `e1aab5a` (2026-09-06) deliberately narrowed the exact behavior it verified (same flip now exits 0 with a reasoned skip) — re-anchor to the pinned-corpus invariants (regression_suite_scope_tests.rs:139/:83/:43) before relying on it | SELF | **weak-unverifiable (T2)** |
| irrevers-f61efd80 | Layer 1: coverage-diff CI gate | 2026-08-15 03:40 | `97b8eba` created coverage.rs + coverage-diff CLI + tests ~1 h before close, but the described report-format/justification scope landed ~9.6 h **after** close (`48d5a60`); CI wiring `d11a6472` | GIT | **partial (T3)** — close predates full described scope |
| irrevers-29a9131c | Layer 1: verify the coverage-diff gate actually blocks an unjustified change | 2026-08-15 13:48 | Block-then-pass experiment recorded only as prose (no hash, no test name); mitigated by independent re-reproduction 2026-09-10/11 and release_gate_integrity_tests pins; residual gap: a pure redirect-channel flip is not flagged by coverage-diff | SELF | **weak-unverifiable (T2)** |
| irrevers-ed77224f | End-to-end integration testing for icg-ci workflow | 2026-08-16 19:07 | `287b866` 17 s before close: icg_ci_integration_tests +397 (`icg_ci_release_candidate_runs_both_layer_one_gates_and_emits_layer_two_report` :78); template pinned by `ci_workflow_gates_actual_pack_bytes_not_fixtures` (:609) | GIT | solid (caveat: driving the actual Argo API is operational-only evidence, not an in-repo test) |

## Explicit weak / unverifiable-evidence section (11 of 42)

One-line why per bead, for downstream scrutiny (ratings per irrevers-fdc1cd74).

### Weak-unverifiable (5)

- **irrevers-c87a3c50** — closed 1.2 s after creation with a supersede-only
  close reason (no commit, no test); the repo verifiably held none of the
  described code at close (tip `200bb7e`). Void rehydration bookkeeping;
  reconcile scope against irrevers-5fdc2e13 / irrevers-f59f9313.
- **irrevers-340ae322** — close rests on a manual docker manifest read plus CI
  pod states; the pods are podGC-deleted and the registry 401s anonymous
  manifest requests, so neither half is re-checkable today — only git-side
  0.1.0 pins remain.
- **irrevers-93baa29a** — entire evidence is manual root-run notes on an
  installed host; nothing in the repo can reproduce a host-level root run —
  trust-the-notes or re-run (expected shape for a verification-only bead,
  still unverifiable).
- **irrevers-b0a453b2** — the mutation experiment exists in no commit, and
  `e1aab5a` deliberately changed the exact behavior it verified — unanchored
  **and** no longer the behavior at HEAD; re-anchor to the pinned-corpus
  invariants before relying on it.
- **irrevers-29a9131c** — block-then-pass experiment recorded only as prose
  (no hash, no test name in the close reason); mitigated by independent
  re-reproduction 2026-09-10/11 and release_gate_integrity_tests pins, but the
  closing record itself is unreproducible; residual redirect-channel-flip gap
  stands.

### Partial (6) — substance real, closure record defective

- **irrevers-eff8909f** — self-authored tag table with no anchor of its own and
  two known-wrong claims (zero-GitHub-Releases line wrong for the mirror;
  range stale); substance does re-verify (all 7 tags resolve at their recorded
  SHAs), which is the only reason this is not weak.
- **irrevers-f59f9313** — close predates its implementing commits
  `9c6951b`/`d1e2b38` by ~21–54 min; the closure record was an unanchored
  completion claim, verifiable only after the fact.
- **irrevers-0f49129d** — no commit or note under its own ID — closed the same
  second successor ff4f17da was created; bundled scope fully delivered via
  b6579270 + ff4f17da.
- **irrevers-b4b37bf0** — no gate code existed at the close instant; children
  delivered same day (`0970190`, declarative-config `0f3f5faf`) and both
  Layer-1 gates confirmed still enforcing.
- **irrevers-f61efd80** — the described report-format/justification scope
  landed ~9.6 h after close (`48d5a60`); the close-time anchor `97b8eba`
  covered only the coverage.rs/CLI half.
- **irrevers-6de781f4** — mechanism half git-verified (`90a9653`, round-trip
  via `c7e9df5`) but the launched-canary operational half rests on a doc
  comment (src/trust_pointer.rs:102) only.

### Solid with caveat (5) — not weak, listed for completeness

irrevers-84b36e47 (Argo run TTL-reaped), irrevers-2cb3dbd2 (sub-clause
explicitly skipped at close), irrevers-9eb4de16 (manual negative control),
irrevers-ed77224f (Argo-API-drive area operational-only),
irrevers-49dbb095 (record linkage + bundled scope).

### First-pass tier crosswalk

The first pass (`closing-evidence-flags.md`) used T1 (no evidence) / T2
(self-reported) / T3 (defective closure record). The mapping is 1:1 except one
move: T1 = c87a3c50; T2 = 340ae322, 93baa29a, b0a453b2, 29a9131c (eff8909f
moved to partial); T3 = f59f9313, 0f49129d, b4b37bf0, f61efd80, 6de781f4.

## Reconciliation against the prior compiled inventory (`final-inventory.md`, 41 beads)

1. **One new closure** — irrevers-49dbb095 (hook predicate fail-open
   boundary), added to Section B. Full extraction in second-pass §1; rated
   solid (caveat).
2. **One rating move** — irrevers-eff8909f: weak (T2) → **partial**, solely
   because its substance re-verifies from the repo (all seven tags resolve at
   their recorded SHAs). Every other bead's rating agrees across passes.
3. **Known defects in `final-inventory.md`, recorded by later passes and NOT
   edited in place** (it is the first pass's deliverable of record):
   - Its icg-ci template line citations (130/149/156) are stale — the
     declarative-config template evolved (deny-regression step reworded;
     coverage-justification mechanism added, `4951607b`); both gates
     re-verified present and enforcing (lines 170/191 at second-pass check
     time).
   - Its closing count paragraph does not sum ("1+5+5+26 = 41" sums to 37;
     "Confirmed solid (26 of 41)" is the clean-only count). Correct over its
     own entries: 11 flagged + 30 unflagged = 41. Per-bead entries unaffected.
4. **Nothing else changed** — zero `src/`/`tests/`/`containers/` drift between
   the extraction HEAD (`c4385ac`) and this compile (only the v0.1.44
   Cargo.toml/lock version bump), so every inherited citation remains valid
   verbatim.

## Compile-time checks (2026-09-12, HEAD `e669d14`)

- All 42 inventory beads re-confirmed `Closed` live via `bead list`.
- Tags v0.1.0–v0.1.6 resolve; 43 cited in-repo SHAs spot-checked resolvable
  (`git cat-file -e`), spanning every section plus both new-pass commits
  (`b5b0f28`, `d8b1d1a`); all four cross-repo declarative-config SHAs
  (`122623ae`, `0f3f5faf`, `d11a6472`, `b2c2a0e5`) resolve in
  `~/declarative-config`.
- `git diff c4385ac..HEAD -- src tests containers` is empty (Cargo.toml/lock
  version bump only) — inherited citations valid verbatim.
- `docs/plan/plan.md` untouched by this chain (last touch `d26a83a`,
  2026-09-11, the separate workflows-guard documentation bead irrevers-a39bdf35).

## Count check

42 beads = **36 GIT-VERIFIED** (incl. 2 via-successors/children) + **5
SELF-REPORTED** + **1 NO VERIFIABLE EVIDENCE**. Ratings: **31 solid** (26
clean + 5 with caveats) + **6 partial** + **5 weak-unverifiable** = **42**,
matching the three section tables row for row (A: 11/3/2, B: 19/1/1, C: 1/2/2).
Sections: 16 + 21 + 5 = 42.
