# Reconciliation — fresh enumeration vs the two closed-bead inventory docs

Deliverable for **irrevers-3828115c** (child 1 of the irrevers-2c3b4637
auto-split). Enumerated fresh on **2026-09-12T01:15Z** at repo HEAD `8baee83`.

Neither inventory doc was edited by this pass; this file only states the fresh
candidate list and the explicit delta against:

- `docs/notes/closed-beads-release-verification-inventory.md` — second-pass
  enumeration (irrevers-5dfe499e, 2026-09-11), **42 beads**.
- `docs/notes/closed-beads-compiled-inventory.md` — compiled evidence
  deliverable (irrevers-d0a73a21, 2026-09-12, HEAD `e669d14`), **42 beads**.

Evidence summaries and ratings are **not** duplicated here; they remain
authoritative in the compiled inventory. This pass is enumeration +
reconciliation only.

## Selection rule (reproducible)

1. **Universe** — `bead list --status closed --limit 500 --json` (JSONL, one
   object per line): **363 closed beads** at 2026-09-12T01:15Z.
2. **Keyword filter** — case-insensitive substring over `title + description`
   for exactly the six task keywords:
   `release`, `verification`, `fail-closed`, `harness`, `gate`, `evidence`
   → **163 raw matches**.
   - Note: the bare word `verification` does not stem, so `verify`/`verified`
     titles match only via another keyword. Adding a `verif` stem would add 20
     raw matches; none of them is inventory material (each is starvation noise,
     pack-rule content, or a bead doc1 already explicitly excluded, e.g.
     irrevers-0e11e6e6, irrevers-63087cdd), so the stem was **not** added.
3. **Explicit inclusion** — irrevers-84b36e47 pinned per the task; it also
   matches the rule directly via "release".
4. **Curation** — a raw match is a candidate iff it is about (a) release
   verification / release process / distribution integrity, (b) the fail-closed
   harness / policy-enforcement mechanism, or (c) the Layer-1 CI gate /
   test-harness infrastructure in icg-ci. The exclusion families applied are
   doc1's own borderline rules (§ "Borderline beads — explicit exclusion
   reasoning"); the full adjudication is below.
5. **Close dates** — the **last `closed` snapshot per bead in
   `.beads/checkpoint/forensic.jsonl`** (`base_status == "closed"`, max
   `closed_at`), not `updated_at` — same method and rationale as doc1 (the
   2026-09-11 evidence pass bulk-touched every inventory bead after closure).

**Candidate list = the same 42 beads both docs already enumerate**: 40 come
from the keyword rule directly; 2 are inherited from the docs' broader filter
(see delta). No new qualifying closure exists.

## Enumerated candidate list (42; close dates freshly computed)

### A — Release verification / release process / distribution integrity (16)

| ID | Title | Close (UTC) |
|---|---|---|
| irrevers-84b36e47 | Verify icg-ci produces a real, complete GitHub release | 2026-09-06 13:06 |
| irrevers-e77615c8 | icg-ci: publish the rule-pack artifact as a release asset | 2026-08-24 04:27 |
| irrevers-37eb1100 | Release-cutting runbook | 2026-08-15 13:57 |
| irrevers-340ae322 | Prove ronaldraygun/argo-guarded-builder:0.1.0 is published and pullable by icg-ci | 2026-08-30 01:56 |
| irrevers-e2bb8fbf | Add --channel to icg trust to match icg update (canary rollout) | 2026-08-30 12:19 |
| irrevers-6de781f4 | Canary rollout via NEEDLE --identifier | 2026-08-15 05:16 |
| irrevers-b6579270 | Per-release deny-rate telemetry and rolling baseline | 2026-08-21 00:54 |
| irrevers-eff8909f | Write inventory of shipped releases v0.1.0-v0.1.6 with dates and commits | 2026-09-10 10:30 |
| irrevers-2cb3dbd2 | Gate the actual modular release packs in icg-ci instead of static fixtures | 2026-08-26 04:03 |
| irrevers-c87a3c50 | Base self-updater and trust pointer (icg update) | 2026-08-14 14:37 |
| irrevers-5fdc2e13 | Trust pointer mechanism | 2026-08-15 03:04 |
| irrevers-f59f9313 | icg update: self-updater command | 2026-08-15 03:26 |
| irrevers-96594031 | Migrate the shipped trust-pointer and rule-pack artifact paths off agent-writable locations | 2026-08-26 01:39 |
| irrevers-ca79d63a | Deploy the binary, rule-pack artifact and trust pointer outside the guarded agent's writable filesystem | 2026-08-23 01:16 |
| irrevers-e00a5381 | icg-ci Argo WorkflowTemplate | 2026-08-15 02:02 |
| irrevers-075634b8 | Make icg update atomically deploy the complete modular production pack directory | 2026-08-26 02:55 |

