# Consolidated closing-evidence summary — all 41 inventory beads

Final child of irrevers-c52de1f2. Merges the self-reported close-evidence
extraction (irrevers-724f465d, bead notes) with the per-category git-evidence
passes (irrevers-2f2c25b5 → `release-verification-{a,b}.md`,
irrevers-f574a666 → `fail-closed-harness-{a,b}.md`, irrevers-2e6ddeb3 →
`ci-gate-test-harness.md`) into one summary covering every bead in
`docs/notes/closed-beads-release-verification-inventory.md`. Written
2026-09-11 at HEAD `16f0d8c`. Evidence gathering only — no plan edits.

Ground truth at consolidation time: all 41 inventory beads confirmed
**Closed** via `bead show`; every commit hash cited below re-resolves with
`git cat-file -e` at HEAD; and `git diff --stat a59a4a7..HEAD` over
`src/ tests/ containers/ packs/` was empty at consolidation. Re-checked
2026-09-11 at HEAD (post-v0.1.36): that diff is no longer empty — the
post-consolidation `.github/workflows` hook-guard work touched
`src/{denial_log,documented_commands,engine,github_workflows,main}.rs`
and six `tests/` files (11 files, +1393 −239) — so line anchors inside
those eleven files are as measured at the consolidation tree (`16f0d8c`)
and are not re-verified against HEAD; every other cited `src/`/`tests/`
file is unchanged since `a59a4a7`, and `containers/`/`packs/` are
untouched, so the workflowtemplate line anchors and pack fixtures still
hold. Per-bead
detail lives in the four per-category files; this file is the one-summary-
per-bead roll-up. The same summary (short form) is appended to each
inventory bead's own notes, attributed to irrevers-e29e74b6.

**PRs: none exist anywhere in the inventory** — this repo works directly on
`main` and the icg-ci template lives in declarative-config, so commits are
the unit of record. PR-based verification is not applicable to any entry.

## Classification legend

- **GIT-VERIFIED AT HEAD** — implementing/verifying commits exist, resolve
  at HEAD, and their content matches the bead's scope; test files named
  where the scope produced them. Sub-shape **GIT-VERIFIED VIA SUCCESSORS**
  marks a split/bookkeeping close whose scope is delivered and verifiable
  under other bead IDs named in the entry.
- **SELF-REPORTED ONLY** — the bead's own notes carry the closing evidence
  (manual verification or an evidence-only deliverable) and no commit
  implements or records it. Not a pejorative: verification-only beads have
  this shape by design, but the evidence is not independently re-checkable
  from git.
- **NO VERIFIABLE EVIDENCE FOUND** — collected in the explicit section at
  the end; nothing is omitted.

---

## Section A — Release verification / release process / distribution integrity (16 beads)

### irrevers-84b36e47 — Verify icg-ci produces a real, complete GitHub release
**GIT-VERIFIED AT HEAD.** Release v0.1.1 published 2026-09-06T13:06:02Z — 25 s
before close — non-draft with all four assets (`icg`, `icg-packs.tar.gz`,
`pack-manifest.json`, `rule-pack.json`); reproduces on every later release.
Unblocking commits `3399989` (`tests/installation_tests.rs`), `c5d391b`
(`tests/icg_ci_integration_tests.rs` + `packs/{docker,git,misc,openbao}.json`),
`eec8e73` (`tests/maintenance_tasks_tests.rs`); the only ID-citing commit
`9350f19` is an empty CI-trigger commit. Caveat: the green run's Argo
Workflow object is TTL-reaped — the surviving GitHub release object is the
evidence. (The bead's own notes were mid-progress only; this classification
comes from the git/live passes, not self-report.)

### irrevers-e77615c8 — icg-ci: publish the rule-pack artifact as a release asset
**GIT-VERIFIED AT HEAD.** `2c541ec` (2026-08-22) added the `build-pack`
command (`src/main.rs`, `src/rule_pack.rs`) and the release-upload step in
`icg-ci-guarded-workflowtemplate.yml`; `c2fcfd9` (~30 s before close,
`src/rule_pack.rs` only). `rule-pack.json` verified as a live release asset
from v0.1.1 onward and consumed by the coverage-diff gate (template line
130). No test files touched. Bead notes empty.

### irrevers-37eb1100 — Release-cutting runbook
**GIT-VERIFIED AT HEAD.** `436bdce` (2026-08-15) added
`docs/runbooks/release-cutting.md` (+108) linked from
`docs/notes/self-update-and-release-gating.md`, exactly matching the close
notes; maintained since (revised by release commit `1b6f6a6`). Docs only, no
test files. Close notes cited no hash; the path's first commit confirms.

