# Closing-evidence verification — first half of the enumerated inventory beads

Working deliverable for **irrevers-98e9188e** (child 2 of the
irrevers-2c3b4637 auto-split). Verified 2026-09-12 at repo HEAD `a58aca7`.

**Scope** — the first 21 beads of the child-1 list in its recorded order
(`docs/notes/closed-beads-reconciliation-2026-09-12.md`): §A rows 1–16 plus
§B rows 1–5. The second half (§B rows 6–21 + §C) is irrevers-25063548's
scope. `docs/plan/plan.md` untouched by this pass.

**Method, per bead** — close reason re-read from the bead record and the last
`closed` snapshot in `.beads/checkpoint/forensic.jsonl`; every commit/test/
doc/tag cited for it by the existing inventories
(`closed-beads-compiled-inventory.md` and, via it,
`docs/notes/evidence/final-inventory.md`) checked against git history:
`git cat-file -e` for resolution (cross-repo SHAs in `~/declarative-config`),
`git show --stat`/`--numstat` for scope, commit-date recomputation for every
close-instant timing claim (dates below converted to UTC), `git log
--format=%B` greps for ID citations, `grep -n` for HEAD test anchors. Live
re-checks where the cited evidence is an external object: `gh release view
v0.1.1 --repo jedarden/irreversible-command-gate` and
`kubectl --server=http://traefik-iad-ci:8001 get workflowtemplate icg-ci`.

**Result: 19 verified · 2 corrected (both minor, substance unaffected) ·
0 unverifiable.** All 31 cited in-repo SHAs and all 4 cross-repo SHAs
resolve; all 7 tags v0.1.0–v0.1.6 resolve at the recorded SHAs. No cited
commit is missing from this repo.

---

## Section A — Release verification / release process / distribution integrity (16)

### 1. irrevers-84b36e47 — VERIFIED
Close reason (forensic 2026-09-06T13:06:27Z): icg-ci-manual-snxhj Succeeded;
v0.1.1 non-draft, 4 assets. Artifacts: live `gh release view v0.1.1` →
`isDraft=false`, `publishedAt=2026-09-06T13:06:02Z` (25 s before close,
exactly as cited), assets `icg`, `icg-packs.tar.gz`, `pack-manifest.json`,
`rule-pack.json`; tag `v0.1.1` → `a4f6e0c`. Unblocking commits re-resolved
with matching scopes: `3399989` (tests/installation_tests.rs +11 −2),
`c5d391b` (tests/icg_ci_integration_tests.rs, packs/openbao.json,
src/pack_manifest.rs +243), `eec8e73` (tests/maintenance_tasks_tests.rs).
`9350f19` cites the ID in its body and is empty (0 files) — "empty CI
trigger" confirmed. Argo-run caveat stands (TTL-reaped); the release object
is the evidence, and it re-verifies live.

### 2. irrevers-e77615c8 — VERIFIED
Close reason (2026-08-24T04:27:11Z): rule-pack artifact published as release
asset. Artifacts: `2c541ec` (feat(icg-ci): publish rule-pack artifact as
release asset) adds `src/rule_pack.rs` +80 (build-pack) and the template
upload step (`containers/argo-guarded-builder/icg-ci-guarded-workflowtemplate.yml`
+11; body: "Uploads both icg binary and rule-pack.json"); `c2fcfd9` cites the
ID, landed 2026-08-24T04:26:39Z — 32 s before close ("~30 s" ✓). Release
asset `rule-pack.json` live (see #1).

### 3. irrevers-37eb1100 — VERIFIED
Close reason (2026-08-15T13:57:16Z): runbook shipped, committed as 436bdce.
Artifacts: `436bdce` adds `docs/runbooks/release-cutting.md` +108 exactly,
2026-08-15T13:56:54Z — 22 s before close; `1b6f6a6` (chore: release v0.1.2)
is the recorded later maintenance (its message documents the runbook
correction).

### 4. irrevers-340ae322 — VERIFIED (as self-reported; git-side corroboration holds)
Close reason (2026-08-30T01:56:20Z): manual `docker manifest inspect` +
icg-ci-rg9n7 pod states. The registry/pod halves remain non-re-checkable
(podGC OnPodCompletion; registry 401s anonymous manifests) — as the
inventories already record. Git-side corroboration re-verified: at the close
tip, `containers/argo-guarded-builder/VERSION` = `0.1.0` and both `image:`
pins (template lines 40, 82) = `ronaldraygun/argo-guarded-builder:0.1.0`.
Correctly classed SELF / weak-unverifiable; nothing new to contradict it.

### 5. irrevers-e2bb8fbf — CORRECTED (minor)
Close reason (2026-08-30T12:19:35Z): help hint + round-trip test, committed
as c7e9df5. Core evidence verifies: `c7e9df5` = `src/main.rs` +2 (help hint)
+ `tests/maintenance_tasks_tests.rs` +134 with
`maintenance_scenario_trust_channel_roundtrip` at line **450** at HEAD
(cited :450 ✓), landed 2026-08-30T12:17:33Z — 2 m 02 s before close.
**Correction:** both inventory docs list "later touches `890429f`, `0fb164d`",
but `0fb164d` (2026-08-27T20:30:56Z) is an **ancestor** of `c7e9df5` — an
*earlier* touch of the same test area, not a later one. Only `890429f`
(2026-09-06T01:25:17Z, not an ancestor) is a genuinely later touch.
Substance of the closure unaffected.

### 6. irrevers-6de781f4 — VERIFIED
Close reason (2026-08-15T05:16:35Z): canary via --channel, committed as
90a9653. Artifacts: `90a9653` cites the ID in its body, landed
2026-08-15T05:16:23Z — 12 s before close; diff uses
`TrustPointerStore::for_channel` 13× across `src/main.rs` +163,
`src/trust_pointer.rs` +80, `src/update.rs` +21. The doc-comment-only
operational half is real: `src/trust_pointer.rs:102` = "Canary channel worker
(launched via NEEDLE --identifier canary-icg)". Partial rating (mechanism
git-verified, launched-worker half unsubstantiated) confirmed accurate.

