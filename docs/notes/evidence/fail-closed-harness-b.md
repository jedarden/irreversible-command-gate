# Closing evidence — fail-closed harness closed beads, batch 4/5 (b)

Evidence for the last ten beads in the "Fail-closed harness / policy enforcement
behavior" section of
`docs/notes/closed-beads-release-verification-inventory.md` (irrevers-c52de1f2,
child of irrevers-622aae24; batch 3/5 covered the section's first ten). Method:
`bead show <id>` for each bead's own description/notes, cross-referenced with
`git log --grep`, per-file history (`git log -- <file>`), and `git show` of each
candidate commit (message body, diff, and test names). No PRs are cited anywhere
in this batch: this repo works directly on `main`, so commits are the unit of
record.

All ten beads were re-confirmed **Closed** at time of writing, so every entry
below is closing evidence, not a status correction. Commit and close timestamps
are UTC; the repo's commits carry -0400 offsets, which the "seconds before
close" correspondences below account for. Three of the ten (the 2026-08-21
batch) carry no bead ID in any commit message — their commits were located by
per-file history and commit/second-level close-time correspondence instead.

---

## irrevers-edb5c4ca — Stop reconciling the fail-closed policy from the hook and wrapper paths

**Verifiable.** Fixed by commit `0a5faa9` (2026-09-08 02:06:25Z, "fix(
irrevers-f891f555, irrevers-edb5c4ca, irrevers-9eb4de16): the guard stops
asking for a lock it cannot hold"), which shares one changeset with the
lock-free read path (irrevers-f891f555) and the regression tests
(irrevers-9eb4de16); this bead closed 36 seconds later (02:07:01Z). Both
required call-site comments are at HEAD: the hook front-end
(`src/main.rs:835`, "Graduation is an operator action: `icg policy
reconcile`") and the wrapper path (`src/main.rs:1238`, "an operator runs
`icg policy reconcile` instead"). The reconcile call itself now lives only in
the operator command handler (`src/main.rs:2539`, inside `icg policy
reconcile`, which prints the outcome). The commit body records the manual
verification this bead's third criterion asks for: negative control against
the deployed v0.1.3 binary with the policy directory at mode 0555 — v0.1.3
emits "Failed to reconcile fail-closed graduation policy … Permission denied",
the fixed build emits nothing on stderr, both allow, and neither creates the
lock. `src/main.rs` is −60/+51 lines net in the commit (the guarded reconcile
plumbing removed); `cargo test` green (216 unit + all integration suites) is
recorded in the same body.

## irrevers-9eb4de16 — Add regression tests for a root-owned policy directory on the hook path

**Verifiable.** Same commit `0a5faa9` (02:06:25Z; bead closed 02:07:02Z), which
grows `tests/fail_closed_runtime_tests.rs` by +175 lines. At HEAD the two
required tests are `hook_invocation_leaves_administrator_owned_policy_untouched`
(`tests/fail_closed_runtime_tests.rs:328` — 0555 policy directory, decision
unchanged, policy state untouched, guarded stderr asserted clean) and
`operator_policy_commands_manage_the_durable_policy`
(`tests/fail_closed_runtime_tests.rs:486` — the companion proving locking was
not deleted everywhere: operator-shaped paths still read and mutate the durable
policy when the store is writable). The "shown to fail without the fix"
criterion is satisfied by the recorded manual negative control rather than a
pre-fix test run: the commit body documents v0.1.3 (pre-fix) warning on the
identical 0555-layout input while the fixed build is silent. Two in-flight
test defects are also documented in the commit body (seed-via-`PolicyStore::save`
self-created the lock it asserted absent; "Mode: FailOpen" matched against
markdown "**Mode:** FailOpen"). One subsequent refinement is part of the
record: the test originally filtered `icg_health_event` lines out of the
stderr assertion (deliberately, pointing at irrevers-0aa08f4e), and commit
`c6dcc3c` removed that filter once the run_started print was gone.

## irrevers-93baa29a — Verify icg policy status and reconcile as root on an installed host layout

**Verifiable — verification-only bead; the evidence is the recorded manual
verification, not a commit.** No code change was in scope and none exists,
which matches the bead's purpose. The bead's own notes (mirrored on parent
irrevers-8d1f79a7, which they were also recorded against per the acceptance
criteria) carry the full reproduction: on the installed host layout
(`/usr/local/bin/icg`, root-owned `/etc/icg`, installed 2026-09-06), both
`policy status` and `policy reconcile` ran as root to completion — exit 0,
empty stderr on live runs; live `reconcile` outcome `Pending { reason: "no
trusted release pointer exists" }` with the documented reason the
clean-release observation cannot advance on this host (no trust pointer, no
qualifying release evidence). Both reconcile directions were still exercised
through the documented `-p` / `--state-store-path` / `--trust-pointer-path`
overrides against throwaway scratch stores, same installed binary, as root:
poison-pill→PoisonPill event consumed exactly once; clean-release→`Clean`
twice then `Clean(Graduated)` (mode committed FailClosed at threshold,
`NoChange` on repeat); poison-pill against an already-FailClosed policy→mode
preserved. The ownership check is a live-vs-scratch contrast: root-owned
`/etc/icg` runs emitted no warning while scratch-store runs reproduced the
documented "owned by uid 1000, not root" warning. Host quirk recorded for
reproduction: bare `sudo` on codinghome is the non-setuid nix-store path; the
working invocation is `/run/wrappers/bin/sudo -n /usr/local/bin/icg …`. Only
exit codes, outcome variants, and transition names are recorded — no policy
value or file content, per the bead's own redaction criterion.

## irrevers-3e6c6fde — Hook and wrapper guarded paths take the fail-closed policy lock via crash recovery

**Verifiable — strongest record in this batch.** The bead is the defect report
(lock-reachability chain from guarded crash recovery into
`PolicyStore::acquire_lock`, found by irrevers-10fa65df's grep verification);
fix commit `0e669f2` (2026-09-08 05:35:00Z, "fix(irrevers-3e6c6fde): crash
recovery records evidence, not policy writes") landed ~2m51s before close
(05:37:51Z), names the bead ID in its message, and restructures exactly along
the bead's suggested direction: the guarded process now records crash evidence
(health crash id plus a counter) in the state store it owns
(`StateStore::record_guard_crash`, `src/state_store.rs` +55, with the new
`ICG_STATE_PATH` override keeping tests out of `/var/cache/icg`), and only the
operator's `icg policy reconcile` turns that counter into a poison-pill event.
The halt decision is unchanged (it reads the lock-free `PolicyStore::load`).
Both failure shapes from the report are regression-covered at HEAD:
`recovered_guard_crash_keeps_administrator_owned_policy_untouched`
(`tests/fail_closed_runtime_tests.rs:406` — hardened layout, read-only policy
dir, seeded crash: evidence recorded, policy untouched) and
`recovered_guard_crash_records_evidence_and_reconciles_into_policy`
(`tests/fail_closed_runtime_tests.rs:151` — end-to-end reconcile consumption
with idempotent replay), plus the reconcile-side unit test in
`tests/fail_closed_policy_tests.rs` (+55). The commit body states all three
crash tests fail against the previous behavior. Operator docs updated in the
same commit (`docs/operators/fail-closed-mode.md` +23,
`docs/notes/fail-closed-policy.md` +13): reconciliation is operator-only, and
crash evidence flows through the state store.

## irrevers-50077acb — Set explicit root-owned 0755 modes on the icg trust directories in argo-guarded-builder

**Verifiable.** Commit `c38b0cd` (2026-09-08 05:26:43Z, "fix: pin root-owned
0755 modes on the icg trust directories"), touching
`containers/argo-guarded-builder/Dockerfile` only (+40/−2), landed ~16 min
before close (05:42:49Z); the bead's notes explain the gap (it finished and
shipped the partial diff the parent bead irrevers-beee1069 had left in the
shared working tree, then waited on a sibling worker's merge to carry it to
origin). All four acceptance items are in the diff and at HEAD: the mkdir RUN
chowns `root:root` and chmods `0755` `/etc/icg /etc/icg/packs
/etc/icg/overrides /var/cache/icg` (Dockerfile lines 58–59); the kaniko
umask-0000 finding is a comment citing the 0.1.0 evidence (line 41); the
ordering comment states why the chmod wins over the later `COPY packs
/etc/icg/packs/` — a COPY into an existing destination only fills it and never
re-modes it (lines 50–51); and a build-time assertion RUN fails the image
build if any of the four dirs is non-root-owned or world-writable. The commit
also ships the semver-shape check replacing the exact `ICG_VERSION` match
(Cargo.toml 0.1.6 vs ARG default 0.1.0 failed every build). The commit message
is explicit that the image itself was not built on this box — kaniko builds in
iad-ci at release time, and the in-build assertion RUN is the standing
verification.

## irrevers-ffdc924b — Add guard health tracking and crash monitoring infrastructure

**Verifiable.** No commit message names the bead ID; the implementing commit
is `a525523` (2026-08-21 02:32:58Z, "feat: add durable guard health tracking"),
13 seconds before close (02:33:11Z), +921 lines across `src/health.rs` (+622),
`src/health_server.rs` (+182 — the status API), `src/main.rs` (+94),
`src/metrics.rs` (+39), `src/telemetry.rs` (+66). Each acceptance item is
traceable in the diff: the tracked metrics are literally the requested fields
(`crash count`, `consecutive_clean_runs`, `last_crash_at` /
`last_crash_timestamp`, `uptime_seconds` appear as persisted/served fields);
persistence is a durable run marker with crash-on-next-start semantics
(`stale_run_marker_is_recorded_as_a_crash_on_next_start`,
`clean_exit_clears_durable_run_marker`); the status surface is the extended
health server (`test_guard_metrics_from_persisted_health`,
`health_snapshot_round_trips_with_telemetry` cover the metrics/telemetry
integration); and crash detection is exit-status classification with signal
and OOM separated (`exit_status_classifies_signals_and_oom_separately`) plus
cgroup OOM-counter evidence for SIGKILL (`cgroup_oom_counter_provides_
evidence_for_sigkill`, `read_oom_kill_count`, `with_oom_events_path`) — the
"detects OOM, segfaults, panic" item. All tests are still in `src/health.rs`'s
inline module at HEAD.

## irrevers-9007792b — Operational monitoring and alerting infrastructure

**Verifiable.** Again no bead ID in any message; the implementing commit is
`a750033` (2026-08-21 03:28:02Z, "feat: add operational monitoring
integration"), 10 seconds before close (03:28:12Z), +1268 lines. The five
numbered gaps in the bead description map directly onto the diff: (1)
dashboards — `monitoring/grafana/icg-overview.json` (+ `monitoring/README.md`);
(2) alerting rules — `monitoring/prometheus/alerts.yml` (+104 lines) with
`monitoring/prometheus/scrape.yml`; (3) log aggregation —
`monitoring/promtail/config.yml`; (4) operational health checks — the monitor
loop and snapshot collection in `src/main.rs` (+182, `run_monitor`,
`collect_snapshot`) and `src/health_server.rs` (+75); (5) integration with
existing systems — Prometheus exposition via `src/monitoring.rs` (+547,
`export_prometheus`, `MonitoringConfig::from_environment`) wiring into the
existing `metrics.rs`/`telemetry.rs` (+72, including
`redacts_payloads_when_full_content_logging_is_disabled` guarding the new
denial exports) and `src/denial_log.rs` (+62), documented in
`docs/monitoring-deployment-guide.md`. Named tests at HEAD in
`src/monitoring.rs`'s inline module: `collects_durable_inputs_and_emits_
operational_metrics` and `malformed_pack_is_visible_as_a_metric` (rule-pack
loading errors visible as a metric — the bead's "rule pack loading errors"
alerting input). The whole `monitoring/` tree is present at HEAD, so the
config artifacts cannot silently disappear.

## irrevers-0d710c9a — Write activation documentation and operational runbooks

**Verifiable.** Implementing commit `859e19e` (2026-08-21 03:05:52Z, "docs:
document fail-closed operations"), 16 seconds before close (03:06:08Z),
+2452 lines across ten docs files. The acceptance checklist maps as follows:
activation guide + configuration reference → `docs/operators/fail-closed-mode.md`
(447 lines); monitoring guide → the same file's monitoring section plus
`docs/monitoring-deployment-guide.md` (from `a750033`, the bead before it in
the same close window); rollback procedures → `docs/runbooks/rollback.md`
(created by `d653ade` the previous day for the poison-pill rollback the
procedures describe) and `docs/runbooks/incident-response.md` (+11 in this
commit); troubleshooting → `docs/operators/troubleshooting.md` (+22);
architecture overview → `docs/design/fail-closed-transition.md` (+11, the
irrevers-aab3854c design the bead was required to incorporate) and
`docs/notes/fail-closed-policy.md` (+86); integration with existing ops and
onboarding docs → `README.md`, `docs/operators/README.md` (+16),
`docs/operators/deployment-guide.md` (+21), and the new
`docs/onboarding-guide.md` (+645). The 1222-line
`docs/operators/training-manual.md` is the operator-facing distillation. All
files exist at HEAD. Sequencing matches the bead's "final bead — docs
describe what exists" constraint: it landed after the four implementation
commits (`bb362fb`, `17971b7`, `3f0f00d`, `a525523`) and immediately after
`a750033`.

## irrevers-1517a263 — cargo test writes into the production denial log on an instrumented host

**Verifiable.** Fix commit `0c062e3` (2026-09-07 20:29:35Z, "fix(
irrevers-1517a263): a test-driven process is refused the live denial log"),
~1m41s before close (20:31:16Z), naming the bead ID. The fix is a library
guard, not per-test discipline, exactly as the bead's done-when preferred:
`denial_log::operational_log_path()` (`src/denial_log.rs` +135) resolves every
operational write's sink — `ICG_DENIAL_LOG` wins, the `/var/cache/icg` default
is refused to a test-driven caller — with detection covering cfg(test), libtest
argv (including `--test-threads=2`, `args_os` so non-UTF-8 hook argv cannot
panic), `target/<profile>/deps/` in argv0, and `CARGO_BIN_EXE_icg` inherited by
a spawned binary; deliberately not keyed on `$CARGO` so `cargo run -- check`
still records. The "ideally a test that fails if the suite writes outside a
tempdir" item is `tests/denial_log_pollution_guard_tests.rs` (+241, new) —
`an_in_process_denial_never_reaches_the_live_log` and
`guard_is_not_bypassed_by_an_exported_sink` (the explicit-sink control
proving the guard cannot have quietly broken logging) at HEAD. The bead's
notes record the manual verification and the re-baseline: the negative control
against the unguarded library reproduced the leak (one probe record landed in
the live log, test failed); with the fix the full suite appended zero records
and did not create the file; ex44's `/var/cache/icg/denials.jsonl` (77 records,
15 fixture/probe ids) was archived to `denials.jsonl.pre-rebaseline-20260907`
rather than deleted, so the trial window starts clean.

## irrevers-0aa08f4e — Routine run_started lifecycle telemetry prints to stderr on every guarded invocation

**Verifiable.** Fix commit `c6dcc3c` (2026-09-08 02:09:45Z, "fix(
irrevers-0aa08f4e): starting a run is not an event"), 16 seconds before close
(02:10:01Z), naming the bead ID. It takes the bead's first option (drop the
print) on recorded evidence: "Nothing reads it: no doc, script, monitor or
test in the repo or on either instrumented host consumes these lines" — which
is the check the bead's options section demanded before choosing — and the run
id stays recoverable via `icg health`. Both done-when code items are at HEAD:
`HealthStore::start_run` no longer prints on the healthy path
(`src/health.rs`, +7/−), while the fault and crash-recovery emitters the bead
said must stay (`crash_detected`, `crash_recorded`, the `*_failed` variants)
are untouched, held there by the new
`a_recovered_crash_still_announces_itself_on_stderr`
(`tests/fail_closed_runtime_tests.rs:553`). The third item — delete the
`icg_health_event` filter in
`hook_invocation_leaves_administrator_owned_policy_untouched` and restore the
plain `stderr.is_empty()` assertion — is done in the same commit
(`tests/fail_closed_runtime_tests.rs:328`). Manual verification is in the
commit body: against the real root-owned `/etc/icg` as the fleet runs it, a
guarded invocation's stderr is zero bytes with the allow decision unchanged,
while deployed v0.1.3 emits two lines on the identical input; `cargo test`
green (60 suites), clippy `-D warnings` clean, and the suite appended nothing
to the live denial log.
