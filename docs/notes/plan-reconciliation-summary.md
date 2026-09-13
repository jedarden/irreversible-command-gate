# Plan-reconciliation summary — consolidated package for the downstream plan.md edit

Deliverable for **irrevers-cd5c5878** (child 4 of 4 in the split of
irrevers-09ea6d91, under umbrella irrevers-f1063fef). Written 2026-09-13 at
repo HEAD `f7b99c8`.

**Scope: research only.** This pass makes no edit to `docs/plan/plan.md`
(untouched since `a813b7b`, 2026-09-12 04:36:20 −0400 — verified via
`git log 19e8b4e..HEAD -- docs/plan/plan.md` = empty, so every line
reference below, child 3's, and the source package's are against the same
file as it stands at HEAD). The actual edit is downstream:
**irrevers-7ea2d082**.

**Method: verify-and-materialize, not re-derive.** The consolidated package
already exists in the closed bead **irrevers-83f3670e** (notes: §1
releases, §2 closed-bead evidence, §3 stale claims, §4 must-not-change
D1–D13, §5 checked-accurate). This document materializes it as the file the
downstream editor consumes, refreshing every fact that could have drifted
and adjudicating the bead-state deltas since 2026-09-12. Nothing was
re-derived or re-adjudicated. Everything below marked "live" was re-checked
today (2026-09-13) by this pass: `gh release list`, `git ls-remote --tags
origin`, `Cargo.toml`, `README.md`, `bead show` ×25, `bead list --status
open`, and spot-checks of the plan.md line anchors.

## 1. The four evidence deliverables

| Part | Deliverable (path) | Bead | Establishes |
|---|---|---|---|
| 1 — shipped releases | `docs/notes/shipped-releases-inventory.md` | irrevers-97e30af1 (Closed) | Every tag and GitHub Release, with dates, assets, and the cutting mechanism |
| 2 — closed-bead evidence | `docs/notes/closed-beads-reconciliation-2026-09-12.md` incl. the 2026-09-13 addendum | irrevers-319a2f7c (Closed) | The 42-bead closed inventory is complete and current; irrevers-84b36e47's closing evidence; anchors each reconciliation parenthetical in plan.md |
| 3 — stale claims | `docs/notes/plan-stale-claims-audit.md` | irrevers-b7b763a9 (Closed) | Claims the shipped evidence contradicts (A1–A9, with line refs) and claims verified accurate (B1–B8) |
| 4 — this document | `docs/notes/plan-reconciliation-summary.md` | irrevers-cd5c5878 | Must-not-change list with rationale; consolidated package; deltas since the source package |

Underlying child 2 artifacts, still authoritative for per-bead evidence:
`docs/notes/closed-beads-compiled-inventory.md` and
`docs/notes/closed-beads-release-verification-inventory.md`, plus
`docs/notes/evidence/*`.

## 2. Shipped-release inventory (child 1; re-verified live 2026-09-13)

- **62 tags** `v0.1.0`–`v0.1.61` on Forgejo origin (`git ls-remote --tags
  origin`, count 62), **61 published GitHub Releases** `v0.1.1`–`v0.1.61`
  on the mirror; only `v0.1.0` lacks a Release. **Latest: `v0.1.61`,
  published 2026-09-13T06:56:38Z** (`gh release list`, this pass).
  `Cargo.toml` is `0.1.61` — version, tag, and Release agree.
- **First verified complete release: `v0.1.1`** — non-draft, four assets
  (`icg`, `icg-packs.tar.gz`, `pack-manifest.json`, `rule-pack.json`),
  published 2026-09-06T13:06:02Z.
- **Mechanism:** releases are cut by CI, not by hand — `build-and-release`
  reads the version from `Cargo.toml` and, if no published Release carries
  that tag, runs the Layer 1 gates, tags Forgejo, builds artifacts, and
  calls `gh release create` itself
  (`docs/runbooks/release-cutting.md:5-10`). The act that cuts a release is
  merging a reviewed version bump to `main` (runbook lines 12-27); manual
  `gh release create` is only a documented fallback (lines 29-31, 96-142).
