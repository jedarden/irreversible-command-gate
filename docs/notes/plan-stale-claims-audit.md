# Stale-claims audit — `docs/plan/plan.md`

Deliverable for **irrevers-b7b763a9** (child 3 of 4 in the split of
irrevers-09ea6d91). Consumes:

- `docs/notes/shipped-releases-inventory.md` (irrevers-97e30af1, child 1)
- `docs/notes/closed-beads-reconciliation-2026-09-12.md` incl. the
  2026-09-13 addendum (irrevers-319a2f7c, child 2)

Audited 2026-09-13 at repo HEAD `f79b01f`; re-verified 2026-09-13 at
`22425b4` (the only audit-cited file that commit touches is
`src/health.rs`, whose crash detection / cgroup OOM classification
survived intact; `docs/plan/plan.md` is still last touched by `a813b7b`,
2026-09-12 04:36:20 −0400, the job-cronjob engine guard) — every line
reference below is against that file as it stands at HEAD. **This pass
makes no edit to `docs/plan/plan.md`**; corrections are the downstream
editing bead's task (irrevers-7ea2d082).

Evidence classes used, in descending strength: live re-verification
(`gh release list`, `gh release view`, Argo workflow status, `bead show`)
on 2026-09-13; the two child artifacts; the closing-evidence files under
`docs/notes/evidence/`; the shipped source tree (`src/`, `packs/`) and
`docs/runbooks/release-cutting.md`.

## Section A — claims the shipped evidence contradicts

Each entry: plan.md location, current text, corrected fact, artifact.

### A1. "no release has ever been cut"

- **Location:** `docs/plan/plan.md:457-458` (Phase 0 reconciliation
  parenthetical, "One residual, actively tracked").
- **Current text:** "**One residual, actively tracked: no release has
  ever been cut.**"
- **Corrected fact:** 62 git tags (`v0.1.0`–`v0.1.61`) and **61 published
  GitHub Releases (`v0.1.1`–`v0.1.61`)** exist; only `v0.1.0` lacks a
  Release; `v0.1.61` is Latest. Re-verified live 2026-09-13:
  `gh release list --repo jedarden/irreversible-command-gate` returns 61
  rows, top `v0.1.61` published `2026-09-13T06:56:38Z` — matching child
  1's inventory row-for-row.
