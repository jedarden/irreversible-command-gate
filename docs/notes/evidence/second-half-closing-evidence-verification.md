# Closing-evidence verification — second half of the enumerated inventory beads

Working deliverable for **irrevers-25063548** (child 3 of the
irrevers-2c3b4637 auto-split; sibling of irrevers-98e9188e's first-half pass).
Verified 2026-09-12 at repo HEAD `a7d6541`.

**Scope** — the second half of the child-1 list in its recorded order
(`docs/notes/closed-beads-reconciliation-2026-09-12.md`): §B rows 6–21 plus
all of §C (21 beads), **plus irrevers-84b36e47 re-verified explicitly per the
task** (it is §A row 1; the first-half pass also carries it as its row 1 —
both rows stand, verified independently). `docs/plan/plan.md` untouched by
this pass.

**Method, per bead** — same as the first-half pass: close reason re-read from
the bead record and the last `closed` snapshot in
`.beads/checkpoint/forensic.jsonl`; every commit/test/doc/tag cited for it by
the existing inventories (`closed-beads-compiled-inventory.md` and, via it,
`docs/notes/evidence/final-inventory.md` and `second-pass-closing-evidence.md`
§1) checked against git history: `git cat-file -e` for resolution (cross-repo
SHAs in `~/declarative-config`), `git show --stat`/`--numstat` for scope,
commit-date recomputation for every close-instant timing claim (commit dates
are `-04:00`; converted to UTC for comparison), `git log --format=%B` /
subject greps for ID citations, `grep -n`/`sed -n` for HEAD test and code
anchors, introducer checks (`git show <sha> -- <file>` / `--diff-filter=A`)
for "who added this test/file" attributions, and per-file drift checks
(`git log <extraction-HEAD>..HEAD -- <file>`) before calling a moved anchor a
correction rather than drift. Live re-checks where the cited evidence is an
external object: `gh release view v0.1.1 --repo
jedarden/irreversible-command-gate` and `kubectl
--server=http://traefik-iad-ci:8001 get workflowtemplate icg-ci` plus the
declarative-config YAML.

**Drift base** — the second-pass extraction froze at `c4385ac` and the
compiled inventory at `e669d14`. `git diff a58aca7..HEAD -- src tests
containers Cargo.toml Cargo.lock` at this pass's HEAD is a Cargo.toml/lock
version bump only; per-file checks found no drift in `src/rollback.rs`,
`src/health.rs`, or `tests/github_workflows_hook_integration_tests.rs` since
`c4385ac`, and exactly one relevant drift commit (`df99df4`, matched_path
carry) for `src/denial_log.rs` — see correction 3.

**Result: 19 verified · 3 corrected (all minor, substance and every
classification unaffected) · 0 unverifiable.** All 29 newly-checked in-repo
SHAs and 2 cross-repo SHAs resolve; tag `v0.1.1` resolves at `a4f6e0c`. One
cited hash that does **not** resolve as a commit — `f0e0f9db` in the
irrevers-49dbb095 row — is in fact a **bead ID** (`irrevers-f0e0f9db`),
which exists and makes the citation true; recorded as a clarification, not an
unverifiable (details in row 17). No cited artifact is missing from this
repo.

---

## Explicitly included

### irrevers-84b36e47 — VERIFIED
Close reason (forensic 2026-09-06T13:06:27.29Z): icg-ci-manual-snxhj
Succeeded; v0.1.1 non-draft, 4 assets. Artifacts: live `gh release view
v0.1.1` → `isDraft=false`, `publishedAt=2026-09-06T13:06:02Z` — **25.3 s
before close**, exactly as cited — assets exactly `icg`, `icg-packs.tar.gz`,
`pack-manifest.json`, `rule-pack.json`; tag `v0.1.1` → `a4f6e0c` ✓.
Unblocking commits re-resolved with matching scopes: `3399989`
(tests/installation_tests.rs +11 −2), `c5d391b` (total +455 −67 incl.
src/pack_manifest.rs **+243** and tests/icg_ci_integration_tests.rs +86 −65),
`eec8e73` (tests/maintenance_tasks_tests.rs +20 −4). `9350f19` is **empty (0
files)** and its body reads "Related: bead irrevers-84b36e47" — "empty CI
trigger citing the ID" confirmed. Argo-run caveat stands (TTL-reaped); the
release object re-verifies live.

---

## Section B rows 6–21 — Fail-closed harness / policy enforcement

### 6. irrevers-fffef435 — VERIFIED
Close reason (2026-08-21T02:53:17.09Z): poison-pill integration, force
controls, coverage, "pushed commit 3f0f00d". Artifacts: `3f0f00d` =
2026-08-21T02:53:01Z — **16.1 s before close** ✓; `src/fail_closed.rs`
**+213 −21 exact**; that same commit added
`operator_force_graduate_and_force_revert_are_durable`, now at
tests/fail_closed_policy_tests.rs:**214** exact ✓; `bb362fb` (file creator,
+167) added the two read-only integration tests, now at :**50**
(`policy_reconciles_unique_clean_releases_and_graduates`) and :**109**
(`poison_pill_resets_open_policy_without_editing_telemetry`) — both exact ✓.

### 7. irrevers-0f49129d — VERIFIED (as partial / bookkeeping close)
Close reason (2026-08-15T03:11:18.03Z) is supersede-only and **verbatim
names the split**: "Superseded: split into irrevers-b6579270 (measurement)
and irrevers-ff4f17da (reaction)" ✓. "Closed the same second successor
ff4f17da was created" is **exact**: ff4f17da created_at
2026-08-15T03:11:18.003Z vs close 03:11:18.033Z — 30 ms apart. No commit
cites the ID (`git log --all --grep` = 0) — "nothing under its own ID"
confirmed. Successor anchors all resolve: `5b4d5df` / `721f3d9` (measurement;
first-half pass re-verified their content) and `d653ade` (reaction, row 8).
Classification partial/T3 stands.

### 8. irrevers-ff4f17da — CORRECTED (minor: test count / one anchor)
Close reason (2026-08-21T01:05:43.21Z): durable rollback, focused tests,
runbooks. Core evidence verifies: `d653ade` = 2026-08-21T01:05:23Z — **20.2 s
before close** ✓; `src/rollback.rs` **+371 −0 exact** with `check_and_rollback`
at src/rollback.rs:**95**; runbook updates real (docs/runbooks/rollback.md
+13 −5, docs/design/fail-closed-transition.md +4 −6).
**Correction:** final-inventory.md claims "five conservative-trigger tests
inline at HEAD (`:212`, `:294`, `:322`, `:341`, `:362`)". The file carries
**exactly four** `#[test]`s — fn lines :294 (`qualifying_fresh_release_
rolls_back_to_exact_prior_pointer`), :322, :341, :362 all exact — and the
fifth cited anchor **:212 is `fn release_is_fresh(...)`, the fresh-release
window helper, not a test**, at `d653ade`, `c4385ac`, and HEAD alike (zero
drift since `c4385ac`). Suggested wording: "four conservative-trigger tests
(:294/:322/:341/:362) plus the :212 fresh-release-window helper they
exercise". Substance (conservative triggers pinned inline at HEAD)
unchanged.

### 9. irrevers-3fc4bdde — VERIFIED
Close reason (2026-08-26T02:19:37.57Z): unified ICG_DISABLED bypass across
check/hook/PATH wrappers. Artifacts: `20808e9` = 2026-08-26T02:19:19Z —
**18.6 s before close** ✓ — adds `src/emergency_bypass.rs` **+78** (present
at HEAD), wires `src/main.rs` +84 −8, adds docs/runbooks/emergency-bypass.md
+22, tests +136 −8; compile fix `9ec6848` = 02:18:31Z — **1 m 06 s earlier** ✓
("1 min earlier"). `emergency_scenario_3_bypass_guard_with_disabled_flag` at
tests/emergency_response_tests.rs:**68** exact ✓ and the file contains no
`#[ignore]` — "un-ignored" confirmed.

### 10. irrevers-f891f555 — VERIFIED
Close reason (2026-09-08T02:06:43.49Z): "Landed in 0a5faa9… PolicyStore::load
was ALREADY lock-free at HEAD… adds the contract." Artifacts: `0a5faa9`
**subject cites the ID** ("fix(irrevers-f891f555, irrevers-edb5c4ca,
irrevers-9eb4de16)…") = 2026-09-08T02:06:25Z — **18.5 s before close** ✓;
tests/fail_closed_runtime_tests.rs **+170 −5 exact** ✓. Lock-free read path
re-confirmed at HEAD: `pub fn load` at src/fail_closed.rs:**622** with zero
`acquire_lock`/`.lock` references in its body (grep = 0).

### 11. irrevers-edb5c4ca — VERIFIED
Close reason (2026-09-08T02:07:01.96Z): same changeset `0a5faa9`; reconcile
deleted, not unwired. Artifacts: shared commit ✓ (subject cites this ID
too); close is **37.0 s after the commit** ("closed 36 s later" ✓ within a
second). Both call-site comments present at HEAD naming `icg policy
reconcile`: src/main.rs:**860** (hook front-end) and :**1263** (guarded
wrapper), plus the doc-comment at :925. Reconcile reachable only from the
operator CLI: `PolicySubcommand::Reconcile` arm at src/main.rs:**2539** ✓
(`reconcile_release_health` call at :2560).

### 12. irrevers-9eb4de16 — VERIFIED
Close reason (2026-09-08T02:07:02.10Z): same changeset `0a5faa9`; two named
tests. Artifacts: closed **0.14 s after** irrevers-edb5c4ca — "closed 1 s
later" ✓ at the records' second granularity (02:07:01 vs 02:07:02). Tests at
HEAD at the exact cited lines:
`hook_invocation_leaves_administrator_owned_policy_untouched` :**328**
(0555 policy dir, stderr asserted clean per close reason) and
`operator_policy_commands_manage_the_durable_policy` :**486** ✓. Refinement
`c6dcc3c` resolves (row 16). The recorded caveat — "fails without the fix"
rests on the commit body's manual negative control, not a recorded pre-fix
test — re-confirmed against the close reason; rating solid (caveat) stands.

### 13. irrevers-93baa29a — VERIFIED (as self-reported / weak-unverifiable)
Close reason (2026-09-08T03:21:52.55Z) is exactly the 2026-09-07 manual
root-run record: `policy status` exit 0, `policy reconcile` exit 0
idempotent, both directions exercised via documented overrides on the
installed host layout. No commit cites the ID (grep = 0) — "no commit was in
scope" confirmed. Nothing in the repo can reproduce a host-level root run;
trust-the-notes-or-rerun classification (weak-unverifiable, expected shape
for a verification-only bead) stands.

### 14. irrevers-3e6c6fde — VERIFIED
Close reason (2026-09-08T05:37:51.47Z): "Fixed in 0e669f2… StateStore::
record_guard_crash … ICG_STATE_PATH override … only icg policy reconcile
converts the counter." Artifacts: `0e669f2` **subject cites the ID** =
2026-09-08T05:35:00Z — **2 m 51.5 s before close** ("~2m51s" **exact**) ✓.
Both regression tests at HEAD at the exact cited lines:
`recovered_guard_crash_records_evidence_and_reconciles_into_policy` :**151**
and `recovered_guard_crash_keeps_administrator_owned_policy_untouched`
:**406** in tests/fail_closed_runtime_tests.rs ✓.

### 15. irrevers-50077acb — VERIFIED
Close reason (2026-09-08T05:42:49.94Z): 0755 modes pinned independent of the
kaniko umask, "committed to main (c38b0cd)". Artifacts: `c38b0cd` =
2026-09-08T05:26:43Z — **16 m 07 s before close** ("~16 min" ✓) —
**Dockerfile-only, +40 −2 exact** ✓. Content at HEAD: `chown -R root:root`
+ `chmod 0755` over `/etc/icg /etc/icg/packs /etc/icg/overrides
/var/cache/icg` (containers/argo-guarded-builder/Dockerfile:**57–59**) and
the build-time assertion `RUN for dir in …` loop at :**89** that fails the
image build on any bad mode — the standing check, present ✓.

### 16. irrevers-ffdc924b — VERIFIED (count note)
Close reason (2026-08-21T02:33:11.06Z): durable guard health tracking,
"Pushed commit a525523". Artifacts: `a525523` = 2026-08-21T02:32:58Z —
**13.1 s before close** ✓; total **+921 −82** exact ✓ across exactly the
claimed files: src/health.rs +583, src/health_server.rs +164, src/metrics.rs
+39, src/telemetry.rs +66, src/main.rs +69. All five named behaviors have
inline tests at HEAD in src/health.rs (19 `#[test]`s — same count at
`a525523` and `c4385ac`, zero drift): crash classification
(`crash_type_from_signal`/`classify_exit_status`), clean-exit clearing
(`mark_clean_exit`), run markers (`health_state_mark_start`), crash records
(`crash_record_creation`), metrics (`health_metrics_running_process`,
`health_metrics_crash_history`). **Note (not a correction):** the inventory's
"six inline tests still at HEAD" undercounts — health.rs alone has carried
19 since the implementing commit; every named behavior is present, so the
substance and rating stand.

### 17. irrevers-9007792b — CORRECTED (minor: one stale line anchor)
Close reason (2026-08-21T03:28:12.59Z): monitoring snapshots,
Prometheus/Grafana/Promtail, alerts, redaction. Core evidence verifies:
`a750033` = 2026-08-21T03:28:02Z — **10.6 s before close** ✓; total
**+1268 −40 exact** ✓ across 14 files including monitoring/grafana/
icg-overview.json, monitoring/prometheus/{alerts,scrape}.yml,
monitoring/promtail/config.yml, src/monitoring.rs +547, src/denial_log.rs
+62. Anchors: tests at src/monitoring.rs:**470**
(`collects_durable_inputs_and_emits_operational_metrics`) and :**545**
(`malformed_pack_is_visible_as_a_metric`) exact ✓; `collect_snapshot` at
src/monitoring.rs:**145** exact ✓; monitor loop `run_monitor` exists at
src/main.rs:**1337** (dispatched at :2836) ✓.
**Correction:** the third anchor, `src/denial_log.rs:1153`, is **blank at
every recorded HEAD** (`e6771f0`, `c4385ac`, HEAD). The object it evidently
targets — the redaction test matching the close reason's "denial logging
with redaction", `redacts_payloads_when_full_content_logging_is_disabled` —
sits at :**1158** at `e6771f0` and :**1157** at HEAD (`df99df4` shifted it
one line). Suggested wording: "redaction test at src/denial_log.rs:1157".
Substance unchanged.

### 18. irrevers-0d710c9a — VERIFIED
Close reason (2026-08-21T03:06:08.15Z): activation/operations guide
published, stale references corrected. Artifacts: `859e19e` =
2026-08-21T03:05:52Z — **16.2 s before close** ✓; **exactly ten files,
+2452 −30 exact** ✓, including docs/operators/fail-closed-mode.md +447,
docs/operators/training-manual.md +1222, docs/onboarding-guide.md +645, plus
the troubleshooting/deployment/incident-response/design updates — all
present at HEAD ✓.

### 19. irrevers-1517a263 — VERIFIED
Close reason (2026-09-07T20:31:16.66Z): test-driven processes refused the
live denial log; "Commit 0c062e3". Artifacts: `0c062e3` **subject cites the
ID** = 2026-09-07T20:29:35Z — **1 m 41.7 s before close** ("~1m41s"
**exact**) ✓; tests/denial_log_pollution_guard_tests.rs **+241 −0 exact** ✓;
`an_in_process_denial_never_reaches_the_live_log` at :**185** exact ✓;
`operational_log_path()` present at src/denial_log.rs:**885** ✓.

### 20. irrevers-0aa08f4e — VERIFIED
Close reason (2026-09-08T02:10:01.83Z): "Landed in c6dcc3c… dropped the
print, not gated behind a debug flag… nothing consumes icg_health_event
output." Artifacts: `c6dcc3c` **subject cites the ID** = 2026-09-08T02:09:45Z
— **16.8 s before close** ✓; the fault-emitter guard test
`a_recovered_crash_still_announces_itself_on_stderr` at
tests/fail_closed_runtime_tests.rs:**553** exact ✓.

### 21. irrevers-49dbb095 — VERIFIED (as solid-with-caveat; one clarification, one nit)
Close reason (2026-09-11T17:31:27.49Z): boundary complete as committed in
b5b0f28 "already on origin/main". Artifacts: `b5b0f28` = 2026-09-11T16:25:31Z
— **65 m 56 s before close** ("66 min" ✓). Boundary at HEAD at all three
cited lines: `fail_open_boundary` src/engine.rs:**1200**,
`input_source_from_pre_tool_use_fail_open` :**1229**, wired at :**1185** —
all exact ✓ (the close reason names the second function, consistent).
Tests: src/engine.rs:**4318** / :**4349** exact ✓ (both introduced by
`b5b0f28`); tests/github_workflows_hook_integration_tests.rs:**364**
(`hook_fails_open_on_unparseable_patch_input`, introduced by `d8b1d1a` =
2026-09-11T15:46:21Z, in by close) ✓; tests/fail_open_tests.rs:**34**/:**46**
✓. The "exactly 13 `#[test]` at close, 19 at HEAD" claim for the integration
file re-measures precisely (13 at `b5b0f28`, 19 at HEAD) ✓.
**Clarification:** the compiled inventory's "bundled scope, f0e0f9db" does
**not resolve as a commit** — it is **bead irrevers-f0e0f9db** ("Add
actionable redirect message and fail-open error handling for the
workflows-write guard"), which exists, is Closed, and whose own notes say
"Implemented in commit b5b0f28 (pushed to origin/main via merge `cda46c5`)"
— so the bundling claim is true, with f0e0f9db being a bead reference, not a
SHA. The commit indeed cites neither bead (grep = 0), and its subject/body
carry exactly f0e0f9db's redirect scope. **Nit:** second-pass §1 records
`d8b1d1a` at 15:46:31Z; actual is 15:46:21Z (10 s transcription slip;
in-by-close unaffected). Caveat (record linkage rests on the close note)
stands.

---

## Section C — CI gate / test-harness infrastructure

### 22. irrevers-b4b37bf0 — VERIFIED (as partial / bookkeeping close)
Close reason (2026-08-15T03:40:26.58Z) is supersede-only: description was its
two children's work; "Recreated narrowed to… proving the gate actually fails
a build" (= irrevers-b0a453b2). Artifacts: both named children exist —
irrevers-7684fa60 ("Implement regression suite generation from
guarded_patterns") and irrevers-69594753 ("Wire the regression suite into
icg-ci") ✓. "No gate code existed at the close instant": src/regression.rs
is introduced by `0970190` = 2026-08-15T13:10:04Z — **~9.5 h after** the
03:40:26Z close, same UTC day ✓ (`--diff-filter=A`); `0970190` adds
src/regression.rs **+614 exact** (+ tests/regression_suite_tests.rs +60) ✓.
declarative-config `0f3f5faf` = 13:27:45Z ("run deny regression suite")
resolves ✓. Live: both Layer-1 gates present in the icg-ci WorkflowTemplate
on iad-ci today; in the declarative-config YAML the regression-suite step
runs at line **170** and coverage-diff at **191** (final-inventory's "line
149/156" citations were already recorded stale by the second pass — not
re-flagged).

### 23. irrevers-b0a453b2 — VERIFIED (as self-reported / weak-unverifiable)
Close reason (2026-08-15T13:33:59.74Z) is the one-line mutation-experiment
record; it exists in **no commit** (nothing to anchor — confirmed by
absence) ✓. Recorded staleness re-verifies: `e1aab5a` =
2026-09-06T14:54:58Z ✓ ("2026-09-06"), and its body states the exact
narrowing — regressions reported in a "`skipped` array carrying the reason",
with `no_deny_regex_rule_is_ever_skipped` failing the build if a deny rule
is ever skipped. Re-anchor tests all exact at HEAD:
tests/regression_suite_scope_tests.rs :**139**
(`every_enabled_rule_is_either_a_case_or_a_reasoned_skip`), :**83**
(`no_deny_regex_rule_is_ever_skipped`), :**43**
(`the_release_gate_corpus_is_unchanged`), plus
tests/icg_ci_integration_tests.rs:**43** ✓. Weak-unverifiable rating stands.

### 24. irrevers-f61efd80 — CORRECTED (minor: both timing figures)
Close reason (2026-08-15T03:40:26.63Z) is supersede-only, same hollow-parent
shape as row 22; both named children exist (irrevers-55c1914e "Add
coverage-diff report format and justification tracking", irrevers-248fca69
"Wire the coverage-diff check into icg-ci") ✓. Substance verifies:
`97b8eba` = 2026-08-15T03:04:19Z created src/coverage.rs **+228**, the
coverage-diff CLI, tests/coverage_diff_tests.rs +112, and **exactly three
fixtures** (current-release-clean.json, current-release-regression.json,
previous-release.json) ✓; report-format/justification scope landed in
`48d5a60` ("add review report and justification tracking": coverage.rs
+154 −61, tests +96 −12) ✓; CI wiring declarative-config `d11a6472` =
13:44:29Z resolves ✓; the gate still runs live (row 22) ✓.
**Corrections (both in the same sentence of final-inventory.md, repeated in
the compiled inventory's weak section):** (1) "`97b8eba` … **~1 h** before
close" — actual gap to the 03:40:26.63Z close is **36 m 07 s**; (2) the
report-format scope "landed **~9.6 h** after close" — `48d5a60` (commit and
author date both 2026-08-15T13:41:38Z) is **10 h 01 m** after close. Same
direction, same conclusion (the close-time anchor covered only the
coverage.rs/CLI half; the described report-format scope postdates the
close), so the T3 classification is unaffected — but both figures should
read "36 min" and "~10 h".

### 25. irrevers-29a9131c — VERIFIED (as self-reported / weak-unverifiable)
Close reason (2026-08-15T13:48:19.48Z) is the prose-only block-then-pass
experiment (exit 2 `regressions_detected` without justification, exit 0
with) — no hash, no test name, by design ✓. The claimed mitigation artifact
exists: docs/notes/evidence/ci-gate-test-harness.md records the independent
live re-reproduction ("Live reproduction 2026-09-10, current tree —
identical results"; both gate reproductions re-executed) ✓. Pinned by
tests/release_gate_integrity_tests.rs (present at HEAD; +284 introduced by
`6eaeb70`, first-half row 9) and fixture fixes `c096a1e` (resolves,
2026-08-17T02:57:51Z, post-close refinement) ✓. The residual
redirect-channel-flip gap is a classification statement, unchanged;
weak-unverifiable rating stands.

### 26. irrevers-ed77224f — VERIFIED
Close reason (2026-08-16T19:07:41.97Z): e2e coverage added, "pushed commit
287b866". Artifacts: `287b866` = 2026-08-16T19:07:24Z — **17.98 s before
close** (cited "17 s"; recomputes to 18.0 s — sub-second rounding, matches
to within 1 s) ✓; tests/icg_ci_integration_tests.rs **+397 −0 exact** ✓;
`icg_ci_release_candidate_runs_both_layer_one_gates_and_emits_layer_two_
report` at :**78** exact ✓; `ci_workflow_gates_actual_pack_bytes_not_
fixtures` at :**609** exact ✓ (pin added later by `6eaeb70`); the
self-update hardening anchors :**330**/:**436** (from `409ca42`) exact ✓.
Caveat (driving the actual Argo API is operational-only evidence) stands;
live green v0.1.1 corroborates (row 1).

---

## Corrections found (3, all minor)

1. **irrevers-ff4f17da** — "five conservative-trigger tests inline at HEAD
   (`:212`, `:294`, `:322`, `:341`, `:362`)": the file has **four** tests
   (fn lines :294/:322/:341/:362 exact); :212 is the `release_is_fresh`
   window helper, not a test, at `d653ade`, `c4385ac`, and HEAD.
2. **irrevers-9007792b** — anchor `src/denial_log.rs:1153` is blank at every
   recorded HEAD; the intended redaction test is at :1157 (HEAD) / :1158
   (`e6771f0`).
3. **irrevers-f61efd80** — "~1 h before close" is actually **36 min**
   (`97b8eba`), and "~9.6 h after close" is actually **10 h 01 m**
   (`48d5a60`).

None of the three changes any classification, class, or rating; all are
recorded here rather than edited into the earlier docs (they are prior
passes' deliverables of record).

## Clarifications (not defects)

- **irrevers-49dbb095** — `f0e0f9db` in the compiled row is **bead
  irrevers-f0e0f9db**, not a commit hash; as a bead it exists (Closed) and
  its own notes claim the same commit `b5b0f28`, which is what makes the
  "bundled scope" caveat true. A reader greping it as a SHA finds nothing.
- **irrevers-ffdc924b** — "six inline tests" undercounts src/health.rs's 19
  (constant since the implementing commit); all named behaviors present.
- **irrevers-ed77224f** — cited "17 s" recomputes to 17.98 s.
- **irrevers-49dbb095 (nit)** — second-pass §1's `d8b1d1a` timestamp reads
  15:46:31Z; actual 15:46:21Z.

## Unverifiable items

**None.** Every artifact cited for these 22 beads resolves: 29 in-repo SHAs
(`3f0f00d`, `d653ade`, `20808e9`, `9ec6848`, `0a5faa9`, `0e669f2`, `c38b0cd`,
`a525523`, `a750033`, `859e19e`, `0c062e3`, `c6dcc3c`, `b5b0f28`, `d8b1d1a`,
`71c8ec7`, `cda46c5`, `0970190`, `e1aab5a`, `48d5a60`, `287b866`, `3399989`,
`c5d391b`, `eec8e73`, `9350f19`, `c096a1e`, `5b4d5df`, `721f3d9`, `bb362fb`,
`97b8eba`), 2 cross-repo SHAs (`0f3f5faf`, `d11a6472`), tag `v0.1.1` →
`a4f6e0c`, and the merge `cda46c5`. The three beads whose closing evidence
is by nature not re-checkable from git (93baa29a manual root run; b0a453b2 /
29a9131c notes-only experiments) were already classified
self-reported/weak-unverifiable by the inventories — this pass confirms
those classifications are accurate, which is a verification, not an
unverifiable finding.

## Roll-up

22 beads (21 scope rows + irrevers-84b36e47 explicitly): **19 verified,
3 corrected, 0 unverifiable.** Checks performed: 29 in-repo SHAs resolved +
scope-checked via numstat (12 stat claims exact to the line); 2 cross-repo
SHAs resolved; 1 tag resolved at its recorded SHA; the close-instant timing
claim recomputed for 17 rows (all match to the stated precision except the
two f61efd80 figures and the ed77224f sub-second rounding); 5 commit-subject
ID citations confirmed (`0a5faa9` names three IDs, `0e669f2`, `0c062e3`,
`c6dcc3c`) plus 1 commit-body citation (`9350f19`) and 1 confirmed-absent
(`b5b0f28`); 24 test/code anchors + 5 secondary anchors checked at HEAD (21
exact, 2 off by ≤5 lines → corrections 2–3, 1 helper-not-test → correction
1); 2 bead-existence checks (children), 4 bead-ID lookups (`f0e0f9db`,
`7684fa60`, `69594753`, `55c1914e`, `248fca69`); 3 live external objects
re-checked (GitHub release v0.1.1, icg-ci WorkflowTemplate on iad-ci,
declarative-config template lines 170/191).