- **Caveat to carry into the edit (child 3, A3 nuance, still current at
  this pass):** two post-`v0.1.61` runs failed at `build-and-release` on
  2026-09-13 — `icg-ci-pgntg` (07:33:16Z) and `icg-ci-czq62` (07:48:59Z),
  both `main: Error (exit code 101)`. That is a separate, uninvestigated
  live pipeline failure; it does not undo the 61 successful cuts, and a
  corrected Phase 0 must not present either the old or the new failure as
  evidence that a release proof is missing.
- Historical note: irrevers-eff8909f's earlier "zero GitHub Releases" line
  was true against origin only and is superseded for the mirror.

## 3. Closed-bead inventory currency and irrevers-84b36e47 (child 2)

- The **42-bead closed inventory stands**: both delta passes found
  additions: none, removals: none; fresh close dates match the compiled
  inventory to the minute for all 42. The 2026-09-13 addendum adjudicated
  the 34-bead closure wave after 2026-09-12T01:15Z (families: meta,
  workflows-guard umbrella, Job/CronJob detection-coverage family,
  CI-run-sense gates, lock-lineage verify tail, pack-rule content,
  artifact-dir checker fix, lab trial) — none qualifies for the inventory.
- **irrevers-84b36e47** ("Verify icg-ci produces a real, complete GitHub
  release"): **Closed, revision 59**. Final close 2026-09-06T13:06:27Z —
  25 s after Release `v0.1.1` published (13:06:02Z); close reason cites
  run `icg-ci-manual-snxhj` reaching Succeeded with `isDraft=false` and
  all four assets. Live re-verified 2026-09-13 (`gh release view v0.1.1`):
  isDraft false, exactly the four assets. Its "reproduces on every later
  release" note is confirmed by child 1's spot-checks (v0.1.57, v0.1.61).
  **The closure matches the reality of shipped releases.**

## 4. Stale claims to correct (child 3 — full text and method there)

| # | plan.md (lines at HEAD) | Claim | Corrected fact |
|---|---|---|---|
| A1 | :457-458 | "no release has ever been cut" | 62 tags / 61 Releases exist; v0.1.61 is Latest |
| A2 | :458-459 | "`gh release list` … returns empty" | The command returns 61 published Releases |
| A3 | :459-460 | "icg-ci runs are currently failing at `build-and-release`" | The release step succeeded for all 61 cuts (but see the new §2 caveat — do not replace it with "CI is all-green" either) |
| A4 | :460-461 | "`irrevers-84b36e47` (in progress)" | Closed rev 59 on 2026-09-06, 25 s after v0.1.1 published |
| A5 | :461-462 | "the genesis bead's only remaining blocker" | No open genesis exists (all five genesis beads Closed); replace with the concrete open-work picture (§6 here) |
| A6 | :462-464 | "nothing downstream should cite 'a released artifact' as existing yet" | Released artifacts with the four-asset set exist, 61 times over |
| A7 | :560-564 | "a human manually runs `gh release create`" (resolved-trigger bullet) | Superseded by the CI auto-cut; reconcile to :474-477 + the runbook (plan.md is internally inconsistent here) |
| A8 | :789-792 | "the remaining caveat is release completeness, not deploy location" | Release-completeness half resolved (61 cuts); README.md:167 still freezes "`v0.1.4` is the current release" (re-checked today) — deploy-location half remains correct (B6) |
| A9 | :78-92 | Fail-closed "Leading hypothesis, **not yet confirmed** … Needs verifying … before `irrevers-cd3f4c44` is implemented" | Verified and **REFUTED** by closed irrevers-0e30c682 (2026-08-15): both harnesses fail open by default; irrevers-cd3f4c44 then shipped icg's own machinery (`src/fail_closed.rs`, `DEFAULT_GRADUATION_THRESHOLD = 3` at `src/fail_closed.rs:34`, crash detection in `src/health.rs`, operator-only `icg policy reconcile`) and is Closed (2026-08-21). Downstream consumer: irrevers-4e649dbf |

## 5. MUST-NOT-CHANGE list (this child)

Statuses below are live as of 2026-09-13 (this pass). Two kinds of
protection, per the source package: **(A)** claims anchored to still-open
beads — preserve as-is; do not upgrade to "done/verified" while the bead is
open, and do not weaken the shipped-design text either; **(B)** historical
record — never rewrite; append new facts, never edit what happened.

### A. Anchored to still-open beads

**D1 — Deploy-location trust model, plan.md:172-198** (root:root
binary/packs/trust-pointer; sudo-gated `icg update`; release-gating
rationale). The design shipped (`a03a7e6`) and stands; it must not be
strengthened to "verified in CI / deployed hosts" while the CI-image
defect chain is open: **irrevers-beee1069** (Open P1 —
`argo-guarded-builder` ships `/etc/icg` world-writable, "defeating the
trust model in every CI pod") and **irrevers-c36bba27** (Open P1 —
publish builder 0.1.1 and pin every icg workflow template to it).
*Delta since the source package:* **irrevers-f839b213 is now Closed**
(2026-09-13T03:45Z — root detection by `geteuid`, commit `d88da13`,
ancestor of v0.1.59). That fixed a checker-internal bug; it does not
touch the image defect, so D1's anchors are beee1069 + c36bba27 and the
protection is unchanged. Host-path claims verified accurate (B6 below);
the scoped CI-image caveat must not be used to "correct" them, and no
wording should imply host claims cover CI pods.

**D2 — Deployment-status reality** (plan.md nowhere claims installation on
a real host). Must not gain such a claim while **irrevers-6b4ded56**
(Open P1 — "Install icg on ex44 per the plan's deploy path and register
both PreToolUse hooks") is open. *Delta:* **irrevers-19835ba1 is now
Closed** — lab practice-mode install of v0.1.3 (`--practice --hook
--agent-user coding`, 2026-09-06; root:root layout verified on lab;
would-deny data collection). That is **practice mode**, not the
production install + hook registration 6b4ded56 tracks: any new
deployment prose must keep the two distinct, and plan.md still must not
claim production installation while 6b4ded56 is open. *Execution-time
note for whoever runs 6b4ded56:* its title names host `ex44`, which was
decommissioned 2026-08-30 and replaced by this box (`codinghome`) —
reconcile the target at execution; that is a bead-title issue, not a
plan.md edit.

**D3 — Fail-closed framing, plan.md:68-76** (two failure classes;
in-process errors fail open unconditionally; process-death governed by
the graduated policy). Verified accurate at HEAD (child 3:
`tests/fail_open_tests.rs` `layer_one_malformed_stdin_fails_open` +
`layer_one_corrupt_rule_pack_fails_open`). Do not reword the split; do
not declare the policy "validated"/"graduated": **the threshold-3
graduation has not been consumed**, and the lock/policy lineage is still
open at its umbrella **irrevers-92e6e55c** (Open P2), plus
**irrevers-91694e78** (Open P2 — artifact-dir security violations as
crash evidence; implementation commit `90ce0fa` landed, bead open) and
**irrevers-557e4451** (Open P2 — build-time mode assertion). *Delta:*
the lineage's verify/document tail — irrevers-8d1f79a7, 10fa65df,
31042f1e (closed 2026-09-12T16:21–17:26Z) and df1fbcc6 — is now Closed,
which strengthens the mechanism story but does not graduate the policy.

**D4 — Coexistence / org-rule-guard shrink narrative, plan.md:9-10 and
:25** ("shrink toward deprecation … per user direction 2026-08-13";
"kubectl-only rump plus the Write/Edit credential-value rule") and
**plan.md:813-821** (smoke-test success criterion: pass on consistent
verdicts, fail only on divergent ones). Must not be rewritten to
"deprecated" / "coexistence ended": the credential-value rule still has
no absorbed channel (B2 below), and the absorption-verification tail is
open — **irrevers-efe57f54** (Open P1 — absorb the remaining workflow and
Job/CronJob content rules), **irrevers-b559d088** (Open P1 — Job/CronJob
hook-predicate umbrella), **irrevers-5117dc94** (Open P1 — fixed
regression-suite cases), **irrevers-466fe313** (Open P1 — coexistence
tests + docs state the absorbed rump). *Delta:* **irrevers-b8938f41 is
now Closed** (2026-09-12) — and closed *with passing verification*
(rust-verify `…-pqwqs` Succeeded at `19e8b4e` = origin/main tip: 695
passed / 0 failed / 1 ignored across 62 test binaries, guard suites
present), which retires the source package's "verification-failed"
caveat on that one bead. The tail is still open, so the protection
stands; the closing evidence may be cited as progress, not completion.

**D5 — Overview absorption claim, plan.md:19-26** ("both have since been
absorbed … (2026-09)"). Preserve verbatim for now. The engine guards ARE
shipped and released (`src/github_workflows.rs`, `src/job_cronjob_yaml.rs`;
wired in `src/engine.rs`; releases through v0.1.61), so reverting would be
wrong — but editing to claim full verification would outrun the open D4
chain. Touch only after that chain closes.

**D6 — kubectl exclusion:** plan.md:14 (`permanently excluded`), :110
(deliberately not a pack), :831 (`Explicitly not attempted`). Standing
design decision with a permanent zero-I/O-determinism rationale. No bead
will ever close it; no correction will ever make it stale.

### B. Historical record — append, never rewrite

**D7 — icg-ci sensor debugging narrative, plan.md:466-505** (trigger
added 2026-08-22; four pushes, three independent failures: header name,
parent key, submit failure; stripped to needle-ci shape). Includes the
live fleet-wide caveat "every other `*-sensor.yml` … likely has the
identical header-key bug and has never actually fired. Fixed for
`icg-ci-sensor.yml` only; not audited fleet-wide" — an **unresolved
finding with no tracking bead**; a doc edit must not resolve it in prose
and must not silently drop it.

**D8 — commit-without-pathspec incident, plan.md:632-649** (commitgraph
2026-08-14, worker claude-code-glm-5-adr018, bead cg-194i4a, ~430
unrelated lines from a concurrent worker; self-corrected by luck), plus
the global-vs-shared-repo scoping rationale that follows it.

**D9 — public-repo/mirror decision, plan.md:506-519** (2026-08-22 user
decision; exposure audit; trigger/clone stay on Forgejo, only the
artifact goes to GitHub).

**D10 — openbao pack shipping record, plan.md:121-155** (shipped in
`01b5cf1` the same day `~/CLAUDE.md`'s agent-write policy reversed; three
rules; the bead-graph gap — rules 1 & 3 shipped without a dedicated bead —
is a historical process fact, not a doc defect to repair).

**D11 — decision provenance stamps:** "resolved 2026-08-13" / "resolved
2026-08-17" markers (:333, :560-580, :840-856), "per user direction
2026-08-13" (:9-10), the 2026-08-14 bf→bead-rs cutover dates (:157-170,
:610-612, :653-656, :674-681), and the "(Reconciled 2026-08-25 …)" phase
stamps (:446, :597, :672, :699, :728, :780).

**D12 — threat-model scoping, plan.md:37-44** (honest-fallible-agent
boundary; the explicit oversell prohibition). Only the S2' light touch
(below) is sanctioned there; the framing stays.

**D13 — Codex coverage gap and channel caveats:** plan.md:46-50
(OpenAI-hosted Codex unreachable — still true; nothing shipped changes
it), plan.md:708-716 + :290-296 (`additionalContext` Claude-Code-only;
Codex accepts-but-does-not-honor, per
`docs/notes/multi-harness-integration.md`). Keep unless re-verified
against current Codex hook docs; no closed bead contradicts them.
(Distinct from A9: irrevers-0e30c682's timeout/error findings feed A9,
not these.)

## 6. Current open-work picture (for the A5 replacement text)

Live `bead list --status open` at this pass: **18 open beads**, plus this
bead in_progress (child 3's count of 19 is the same set with this bead
still unclaimed). By family:

- *Plan reconciliation itself:* irrevers-f1063fef (umbrella),
  irrevers-09ea6d91 (the audit split), irrevers-7ea2d082 (downstream
  plan.md edit; its title freezes the range at "v0.1.0-v0.1.6" — the
  corrected facts span to v0.1.61), irrevers-4e649dbf (fail-open/fail-closed
  docs), irrevers-cd5c5878 (this bead), irrevers-a06a470c (doc-consistency
  check), irrevers-25bafcf7 (post-reconciliation verification).
- *Builder-image trust defect:* irrevers-beee1069, irrevers-c36bba27,
  irrevers-557e4451, irrevers-91694e78.
- *Absorption tail:* irrevers-efe57f54, irrevers-b559d088,
  irrevers-5117dc94, irrevers-466fe313.
- *Fail-closed policy lock lineage:* irrevers-92e6e55c (umbrella).
- *Deploy:* irrevers-6b4ded56.
- *Unrelated defect/ops:* irrevers-04de9cac (integration tests write crash
  records into the production `/var/cache/icg/health-state.json`),
  irrevers-3e313b79 (lexer `$( )` recursion).

## 7. Checked and still accurate — do not flag

From child 3 §B and the source package §5; none of these may be
"corrected" by the downstream edit:

- **B1** Overview absorption wiring (guards built-in, no such pack files;
  `pack_id` attributions only).
- **B2** Write/Edit credential-value rule has no absorbed channel
  (`packs/secrets.json` `tool_keywords: []`/`applies_to: []`;
  `src/engine.rs:590` names exactly storage-class + image-tag as
  content-mode packs).
- **B3** Docker wrapper coverage "once Phase 4's irrevers-54d477dd pack
  ships (not before)" (:204-207) — the condition is now satisfied, making
  the sentence historical-conditional; at most note the "once" happened.
- **B4** Codex hook maturity "~5 months old as of this writing" (:216-218)
  — date-anchored prose; leave as-is.
- **B5** The release-gating *premise* of the self-updater (:189-192,
  :588-591) — survives A7 in superseded form (gate now = reviewed
  version-bump merge); only the A7 bullet needs the correction.
- **B6** Deploy-location host claims (:172-198, :454-456) — accurate for
  the guarded host (irrevers-93baa29a, closed 2026-09-07); the CI-image
  defect (D1) does not contradict them.
- **B7** Phase 0/1/3/4/5 reconciliation bead citations (:446-455, :597-611,
  :699-706, :728-744, :780-792) — covered by the 42-bead inventory and its
  delta passes; nothing invalidated.
- **B8** "No standing daemon" framing (:79-82, :333-337) — consistent with
  what shipped after the A9 refutation.
- The two-failure-classes split (:68-76) and fail-open-while-unproven
  wording (:72-73); Phase 0's reconciled ship-claims (runbook on disk;
  icg-ci template + push sensor live; trust pointer + `icg update`
  shipped); plan.md:41-44 threat-model conditional — literally true, its
  condition now met (**S2'**): may be touched only to note the condition
  is met, nothing more.
- The four Open Questions (:838-855) are struck through as resolved and
  the resolutions match shipped evidence.

## 8. Application notes for irrevers-7ea2d082

1. Replace plan.md:456-463 with the corrected release record (§2 here)
   and the concrete open-work picture (§6 here) — not with a new
   singular "only remaining blocker".
2. Reconcile the resolved-trigger bullet (:560-564) to :474-477 + the
   runbook (A7); keep the gate premise (B5) pointing at where the gate
   now lives.
3. When rewriting the fail-closed hypothesis (:78-92), respect the
   phrasing tension child 3 records: design/operator docs read as if
   harness deny-on-failure config is generally available; per
   irrevers-0e30c682 it is not (Claude Code: none; Codex: opt-in only).
4. Do not touch anything in §5 (must-not-change) or §7 (checked
   accurate); D7's fleet-wide sensor caveat must survive any edit.
5. The new post-v0.1.61 `build-and-release` failures (§2 caveat) are
   uninvestigated — neither describe CI as all-green nor revive the
   "releases missing" claim; state both facts if Phase 0 mentions CI
   state at all.
6. Corrected facts span releases v0.1.1–**v0.1.61**; if the edit freezes
   any range, freeze the current one.

## Ready statement

**Ready for the downstream plan.md edit (irrevers-7ea2d082).** All four
evidence deliverables are on disk at the paths in §1, every fact in this
package was live-re-verified on 2026-09-13 at HEAD `f7b99c8`, and the
bead-state deltas since the source package are adjudicated above. No
evidence needs re-deriving to apply the corrections; the edit itself is
the only remaining step in this reconciliation.