### 7. irrevers-b6579270 — VERIFIED
Close reason (2026-08-21T00:54:24Z): durable deny-rate telemetry + baseline,
committed as 5b4d5df. Artifacts: `5b4d5df` landed 2026-08-21T00:53:25Z —
59 s before close (✓ "59 s"), touching `src/engine.rs`, `src/state_store.rs`
(+552), `tests/release_telemetry_tests.rs` +31; `721f3d9` (feat: integrate
rolling telemetry with auto-rollback) wires the poison-pill side (30
"poison" hits in its diff). Both named tests at HEAD at the cited lines:
`engine_persists_per_release_evaluation_and_deny_counts` :19,
`engine_telemetry_feeds_poison_pill_rollback` :47 ✓.

### 8. irrevers-eff8909f — VERIFIED (substance; known-wrong claims already flagged)
Close reason (2026-09-10T10:30:14Z): tag table recorded in bead notes. All
seven tags resolve at the exact SHAs recorded in the bead's table: v0.1.0
`f0fe556`, v0.1.1 `a4f6e0c`, v0.1.2 `1b6f6a6`, v0.1.3 `bb1beb4`, v0.1.4
`e120f73`, v0.1.5 `c70033d`, v0.1.6 `aab687d`. The two known defects (the
"zero GitHub Releases" line — false for the mirror, which has v0.1.1+ live —
and the stale range) were already flagged by later passes; nothing new.

### 9. irrevers-2cb3dbd2 — VERIFIED
Close reason (2026-08-26T04:03:36Z): real pack bytes gated in icg-ci.
Artifacts: `6eaeb70` landed 2026-08-26T04:03:26Z — 10 s before close (✓
"10 s"), with `tests/release_gate_integrity_tests.rs` **+284**,
`tests/icg_ci_integration_tests.rs` **+110**, and the template switch to
real pack bytes (+78) — all three stat claims exact. Follow-up `b8aed51`
(2026-08-26T07:21:01Z, post-close as described). The gate pins the shipped
template at HEAD: `ci_workflow_gates_actual_pack_bytes_not_fixtures` :609;
template lines 117/156/202–205 run build-pack / coverage-diff over real pack
bytes.