### irrevers-340ae322 — Prove argo-guarded-builder:0.1.0 is published and pullable by icg-ci
**SELF-REPORTED ONLY.** Close notes record the manual verification: `docker
manifest inspect ronaldraygun/argo-guarded-builder:0.1.0` succeeded (digest
`sha256:dd3a46c3…98ef5`) and workflow `icg-ci-rg9n7`'s compatibility pods
reached Running on the pinned image; the pod check is not re-verifiable
(podGC OnPodCompletion). No commit references the publish/pull claim itself —
it is a registry/cluster event, not a commit. Git-side corroboration:
`containers/argo-guarded-builder/VERSION` and both `image:` pins still read
0.1.0 at HEAD, and later audit passes re-observed fresh icg-ci pods pulling
0.1.0 (e.g. `icg-ci-czdkx`, 2026-09-11). The registry now 401s anonymous
manifest requests.

### irrevers-e2bb8fbf — Add --channel to icg trust (canary rollout)
**GIT-VERIFIED AT HEAD.** `c7e9df5` is the exact close-note claim:
`src/main.rs` (+2 help hint) and `tests/maintenance_tasks_tests.rs` (+134)
adding `maintenance_scenario_trust_channel_roundtrip` (line 450 at HEAD,
covering trust-set → update round-trip plus channel isolation); built on
pre-existing coverage (`maintenance_scenario_trust_channel_support` :404,
inline `src/trust_pointer.rs` channel tests, `update_channels_get_isolated_
default_paths` `src/update.rs:967`). Later touches of the same surface:
`890429f`, `0fb164d`.

### irrevers-6de781f4 — Canary rollout via NEEDLE --identifier
**GIT-VERIFIED AT HEAD — mechanism only; operational half unverified.**
`90a9653` (2026-08-15, same day as close) added channel support across
`src/trust_pointer.rs` (+80, `TrustPointer::for_channel`, inline
`test_for_channel_path`/`test_channel_isolation`), `src/update.rs`, and
`src/main.rs`; the end-to-end round-trip arrived later via irrevers-e2bb8fbf
(`c7e9df5`). The operational half — an actually launched `canary-icg` NEEDLE
worker — has **no verifiable evidence**: only a doc comment survives
(`src/trust_pointer.rs:102`). Bead notes empty; linkage by date/scope match.

### irrevers-b6579270 — Per-release deny-rate telemetry and rolling baseline
**GIT-VERIFIED AT HEAD.** `5b4d5df` (59 s before close; `src/engine.rs`,
`src/state_store.rs`, `tests/release_telemetry_tests.rs`) + `721f3d9`
(baseline/deviation wired to poison-pill rollback, `src/main.rs`). Tests at
HEAD: `engine_persists_per_release_evaluation_and_deny_counts` (:19),
`engine_telemetry_feeds_poison_pill_rollback` (:47). Bead notes empty;
commits located by keyword, not ID.

### irrevers-eff8909f — Inventory of shipped releases v0.1.0–v0.1.6
**SELF-REPORTED ONLY (evidence-only bead, corroborated).** The deliverable
is the tag table in the bead's own notes — no commits carry it (ID and
keyword greps: no hits). Its content independently verifies: all seven tags
exist as lightweight tags at the recorded SHAs (v0.1.0 `f0fe556` … v0.1.6
`aab687d`). Nuance for reconciliation: the "zero GitHub Releases" line was
true only against origin (Forgejo); the GitHub mirror has contiguous
releases from v0.1.1 with the ceiling now far past the bead's stated range
(v0.1.22 Latest at the irrevers-2f2c25b5 pass; re-checked 2026-09-11 —
v0.1.36 Latest, 30 releases, all four assets present on every one).

### irrevers-2cb3dbd2 — Gate the actual modular release packs in icg-ci
**GIT-VERIFIED AT HEAD.** `6eaeb70` landed 10 s before close:
`tests/release_gate_integrity_tests.rs` (+284, new —
`mutating_real_pack_causes_coverage_gate_to_fail`,
`widening_safe_pattern_in_real_pack_causes_coverage_gate_to_fail`,
`pack_manifest_provides_cryptographic_verification`,
`mutating_pack_after_manifest_causes_verification_failure`),
`tests/icg_ci_integration_tests.rs` (+110), and the template switch to real
pack bytes — the "gates the ACTUAL packs" comment still stands
(`icg-ci-guarded-workflowtemplate.yml`, commands at lines 117/121/156/162/
202–224). Follow-up `b8aed51` (+61 −80). Partial sub-clause: deny-regression
generation over prior-release packs is skipped (runs on current packs). The
bead's own notes were a handoff, not a completion claim.