### B — Fail-closed harness / policy enforcement behavior (21)

| ID | Title | Close (UTC) |
|---|---|---|
| irrevers-8d2d4a73 | Engine: unconditional fail-open on parse failure or exception | 2026-08-15 13:22 |
| irrevers-aab3854c | Design fail-closed transition state machine and graduation criteria | 2026-08-16 03:03 |
| irrevers-cd3f4c44 | Graduated fail-open to fail-closed policy for guard crashes | 2026-08-21 01:25 |
| irrevers-8a24ad8d | Implement fail-open baseline and fail-closed enforcement modes | 2026-08-21 02:44 |
| irrevers-019c36d3 | Implement fail-closed policy transition mechanism | 2026-08-21 03:08 |
| irrevers-fffef435 | Integrate with poison-pill mechanism for automatic graduation | 2026-08-21 02:53 |
| irrevers-0f49129d | Poison-pill auto-rollback | 2026-08-15 03:11 |
| irrevers-ff4f17da | Poison-pill auto-rollback: revert the trust pointer on a deny-rate spike | 2026-08-21 01:05 |
| irrevers-3fc4bdde | Apply the documented ICG_DISABLED emergency bypass to hook and PATH-wrapper enforcement | 2026-08-26 02:19 |
| irrevers-f891f555 | Make the fail-closed policy read path lock-free for guarded invocations | 2026-09-08 02:06 |
| irrevers-edb5c4ca | Stop reconciling the fail-closed policy from the hook and wrapper paths | 2026-09-08 02:07 |
| irrevers-9eb4de16 | Add regression tests for a root-owned policy directory on the hook path | 2026-09-08 02:07 |
| irrevers-93baa29a | Verify icg policy status and reconcile as root on an installed host layout | 2026-09-08 03:21 |
| irrevers-3e6c6fde | Hook and wrapper guarded paths take the fail-closed policy lock via crash recovery | 2026-09-08 05:37 |
| irrevers-50077acb | Set explicit root-owned 0755 modes on the icg trust directories in argo-guarded-builder | 2026-09-08 05:42 |
| irrevers-ffdc924b | Add guard health tracking and crash monitoring infrastructure | 2026-08-21 02:33 |
| irrevers-9007792b | Operational monitoring and alerting infrastructure | 2026-08-21 03:28 |
| irrevers-0d710c9a | Write activation documentation and operational runbooks | 2026-08-21 03:06 |
| irrevers-1517a263 | cargo test writes into the production denial log on an instrumented host | 2026-09-07 20:31 |
| irrevers-0aa08f4e | Routine run_started lifecycle telemetry prints to stderr on every guarded invocation | 2026-09-08 02:10 |
| irrevers-49dbb095 | Wire fail-open boundary around the hook predicate pipeline | 2026-09-11 17:31 |

### C — CI gate / test-harness infrastructure (5)

| ID | Title | Close (UTC) |
|---|---|---|
| irrevers-b4b37bf0 | Layer 1: regression-suite CI gate | 2026-08-15 03:40 |
| irrevers-b0a453b2 | Layer 1: verify the regression-suite gate actually fails the build | 2026-08-15 13:33 |
| irrevers-f61efd80 | Layer 1: coverage-diff CI gate | 2026-08-15 03:40 |
| irrevers-29a9131c | Layer 1: verify the coverage-diff gate actually blocks an unjustified change | 2026-08-15 13:48 |
| irrevers-ed77224f | End-to-end integration testing for icg-ci workflow | 2026-08-16 19:07 |

All 42 were re-confirmed `closed` in the live list at enumeration time. Fresh
close dates match the compiled inventory's dates **to the minute for all 42**
(zero mismatches). Titles are quoted from the live bead records; the compiled
inventory has two cosmetic wording variants (eff8909f: en-dash "v0.1.0–v0.1.6";
0f49129d: "(original bundled)" suffix) — no substance.

## Delta vs `closed-beads-release-verification-inventory.md` (42)

**Additions: none.** No closed bead outside its 42 qualifies under the
curation rule:

- Its 359-bead universe grew to 363, but the +4 are irrevers-5dfe499e (the
  second-pass bead itself), irrevers-4bc81065, irrevers-fdc1cd74 and
  irrevers-d0a73a21 — all evidence-pipeline meta-beads, excluded under doc1's
  own meta-bead rule.
- Every bead it closed after irrevers-49dbb095 (2026-09-11T17:31Z) and before
  its snapshot was already adjudicated by it: workflows-guard verification /
  coverage (irrevers-96cc00fe, ed0f35d4, f2d5875a, be464cd7, a39bdf35, …) and
  evidence-pipeline meta (irrevers-c52de1f2, f58a53ca, fa1b03f9, …).
- Nothing closed after the compiled inventory froze
  (irrevers-d0a73a21, 2026-09-12T00:37:11Z) at all.

**Removals: none.** All 42 remain closed and in scope. Two rule-coverage
notes — under-selection by the task's six keywords, **not** doc defects; both
beads stay in the candidate list:

- **irrevers-96594031** — caught by doc1's broader filter via "artifact"
  (trust-pointer/rule-pack placement); none of the six keywords appears in its
  title/description. Retained: release-distribution integrity.
- **irrevers-e2bb8fbf** — caught via "canary"/"rollout"/"icg update"; not by
  the six keywords. Retained: canary release-channel mechanism.

## Delta vs `closed-beads-compiled-inventory.md` (42)

**Additions: none; removals: none.** Its 42 table rows are set-identical to
doc1's (verified by parsing both). It postdates doc1 and inherits the same set;
its own § "Reconciliation against the prior compiled inventory" already records
the only two content changes of record (irrevers-49dbb095 added;
irrevers-eff8909f re-rated). Since its compile at HEAD `e669d14`, product code
drift is `Cargo.toml`/`Cargo.lock` only (v0.1.46 version bump → HEAD
`8baee83`), so nothing in its per-bead evidence citations is invalidated.

## Raw-match adjudication (163 − 40 direct = 123 excluded)

Excluded raw matches, by family (doc1's borderline rules, applied verbatim;
full ID lists so the partition is auditable):

1. **Evidence-pipeline meta-beads (39)** — process beads about producing or
   verifying this very inventory: irrevers-622aae24, irrevers-5dfe499e,
   irrevers-f7a52307, irrevers-4bc81065, irrevers-fdc1cd74, irrevers-d0a73a21,
   irrevers-c52de1f2, irrevers-f58a53ca, irrevers-fa1b03f9, irrevers-fc3a499d,
   irrevers-3733444a, irrevers-423f1be6, irrevers-2f2c25b5, irrevers-f574a666,
   irrevers-111f3abb, irrevers-083f4796, irrevers-2a2c9dd5, irrevers-e942e828,
   irrevers-e3591eec, irrevers-b54f645b, irrevers-8d6e1589, irrevers-724f465d,
   irrevers-552eca96, irrevers-ac90fa0a, irrevers-02651160, irrevers-6e66a908,
   irrevers-6e7896b1, irrevers-cf8d92ad, irrevers-e29e74b6, irrevers-2e6ddeb3,
   irrevers-67412329, irrevers-58a52917, irrevers-5948d5b0, irrevers-61ad3e1e,
   irrevers-90baf605, irrevers-d8ee88ce, irrevers-e1ac3654, irrevers-8176471b,
   irrevers-d364e844.
2. **Workflows-guard detection-coverage family (14)** — enforcement coverage
   of the .github/workflows deny rule and its verification, tracked under its
   own umbrella irrevers-e58ddf25: irrevers-a1d7192b, irrevers-9f3b35e9,
   irrevers-be5b5b6f, irrevers-ed04668c, irrevers-ed0f35d4, irrevers-96cc00fe,
   irrevers-7f27bff5, irrevers-2d8a70a5, irrevers-6da41176, irrevers-a39bdf35,
   irrevers-3218ad7f, irrevers-9479a470, irrevers-f2d5875a, irrevers-be464cd7
   (the last is "run CI on a change", which doc1 explicitly excludes as a
   different sense of "CI gate").
3. **Starvation-alert / [Unravel] noise (20)** — irrevers-1f896498,
   2736f14b, 3426e1aa, 5088e02a, 60754e7c, 6e0310c5, 75697959, 8d294b35,
   9bf20b48, b0a2f499, d5554b2c, e118101a, f5a58095, c06c1b5d, d2230fad,
   59a4f2a9, 30b6cb76, 36df2884, e2df5eb8, ed36f5be (all `irrevers-`-prefixed).
