# Closing evidence — second enumeration pass (42 beads)

Deliverable for **irrevers-4bc81065** ("Extract closing evidence for each
enumerated release-verification bead"). Starts from the second enumeration
child's list — **irrevers-5dfe499e**, committed as `71c8ec7` in
`docs/notes/closed-beads-release-verification-inventory.md` (16
release-verification + 21 fail-closed-harness + 5 CI-gate = 42 IDs, anchor
irrevers-84b36e47 present). Written 2026-09-11 at repo HEAD `c4385ac`.

The second list is the first pass's 41 beads plus exactly one new closure,
**irrevers-49dbb095**. Per-bead detail for the 41 inherited entries (full
commit lists, test line numbers, verification method, flag rationale) lives in
`docs/notes/evidence/final-inventory.md` and its source files; this document
does not duplicate it. What this pass adds:

1. a fresh, full evidence extraction for irrevers-49dbb095 (section 1);
2. an independent re-verification of the inherited record at current HEAD
   (section 2), plus a per-bead one-line index over all 42 (section 3);
3. an explicit gap list (section 4).

## Sources used (and how each item below is sourced)

- **Close note** — the bead's own `close_reason`, taken from the last
  `closed` snapshot in `.beads/checkpoint/forensic.jsonl` (extracted for all
  42 IDs this pass; matches the enumeration doc's close dates row for row).
  Close dates are the forensic last-close instant, not `updated_at`: the
  2026-09-11T08:34Z evidence pass appended short-form summaries to every
  inventory bead's notes, mass-drifting `updated_at`.
- **Commit SHA** — verified resolvable at HEAD via `git cat-file -e` this
  pass (in-repo) and in `~/declarative-config` (cross-repo).
- **Test name** — verified present at HEAD by grep over `src/` and `tests/`
  this pass.
- **ID→commit link** — `git log --all --grep=<id>` per bead this pass.
- **Inherited classification/flag** — `final-inventory.md` + `closing-evidence-flags.md`,
  re-checked, not re-derived.

## 1. Fresh extraction — irrevers-49dbb095 (the one new closure)

**irrevers-49dbb095 — "Wire fail-open boundary around the hook predicate
pipeline"** · close **2026-09-11T17:31:27Z** · **GIT-VERIFIED · solid
(caveat)** — the caveat is record linkage, not missing evidence.

- *Close note* (source: forensic close_reason): claims completion as
  committed in `b5b0f28` "already on origin/main"; boundary =
  `Engine::input_source_from_pre_tool_use_fail_open` collapsing structured
  `InvalidInput` and panics to `None` (plain allow); `catch_unwind` around
  stdin read and `evaluate_content`/`evaluate_content_batch`, the latter two
  still honoring an operator fail-closed policy; tests re-run green.
- *Commit* (source: git): `b5b0f2880b3fec65c5970c3ccc4ccdea7bc59630`,
  authored 2026-09-11 16:25:31Z — **66 minutes before close** — on
  `origin/main` and an ancestor of HEAD. Diff: `src/engine.rs` +117,
  `src/main.rs` +12, `src/github_workflows.rs` +52, docs — exactly the files
  the close note claims, no test-infra or network code.
- *Boundary at HEAD* (source: grep): `fail_open_boundary` helper
  `src/engine.rs:1200`, `input_source_from_pre_tool_use_fail_open`
  `src/engine.rs:1229`, wired at `src/engine.rs:1185`.
- *Tests at HEAD* (source: grep):
  - `fail_open_boundary_turns_an_injected_pipeline_fault_into_an_allow` — src/engine.rs:4318 (introduced by `b5b0f28`)
  - `fail_open_conversion_allows_unparseable_patches_and_keeps_real_detections` — src/engine.rs:4349 (introduced by `b5b0f28`)
  - `hook_fails_open_on_unparseable_patch_input` — tests/github_workflows_hook_integration_tests.rs:364, e2e through the compiled binary (introduced by `d8b1d1a`, 15:46:31Z, also in by close)
  - `layer_one_malformed_stdin_fails_open` / `layer_one_corrupt_rule_pack_fails_open` — tests/fail_open_tests.rs:34/:46 (pre-existing cases re-run)
  - "all 13 github_workflows_hook_integration_tests": the file carries
    **exactly 13** `#[test]` at both `d8b1d1a` and `b5b0f28` — the claim is
    precise for close time. It is 19 at HEAD (six tests added after close);
    post-close growth, not a discrepancy.
- *Gaps*: (a) `b5b0f28` never cites the bead ID — `git log --grep` finds
  only the enumeration doc commit `71c8ec7` under this ID, so the
  bead↔commit link rests on the close note alone; (b) the commit is bundled:
  its subject and first half cover the workflows **redirect-message** scope
  that the enumeration deliberately excluded via irrevers-f0e0f9db
  ("superseded by 49dbb095"), so the commit serves two beads while
  back-referencing neither.

## 2. Re-verification of the inherited 41 at HEAD `c4385ac`

Method: every 7-char SHA cited in `final-inventory.md` resolved via
`git cat-file -e`; every cited test function name grepped at HEAD; every
cited `tests/*.rs` file confirmed present; all four cross-repo
declarative-config SHAs (`122623ae`, `0f3f5faf`, `d11a6472`, `b2c2a0e5`)
resolved there; tags v0.1.0–v0.1.6 re-confirmed resolving; all 42 beads
re-confirmed `Closed` live.

**Verdict: zero in-repo drift.** Nothing under `src/`, `tests/`, or
`containers/` has changed since the prior compile HEAD `e6771f0` (the only
intervening content commit, `827345d`, is the v0.1.43 version bump touching
Cargo.toml/README/docs). Every in-repo SHA, test name, test file, and test
line citation in the inherited record is therefore still valid verbatim.
50/50 cited in-repo SHAs resolve; 34/34 cited identifiers exist (25 as
functions/tests, 9 as `tests/*.rs` files).

Two cross-repo notes (declarative-config is a separate repo and has moved):

- The icg-ci template's regression gate step was **reworded**: the literal
  "deny-regression" no longer appears; the gate now reads "Generate and
  validate the fixed deny-must-still-fire suite"
  (`cargo run -- regression-suite packs --release-gate`, template line 170)
  and still fails the build on regression. The coverage-diff gate still runs
  (template line 191) and gained a recorded-justification mechanism
  (`packs/coverage-justifications.md`, declarative-config `4951607b`) plus
  auto-bump and `pack-manifest --verify` changes. **All template line-number
  citations in `final-inventory.md` (130/149/156) are stale**, though both
  gates demonstrably still exist and enforce.
- The docker digest prefix in the irrevers-340ae322 entry is a registry
  digest, not a commit — not resolvable by design (already flagged T2).

One defect found in the inherited document itself (corrected here, not
edited in place): `final-inventory.md`'s closing count paragraph is
arithmetically wrong — "1 (T1) + 5 (T2) + 5 (T3) + 26 solid = 41" sums to
37, and "Confirmed solid (26 of 41)" mislabels the clean-only count. The
correct roll-up over its own per-bead entries is **11 flagged (1+5+5) + 30
unflagged (26 clean + 4 solid-with-caveat) = 41**. The per-bead entries
themselves are internally consistent; only the summary arithmetic slipped.

## 3. Per-bead index (all 42)

Classification: GIT = git-verified (VIA = via successors/children),
SELF = self-reported only, NONE = no verifiable evidence. Flag: T1/T2/T3 or
solid / solid (caveat). "Detail" = which document carries the full entry
(FI = `final-inventory.md` §; this doc §1 for the new bead).

| ID | Close (UTC) | Primary anchor (source: close note + git/test) | Class | Flag | Detail |
|---|---|---|---|---|---|
| irrevers-84b36e47 | 09-06 13:06 | release v0.1.1 non-draft, 4 assets, published 25s before close (close note + `gh release`); unblocks `3399989`/`c5d391b`/`eec8e73`, trigger `9350f19` | GIT | solid (caveat: Argo run TTL-reaped) | FI·A |
| irrevers-e77615c8 | 08-24 04:27 | `2c541ec` build-pack + upload; `c2fcfd9` 30s before close (close note names it; commit cites ID); `rule-pack.json` asset since v0.1.1 | GIT | solid | FI·A |
| irrevers-37eb1100 | 08-15 13:57 | `436bdce` docs/runbooks/release-cutting.md +108 (close note names it) | GIT | solid | FI·A |
| irrevers-340ae322 | 08-30 01:56 | manual `docker manifest inspect` + pod states — unrecheckable (podGC, registry 401); git-side 0.1.0 pins only | SELF | T2 | FI·A |
| irrevers-e2bb8fbf | 08-30 12:19 | `c7e9df5`; test `maintenance_scenario_trust_channel_roundtrip` (tests/maintenance_tasks_tests.rs) | GIT | solid | FI·A |
| irrevers-6de781f4 | 08-15 05:16 | `90a9653` (commit cites ID) mechanism; launched-canary half undocumented | GIT | T3 | FI·A |
| irrevers-b6579270 | 08-21 00:54 | `5b4d5df` 59s before close + `721f3d9`; tests `engine_persists_per_release_evaluation_and_deny_counts`, `engine_telemetry_feeds_poison_pill_rollback` (tests/release_telemetry_tests.rs) | GIT | solid | FI·A |
| irrevers-eff8909f | 09-10 10:30 | tag table in bead notes; content corroborated (all 7 tags re-verified this pass) | SELF | T2 | FI·A |
| irrevers-2cb3dbd2 | 08-26 04:03 | `6eaeb70` 10s before close: release_gate_integrity_tests +284, icg_ci_integration_tests +110, template→real packs | GIT | solid (caveat: sub-clause skipped) | FI·A |
| irrevers-c87a3c50 | 08-14 14:37 | none — closed 1s after creation, rehydration artifact (close reason itself says "Superseded: split into icg-2usc/icg-26le") | NONE | T1 | FI·A |
| irrevers-5fdc2e13 | 08-15 03:04 | `97b8eba` 14s before close: src/trust_pointer.rs +254 | GIT | solid | FI·A |
| irrevers-f59f9313 | 08-15 03:26 | `9c6951b` + `d1e2b38` landed 21–54 min **after** close; tests later `287b866`→`409ca42` | GIT | T3 | FI·A |
| irrevers-96594031 | 08-26 01:39 | `d1e2b38` + `a03a7e6` (close note names both); `verify_artifact_directory_security` (`4d5c1a5`) | GIT | solid | FI·A |
| irrevers-ca79d63a | 08-23 01:16 | `a03a7e6` 2 min before close: runtime state → /var/cache/icg | GIT | solid | FI·A |
| irrevers-e00a5381 | 08-15 02:02 | declarative-config `122623ae` 29s before close; `0f3f5faf`/`d11a6472`/`b2c2a0e5` (all 4 re-resolved this pass) | GIT | solid (cross-repo) | FI·A |
| irrevers-075634b8 | 08-26 02:55 | `409ca42` 10s before close: src/update.rs +642; tests `trusted_release_update_replaces_complete_pack_directory_and_preserves_enforcement`, `malformed_release_archive_cannot_partially_deploy_or_escape_the_pack_root` | GIT | solid | FI·A |
| irrevers-8d2d4a73 | 08-15 13:22 | `48acfc1` 14s before close; `layer_one_malformed_stdin_fails_open`, `layer_one_corrupt_rule_pack_fails_open` (tests/fail_open_tests.rs:34/:46) | GIT | solid | FI·B |
| irrevers-aab3854c | 08-16 03:03 | `16f84c1` docs/design/fail-closed-transition.md +342 (docs-only, matches scope) | GIT | solid | FI·B |
| irrevers-cd3f4c44 | 08-21 01:25 | `bb362fb` 16s before close: src/fail_closed.rs + tests +167; chain `bb362fb`/`17971b7`/`3f0f00d`/`859e19e`; `DEFAULT_GRADUATION_THRESHOLD = 3` | GIT | solid | FI·B |
| irrevers-8a24ad8d | 08-21 02:44 | `17971b7` 15s before close; `recovered_guard_crash_denies_in_fail_closed_mode`, `lifecycle_reports_recovered_crash_once` (tests/fail_closed_runtime_tests.rs) | GIT | solid | FI·B |
| irrevers-019c36d3 | 08-21 03:08 | close note names all four commits + verification commands (`bb362fb`/`17971b7`/`3f0f00d`/`859e19e`) | GIT | solid | FI·B |
| irrevers-fffef435 | 08-21 02:53 | `3f0f00d` 16s before close; `operator_force_graduate_and_force_revert_are_durable`; plus `bb362fb` tests | GIT | solid | FI·B |
| irrevers-0f49129d | 08-15 03:11 | none under own ID — split close; delivered via b6579270 + ff4f17da | GIT VIA | T3 | FI·B |
| irrevers-ff4f17da | 08-21 01:05 | `d653ade` 20s before close: src/rollback.rs +371; five trigger tests | GIT | solid | FI·B |
| irrevers-3fc4bdde | 08-26 02:19 | `20808e9` 18s before close (+`9ec6848`); `emergency_scenario_3_bypass_guard_with_disabled_flag` (tests/emergency_response_tests.rs) | GIT | solid | FI·B |
| irrevers-f891f555 | 09-08 02:06 | `0a5faa9` 18s before close (commit cites ID); `hook_invocation_leaves_administrator_owned_policy_untouched`, `operator_policy_commands_manage_the_durable_policy` | GIT | solid | FI·B |
| irrevers-edb5c4ca | 09-08 02:07 | same changeset `0a5faa9` (commit cites ID) | GIT | solid | FI·B |
| irrevers-9eb4de16 | 09-08 02:07 | same changeset `0a5faa9` (commit cites ID); tests :328/:486; `c6dcc3c` refinement | GIT | solid (caveat: negative control manual) | FI·B |
| irrevers-93baa29a | 09-08 03:21 | manual root-run notes only (verification-only bead, expected shape) | SELF | T2 | FI·B |
| irrevers-3e6c6fde | 09-08 05:37 | `0e669f2` ~2m51s before close (commit cites ID); tests :406/:151; negative control in commit body | GIT | solid | FI·B |
| irrevers-50077acb | 09-08 05:42 | `c38b0cd` ~16 min before close: Dockerfile 0755 modes + build-time assertion RUN | GIT | solid | FI·B |
| irrevers-ffdc924b | 08-21 02:33 | `a525523` 13s before close +921; six inline tests at HEAD | GIT | solid | FI·B |
| irrevers-9007792b | 08-21 03:28 | `a750033` 10s before close +1268; tests :470/:545, src/denial_log.rs:1153 | GIT | solid | FI·B |
| irrevers-0d710c9a | 08-21 03:06 | `859e19e` 16s before close +2452, ten docs at HEAD | GIT | solid | FI·B |
| irrevers-1517a263 | 09-07 20:31 | `0c062e3` ~1m41s before close (commit cites ID); tests/denial_log_pollution_guard_tests.rs +241 (`an_in_process_denial_never_reaches_the_live_log`) | GIT | solid | FI·B |
| irrevers-0aa08f4e | 09-08 02:10 | `c6dcc3c` 16s before close (commit cites ID); `a_recovered_crash_still_announces_itself_on_stderr` (tests/fail_closed_runtime_tests.rs:553) | GIT | solid | FI·B |
| **irrevers-49dbb095** | **09-11 17:31** | `b5b0f28` 66 min before close (link via close note only — commit does not cite ID); `input_source_from_pre_tool_use_fail_open` src/engine.rs:1229; tests src/engine.rs:4318/:4349, tests/github_workflows_hook_integration_tests.rs:364, tests/fail_open_tests.rs:34/:46 | GIT | solid (caveat: linkage + bundled scope) | §1 here |
| irrevers-b4b37bf0 | 08-15 03:40 | none at close instant — split close; children `0970190` + declarative-config `0f3f5faf` same day; gate re-confirmed live this pass (template line 170) | GIT VIA | T3 | FI·C |
| irrevers-b0a453b2 | 08-15 13:33 | notes-only mutation experiment; `e1aab5a` later superseded the exact behavior | SELF | T2 | FI·C |
| irrevers-f61efd80 | 08-15 03:40 | `97b8eba` ~1h before close; described scope `48d5a60` ~9.6h **after** close; declarative-config `d11a6472` | GIT | T3 | FI·C |
| irrevers-29a9131c | 08-15 13:48 | notes-only block-then-pass experiment; independently re-reproduced 09-10/11; pinned by release_gate_integrity_tests | SELF | T2 | FI·C |
| irrevers-ed77224f | 08-16 19:07 | `287b866` 17s before close: tests/icg_ci_integration_tests.rs +397 (`icg_ci_release_candidate_runs_both_layer_one_gates_and_emits_layer_two_report`) | GIT | solid (caveat: Argo-drive is operational evidence) | FI·C |

## 4. Gaps and caveats (explicit, none omitted)

Inherited (unchanged from `closing-evidence-flags.md`, re-checked this pass):

- **T1 (no verifiable evidence):** irrevers-c87a3c50 — 1-second
  rehydration-artifact close; scope lives under irrevers-5fdc2e13 /
  irrevers-f59f9313.
- **T2 (self-reported only):** irrevers-340ae322, irrevers-eff8909f,
  irrevers-93baa29a, irrevers-b0a453b2, irrevers-29a9131c.
- **T3 (work verified, closure record defective):** irrevers-0f49129d,
  irrevers-b4b37bf0, irrevers-f59f9313, irrevers-f61efd80, irrevers-6de781f4.

New this pass:

- **irrevers-49dbb095** — closure record accurate and commit-anchored, but
  the implementing commit does not cite the bead ID, and the same commit
  also carries the redirect-message scope the enumeration excluded
  (irrevers-f0e0f9db). Recorded as solid (caveat).
- **Cross-repo staleness** — `final-inventory.md`'s icg-ci template line
  numbers (130/149/156) no longer match the evolved template; both gates
  verified still present and enforcing (lines 170/191 at check time).
- **Inherited-doc arithmetic** — the count-check paragraph in
  `final-inventory.md` does not sum (37 ≠ 41; "26 of 41" is the clean-only
  count). Corrected roll-up in §2 above. Per-bead entries unaffected.

## Count check (this pass)

42 beads = **36 GIT-VERIFIED** (incl. 2 via-successors/children) + **5
SELF-REPORTED** + **1 NO VERIFIABLE EVIDENCE**. Flags: **1 (T1) + 5 (T2) +
5 (T3) + 31 solid (26 clean + 5 with caveats: 84b36e47, 2cb3dbd2, 9eb4de16,
ed77224f, 49dbb095) = 42.** `docs/plan/plan.md` untouched by this pass.