### irrevers-c87a3c50 — Base self-updater and trust pointer (icg update)
**NO VERIFIABLE EVIDENCE FOUND** — see the dedicated section below.

### irrevers-5fdc2e13 — Trust pointer mechanism
**GIT-VERIFIED AT HEAD.** `97b8eba` landed 14 s before close:
`src/trust_pointer.rs` (+254; `TrustPointer`, atomic write-then-rename
`TrustPointerStore`) + the `icg trust` CLI; the six close-note tests are
still at HEAD (`test_trust_pointer_create`,
`test_trust_pointer_with_justification`, `test_store_save_and_load`,
`test_store_get_trusted_ref`, `test_store_is_trusted`,
`test_atomic_write`). Carried caveat: the shipped default path was then
agent-writable — fixed next day by `d1e2b38` (irrevers-96594031's scope).
The same commit bundled the coverage-diff origin (`src/coverage.rs`,
`tests/coverage_diff_tests.rs`, three fixtures — irrevers-f61efd80's).

### irrevers-f59f9313 — icg update: self-updater command
**GIT-VERIFIED AT HEAD, with a late-landing caveat.** The implementing pair
landed ~21–54 min **after** close, bundled under unrelated subjects:
`9c6951b` (`Commands::Update` in `src/main.rs`) and `d1e2b38` (`src/update.rs`
+317 — one-shot GitHub Releases check, artifact download per trust pointer,
atomic write-then-rename; also flipped the trust-pointer default to
`/etc/icg/`). Tests followed in `287b866`
(`tests/icg_ci_integration_tests.rs`), later superseded by `409ca42`. The
bead's notes were an unanchored completion claim (no hash, no test names) —
the git pass supplies the anchors.

### irrevers-96594031 — Migrate trust-pointer/rule-pack paths off agent-writable locations
**GIT-VERIFIED AT HEAD.** Both close-note commits verify: `d1e2b38` (trust
pointer → root-owned `/etc/icg/trust-pointer.json`) and `a03a7e6` (2026-08-22,
"use root-owned system paths": `denial_log.rs`/`health.rs`/`state_store.rs`
→ `/var/cache/icg/`, `dirs` dependency removed, Cargo +8 −123 across both
files). The scoped check exists as described:
`verify_artifact_directory_security()` (`src/trust_pointer.rs:127`,
introduced `4d5c1a5`); `tests/installation_tests.rs` exercises the installed
layout. Neither implementing commit touched a test file. ID grep surfaces
only the `943b3ca` reopen record.

### irrevers-ca79d63a — Deploy binary/pack/trust pointer outside agent-writable fs
**GIT-VERIFIED AT HEAD.** `a03a7e6` (2 min before close) matches the close
notes line for line: exactly the three named files moved to `/var/cache/icg/`,
`dirs` removed; `/etc/icg/` placement had already landed via `d1e2b38`. The
duplicate-claim question raised at extraction is resolved: `a03a7e6`'s
runtime-state scope belongs to **this** bead; `d1e2b38`'s trust-pointer move
belongs to irrevers-96594031. No test files in the implementing commit.
Post-close hardening (`c38b0cd`…) belongs to irrevers-50077acb.

### irrevers-e00a5381 — icg-ci Argo WorkflowTemplate
**GIT-VERIFIED (cross-repo).** The artifact lives in declarative-config as
the description required: `122623ae` created
`k8s/iad-ci/argo-workflows/icg-ci-workflowtemplate.yml` (+107) 29 s before
close; same-week buildout `0f3f5faf` (deny-regression stage), `d11a6472`
(coverage-diff stage), `b2c2a0e5` (push tags to Forgejo before the GitHub
release) — all four re-resolved in `~/declarative-config`. Live-verified:
the `icg-ci` template is applied on iad-ci. Correctly no evidence in this
repo. Bead notes empty.

### irrevers-075634b8 — icg update atomically deploys the modular pack directory
**GIT-VERIFIED AT HEAD.** `409ca42` landed 10 s before close:
`src/update.rs` +642 −89 (modular-archive selection, per-pack validation,
traversal/symlink rejection, atomic swap with rollback, channel dirs) wired
through `src/main.rs`; the two demanded end-to-end tests are at HEAD in
`tests/icg_ci_integration_tests.rs` —
`trusted_release_update_replaces_complete_pack_directory_and_preserves_
enforcement` (:330) and
`malformed_release_archive_cannot_partially_deploy_or_escape_the_pack_root`
(:436) — plus operator docs updated in-commit. Bead notes empty.

**Section A counts: 13 git-verified at HEAD (one with mechanism-only and one
with late-landing caveats), 2 self-reported only, 1 no verifiable evidence
found. Total 16 — matches the inventory row count.**

---

## Section B — Fail-closed harness / policy enforcement behavior (20 beads)

### irrevers-8d2d4a73 — Engine: unconditional fail-open on parse failure or exception
**GIT-VERIFIED AT HEAD.** `48acfc1` landed 14 s before close: every check
path in `src/engine.rs` (+264) and `src/main.rs` wrapped, plus both
acceptance fixtures `tests/fixtures/malformed-stdin.json` and
`tests/fixtures/corrupt-rule-pack.json` wired as
`tests/fail_open_tests.rs` `layer_one_malformed_stdin_fails_open` and
`layer_one_corrupt_rule_pack_fails_open` (assert allow, neither crash nor
deny; still at HEAD). Bead notes empty.

### irrevers-aab3854c — Design fail-closed transition state machine
**GIT-VERIFIED AT HEAD.** `16f84c1` added
`docs/design/fail-closed-transition.md` (+342) covering every criterion
(state machine :39, per-state fleet behavior :73, graduation :136,
transition-triggers table :190, poison-pill integration :206, durable
state/crash recovery :253, emergency rollback :308). Docs only — no test
files, consistent with the design-only scope. Close was a verification pass
over the committed doc, as the notes state.

### irrevers-cd3f4c44 — Graduated fail-open to fail-closed policy for guard crashes
**GIT-VERIFIED AT HEAD.** `bb362fb` landed 16 s before close: created
`src/fail_closed.rs` (800 lines, durable `PolicyStore`) +
`tests/fail_closed_policy_tests.rs` (+167 —
`policy_reconciles_unique_clean_releases_and_graduates`, now :50). The full
scoped chain is `bb362fb`/`17971b7`/`3f0f00d`/`859e19e` (the same four the
child irrevers-019c36d3's notes name). `DEFAULT_GRADUATION_THRESHOLD = 3`
(`src/fail_closed.rs:34`) resolves the "threshold TBD" placeholder. Bead
notes empty.

### irrevers-8a24ad8d — Implement fail-open baseline and fail-closed enforcement modes
**GIT-VERIFIED AT HEAD.** `17971b7` landed 15 s before close: wired the
configured `PolicyMode` into `src/engine.rs`/`src/health.rs` and added
`tests/fail_closed_runtime_tests.rs` (+132) with the acceptance matrix —
fail-open baseline (renamed at HEAD to
`recovered_guard_crash_records_evidence_and_reconciles_into_policy` by
`0e669f2`, guarantee intact), `recovered_guard_crash_denies_in_fail_closed_
mode` (:240), `lifecycle_reports_recovered_crash_once` (:298). Mode-store
unit test `engine_uses_fail_closed_mode_for_guard_load_failure` came from
`bb362fb` (now `tests/fail_closed_policy_tests.rs:250`). Bead notes empty.

### irrevers-019c36d3 — Implement fail-closed policy transition mechanism
**GIT-VERIFIED AT HEAD — strongest record in the section.** The bead's own
notes name the four implementing commits (`bb362fb`, `17971b7`, `3f0f00d`
transition audit, `859e19e` operator docs, 447 lines) and the exact
verification commands (`cargo test --lib fail_closed`, `--test
fail_closed_policy_tests`, `--test fail_closed_runtime_tests`); all four
resolve with the claimed subjects, `859e19e` 3 min before close. Notes
record no new source was needed.

### irrevers-fffef435 — Integrate with poison-pill mechanism for automatic graduation
**GIT-VERIFIED AT HEAD.** `3f0f00d` landed 16 s before close: `src/fail_
closed.rs` +213 −21 and `operator_force_graduate_and_force_revert_are_
durable` in `tests/fail_closed_policy_tests.rs` (now :214). Read-only
poison-pill integration from `bb362fb`:
`policy_reconciles_unique_clean_releases_and_graduates` (:50) +
`poison_pill_resets_open_policy_without_editing_telemetry` (:109);
threshold serde-defaults to 3. Bead notes empty.

### irrevers-0f49129d — Poison-pill auto-rollback (original bundled)
**GIT-VERIFIED VIA SUCCESSORS (split close).** Closed the same second
successor irrevers-ff4f17da was created ("Recreated from irrevers-0f49129d,
which bundled measurement with reaction"); the measurement half split to
irrevers-b6579270 (`5b4d5df`, `721f3d9`) and the reaction half shipped under
ff4f17da (`d653ade`, `src/rollback.rs`). **No commit or note lands under
this ID itself** — the split record plus the successors' delivery is the
evidence; the bundled scope is fully delivered and verifiable. Bead notes
empty. Flagged for reconciliation alongside c87a3c50 because its closure,
too, is a bookkeeping event rather than a completion claim.

### irrevers-ff4f17da — Poison-pill auto-rollback: revert trust pointer on deny-rate spike
**GIT-VERIFIED AT HEAD.** `d653ade` landed 20 s before close:
`src/rollback.rs` (+371, `check_and_rollback`) + `docs/runbooks/rollback.md`
updates. Conservative-trigger tests inline at HEAD: `release_is_fresh`
(:212), `qualifying_fresh_release_rolls_back_to_exact_prior_pointer` (:294),
`small_sample_does_not_rollback_even_with_all_denials` (:322),
`anomaly_after_early_window_does_not_rollback` (:341),
`no_previous_pointer_is_not_guessed` (:362). Bead notes empty.

### irrevers-3fc4bdde — Apply the ICG_DISABLED emergency bypass to hook and wrapper
**GIT-VERIFIED AT HEAD.** `20808e9` landed 18 s before close (compile fix
`9ec6848` one minute earlier, `src/telemetry.rs` only): added
`src/emergency_bypass.rs`, routed Hook and the argv[0] wrapper through the
bypass (`src/main.rs` +84 −8, `front_end` telemetry without command data).
Named tests in `tests/emergency_response_tests.rs` (+136 −8):
`emergency_bypass_hook_returns_json_allow_before_fail_closed_loading` and
`emergency_bypass_executes_a_real_shadowed_binary_without_command_logging`;
`#[ignore]` stripped from `emergency_scenario_3_bypass_guard_with_disabled_
flag` (now :68). Bead notes empty.

### irrevers-f891f555 — Lock-free fail-closed policy read path for guarded invocations
**GIT-VERIFIED AT HEAD.** `0a5faa9` names the bead ID in its subject, 18 s
before close: `PolicyStore::load` (`src/fail_closed.rs:622`) no longer
creates/writes the `.lock`; graduation reconcile moved to the operator
command `icg policy reconcile` (`src/main.rs:2539`); commit body records the
0555 negative control against deployed v0.1.3 (warns) vs fixed build
(silent). Tests: `tests/fail_closed_runtime_tests.rs` +170 −5 —
`hook_invocation_leaves_administrator_owned_policy_untouched`,
`operator_policy_commands_manage_the_durable_policy`. Bead notes empty.

### irrevers-edb5c4ca — Stop reconciling the fail-closed policy from hook and wrapper
**GIT-VERIFIED AT HEAD.** The same shared changeset `0a5faa9` (closed 36 s
later): both required call-site comments at HEAD (`src/main.rs:835-839`
hook, `:1239-1241` wrapper, naming `icg policy reconcile`); reconcile now
only at `src/main.rs:2539`; `src/main.rs` +14 −46; same 0555 negative
control and green `cargo test` recorded in the commit body. Bead notes
empty.

### irrevers-9eb4de16 — Regression tests for a root-owned policy directory on the hook path
**GIT-VERIFIED AT HEAD.** Same changeset `0a5faa9` (closed 1 s later):
`tests/fail_closed_runtime_tests.rs` +170 −5 →
`hook_invocation_leaves_administrator_owned_policy_untouched` (:328, 0555
policy dir, stderr asserted clean) and
`operator_policy_commands_manage_the_durable_policy` (:486). Caveat: the
"fails without the fix" criterion is met by the commit body's manual
negative control (v0.1.3 warns on the identical layout), not by a recorded
pre-fix test run. Refinement `c6dcc3c` (+32 −15) later removed the
`icg_health_event` stderr filter. Bead notes empty.

### irrevers-93baa29a — Verify icg policy status/reconcile as root on installed host
**SELF-REPORTED ONLY (verification-only bead — the expected shape).** No
commit was in scope and none exists; the evidence is the recorded manual
verification in the bead's notes (2026-09-07, via
`/run/wrappers/bin/sudo -n /usr/local/bin/icg`): `policy status` exit 0;
`policy reconcile` exit 0, `Pending{no trusted release pointer}`, idempotent,
writes nothing; scratch-store exercises cover both directions (PoisonPill
consumed once; Clean×2 → Clean(Graduated) with FailClosed committed;
pill-while-closed preserved); live root-owned `/etc/icg` runs are silent
while a uid-1000 scratch dir reproduces the documented ownership warning.
Pointer note also on parent irrevers-8d1f79a7 (open, outside the
inventory). ID grep: no hits — expected for this bead type.

### irrevers-3e6c6fde — Hook/wrapper guarded paths take the policy lock via crash recovery
**GIT-VERIFIED AT HEAD — strongest record in the section.** Fix `0e669f2`
names the bead ID, ~2m51s before close: guarded crash recovery now records
evidence via `StateStore::record_guard_crash` (`src/state_store.rs` +55,
new `ICG_STATE_PATH` override keeping tests off `/var/cache/icg`); only
`icg policy reconcile` converts the counter to a poison-pill event.
Regression coverage at HEAD:
`recovered_guard_crash_keeps_administrator_owned_policy_untouched`
(`tests/fail_closed_runtime_tests.rs:406`),
`recovered_guard_crash_records_evidence_and_reconciles_into_policy` (:151),
plus `tests/fail_closed_policy_tests.rs` (+55). Commit body: all three
crash tests fail against the previous behavior. Operator docs updated
in-commit. Bead notes empty.

### irrevers-50077acb — Root-owned 0755 modes on icg trust dirs in argo-guarded-builder
**GIT-VERIFIED AT HEAD.** `c38b0cd` (~16 min before close), Dockerfile only
(+40 −2 — no test files by nature): `root:root`/`0755` on `/etc/icg`,
`/etc/icg/packs`, `/etc/icg/overrides`, `/var/cache/icg` (lines 58–59);
kaniko umask-0000 finding as a comment (:41); chmod-before-COPY ordering
rationale (:50–51); a build-time assertion RUN fails the image build on any
non-root-owned/world-writable trust dir — the standing verification, exercised
at the next kaniko release build. Close notes name the commit and explain
the gap (sibling merge carried it to origin).

### irrevers-ffdc924b — Guard health tracking and crash monitoring infrastructure
**GIT-VERIFIED AT HEAD.** `a525523` landed 13 s before close, +921 lines
(`src/health.rs` +622, `src/health_server.rs` +182, `src/main.rs` +94,
`src/metrics.rs` +39, `src/telemetry.rs` +66). Each acceptance item
traceable, with inline tests still at HEAD:
`stale_run_marker_is_recorded_as_a_crash_on_next_start` (:1438),
`clean_exit_clears_durable_run_marker` (:1458),
`exit_status_classifies_signals_and_oom_separately` (:1475),
`cgroup_oom_counter_provides_evidence_for_sigkill` (:1499),
`test_guard_metrics_from_persisted_health`,
`health_snapshot_round_trips_with_telemetry`. No work commit names the ID
(ID grep hits only the later evidence-pass commit `9f404dd`). Bead notes
empty.

### irrevers-9007792b — Operational monitoring and alerting infrastructure
**GIT-VERIFIED AT HEAD.** `a750033` landed 10 s before close, +1268 lines:
`monitoring/grafana/icg-overview.json`, `monitoring/prometheus/alerts.yml`
(+104) + `scrape.yml`, `monitoring/promtail/config.yml` (whole tree present
at HEAD), monitor loop `run_monitor` (`src/main.rs` +182), snapshot
collection `collect_snapshot` (`src/monitoring.rs:145`), Prometheus
exposition `export_prometheus` (`src/monitoring.rs` +547). Inline tests at
HEAD: `collects_durable_inputs_and_emits_operational_metrics` (:470),
`malformed_pack_is_visible_as_a_metric` (:545),
`redacts_payloads_when_full_content_logging_is_disabled`
(`src/denial_log.rs:1153`). No work commit names the ID (`8ae399c`'s body
mention is a later audit note; `d7b84c9` is a different feature). Bead
notes empty.

### irrevers-0d710c9a — Activation documentation and operational runbooks
**GIT-VERIFIED AT HEAD.** `859e19e` landed 16 s before close, +2452 lines
across ten docs: `docs/operators/fail-closed-mode.md` (447 lines at
creation), `docs/operators/training-manual.md` (1222),
`docs/onboarding-guide.md` (+645), plus rollback/incident-response/
troubleshooting/architecture/README updates — all present at HEAD. Docs
only, no test files, consistent with scope; sequencing correct (after the
four implementation commits, before `a750033`). Bead notes empty.

### irrevers-1517a263 — cargo test writes into the production denial log
**GIT-VERIFIED AT HEAD.** `0c062e3` names the bead ID, ~1m41s before close:
`denial_log::operational_log_path()` (`src/denial_log.rs` +135) resolves
every operational write's sink — `ICG_DENIAL_LOG` wins, the `/var/cache/icg`
default is refused to test-driven callers (cfg(test), libtest argv incl.
`--test-threads=2`, `target/<profile>/deps/` argv0, `CARGO_BIN_EXE_icg`;
deliberately not keyed on `$CARGO`). Tests: `tests/denial_log_pollution_
guard_tests.rs` (+241) — `an_in_process_denial_never_reaches_the_live_log`
(:185), `guard_is_not_bypassed_by_an_exported_sink` (:65). Commit body and
notes record the negative control and the `denials.jsonl.pre-rebaseline-
20260907` archive on codinghome.

### irrevers-0aa08f4e — run_started lifecycle telemetry prints to stderr on every guarded invocation
**GIT-VERIFIED AT HEAD.** `c6dcc3c` names the bead ID, 16 s before close:
`HealthStore::start_run` no longer prints on the healthy path
(`src/health.rs` +6 −1), chosen on recorded evidence that nothing consumes
the lines; the fault/crash emitters the bead said must stay are held there
by `a_recovered_crash_still_announces_itself_on_stderr`
(`tests/fail_closed_runtime_tests.rs:553`); the done-when filter removal is
done in the same commit (:328, plain `stderr.is_empty()`). Commit body:
zero-byte stderr against live `/etc/icg` vs two lines on deployed v0.1.3;
`cargo test` green (60 suites), clippy clean. Bead notes empty.

**Section B counts: 19 git-verified at HEAD (one of them via successors),
1 self-reported only (a verification-only bead), 0 no verifiable evidence
found. Total 20 — matches the inventory row count.**

---

## Section C — CI gate / test-harness infrastructure (5 beads)

### irrevers-b4b37bf0 — Layer 1: regression-suite CI gate
**GIT-VERIFIED VIA CHILDREN (split-bookkeeping close).** At the close
instant no regression-suite code existed — the parent closed at the moment
its scope split (verification spun out as irrevers-b0a453b2, created 43 ms
before the parent's close, on top of existing children). The children
delivered same day: irrevers-7684fa60 → `0970190` (`src/regression.rs` +614,
`regression-suite` CLI, `tests/regression_suite_tests.rs` +60) and
irrevers-69594753 → declarative-config `0f3f5faf` (build-failing
deny-regression step). The gate still runs in CI:
`icg-ci-guarded-workflowtemplate.yml:149` over real pack bytes (since
`6eaeb70`/declarative-config `083fd82e`). ID grep: no hits in either repo.

### irrevers-b0a453b2 — Verify the regression-suite gate actually fails the build
**SELF-REPORTED ONLY.** The close-time evidence is notes-only — the
2026-08-15 mutation experiment (deny → `additional_context` on
`vault-kv-destroy` made `cargo test` exit 101 and the gate exit 1; restore →
green) exists in no commit. Caveat recorded for reconciliation: `e1aab5a`
(2026-09-06) deliberately narrowed that exact edge — the same flip now
exits 0 with a reasoned skip at the command level, and enforcement moved to
the pinned-corpus invariants at HEAD
(`the_release_gate_corpus_is_unchanged`
`tests/regression_suite_scope_tests.rs:139`,
`no_deny_regex_rule_is_ever_skipped` (:83),
`every_enabled_rule_is_either_a_case_or_a_reasoned_skip` (:43),
`assert_denies_generated_cases` `tests/icg_ci_integration_tests.rs:43`;
five focused suites 27/27 green). What the bead verified was real and
reproducible when performed.

### irrevers-f61efd80 — Layer 1: coverage-diff CI gate
**GIT-VERIFIED AT HEAD, with closure-timing caveats.** `97b8eba`
(grep-invisible; recovered via `--diff-filter=A -- src/coverage.rs`) created
`src/coverage.rs`, the `coverage-diff` CLI, `tests/coverage_diff_tests.rs`
(+112), and the three fixtures ~1 h before close — but the described
report-format/justification scope did **not** exist at close; it landed
~9.6 h **after** close in `48d5a60` (`coverage-diff/v1`,
`has_explicit_justification`, +96 −12 tests). CI wiring: declarative-config
`d11a6472`. The only ID-citing commit is docs-only `1fe1a56`. The gate
still runs at template line 156. Close notes carried only a terminology
reconciliation.

### irrevers-29a9131c — Verify the coverage-diff gate blocks an unjustified change
**SELF-REPORTED ONLY.** The block-then-pass experiment lives only in the
close notes (ID/keyword greps find no implementing trace): coverage-diff
previous → current-regression without `--justification` → exit 2,
`regressions_detected`, justification REQUIRED; with justification → exit
0. Independently re-reproduced identically on 2026-09-10/11. Real-pack
regression detection is pinned at HEAD by
`tests/release_gate_integrity_tests.rs`
(`mutating_real_pack_causes_coverage_gate_to_fail`,
`widening_safe_pattern_in_real_pack_causes_coverage_gate_to_fail`); the
fixture inconsistencies the notes flagged were fixed by fixture-only
`c096a1e`. Residual: a pure redirect-channel flip is not flagged by
coverage-diff — channel enforcement rests on the pinned regression corpus.

### irrevers-ed77224f — End-to-end integration testing for the icg-ci workflow
**GIT-VERIFIED AT HEAD.** `287b866` landed 17 s before close:
`tests/icg_ci_integration_tests.rs` (+397) —
`icg_ci_release_candidate_runs_both_layer_one_gates_and_emits_layer_two_
report` (:78); the self-update area later hardened by `409ca42` into
`trusted_release_update_replaces_complete_pack_directory_and_preserves_
enforcement` (:330) and
`malformed_release_archive_cannot_partially_deploy_or_escape_the_pack_root`
(:436); the shipped template is pinned by
`ci_workflow_gates_actual_pack_bytes_not_fixtures` (:609, from `6eaeb70`).
Area (1) — driving the actual Argo API — is operational only: README-only
commits `e47419a`/`1994187` plus the green four-asset releases. All four
integration tests pass in the current tree.

**Section C counts: 3 git-verified at HEAD (one via children, one with a
late-landing caveat), 2 self-reported only, 0 no verifiable evidence found.
Total 5 — matches the inventory row count.**

---

## No verifiable evidence found

Exactly **one** inventory bead has no verifiable evidence for its own
closure. It is listed here explicitly rather than omitted:

- **irrevers-c87a3c50 — Base self-updater and trust pointer (icg update).**
  Closed 2026-08-14T14:37:11Z, **one second after creation**, with no notes
  and no assignee — a bead-forge→bead-rs migration-rehydration artifact
  (rehydration commit `0941686` landed two seconds later), not a completion
  record. At that moment the repo contained no self-updater or trust-pointer
  code at all. No implementing commit is attributable to this bead. The work
  it describes later landed under the two successors its own description
  points at — irrevers-5fdc2e13 (`97b8eba`) and irrevers-f59f9313
  (`9c6951b` + `d1e2b38`) — both git-verified above. The closure was never
  reopened despite having the same false-closure shape `943b3ca` documented
  for sibling beads. **Recommended for the downstream plan-reconciliation
  pass:** treat this closure as void bookkeeping and reconcile the scope
  against the two successor beads.

Near-misses that are flagged but **not** classed as no-evidence, because
verifiable evidence for the described work exists (under other IDs or with
recorded caveats):

- **irrevers-0f49129d** — split close, no direct evidence under its own ID;
  scope delivered and git-verified via irrevers-b6579270 + irrevers-ff4f17da.
- **irrevers-b4b37bf0** — split-bookkeeping close (no code at the close
  instant); delivered same day via children 7684fa60/69594753
  (`0970190`, declarative-config `0f3f5faf`).
- **irrevers-6de781f4** — mechanism git-verified (`90a9653`), but the
  operational half (a launched `canary-icg` worker) has no verifiable
  evidence beyond a doc comment.
- **irrevers-f59f9313** and **irrevers-f61efd80** — closes that predate
  their described work landing (~21–54 min and ~9.6 h respectively); both
  real and at HEAD now.

## Coverage cross-check

| Inventory section | Rows | Summaries written | Git-verified at HEAD | Self-reported only | No verifiable evidence |
|---|---|---|---|---|---|
| Release verification / release process / distribution integrity | 16 | 16 | 13 | 2 | 1 |
| Fail-closed harness / policy enforcement behavior | 20 | 20 | 19 | 1 | 0 |
| CI gate / test-harness infrastructure | 5 | 5 | 3 | 2 | 0 |
| **Total** | **41** | **41** | **35** | **5** | **1** |

Coverage is complete: 41 summaries for 41 inventory rows, all three
sections matching. The same short-form summaries are recorded on each
inventory bead's own notes (attributed to irrevers-e29e74b6) and the
consolidation bead's notes.
