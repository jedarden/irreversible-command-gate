# Closing evidence — CI-gate / test-harness closed beads, batch 5/5

Evidence for all five beads in the "CI gate / test-harness infrastructure"
section of `docs/notes/closed-beads-release-verification-inventory.md`
(irrevers-c52de1f2, child of irrevers-622aae24). Method: `bead show <id>` for
each bead's own description/notes, cross-referenced with `git log --grep`,
`git log -S`, and per-file history in this repo; the two icg-ci wiring
commits live in declarative-config and were verified in a local
`~/declarative-config` checkout (fetched 2026-09-10). Gate behavior was
reproduced live against the current tree on 2026-09-10. Evidence gathering
only — no plan edits.

Audit pass (irrevers-58a52917, 2026-09-10): every commit SHA, close-timestamp
delta, test name, and line citation below was re-resolved against the repos
and beads; the five focused test suites were re-run green (27/27, exit 0) and
both live gate reproductions re-executed with identical results. Two
corrections came out of the audit and are applied in place: the
`48d5a60` test count (12 → 11 at that commit) and the
`regression_suite_scope_tests` name-to-line mapping.

All five beads were confirmed **Closed** at time of writing, so every entry
below is closing evidence, not a status correction. Timestamps are UTC unless
a `-0400` offset is shown; bead timestamps are UTC.

---

## irrevers-b4b37bf0 — Layer 1: regression-suite CI gate