4. **Duplicate lineages / fixture batches of already-listed gates (5)** —
   counted once under canonical IDs per doc1: irrevers-248fca69,
   irrevers-10ddba79 (coverage-diff wiring and report format = f61efd80's
   scope), irrevers-ac10bfe6, irrevers-8466fec9 (coverage-diff fixture
   batches), irrevers-d3184c55 (regression-suite curation batch).
5. **Other out-of-scope (45)** — engine/detection internals (lexer heredoc
   e8359267, safe_patterns precedence 9546468c, rule-pack schema 40114159 /
   b25f2dd8, per-rule enable/disable 012be0c8), pack-rule content (beads-pack
   predicate 0c6358a4, bead-CLI deprecation 480aa9c5 / 692a56c3,
   git-commit-without-pathspec 9ef830ec, beads-shared-checkout-write
   ff094e1f), hook front-end response features (updatedInput 65aadcff /
   87978118 / 886002b2, hook JSON c7ac905f, additionalContext a0ced256 /
   d7007aec, Codex adapter 402cf2ec / 8b5faeb9, apply_patch normalize
   5d080721, redirect dispatch ed36e484), feature/ops work (signed override
   e354aca2, practice mode 195d05cc, status --trend ad7b635e, telemetry stats
   e073104f, guard CI pods 36244640, watchdog feasibility 0e30c682), scenario/
   coverage housekeeping (05207cfa, d0226630, 754fa0a1, c4c5287e, aa1b828d,
   456a8c3f), repo hygiene/docs (bcffaf6d, 12d2f9b2, 46eadc57, d16ae96a,
   3ecf4e5c, dc1235d4, fff4f0a4), process/review scaffolding (cbb20021), and 5
   duplicate-title "Genesis:
   irreversible-command-gate Implementation" beads (18ce85c1, 33ca07f7,
   9a4f4caf, 9a64775d, ae8fa050).

## Scope guards honored

- Neither inventory doc was edited (no additions, no removals, no rewording).
- `docs/plan/plan.md` untouched (last touch remains `d26a83a`, 2026-09-11,
  per the compiled inventory's check).

## Addendum — delta pass 2026-09-13 (irrevers-319a2f7c)

Delta-only re-run of the selection rule above. Repo HEAD has moved past
`8baee83` (now `c3ab51e`); nothing below edits either inventory doc or any
existing entry in this file.

**Method.** Same rule, same sources: closed universe via
`bead list --status closed --limit 500 --json` (now **397**, was 363), close
dates from the last `closed` snapshot in `forensic.jsonl` cross-checked
against `kind == "closed"` events — the two sources agree to the minute on
every bead. `updated_at` was not used (the 2026-09-11 evidence pass
bulk-touched closed beads).

**Delta: 34 beads closed after 2026-09-12T01:15Z; none qualifies.** The
inventory's 42 remain complete. Adjudication by family (34 IDs, auditable
partition):

1. **Evidence/inventory/plan-reconciliation meta (11)** — the inventory
   pipeline itself and the plan.md stale-claim work, excluded under family 1
   above: irrevers-3828115c, 98e9188e, 25063548, 92165552, fc249ab4,
   2c3b4637, 97e30af1 (child 1 of this split; its deliverable is
   `shipped-releases-inventory.md`), d5ffa7cf, 83f3670e, 3dc8cb4d, b04ff68f.
2. **Workflows-guard umbrella (1)** — irrevers-e58ddf25 itself closed
   2026-09-12T02:33Z. Family 2 above already excludes its 14 children; the
   umbrella joins them.
3. **Job/CronJob detection-coverage family (13)** — enforcement coverage of
   the `kind: Job/CronJob` deny rule and its verification, exact parallel to
   family 2: irrevers-a3af4241, ddd9bd7d, 1ecbc527, 65a037f3, 49bbb22b,
   84228cd5, 714874b7, 8de3e0d1, 599d1bfc, b5e5f88c, 2d1c55b0, b4b5027f,
   5b3376f3.
4. **"Run CI/tests over the change" sense of gate (3)** — be464cd7
   precedent: irrevers-b8938f41, d73064bb, df1fbcc6.
