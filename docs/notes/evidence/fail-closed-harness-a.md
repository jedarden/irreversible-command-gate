# Closing evidence — fail-closed harness closed beads, batch 3/5 (a)

Evidence for the first ten beads in the "Fail-closed harness / policy enforcement
behavior" section of
`docs/notes/closed-beads-release-verification-inventory.md` (irrevers-c52de1f2,
child of irrevers-622aae24). Method: `bead show <id>` for each bead's own
description/notes, cross-referenced with `git log --grep`, `git log -S`, per-file
history, and `git show` of each candidate commit (message body, diff, and test
names). No PRs are cited anywhere in this batch: this repo works directly on
`main`, so commits are the unit of record.

All ten beads were confirmed **Closed** at time of writing, so every entry below
is closing evidence, not a status correction. Commit and close timestamps are
UTC; the repo's commits carry -0400 offsets, so the "seconds before close"
correspondences below account for that offset.

---

## irrevers-8d2d4a73 — Engine: unconditional fail-open on parse failure or exception

**Verifiable.** Implementing commit `48acfc1` (2026-08-15, "fix(engine): fail
open on in-process check errors") landed 14 seconds before the bead closed
(13:21:58Z commit, 13:22:12Z close). It wraps every check path in `src/engine.rs`
(264 lines changed) and `src/main.rs`, and adds both acceptance fixtures from the
bead description: `tests/fixtures/malformed-stdin.json` (truncated hook JSON) and
`tests/fixtures/corrupt-rule-pack.json`, each wired as a Layer 1 regression case
in the new `tests/fail_open_tests.rs` — `layer_one_malformed_stdin_fails_open`
and `layer_one_corrupt_rule_pack_fails_open`, both asserting an allow exit and
neither a crash nor a deny. Both tests and fixtures are still present at HEAD,
so the guarantee cannot silently regress.

## irrevers-aab3854c — Design fail-closed transition state machine and graduation criteria

**Verifiable.** Design-only deliverable per the bead's own acceptance criteria:
commit `16f84c1` (2026-08-15, "docs: design fail-closed transition state
machine") added `docs/design/fail-closed-transition.md` (342 lines). The doc as
it stands covers every criterion: the Fail-Open → Graduating → Fail-Closed state
machine diagram ("State machine", line 39), per-state fleet behavior (line 73),
graduation criteria (line 136), a transition-triggers table (line 190),
integration with poison-pill tracking (line 206), durable state and crash
recovery (line 253), and the emergency rollback path (line 308). The bead
closed 2026-08-16T03:03Z, ~7.5h after the commit, and its close notes state the
close was a verification pass over the already-committed design ("verified
existing docs/design/fail-closed-transition.md meets all acceptance criteria").
No code changes in the commit — consistent with the design-only scope.

## irrevers-cd3f4c44 — Graduated fail-open to fail-closed policy for guard crashes

**Verifiable.** Parent feature bead for the graduation mechanism. Commit
`bb362fb` (2026-08-20, "feat: graduate guard crash policy to fail-closed")
landed 16 seconds before close (01:25:30Z commit, 01:25:46Z close) and created
`src/fail_closed.rs` (800 lines — the durable `PolicyStore`) plus
`tests/fail_closed_policy_tests.rs`. The full chain the bead scoped — graduation
signal, runtime enforcement, transition audit, operator docs — is the four
commits also named by child bead irrevers-019c36d3's close notes: `bb362fb`,
`17971b7`, `3f0f00d`, `859e19e`. Key graduation test from `bb362fb`:
`policy_reconciles_unique_clean_releases_and_graduates`. The default threshold
of 3 consecutive eligible clean releases (`DEFAULT_GRADUATION_THRESHOLD`,
`src/fail_closed.rs:34`) is the "exact threshold TBD at implementation time"
resolved.

## irrevers-8a24ad8d — Implement fail-open baseline and fail-closed enforcement modes

**Verifiable.** The mode-configurable runtime behavior is commit `17971b7`
(2026-08-20, "feat: enforce configured guard crash policy at runtime"), which
landed 15 seconds before close (02:44:07Z commit, 02:44:22Z close). It wires the
configured `PolicyMode` into `src/engine.rs` and `src/health.rs` and adds
`tests/fail_closed_runtime_tests.rs` with exactly the acceptance matrix: the
fail-open baseline (`recovered_guard_crash_is_fail_open_by_default_and_persisted`
— default is fail-open, as the bead required) and the fail-closed halt
(`recovered_guard_crash_denies_in_fail_closed_mode`), plus
`lifecycle_reports_recovered_crash_once` for persistence/recovery across
restarts. The mode store itself and policy-level unit tests came from
`bb362fb` (`engine_uses_fail_closed_mode_for_guard_load_failure`).

Audit note (2026-09-10): the baseline test above no longer resolves by its
original name at HEAD — `0e669f2` (2026-09-08, irrevers-3e6c6fde) renamed it to
`recovered_guard_crash_records_evidence_and_reconciles_into_policy` when crash
recovery was reworked to record evidence instead of policy writes. The successor
still asserts the fail-open default (hook exits success with a
`"permissionDecision":"allow"` body), so the guarantee the bead required is
intact under the new name; `recovered_guard_crash_denies_in_fail_closed_mode`
and `lifecycle_reports_recovered_crash_once` are unchanged at HEAD
(`tests/fail_closed_runtime_tests.rs:240` and `:298`).

## irrevers-019c36d3 — Implement fail-closed policy transition mechanism

**Verifiable — strongest record in this batch.** The bead's own close notes name
the implementing commits (`bb362fb`, `17971b7`, `3f0f00d`, `859e19e`) and the
exact verification commands run at close: `cargo test --lib fail_closed`,
`cargo test --test fail_closed_policy_tests`,
`cargo test --test fail_closed_runtime_tests`. All four commits are in history:
`bb362fb` (durable PolicyStore + graduation), `17971b7` (runtime crash
enforcement), `3f0f00d` (2026-08-20, "feat: audit fail-closed graduation
transitions" — transition audit + `operator_force_graduate_and_force_revert_are_durable`),
`859e19e` (2026-08-20, "docs: document fail-closed operations" —
`docs/operators/fail-closed-mode.md`, 447 lines, and the runbook/training
updates). `859e19e` landed 3 minutes before the 03:08:53Z close. The close notes
also record that no additional source changes were needed — the mechanism was
already shipped by the child beads' work.

## irrevers-fffef435 — Integrate with poison-pill mechanism for automatic graduation

**Verifiable.** Commit `3f0f00d` (2026-08-20, "feat: audit fail-closed
graduation transitions") landed 16 seconds before close (02:53:01Z commit,
02:53:17Z close); it extends `src/fail_closed.rs` by 234 lines (+213 -21) and
adds the
manual-override acceptance item verbatim —
`operator_force_graduate_and_force_revert_are_durable` in
`tests/fail_closed_policy_tests.rs`. The read-only integration with poison-pill
data (the bead's central constraint) came from `bb362fb`:
`policy_reconciles_unique_clean_releases_and_graduates` drives graduation off
`PoisonPillConfig` release health recorded by the telemetry store
(`record_release` helper), and the companion test
`poison_pill_resets_open_policy_without_editing_telemetry` asserts the
integration never writes telemetry. Threshold is configurable with the required
default: serde-defaulted `graduation_threshold` field =
`DEFAULT_GRADUATION_THRESHOLD = 3`.

## irrevers-0f49129d — Poison-pill auto-rollback

**Closed by split, not by implementation — and the successor carries the
implementation.** The bead closed 2026-08-15T03:11:18Z, the same second
`irrevers-ff4f17da` was created (03:11:18.003Z create vs 03:11:18.033Z update),
and ff4f17da's description states the reason: "Recreated from
irrevers-0f49129d, which bundled measurement with reaction." The measurement
half was split to `irrevers-b6579270` (its description says "Split out of
irrevers-0f49129d"). So this bead's evidence is the split record plus the
successor's delivery: the reaction half shipped as `d653ade` (2026-08-20, "feat:
add conservative poison-pill auto-rollback", `src/rollback.rs`) under
ff4f17da, and the measurement half as deny-rate telemetry under b6579270. No
commit lands directly under this bead ID; the original bundled scope is fully
delivered, just under the two successors.

## irrevers-ff4f17da — Poison-pill auto-rollback: revert the trust pointer on a deny-rate spike

**Verifiable.** Implementing commit `d653ade` (2026-08-20, "feat: add conservative
poison-pill auto-rollback") landed 20 seconds before close (01:05:23Z commit,
01:05:43Z close). It adds `src/rollback.rs` (371 lines, `check_and_rollback`)
and runbook updates (`docs/runbooks/rollback.md`). The "conservative trigger"
kill-pass objection is answered directly in the inline test module: the
first-M-commands window (`release_is_fresh`, `max_current_evaluations` in
`PoisonPillConfig`), `qualifying_fresh_release_rolls_back_to_exact_prior_pointer`,
`small_sample_does_not_rollback_even_with_all_denials` (a legitimately bad day
must not read as a bad release), `anomaly_after_early_window_does_not_rollback`,
and `no_previous_pointer_is_not_guessed`. The asymmetry the bead required
(revert automatic, adopt manual) is preserved in the design doc's transition
table.

## irrevers-3fc4bdde — Apply the documented ICG_DISABLED emergency bypass to hook and PATH-wrapper enforcement

**Verifiable.** Implementing commit `20808e9` (2026-08-25, "fix: apply emergency
bypass to enforcement frontends") landed 18 seconds before close (02:19:19Z
commit, 02:19:37Z close), with compile fix `9ec6848` ("restore_emergency_bypasses
compilation error", `src/telemetry.rs`) one minute earlier. `20808e9` adds
`src/emergency_bypass.rs` and routes Hook and the argv[0] wrapper through the
bypass in `src/main.rs` (+84 -8 lines), each emitting
`icg_emergency_bypass event=activated front_end=...` telemetry without command
data. Both verification requirements from the bead description have named
tests in `tests/emergency_response_tests.rs`: the hook JSON path
(`emergency_bypass_hook_returns_json_allow_before_fail_closed_loading`) and the
real shadowed-binary invocation
(`emergency_bypass_executes_a_real_shadowed_binary_without_command_logging`,
which also asserts the command secret never appears in output). The "enable
emergency_scenario_3 instead of leaving it ignored" item is visible in the
diff: `#[ignore] // Feature not implemented: icg does not support ICG_DISABLED
bypass` is stripped from `emergency_scenario_3_bypass_guard_with_disabled_flag`
and the test is extended. One nuance: the bead description also asks for the
fail-open/fail-closed interaction to be defined; that is documented in
`docs/operators/fail-closed-mode.md` (touched by the same commit) and
`docs/runbooks/emergency-bypass.md` (+22 lines in the same commit).

## irrevers-f891f555 — Make the fail-closed policy read path lock-free for guarded invocations

**Verifiable.** Implementing commit `0a5faa9` (2026-09-07, "fix(irrevers-f891f555,
irrevers-edb5c4ca, irrevers-9eb4de16): the guard stops asking for a lock it
cannot hold") — the only commit in this batch that names its bead ID in the
message — landed 18 seconds before close (02:06:25Z commit, 02:06:43Z close).
`PolicyStore::load` (`src/fail_closed.rs:622`) performs no create/write open of
the `.lock` file; the guarded hook/wrapper front-ends stopped calling
graduation reconcile entirely (moved to the operator command `icg policy
reconcile` in `src/main.rs`), and both `load`'s and `acquire_lock`'s contracts
are documented in-source so the paths cannot drift back together. The commit
body records the manual verification: negative control against the deployed
v0.1.3 binary with the policy directory at mode 0555 — v0.1.3 warns "Failed to
reconcile fail-closed graduation policy … Permission denied", the fixed build
emits nothing on stderr, both allow, and neither creates the lock. Regression
test: `hook_invocation_leaves_administrator_owned_policy_untouched` (plus
`operator_policy_commands_manage_the_durable_policy` for the mutating path) in
`tests/fail_closed_runtime_tests.rs`; mutating paths keep the exclusive lock,
and the commit records `cargo test` green (216 unit + integration suites).