**Verifiable, as a split-bookkeeping close.** The parent closed at
2026-08-15T03:40:26Z at the moment its scope was split: residual verification
spun out as irrevers-b0a453b2 (created 03:40:26.543Z, 43 ms before the
parent's close timestamp) on top of the already-existing children
irrevers-7684fa60 (suite generation) and irrevers-69594753 (icg-ci wiring).
At the close instant no regression-suite code existed — the actual gate
landed the same day through the children:

- `0970190` (2026-08-15 09:10:04 -0400 = 13:10:04Z, "feat(regression):
  generate fixed deny suite") added `src/regression.rs` (614 lines), the
  `regression-suite` CLI, and `tests/regression_suite_tests.rs` — **15
  seconds** before irrevers-7684fa60 closed (13:10:19Z).
- declarative-config `0f3f5faf` (09:27:45 -0400 = 13:27:45Z, "ci(icg): run
  deny regression suite") wired the suite into
  `k8s/iad-ci/argo-workflows/icg-ci-workflowtemplate.yml` as a build-failing
  step — **20 seconds** before irrevers-69594753 closed (13:28:05Z).

The gate still runs in CI today:
`containers/argo-guarded-builder/icg-ci-guarded-workflowtemplate.yml:149`
invokes `regression-suite` on the merged release pack ("This now gates the
ACTUAL packs being released, not static fixtures" — switched from static
fixtures by `6eaeb70`/declarative-config `083fd82e`, 2026-08-26).

## irrevers-b0a453b2 — Layer 1: verify the regression-suite gate actually fails the build

**Verifiable at close time; behavior has since been deliberately narrowed
(see caveat).** The bead's notes (closed 2026-08-15T13:33:59Z) record the
full mutation experiment in an isolated snapshot: baseline `cargo test` and
the exact icg-ci gate command green; weakening `vault-kv-destroy`'s redirect
channel from `deny` to `additional_context` made `cargo test` exit 101 and
the gate exit 1 with "fixed regression cases require a deny redirect";
restore → green again; `tests/fail_open_tests.rs` (malformed-stdin,
corrupt-rule-pack) confirmed included in the same `cargo test` run.

Live reproduction 2026-09-10, current tree, all on /tmp copies (shared
checkout untouched):

- Baseline: `regression-suite tests/fixtures/previous-release.json --output …`
  exits 0 and generates **8** deny-expected cases (5 command-mode, 3
  content-mode) — up from the 4 the notes recorded, after `e1aab5a`
  (2026-09-06, "fix(regression): make the documented per-pack suite work on
  every pack") expanded generation.
- The same deny → `additional_context` mutation on a /tmp copy of the fixture
  now exits **0** with a reasoned skip ("redirect channel is
  AdditionalContext, which never denies; covered by the engine tests for that
  channel instead"). `e1aab5a` deliberately stopped the command from being
  fatal for legitimately non-deny channels (rewrite/warning/predicate — 13 of
  26 shipped rules), and coverage-diff reports `no_regressions` for a pure
  channel flip, so **at the command level this specific weakening is no
  longer a gate failure**.
- Enforcement for the shipped packs moved into the repo's own test suite,
  which icg-ci also runs build-failing:
  `the_release_gate_corpus_is_unchanged` (pins the ten-case release-gate
  corpus by ID, `tests/regression_suite_scope_tests.rs:139`),
  `no_deny_regex_rule_is_ever_skipped` (:83), and
  `every_enabled_rule_is_either_a_case_or_a_reasoned_skip` (:43), plus
  `assert_denies_generated_cases` executing the suite against the engine
  (`tests/icg_ci_integration_tests.rs:43`). All green today: the five
  focused suites (`regression_suite_tests`, `regression_suite_scope_tests`,
  `coverage_diff_tests`, `release_gate_integrity_tests`,
  `icg_ci_integration_tests`) pass 27/27, exit 0.

The verification the bead describes was real and reproducible when performed;
the fail-on-weakening edge it exercised was consciously relaxed six weeks
later and replaced with the pinned-corpus invariant above.

## irrevers-f61efd80 — Layer 1: coverage-diff CI gate

**Verifiable, with a closure-timing caveat.** The parent closed
2026-08-15T04:05:57Z, ~1 hour after `97b8eba` (03:04:19Z — the same big-bang
commit that carried the trust pointer) created `src/coverage.rs`, the
`coverage-diff` CLI (binary long-about: "coverage-diff CI tool"), and
`tests/coverage_diff_tests.rs`, and ~2 minutes after child irrevers-55c1914e
closed. Caveat: the description's full scope — structured report format and
explicit justification field — did **not** exist at close (`git show
97b8eba:src/coverage.rs` contains zero justification references); it landed
~9.6 hours later in `48d5a60` (09:41:38 -0400 = 13:41:38Z, "feat(coverage):
add review report and justification tracking") with
`COVERAGE_DIFF_REPORT_FORMAT = "coverage-diff/v1"`,
`has_explicit_justification`, the Layer 2 report renderer, and 11 tests in
`tests/coverage_diff_tests.rs` (12 in the current tree after later
additions). CI wiring: declarative-config `d11a6472`
(09:44:29 -0400 = 13:44:29Z, "ci(icg): add coverage diff gate"). The only
commit in this repo whose message cites the bead is the follow-up naming
reconciliation `1fe1a56` ("docs(plan, notes): reconcile destructive_patterns
vs guarded_patterns naming") — the correction the bead's close notes
describe. The gate still runs in CI today: template line 156.

## irrevers-29a9131c — Layer 1: verify the coverage-diff gate actually blocks an unjustified change

**Verifiable.** The bead's notes (closed 2026-08-15T13:48:19Z, four minutes
after the `d11a6472` CI wiring) record the block-then-pass experiment against
`tests/fixtures/previous-release.json` →
`tests/fixtures/current-release-regression.json`: without `--justification`,
`status: regressions_detected` + `justification: REQUIRED`, exit 2; with an
explicit justification, the same report, exit 0.

Live reproduction 2026-09-10, current tree — identical results:

```
coverage-diff tests/fixtures/previous-release.json \
              tests/fixtures/current-release-regression.json
  → exit 2, status: regressions_detected,
    justification: REQUIRED: provide --justification with the release approval rationale
… --justification "Intentional rule-pack change approved for Layer 1 gate verification."
  → exit 0, status: regressions_detected (justified)
```

The gate's regression detection against *real packs* is pinned by
`tests/release_gate_integrity_tests.rs`:
`mutating_real_pack_causes_coverage_gate_to_fail` (removes `git-force-push`
from a copy of `packs/git.json`, asserts exit 2 + "justification: REQUIRED")
and `widening_safe_pattern_in_real_pack_causes_coverage_gate_to_fail` — both
green today. The two fixture inconsistencies the verification notes recorded
("expected 3 widened safe patterns but detector found 4"; "current-release-clean
itself widens safe-image-tag so the nominal clean comparison exits 2") were
resolved the next day by `c096a1e` (2026-08-16, "test: add comprehensive
coverage diff fixtures"); `test_detects_widened_safe_patterns` now asserts
exactly 4 and `test_no_regressions_clean_release` passes.

One residual scope note, recorded rather than smoothed over: a pure
redirect-channel flip (deny → additional_context) is **not** flagged by
coverage-diff — the model tracks removed/disabled/widened/narrowed patterns,
not channels (verified live: `no_regressions`, exit 0 against the mutated
copy described under irrevers-b0a453b2). Channel-weakening enforcement for
shipped packs rests on the pinned regression corpus instead.

## irrevers-ed77224f — End-to-end integration testing for icg-ci workflow

**Verifiable.** Implementing commit `287b866` (2026-08-16 15:07:24 -0400 =
19:07:24Z, "test: add icg-ci end-to-end integration coverage") added
`tests/icg_ci_integration_tests.rs` (397 lines) — **17 seconds** before the
bead closed (19:07:41Z). Mapping to the five areas the description listed:

- (2) regression-suite generation/validation and (3) coverage-diff with the
  Layer 1/2 process: `icg_ci_release_candidate_runs_both_layer_one_gates_and_emits_layer_two_report`
  (line 78 — runs both gates on a release candidate and asserts the generated
  deny cases actually deny via the engine, plus Layer 2 report emission).
- (4) complete self-update workflow: later hardened in-place by `409ca42`
  (2026-08-25) into
  `trusted_release_update_replaces_complete_pack_directory_and_preserves_enforcement`
  (line 330) and
  `malformed_release_archive_cannot_partially_deploy_or_escape_the_pack_root`
  (line 436).
- (5) trust-pointer updates × rule-pack deployment interaction: covered by the
  same trusted-release-update test (`write_trust_pointer` helper, line 308).
- (1) the actual icg-ci Argo WorkflowTemplate run: not driven through the
  Argo API by any test. What exists is
  `ci_workflow_gates_actual_pack_bytes_not_fixtures` (line 609, added
  `6eaeb70` 2026-08-26), which pins the shipped template to gating real pack
  bytes; the end-to-end proof at the workflow level is operational — the
  green four-asset GitHub releases v0.1.1+ produced by live icg-ci runs
  (see `release-verification-a.md`, irrevers-84b36e47).

All four integration tests pass in the current tree (2026-09-10 run above).

---

## Coverage cross-check (Part 2)

Union of the five evidence files against all 41 IDs in
`docs/notes/closed-beads-release-verification-inventory.md`, checked by
extracting `## irrevers-<id>` headers mechanically:

| Evidence file | IDs covered | Count |
|---|---|---|
| `release-verification-a.md` | 84b36e47, e77615c8, 37eb1100, 340ae322, e2bb8fbf, 6de781f4, b6579270, eff8909f | 8 |
| `release-verification-b.md` | 2cb3dbd2, c87a3c50, 5fdc2e13, f59f9313, 96594031, ca79d63a, e00a5381, 075634b8 | 8 |
| `fail-closed-harness-a.md` | 8d2d4a73, aab3854c, cd3f4c44, 8a24ad8d, 019c36d3, fffef435, 0f49129d, ff4f17da, 3fc4bdde, f891f555 | 10 |
| `fail-closed-harness-b.md` | edb5c4ca, 9eb4de16, 93baa29a, 3e6c6fde, 50077acb, ffdc924b, 9007792b, 0d710c9a, 1517a263, 0aa08f4e | 10 |
| `ci-gate-test-harness.md` (this file) | b4b37bf0, b0a453b2, f61efd80, 29a9131c, ed77224f | 5 |
| **Total** | | **41** |

**Result: complete.** The 41-ID union equals the enumeration exactly —
no missing ID, no duplicated ID, no ID covered twice across files. Every
enumerated bead is closed; none required a status correction.

Two cross-file observations worth carrying forward:

- The declarative-config commit SHA `4ace4c4b` cited in child bead
  irrevers-248fca69's close notes (coverage-diff CI wiring) exists in **no**
  repo — searched this repo and `~/declarative-config` across all refs after
  a fresh fetch. The note's described content and its +17 s close timing
  match declarative-config `d11a6472`
  (`d11a647251ff4ba98783333de72ba3f7c2eabe6f`), which is the actual wiring
  commit; the cited SHA appears to be a stale or mis-transcribed value, not
  lost work.
- Two evidence files record closes that predate their described work
  (irrevers-c87a3c50 in batch 2; irrevers-f61efd80 above, ~9.6 h). Both have
  the same shape: closed on a scope split or a work-in-progress boundary,
  with the real implementation verifiably landing the same day. Neither was
  reopened; both are documented here rather than silently treated as clean.