### 10. irrevers-c87a3c50 — CORRECTED (minor arithmetic)
Close reason (2026-08-14T14:37:11Z): supersede-only ("Superseded: split into
icg-2usc / icg-26le") ✓; closed ~1.2 s after creation ✓; rehydration commit
`0941686` 2 s later (2026-08-14T14:37:13Z) ✓; tip at close = `200bb7e`
(2026-08-13T17:21:51Z) re-confirmed via `git log --until` ✓;
`src/trust_pointer.rs` first appears in `97b8eba` ✓ (git log
--diff-filter=A). **Correction:** the gap to `97b8eba`
(2026-08-15T03:04:19Z) is **~12.4 h**, not "~9.5 h" as both
`final-inventory.md` and the compiled inventory state. Direction and
substance (no described code existed at close) unchanged.

### 11. irrevers-5fdc2e13 — VERIFIED
Close reason (2026-08-15T03:04:33Z): trust pointer mechanism, committed as
97b8eba. Artifacts: `97b8eba` landed 2026-08-15T03:04:19Z — 14 s before
close (✓ "14 s"); `src/trust_pointer.rs` **+254** exact plus the `icg trust`
CLI (`src/main.rs` +191). All six close-note tests at HEAD:
`test_trust_pointer_create` :564, `test_trust_pointer_with_justification`
:571, `test_store_save_and_load` :581, `test_store_get_trusted_ref` :601,
`test_store_is_trusted` :619, `test_atomic_write` :637. Carried note
confirmed: `d1e2b38` (next day) touched `src/trust_pointer.rs` to move the
default path root-owned.

### 12. irrevers-f59f9313 — VERIFIED
Close reason (2026-08-15T03:26:55Z): unanchored completion claim. Artifacts:
`9c6951b` landed 2026-08-15T03:47:30Z — **20 m 35 s after close** (✓ "~21
min") and does add `Commands::Update` (`src/main.rs`), bundled under the
unrelated subject "feat(engine): implement content-mode input acquisition";
`d1e2b38` landed 2026-08-15T04:21:04Z — **54 m 09 s after close** (✓ "~54
min"), `src/update.rs` **+317** exact, under the unrelated subject
"docs(plan): decide install path…". Tests via `287b866` → superseded by
`409ca42` (see #16). Every element of the T3 "close predates work, bundled
under unrelated subjects" record re-verifies.

### 13. irrevers-96594031 — VERIFIED
Close reason (2026-08-26T01:39:15Z): migration already done in d1e2b38 and
a03a7e6. Artifacts: `d1e2b38` retargets the trust pointer (src/trust_pointer.rs,
18 lines) and adds `src/update.rs` +317; `a03a7e6` moves runtime state to
/var/cache/icg. The scoped check is present as described:
`verify_artifact_directory_security()` at `src/trust_pointer.rs:`**127** at
HEAD; `git log -S` oldest hit = `4d5c1a5` (2026-08-15T04:50:15Z) — the
cited introducer ✓ (later refined by `d88da13`, 2026-09-08, consistent with
the recorded history).

### 14. irrevers-ca79d63a — VERIFIED
Close reason (2026-08-23T01:16:06Z): runtime state moved to /var/cache/icg,
dirs dependency removed. Artifacts: `a03a7e6` (fix(deployment): use
root-owned system paths for all artifacts) landed 2026-08-23T01:14:14Z —
1 m 52 s before close (✓ "~2 min"); stat matches the close note line for
line: exactly the three runtime-state files (`src/denial_log.rs`,
`src/health.rs`, `src/state_store.rs`) plus `Cargo.toml` −1
(`dirs` dependency removed) and `Cargo.lock`.

### 15. irrevers-e00a5381 — VERIFIED (cross-repo + live)
Close reason (2026-08-15T02:02:17Z): template created in declarative-config.
Artifacts: declarative-config `122623ae` (feat(iad-ci): add icg-ci Argo
WorkflowTemplate) adds the template +107, landed 2026-08-15T02:01:48Z —
**29 s before close** (✓ "29 s"); buildout SHAs `0f3f5faf` (deny-regression),
`d11a6472` (coverage-diff), `b2c2a0e5` (tag push) all resolve in
`~/declarative-config`. Live: `icg-ci` workflowtemplate present on iad-ci
(read-only kubectl). Correctly no evidence in this repo for the creation
event itself.

### 16. irrevers-075634b8 — VERIFIED
Close reason (2026-08-26T02:55:13Z): atomic modular-archive swap with
rollback. Artifacts: `409ca42` landed 2026-08-26T02:55:03Z — 10 s before
close (✓ "10 s"); `git show --numstat` gives `src/update.rs` **+642 −89**
exact. Both end-to-end tests at HEAD at the cited lines:
`trusted_release_update_replaces_complete_pack_directory_and_preserves_enforcement`
:330, `malformed_release_archive_cannot_partially_deploy_or_escape_the_pack_root`
:436 ✓.

---

## Section B (first 5 rows) — Fail-closed harness / policy enforcement

### 17. irrevers-8d2d4a73 — VERIFIED
Close reason (2026-08-15T13:22:12Z): unconditional fail-open on in-process
errors. Artifacts: `48acfc1` (fix(engine): fail open on in-process check
errors) landed 2026-08-15T13:21:58Z — 14 s before close (✓ "14 s"); wraps
`src/engine.rs` +264 and `src/main.rs` +17; adds
`tests/fail_open_tests.rs` +57 and both acceptance fixtures
(`malformed-stdin.json`, `corrupt-rule-pack.json`). Both named tests at HEAD
at the cited lines: `layer_one_malformed_stdin_fails_open` :34,
`layer_one_corrupt_rule_pack_fails_open` :46 ✓.

### 18. irrevers-aab3854c — VERIFIED
Close reason (2026-08-16T03:03:17Z): existing design doc covers all
criteria, no code changes. Artifacts: `16f84c1` (docs: design fail-closed
transition state machine) adds `docs/design/fail-closed-transition.md`
**+342** exact — docs-only, matching the design-only scope; present at
HEAD.

### 19. irrevers-cd3f4c44 — VERIFIED
Close reason (2026-08-21T01:25:46Z): durable graduation, committed as
bb362fb. Artifacts: `bb362fb` landed 2026-08-21T01:25:30Z — 16 s before
close (✓ "16 s"); creates `src/fail_closed.rs` **+800** and
`tests/fail_closed_policy_tests.rs` **+167** — both stat claims exact. Full
chain `bb362fb`/`17971b7`/`3f0f00d`/`859e19e` resolves (see also #20, #21).
`DEFAULT_GRADUATION_THRESHOLD: u32 = 3` at `src/fail_closed.rs:34` at HEAD ✓
(resolves the "threshold TBD" placeholder as recorded).

### 20. irrevers-8a24ad8d — VERIFIED
Close reason (2026-08-21T02:44:22Z): runtime fail-open/fail-closed
enforcement. Artifacts: `17971b7` landed 2026-08-21T02:44:07Z — 15 s before
close (✓ "15 s"); wires PolicyMode into `src/engine.rs`/`src/health.rs`;
`tests/fail_closed_runtime_tests.rs` **+132** exact. Acceptance matrix at
HEAD at the cited lines: `recovered_guard_crash_denies_in_fail_closed_mode`
:240, `lifecycle_reports_recovered_crash_once` :298 ✓.

### 21. irrevers-019c36d3 — VERIFIED
Close reason (2026-08-21T03:08:53Z): mechanism already on origin/main, no
scoped changes. The inventories' claim that the bead's **own notes** name all
four implementing commits and the exact verification commands re-checks
directly against `bead show`: notes name `bb362fb`, `17971b7`, `3f0f00d`,
`859e19e` and the three commands (`cargo test --lib fail_closed`,
`--test fail_closed_policy_tests`, `--test fail_closed_runtime_tests`). All
four resolve with the claimed subjects; `859e19e` landed 2026-08-21T03:05:52Z
— 3 m 01 s before close (✓ "3 min"). (Note: the forensic close_reason string
alone does not name the commits — the notes do; the inventories attributed it
to the notes, which is correct.)

---

## Corrections found (2, both minor)

1. **irrevers-e2bb8fbf** — "later touches `890429f`, `0fb164d`" (compiled
   inventory §A row; final-inventory.md same): `0fb164d` predates
   `c7e9df5` and is its ancestor — it is an earlier touch, not a later one.
   Suggested wording: "later touch `890429f`; earlier related touch
   `0fb164d`".
2. **irrevers-c87a3c50** — "`src/trust_pointer.rs` first appears ~9.5 h
   later in `97b8eba`" (both docs): actual gap from close
   (2026-08-14T14:37:11Z) to `97b8eba` (2026-08-15T03:04:19Z) is **~12.4 h**.

Neither correction changes any classification or rating; both are recorded
here rather than edited into the earlier docs (they are prior passes'
deliverables of record).

## Unverifiable items

None new. The two entries whose closing evidence is not re-checkable from
git (irrevers-84b36e47's TTL-reaped Argo run; irrevers-340ae322's registry
manifest + CI pods) were already classified and caveated by the inventories,
and in both cases the surviving corroborating object re-verified live
(GitHub release v0.1.1; git-side 0.1.0 pins).

## Roll-up

21 beads: **19 verified, 2 corrected, 0 unverifiable.** Checks performed:
31 in-repo SHAs resolved + scoped; 4 cross-repo SHAs resolved; 7 tags
resolved at recorded SHAs; the close-instant timing claim recomputed for
every bead whose citation states one (~20; all match, s-for-s where the docs
gave seconds — except the two corrections noted); 3 commit-body ID citations
confirmed; 16 test-function anchors + 3 code anchors
(`src/trust_pointer.rs:102`, `:127`, `src/fail_closed.rs:34`) confirmed at
HEAD; 2 live external objects re-checked.