- **Artifact:** `docs/notes/shipped-releases-inventory.md` (§ "The two
  facts, stated separately" + inventory table); live `gh release list`.

### A2. "`gh release list` … returns empty"

- **Location:** `docs/plan/plan.md:458-459`.
- **Current text:** "`gh release list` on
  `jedarden/irreversible-command-gate` returns empty and icg-ci runs are
  …"
- **Corrected fact:** the same command returns 61 published Releases
  (none draft, none prerelease). Distinct claim from A1 — cite both
  halves separately when editing.
- **Artifact:** as A1.

### A3. "icg-ci runs are currently failing at `build-and-release`"

- **Location:** `docs/plan/plan.md:459-460`.
- **Current text:** "… and icg-ci runs are currently failing at
  `build-and-release`".
- **Corrected fact:** stale as support for the "no release" claim. The
  release step **succeeded** for every one of the 61 releases
  (`v0.1.1`, 2026-09-06, through `v0.1.61`, 2026-09-13), each published
  minutes after its tag by the pipeline itself (child 1, § "Observations
  for the plan reconciliation"). **Nuance recorded for accuracy, not
  rehabilitation:** two fresh runs *have* failed at `build-and-release`
  on 2026-09-13 — `icg-ci-pgntg` (07:33:16Z) and `icg-ci-czq62`
  (07:48:59Z), both `main: Error (exit code 101)` — i.e. *after*
  `v0.1.61` published at 06:56:38Z. That is a new, separate live
  pipeline failure (uninvestigated here; runs `icg-ci-677z7` /
  `icg-ci-hfm4q` were Running/Pending at check time), and it does not
  undo 61 successful cuts. A corrected Phase 0 must not present either
  sentence as evidence that a release proof is missing.
- **Artifact:** child 1 inventory; live
  `kubectl --server=http://traefik-iad-ci:8001 get workflow
  icg-ci-pgntg icg-ci-czq62 -n argo-workflows`.

### A4. "`irrevers-84b36e47` (in progress)"

- **Location:** `docs/plan/plan.md:460-461`.
- **Current text:** "`irrevers-84b36e47` (in progress) is the
  verification bead and the genesis bead's only remaining blocker."
- **Corrected fact:** **Closed, revision 59** (live `bead show`,
  2026-09-13). Final close `2026-09-06T13:06:27Z` — 25 s after Release
  `v0.1.1` published (`13:06:02Z`); close reason cites run
  `icg-ci-manual-snxhj` reaching Succeeded with `isDraft=false` and all
  four assets. Live re-check of `v0.1.1` (2026-09-13): `isDraft: false`,
  four assets (`icg`, `icg-packs.tar.gz`, `pack-manifest.json`,
  `rule-pack.json`). (Two earlier 2026-08-26 close events precede
  reopens; the final 2026-09-06 close is the one of record.)
- **Artifact:** `docs/notes/closed-beads-reconciliation-2026-09-12.md`,
  § "Anchor bead irrevers-84b36e47 — state and closing evidence
  (2026-09-13)"; live `bead show irrevers-84b36e47`.

### A5. "… and the genesis bead's only remaining blocker"

- **Location:** `docs/plan/plan.md:461-462` (same sentence as A4).
- **Current text:** "… and the genesis bead's only remaining blocker."
- **Corrected fact:** there is **no open genesis bead and no remaining
  genesis blocker**. All five genesis-titled beads are **Closed**:
  `irrevers-18ce85c1`, `irrevers-33ca07f7`, `irrevers-9a4f4caf`,
  `irrevers-9a64775d`, `irrevers-ae8fa050` (live `bead show`, 2026-09-13).
  The last one's closing notes (irrevers-ae8fa050, investigation
  2026-08-26) describe the pre-release world and were superseded by the
  v0.1.1+ cuts. What actually remains open is **19 open beads** (live
  `bead list --status open`, 2026-09-13), by family:
  - *Plan reconciliation itself (this split's family):*
    irrevers-f1063fef (umbrella, "…v0.1.6 releases…"), irrevers-09ea6d91
    (the audit split this bead belongs to), irrevers-7ea2d082
    (downstream plan.md edit; its title freezes the range at
    "v0.1.0-v0.1.6" — the corrected fact now spans to v0.1.61),
    irrevers-4e649dbf (fail-open/fail-closed docs), irrevers-cd5c5878
    (must-not-change list), irrevers-a06a470c (doc-consistency check),
    irrevers-25bafcf7 (post-reconciliation verification).
  - *Builder-image trust defect:* irrevers-beee1069 (image ships
    `/etc/icg` world-writable — see B6), irrevers-c36bba27 (publish
    builder 0.1.1), irrevers-557e4451 (build-time mode assertion),
    irrevers-91694e78 (artifact-dir violation as fail-closed evidence).
  - *Absorption tail:* irrevers-efe57f54, irrevers-b559d088,
    irrevers-5117dc94, irrevers-466fe313.
  - *Fail-closed policy lock lineage:* irrevers-92e6e55c (umbrella).
  - *Unrelated defect/ops:* irrevers-04de9cac (integration tests write
    crash records into the production
    `/var/cache/icg/health-state.json`), irrevers-3e313b79 (lexer
    `$( )` recursion), irrevers-6b4ded56 ("Install icg on ex44" —
    itself stale: `hetzner-ex44` was decommissioned 2026-08-30 and
    replaced by this box; see `~/CLAUDE.md`).

  Membership re-verified at `22425b4`: irrevers-e23d37cc (telemetry torn
  `.tmp`), open when the audit was first written, was **Closed** (rev 4)
  by `22425b4`; the fix surfaced irrevers-04de9cac, which took its open
  slot — the count stays 19, the names above are the current set.
  A corrected Phase 0 should replace "the genesis bead's only remaining
  blocker" with this concrete open-work picture, not with a new singular
  blocker.
- **Artifact:** live `bead show` per ID; `bead list --status open`;
  `docs/notes/closed-beads-reconciliation-2026-09-12.md` (families 1–8).

### A6. "nothing downstream should cite 'a released artifact' as existing yet"

- **Location:** `docs/plan/plan.md:462-464`.
- **Current text:** "The phase's build deliverables are complete; the
  end-to-end release proof is not, so nothing downstream should cite "a
  released artifact" as existing yet."
- **Corrected fact:** released artifacts exist and carry real assets on
  every release — the four-asset set (`icg` binary, `icg-packs.tar.gz`,
  `pack-manifest.json`, `rule-pack.json`) verified on `v0.1.1`
  (re-verified live 2026-09-13), `v0.1.57`, and `v0.1.61` (child 1, §
  "Artifact evidence"). The end-to-end release proof the sentence demands
  is the thing that happened, 61 times.
- **Artifact:** child 1 inventory; reconciliation addendum anchor section.

### A7. Release-cutting trigger stated as a human-run `gh release create`

- **Location:** `docs/plan/plan.md:560-564` (Phase 0 bullet
  "Release-cutting trigger, resolved (2026-08-13)").
- **Current text:** "**Release-cutting trigger, resolved (2026-08-13):**
  a human manually runs `gh release create` once `icg-ci` has passed on
  the target commit — no additional approval-workflow layer beyond
  that."
- **Corrected fact:** superseded by the shipped mechanism. The runbook
  states it directly: "**Read this first: the release is cut by CI, not
  by hand.**" — `build-and-release` reads the version from `Cargo.toml`
  and, if no published release carries that tag, runs the Layer 1 gates,
  tags Forgejo, builds artifacts, and calls `gh release create` itself
  (`docs/runbooks/release-cutting.md:5-10`, which also says the earlier
  manual-revision model "has not been how it works since the template
  gained its release step", lines 13-18). The act that cuts a release is
  now **merging a reviewed version bump to `main`** (runbook lines
  12-27); manual `gh release create` survives only as a documented
  fallback for "CI produced artifacts but the release was not created"
  (runbook lines 29-31, 96-142). Note plan.md is *internally*
  inconsistent here: its own Trigger subsection (`plan.md:474-477`)
  already describes the version-gated auto-cut; the resolved-trigger
  bullet contradicts it. Downstream editors should reconcile 560-564 to
  match 474-477 + the runbook.
- **Artifact:** `docs/runbooks/release-cutting.md` (lines 5-31);
  child 1 § "Observations" (publish-minutes-after-tag cadence).

### A8. "the remaining caveat is release completeness, not deploy location"

- **Location:** `docs/plan/plan.md:789-792` (Phase 5 reconciliation,
  README "What this does not do" item).
- **Current text:** "… that work landed (`a03a7e6`), and the remaining
  caveat is release completeness, not deploy location."
- **Corrected fact:** the release-completeness caveat is resolved — 61
  releases exist (A1). The sentence is a present-tense characterization
  of README and is now doubly out of date: README's own status paragraph
  has since moved (it currently names "`v0.1.4` is the current release
  (2026-09-08)", `README.md:167`) — itself stale against v0.1.61. Both
  plan.md's characterization and README's freeze point need a pass by
  whoever owns those docs; the deploy-location half of the sentence
  remains correct (see B6).
- **Artifact:** child 1 inventory; `README.md:163-184`; live
  `gh release list`.

### A9. Architecture fail-closed hypothesis: "Leading hypothesis, **not yet confirmed** … Needs verifying against both harnesses' actual hook specs"

- **Location:** `docs/plan/plan.md:78-92` (Architecture, "Open
  implementation question `irrevers-cd3f4c44` doesn't resolve on its
  own"), specifically the hypothesis at lines 83-92.
- **Current text:** "Leading hypothesis, **not yet confirmed**: the
  fail-closed transition doesn't need icg's own watchdog at all if
  Claude Code's and Codex's own PreToolUse hook systems already have
  configurable behavior for 'the hook command errored, timed out, or
  never responded' … Needs verifying against both harnesses' actual hook
  specs before `irrevers-cd3f4c44` is implemented; if neither harness
  supports it, this finalist needs to either accept the standing-daemon
  cost after all or be re-scoped."
- **Corrected fact:** the verification was done and the hypothesis was
  **REFUTED** — by dedicated bead `irrevers-0e30c682` ("Verify Claude
  Code and Codex hook-error/timeout configuration can substitute for a
  watchdog"), **Closed 2026-08-15**, whose notes record: Claude Code
  hook timeout/error → **fails open** (only exit code 2 blocks; HTTP
  errors and connection failures fail open); Codex timeout → **fails
  open by default**, fail-closed only behind opt-in
  (`echo closed > ~/.acp/failmode`). Consequence recorded on the bead:
  the graduated policy cannot rely on harness-native behavior.
  `irrevers-cd3f4c44` was then implemented and **Closed 2026-08-21**
  (rev 4) with icg's own infrastructure, not a harness setting and not a
  standing daemon: `src/fail_closed.rs` (durable `PolicyStore`,
  `DEFAULT_GRADUATION_THRESHOLD = 3` at `src/fail_closed.rs:34`),
  runtime enforcement (`17971b7`), transition audit (`3f0f00d`), and
  crash detection in `src/health.rs` (stale-run-marker-as-crash,
  exit-status classification separating signals from OOM via cgroup
  counters) feeding operator-only `icg policy reconcile`. The
  paragraph's *framing* question ("watchdog cost vs. harness setting vs.
  re-scope") is therefore answered: none of the three — a third shape
  shipped (durable crash evidence + operator reconcile; see
  `docs/design/fail-closed-transition.md:32-33`, which locates the
  fail-closed boundary in "the harness's configured 'deny on hook error
  or timeout' behavior … [or] the wrapper/supervisor contract" plus the
  shipped store).
- **Artifact:** live `bead show irrevers-0e30c682` (research findings +
  REFUTED verdict, with sources);
  `docs/notes/evidence/fail-closed-harness-a.md` (§ irrevers-cd3f4c44)
  and `…-b.md` (§§ irrevers-f891f555, irrevers-3e6c6fde, 
  irrevers-93baa29a — the shipped fail-closed machinery and its host
  verification); `src/fail_closed.rs`, `src/health.rs`. (Downstream
  consumer already exists: irrevers-4e649dbf, "Update fail-open/fail-
  closed architecture docs to confirm harness behavior".)

## Section B — suspected stale, verified accurate (do not churn)

These were candidates for Section A and **check out**; downstream
editors should leave them alone.

### B1. Overview absorption claim: `github-workflows` + `job-cronjob-yaml`

`docs/plan/plan.md:19-24` says both org-rule-guard rules were absorbed
by "built-in engine guards (pack attributions `github-workflows` and
`job-cronjob-yaml`) … on Write/Edit and Codex `apply_patch` (2026-09)".
Verified: `src/github_workflows.rs` and `src/job_cronjob_yaml.rs` exist,
both are wired inside content evaluation
(`src/engine.rs:2426-2434` and `:2447-2453`) — content mode serves
Write/Edit and normalized multi-file Codex patches — with tests at
`src/engine.rs:3991-3995`, `:4037-4062`, `:4110+`, `:4359-4369`. There
are deliberately **no pack files** of those names under `packs/` — the
guards are built-in and the quoted strings are denial `pack_id`
attributions, exactly as the Overview says. The remaining open
absorption beads (efe57f54, b559d088, 5117dc94, 466fe313) are tail work
(fixed regression cases, coexistence docs), not the absence of the
guards.

### B2. "Write/Edit credential-value rule has no absorbed channel yet"

`docs/plan/plan.md:16-18` and the Phase 1/Architecture `secrets`
treatment (`:100-109`, `:258-266`). Verified: `packs/secrets.json` has
`tool_keywords: []` and `applies_to: []` — Bash-command-string scanning
only, hook front-end only; `src/engine.rs:590` still names exactly
storage-class and image-tag as the content-mode packs. No Write/Edit
credential channel has shipped. The Overview's "kubectl-only rump plus
the Write/Edit credential-value rule" formula (lines 24-27) stands.

### B3. Docker wrapper coverage "once Phase 4's `irrevers-54d477dd` pack ships (not before)"

`docs/plan/plan.md:204-207`. The condition is now *satisfied*, which
makes the sentence historical-conditional, not false: `packs/docker.json`
exists with `tool_keywords: ["docker"]`, and irrevers-54d477dd is
Closed (rev 3). Do not rewrite the mechanism sentence; at most a reader
should know the "once" has already happened.

### B4. Codex hook maturity "~5 months old as of this writing"

`docs/plan/plan.md:216-218`. Explicitly date-anchored prose ("as of this
writing"), not a present-tense state claim — it was true when written
and makes no claim about today. Leave as-is unless the surrounding
sentence is being rewritten anyway.

### B5. The "release-cutting is separately human-gated" *premise* (self-updater safety argument)

`docs/plan/plan.md:189-192` and `:588-591` premise the self-updater's
safety and the `icg update` exception on release-cutting being
human-gated. The gate survives A7's mechanism change — it moved from
"human runs `gh release create`" to "a reviewed version bump is merged
to `main`" (runbook lines 12-27). The premise holds in superseded form;
only the resolved-trigger bullet (A7) needs the correction, not these
premises — though downstream editors may wish to point them at the
runbook's current statement of where the gate lives.

### B6. Deploy-location claims (root-owned paths)

`docs/plan/plan.md:172-198` and the Phase 0 reconciliation's "root-owned
deploy paths landed" (`:454-456`). Verified accurate for the guarded
host: live root-owned `/etc/icg` + installed `/usr/local/bin/icg`
verified by irrevers-93baa29a (closed 2026-09-07, see
`docs/notes/evidence/fail-closed-harness-b.md`). **Scoped caveat, not a
contradiction:** the *CI builder image* `ronaldraygun/argo-guarded-builder:0.1.0`
separately ships `/etc/icg` world-writable (mode 40777) — tracked open
as irrevers-beee1069, with the fixed image unpublished (irrevers-c36bba27).
The host-path claims are true and should not be "corrected" by that
defect; they also do not cover CI pods, so no wording change should
imply they do.

### B7. Phase 0/1/3/4/5 reconciliation bead citations

The closed IDs cited in the reconciliation parentheticals
(`plan.md:446-455`, `:597-611`, `:699-706`, `:728-744`, `:780-792`) are
covered by the 42-bead closed inventory and its two reconciliation
passes (additions: none; removals: none — see child 2's doc § "Delta"),
which re-confirmed all 42 closed with close dates matching to the
minute. Nothing in those parentheticals is invalidated by fresher
evidence; their one *derived* statement that went stale is A8.

### B8. The "no standing daemon" architecture framing

`docs/plan/plan.md:79-82` (watchdog killed at ideation) and `:333-337`
("No persistent process to update" / per-invocation guard). Consistent
with what shipped after A9's refutation: the fail-closed mechanism
deliberately took the durable-evidence + operator-reconcile shape
rather than resurrecting a standing watchdog. The refutation did not
produce a daemon; these framing claims remain true.

## Not listed, for completeness

- The Phase 0 trigger war story (`plan.md:466-505`) and the 2026-08-22
  mirror/publication narrative (`:506-519`) are dated historical
  narratives of events as they happened; nothing later contradicts them.
- `plan.md:474-477` (version-gated auto-cut) is the *correct* half of
  the internal inconsistency noted in A7.
- All four Open Questions (`:838-855`) are struck through as resolved;
  the resolutions match shipped evidence.