5. **Fail-closed lock-ownership verification tail (3, borderline —
   documented)** — irrevers-8d1f79a7, 10fa65df, 31042f1e, closed
   2026-09-12T16:21–17:26Z. These are topical category (b), but the lineage
   (umbrella irrevers-92e6e55c, **still Open**) is already represented in
   inventory B by five beads closed 2026-09-08: 93baa29a (the root
   policy-command verification), f891f555, edb5c4ca, 9eb4de16, 0aa08f4e.
   The new closes are that lineage's verify/document tail: 10fa65df
   re-verifies at HEAD the lock-free property 3e6c6fde (in B) fixed;
   31042f1e adds ownership-table rows to the deployment guide for the same
   mechanism; 8d1f79a7 is the parent whose verification core is 93baa29a.
   Excluded under family 4 (duplicate lineage, counted once under canonical
   IDs).
6. **Pack-rule content (1)** — irrevers-05855de6 (beads-shared-checkout-write
   over-match fix), ff094e1f precedent from family 5.
7. **Artifact-dir security-check fix (1, borderline — documented)** —
   irrevers-f839b213, closed 2026-09-13T03:45Z: root-detection fix inside
   `verify_artifact_directory_security` (trust_pointer.rs). Its umbrella
   irrevers-beee1069 (**still Open**) is already represented in B by
   50077acb. The bead is internal to the checker's own logic, not artifact
   integrity itself; its only keyword hit ('gate') is the verb "gates" in
   the description. Excluded under families 4/5.
8. **Lab deployment trial (1)** — irrevers-19835ba1, no keyword match, ops
   practice-mode deployment, not release process.

When the two still-open umbrellas (92e6e55c, beee1069) eventually close,
this addendum's reasoning pre-answers their adjudication: both are lineages
already counted in B.

## Anchor bead irrevers-84b36e47 — state and closing evidence (2026-09-13)

- **Status: Closed, revision 59** (`bead show`). Final close event
  2026-09-06T13:06:27Z — 25 s after Release v0.1.1 published at
  2026-09-06T13:06:02Z; close reason cites run `icg-ci-manual-snxhj`
  reaching Succeeded with `isDraft=false` and all four assets. (Two earlier
  2026-08-26 close events precede reopens; rule step 5's max-`closed_at`
  correctly selects the 2026-09-06 one, matching the inventory's date.)
- **Live re-verification 2026-09-13**: `gh release view v0.1.1` returns
  `isDraft: false`, published 2026-09-06T13:06:02Z, exactly four assets —
  `icg` (13,162,400 bytes), `icg-packs.tar.gz`, `pack-manifest.json`,
  `rule-pack.json` — i.e. the done-when criterion (binary + rule-pack
  asset) holds.
- **Cross-check vs child 1** (`docs/notes/shipped-releases-inventory.md`,
  irrevers-97e30af1): consistent. v0.1.1 is the **earliest** GitHub Release
  (v0.1.0 is the sole tag-only version), same publish timestamp to the
  second; 61 Releases v0.1.1–v0.1.61 now exist, Latest v0.1.61
  (2026-09-13T06:56:38Z), with the same four assets reproducing on later
  releases (child 1 spot-checked v0.1.57/v0.1.61) — matching this bead's
  "reproduces on every later release" note. **The closure matches the
  reality of shipped releases**; its evidence classification
  (git-verified at head, 2026-09-11 consolidation) stands.
- **plan.md discrepancy confirmed, not corrected here**: `docs/plan/plan.md`
  lines 457–461 still assert "**One residual, actively tracked: no release
  has ever been cut.** `gh release list` … returns empty …
  `irrevers-84b36e47` (in progress)". Both halves are stale — the bead has
  been Closed since 2026-09-06 and 61 Releases exist. Correcting plan.md is
  the downstream plan-reconciliation children's task (child 1's doc §
  "Observations" supplies the replacement text); this delta pass records
  the state only, per its scope guard.

## Reproduction

```bash
bead list --status closed --limit 500 --json > /tmp/closed.jsonl
python3 - <<'EOF'
import json, re
KWS = ['release', 'verification', 'fail-closed', 'harness', 'gate', 'evidence']
rows = [json.loads(l) for l in open('/tmp/closed.jsonl')]
t = lambda r: ((r.get('title') or '') + '\n' + (r.get('description') or '')).lower()
raw = {r['id'] for r in rows if any(k in t(r) for k in KWS)}
print(len(rows), 'closed;', len(raw), 'raw matches')
EOF
# close dates: last closed snapshot per bead in .beads/checkpoint/forensic.jsonl
# (base_status == "closed", max closed_at) — see "Selection rule", step 5.
```
