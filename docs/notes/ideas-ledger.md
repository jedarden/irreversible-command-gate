# Ideas ledger

Dated runs of `/plan-idea-gen`, anchored to `docs/plan/plan.md`. Every idea
ever generated is recorded here, with its verdict, so future runs dedupe
against this instead of re-litigating settled ground.

## 2026-08-13 — first run (POOL 100, KEEP 10)

GOAL: dual-layer (PATH-wrapper + native PreToolUse hooks) guard for Claude
Code and Codex CLI that blocks irreversible commands, redirects to a
sanctioned alternative in one actionable step, extends `org-rule-guard.py`,
and self-updates via a verified, human-gated release pipeline.

CONSTRAINTS used as kill criteria: no GH Actions; no `kind: Job`/`CronJob`;
engine stays hardcoded/zero-I/O/fails-open; rule data modular pack-per-tool;
rule source not agent-writable or agent-triggerable-trusted; every block
needs an actionable redirect (deny+reason default, `updatedInput` only for
intent-preserving substitution); must cover both Claude Code and Codex via
both front-ends; cloud-hosted Codex is an accepted gap; self-update
requires the 4-layer release-integrity design; beads protection is a `.git`
file-vs-directory predicate, not a path block; NEEDLE worktree-creation
blocking is out of scope; ArgoCD-aware kubectl scoping not attempted;
threat model is an honest-mistake backstop, not adversarial-agent defense.

Stats: 100 generated (8 lenses) → 35 triage survivors (+4 crossover
hybrids = 39) → 23 advanced through pairwise ranking (cluster cap 2) → 15
survived the adversarial kill pass (8 killed) → 1 completeness gap round
added 4 more, all surviving → **10 finalists selected**.

### Lens 1 — invert-the-problem
1. Allowlist-first / default-deny mode — ranked, **KILLED** in kill pass: would flood benign commands with denials, defeating the "rare backstop" framing.
2. Agent self-declares structured intent before acting — triaged out: requires harness cooperation the current design specifically avoids needing.
3. Reversibility certificate before risky command — triaged out: no general mechanism to verify an arbitrary backup plan is sane.
4. Guard the human too, not just the agent — triaged out: scope conflicts with the stated threat model.
5. Success-verification gate (mandate a follow-up check) — triaged out: needs cross-turn session state the engine doesn't have.
6. Undo-log-first automatic snapshotting — triaged out: too heterogeneous across guarded tools for the payoff.
7. Capability-grant requests instead of raw commands — ranked, **KILLED** in kill pass: inverts the "needs zero agent cooperation" strength; this is SEAM's shape (a separate project, `~/SEAM`, hiding secrets via a server-side proxy rather than intercepting commands), not this project's.
8. Every denial auto-becomes a regression test case — ranked, **FINALIST**.
9. Dry-run-in-sandbox for ambiguous commands — triaged out: per-tool sandboxing is its own hard problem.
10. Agent proposes rule refinements, human approves — triaged out: overlaps the already-designed review process, not distinct enough.
11. Flag commands matching neither safe nor bad pattern — triaged out: weaker subset of #1, dies with it.
12. Full default-deny inversion — triaged out: near-duplicate of #1.

### Lens 2 — adjacent-domain transplant
13. Circuit breaker on N denials in a window — triaged survivor, cut by cluster cap at pairwise ranking.
14. Aviation checklist gate via recent doc-read — triaged out: gameable, doesn't verify comprehension.
15. Two-person rule via second independent session — triaged out: doesn't fit a solo-operator context or the stated threat model.
16. Anti-cheat behavioral fingerprinting — triaged out: contradicts deterministic-engine constraint.
17. Medical interlock via live audit-log check — triaged out: requires I/O in the hot path.
18. Four-eyes on release-cutting — triaged out: duplicate of the already-decided Layer 2 review design.
19. ATC-style read-back confirmation — ranked, **KILLED** in kill pass: can't deterministically verify a restatement is correct without an LLM judge.
20. Immune self/non-self baseline — triaged out: same non-determinism objection as #16.
21. Fraud-style composite risk scoring — triaged out: undermines the "one clear reason per denial" redirect design.
22. Munitions safe/arm switch — triaged out: near-duplicate of the capability-grant idea, more ceremony than value.
23. Kitchen-expediter review queue — triaged out: same "no real second party" objection as #15.
24. Escrow/holdback delay window — triaged out: doesn't map onto a synchronous PreToolUse hook's execution model.
25. Building-code staleness stamps on rule packs — triaged out: subsumed by the test-suite-staleness meta-check (#63).

### Lens 3 — remove-a-constraint
26. LLM-assisted triage for the ambiguous middle tier — ranked, **KILLED** in kill pass: can't regression-test non-deterministic behavior, undermines Layer 1.
27. (GH Actions hypothetical — not generated as a real candidate; kept in the pool as a deliberate demonstration that the constraint is a hard kill.)
28. Claude-Code-only premium fast path — triaged out: contradicts the just-settled "both harnesses" requirement.
29. Signed runtime rule bundles, remove hardcoded-engine — triaged out: re-opens a question `runtime-config-vs-hardcoded.md` already settled.
30. Plugin-registry for external pack contributors — triaged survivor, cut by cluster cap.
31. Ship `updatedInput` for a small vetted set from day one — merged into #93.
32. Opt-in slow-path live I/O behind a flag — triaged out: same shape as the ArgoCD-scoping idea the plan already declined.
33. Offline/local release path for degraded connectivity — triaged out (not separately clustered above; folded as a variant of the deploy-mechanism work already designed).
34. Centralized OPA-style policy server — triaged out: reintroduces a single point of failure the design otherwise avoids.
35. Full AST-based shell parsing — ranked, **KILLED** in kill pass: the precision gain mostly matters for adversarial evasion, which is out of scope for the stated threat model.
36. Pure POSIX-sh, remove Rust assumption — triaged out: contradicts the Rust-release infrastructure Phase 0 already builds on.
37. Piggyback entirely on Homebrew/nixpkgs distribution — triaged out: doesn't substitute for the deliberately-designed verification layers.

### Lens 4 — 10x-cheaper/simpler
38. Single ~150-line POSIX shell script MVP — ranked, **survived kill pass**, not selected as a numbered finalist (a sequencing suggestion more than a standalone feature) — worth doing as how Phase 1 ships, not a separate deliverable.
39. Fold new rules directly into `org-rule-guard.py` — triaged out: reproduces the exact self-edit gap this project exists to close.
40. No self-update, manual `git pull` only — triaged out: undoes already-settled release-integrity design.
41. PATH-wrapper only, cut native hooks — triaged out: re-litigates the just-settled "both layers" decision.
42. Literal exact-string denylist, no regex — triaged out: regression versus `org-rule-guard.py`'s existing capability.
43. One bucket, no tiers/severity — triaged out: contradicts the already-built Data Model.
44. Regression suite only, skip coverage-diff — triaged out: undoes exactly the gap Layer 1's second check was designed to close.
45. No Codex support in v1 — triaged out: contradicts the explicit requirement from this conversation.
46. Fork/wrap `destructive_command_guard` directly — ranked, **KILLED** in kill pass: its architecture wasn't designed around this project's redirect/multi-harness/release-integrity requirements; #80 gets the collaboration value more cheaply.
81. Bash functions in `.bashrc`, no compiled binary — triaged out: easily bypassed by non-interactive subshells, weaker than a real PATH-first binary.
82. Single flat JSON denylist, no packs — triaged out: contradicts the modular pack-per-tool architecture.

### Lens 5 — power-user workflow
47. Per-repo signed override file — ranked, **FINALIST**.
48. Fleet-wide denial dashboard — ranked, survived kill pass, not selected (real infra cost, sequenced to Phase 3+).
49. Auto-file a bf bead after 3+ repeat denials — ranked, survived kill pass, not selected (solid but the most "nice to have" of the reliability set).
50. Per-agent-identity policy scoping via NEEDLE tsnet — ranked, cut by cluster cap (superseded by hybrid #H3, which was itself killed for the same unmet-dependency reason).
51. `icg explain "<command>"` dry-run CLI — triaged survivor, cut by cluster cap (overlaps #70/#97 territory).
54. Multi-repo policy inheritance — triaged out: a Components detail of the already-decided pack architecture, not a distinct idea.
55. Beads link back to the denial event — triaged survivor, cut by cluster cap.
56. Canary rollout to a NEEDLE `--identifier` subset — ranked, **FINALIST**.
95. User-authored tests get the same Layer-1 protection — ranked, cut by cluster cap.
96. Fleet-wide forensics query tool — triaged out: duplicate of #48's underlying data.

### Lens 6 — failure-mode/reliability-driven
57. Self-health-check at startup — triaged out (folded into general Phase 0 hardening, not a distinct standalone idea).
58. Distinguish guard-crashed from no-violations-found — ranked, cut by cluster cap.
59. Staleness heartbeat on self-update — merged into the fleet-health cluster idea, cut by cluster cap.
60. Poison-pill auto-rollback — ranked, **FINALIST**.
61. Split-brain version detection — merged into the fleet-health cluster idea, cut by cluster cap.
62. Graceful self-updater-failure degradation — merged into the fleet-health cluster idea, cut by cluster cap.
63. Test-suite staleness meta-check — triaged survivor, cut by cluster cap.
64. Periodic PATH-precedence self-check — ranked, cut by cluster cap.
65. Degrade to hook-only if wrapper corrupted — ranked, cut by cluster cap (strong, but cluster cap bound it).
66. Check-latency-creep monitoring — triaged out at the pre-ranking trim: lower priority than its cluster-mates, resurfaced only if capacity allows.
97. `icg status` self-reports known blind spots — ranked, **FINALIST** (merged with #72).
98. Absolute-path wrapper-bypass detection via the hook layer — ranked, cut by cluster cap.

### Lens 7 — novice-user/intuitiveness
67. Plain-English rule pack summaries — triaged out: documentation task, not a distinct idea.
68. First-run interactive tour — triaged survivor, cut by cluster cap.
69. Per-rule doc page links — triaged out: minor, table-stakes.
70. "Why was I blocked" query tool — triaged survivor, cut by cluster cap (overlaps #51/#97).
71. Plain-consequence severity language — triaged out: wording choice, not a distinct idea.
72. `icg status` command — ranked, **FINALIST** (merged into #97's entry).
73. Progressive disclosure verbosity — triaged out: minor UX polish.
83. Color-coded terminal output — triaged out: cosmetic.
84. Practice/dry-run mode, never actually blocks — ranked, **FINALIST**.
85. Emoji severity markers — triaged out: cosmetic.

### Lens 8 — what would a competitor ship first
74. Hosted/managed policy-service option — triaged out: conflicts with the self-hosted, per-host design.
75. VS Code extension — triaged out: a substantial separate product surface.
76. Public benchmark/leaderboard of coverage — triaged out: marketing, not a functional capability.
77. Curl-to-install pinned to a verified release — ranked, **FINALIST** (folded in as small/cheap installer work).
78. Compatibility shim for `destructive_command_guard`'s pack format — triaged survivor, cut by cluster cap.
79. Chat-first Telegram approval flow — ranked, survived kill pass (scoped to async notify, not a blocking gate), not selected — worth doing but ranked below the top 10.
80. Contribute the OpenBao/Vault pack upstream — ranked, survived kill pass, not selected in the top 10 (process/relationship value, not a technical deliverable with the same shape as the others).
86. Federated community rule-pack marketplace — triaged out: duplicate of #30.
87. "Guarded" README badge — triaged out: cosmetic/marketing.
99. Public "why not just use X" comparison doc — triaged out: this content already exists in `docs/research/prior-art.md`.

### Additional pool entries
88-90. Actuarial adaptive verbosity / escalating reminders / letter-grade summary — 88 and 89 triaged out (stateful-tracking, contradicts deterministic-engine simplicity); 90 ranked and reached the kill pass, **KILLED**: aggregating severities into one grade obscures specific actionable violations, against this project's own redirect-specificity principle.
91. Harden against a human talking the agent into bypassing — triaged out: directly contradicts the stated threat-model boundary.
92. Track a positive "clean streak" signal — triaged out: same stateful-tracking objection as #88/89.
93. Ship one showcase `updatedInput` rule (force-push stripping) in Phase 1 — ranked, **FINALIST** (merged with #31).
94. Structured incident-report-to-rule-pack-PR intake process — triaged out: overlaps the already-designed review process.
100. SBOM-style per-release change manifest — triaged out: duplicate of Layer 1's coverage-diff, already designed.

### Crossover hybrids (Step 4)
- H1 (Lens1-#8 × Lens6-#60, "self-healing coverage") — ranked, ideas underlying it selected as **FINALISTS #1 and #3** (kept as two related-but-independently-shippable finalists rather than one coupled deliverable, per the kill pass's dependency objection).
- H2 (#56 × merged-59/61/62, "canary-aware fleet health") — folded into finalist #4 (56)'s scope note.
- H3 (#50 × #47, "scoped grant matrix") — ranked, **KILLED** in kill pass: hard-depends on NEEDLE per-worker tsnet identity, which doesn't exist yet (same blocker as SEAM's own Phase 7). #47 alone (no identity dependency) carries forward as a finalist instead.
- H4 (#84 × #53, "shadow mode against live traffic") — triaged survivor, cut by cluster cap; #84 alone carries forward as a finalist.

### Completeness gap round (Step 7)
Identified gaps: near-zero coverage-breadth expansion beyond Phase 1's fixed
rule list; no tooling to help a human author a new rule pack safely; no
mitigation for Codex's confirmed-still-churning hook API specifically.

- G1. New rule pack: Docker/container destructive ops (`system prune -a`, `volume rm`, forced image removal) — **FINALIST**.
- G2. New rule pack: Backblaze B2 CLI destructive ops — survived, not selected: genuine open uncertainty whether agents invoke a `b2` CLI directly versus only via library/API calls, which would make a CLI-layer pack moot. Worth revisiting once that's confirmed.
- G3. `icg new-pack <tool>` scaffolding tool, pre-filling the required Pack/GuardedPattern fields plus a regression-test stub — **FINALIST**.
- G4. Codex-hook-version compatibility test matrix in `icg-ci`, run against recent Codex CLI releases — **FINALIST**.

## Finalists (10)

1. Auto-denial-becomes-test (Lens 1, #8)
2. Pack-authoring scaffold tool (Gap round, G3)
3. Poison-pill auto-rollback (Lens 6, #60)
4. Canary rollout via NEEDLE `--identifier` (Lens 5, #56)
5. `icg status` with blind-spot self-report (Lens 6/7, #97/#72 merged)
6. Codex hook-version compatibility matrix (Gap round, G4)
7. Per-repo signed override (Lens 5, #47)
8. Practice/dry-run mode (Lens 7, #84)
9. Docker destructive-ops pack (Gap round, G1)
10. Early showcase `updatedInput` rule — force-push stripping (Lens 3/redirect-acceleration, #93)

### Finalist dossiers

1. **Auto-denial-becomes-test** (Lens 1, #8) — bead `icg-rri`
   - **Pitch:** Every denial the guard fires automatically becomes a permanent regression-test case, so coverage can only grow, never silently regress.
   - **Why it won:** Paired with poison-pill auto-rollback (#60) as hybrid H1 ("self-healing coverage"), but the kill pass's dependency objection required the two ship as independently-shippable finalists rather than one coupled deliverable — #8 stands on its own because, unlike killed lens-mates #1 (allowlist-first, would flood benign commands with denials), #7 (capability-grant, "SEAM's shape"), and #26 (LLM-assisted triage, non-deterministic and unregression-testable), it stays fully deterministic and needs zero harness cooperation.
   - **Complexity:** M
   - **First step:** Wire the denial-dispatch path to serialize each fired `GuardedPattern` match into a regression-test stub (command + expected verdict) in the existing suite, before automating it unattended.
   - **Strongest surviving objection:** Per plan.md, it "needs a curation step so the suite doesn't grow unbounded" — without de-duplication, every near-identical denial permanently bloats the suite.

2. **Pack-authoring scaffold tool** (Gap round, G3) — bead `icg-ncf`
   - **Pitch:** `icg new-pack <tool>` scaffolds a new rule pack, pre-filling the required Pack/GuardedPattern fields plus a regression-test stub.
   - **Why it won:** Emerged from the completeness-gap round's identified hole — "no tooling to help a human author a new rule pack safely" — a gap none of the other 9 finalists filled.
   - **Complexity:** S
   - **First step:** Build the CLI subcommand to emit a pre-filled Pack/GuardedPattern skeleton modeled on an existing pack (e.g. `vault`) as the template.
   - **Strongest surviving objection:** No specific kill-pass caveat was recorded for this idea, but it inherits the project's standing constraint that rule source is "not agent-writable or agent-triggerable-trusted" — the scaffold must still route new packs through Layer 1/2 review, not let authorship itself become a trust shortcut.

3. **Poison-pill auto-rollback** (Lens 6, #60) — bead `icg-2ck`
   - **Pitch:** A bad release automatically reverts itself rather than waiting for a human to notice.
   - **Why it won:** The other half of hybrid H1 with #8 — the kill pass's dependency objection meant the two ship independently rather than as one coupled feature, so #60 had to stand alone as a real, self-contained extension of Phase 0's Layer 4 self-updater rather than assuming #8's test-generation exists first.
   - **Complexity:** M
   - **First step:** Define the poison-pill health signal (what makes a release "bad") and wire the self-updater's trust pointer to automatically fall back to the last-known-good release when it fires.
   - **Strongest surviving objection:** Must work standalone, without depending on #8/`icg-rri`'s auto-generated tests existing yet — the kill pass explicitly rejected coupling the two into one deliverable.

4. **Canary rollout via NEEDLE `--identifier`** (Lens 5, #56) — bead `icg-l75`
   - **Pitch:** Roll a new release out to a named subset of hosts before it goes fleet-wide.
   - **Why it won:** Absorbed the fleet-health signals from hybrid H2 (`#56 × merged-59/61/62`, "canary-aware fleet health") — staleness heartbeat, split-brain version detection, graceful degradation — as a scope note rather than shipping those as separate finalists, making #56 the single concrete Layer 4 staged-rollout implementation.
   - **Complexity:** M
   - **First step:** Add a `--identifier`-scoped rollout gate to the self-updater so a release can target a named host subset before going fleet-wide.
   - **Strongest surviving objection:** Must stay scoped to the `--identifier` subsets NEEDLE already supports today — the closely-related hybrid H3 (`#50 × #47`, "scoped grant matrix") was killed specifically for hard-depending on NEEDLE per-worker tsnet identity, which doesn't exist yet; canary rollout must not quietly grow into needing that same missing infrastructure.

5. **`icg status` with blind-spot self-report** (Lens 6/7, #97/#72 merged) — bead `icg-1tj`
   - **Pitch:** A status command that reports the guard's own known blind spots (e.g. the cloud-hosted-Codex gap), not just "healthy/unhealthy."
   - **Why it won:** Two independently-generated ideas (#97 from the reliability lens, #72 from the novice-UX lens) converged on the same command and were merged rather than shipped as competing tools.
   - **Complexity:** S
   - **First step:** Implement `icg status` reading the guard's own documented blind-spot list (cloud Codex gap, deferred Layer 3, etc.) plus live self-health state, and print both.
   - **Strongest surviving objection:** Must stay a read-only report of already-known state — this pool repeatedly kills ideas that require new persistent usage-history tracking (#88/#89's "same stateful-tracking objection," #92's cold-start problem); `icg status` has to avoid sliding into that territory.

6. **Codex hook-version compatibility matrix** (Gap round, G4) — bead `icg-z5n`
   - **Pitch:** Run `icg-ci` against a matrix of recent Codex CLI releases to catch hook-API drift before it ships.
   - **Why it won:** Directly answers the completeness-gap round's third identified hole — "no mitigation for Codex's confirmed-still-churning hook API specifically" — grounded in plan.md's Architecture section, which notes Codex's hook API is "notably younger and still stabilizing (~5 months old as of this writing)."
   - **Complexity:** M
   - **First step:** Add an `icg-ci` matrix job that runs the Codex hook adapter against a pinned set of recent Codex CLI releases, failing the build on adapter/API drift.
   - **Strongest surviving objection:** The matrix itself needs ongoing maintenance as Codex's still-churning hook API keeps moving — a compatibility check against a moving target is only as good as how often the pinned version set gets refreshed.

7. **Per-repo signed override** (Lens 5, #47) — bead `icg-2i8`
   - **Pitch:** A signed, per-repo file that lets an operator explicitly override a specific rule, routed through the normal review layers.
   - **Why it won:** Its natural extension, hybrid H3 (`#50 × #47`, "scoped grant matrix"), was killed for hard-depending on NEEDLE per-worker tsnet identity — infrastructure that doesn't exist yet (the same blocker later noted for SEAM's own Phase 7). #47 alone carries no such dependency and survives on that basis.
   - **Complexity:** M
   - **First step:** Define the per-repo override file format (signed, Layer 1/2-reviewed) and the verification check the engine runs before honoring it.
   - **Strongest surviving objection:** If override scope is ever extended from per-repo to per-identity, it hits the exact missing-tsnet-identity wall that killed H3 — the design has to resist that scope creep.

8. **Practice/dry-run mode** (Lens 7, #84) — bead `icg-59u`
   - **Pitch:** A mode that runs the full check path and reports what would have been blocked, without ever actually blocking.
   - **Why it won:** Its hybrid pairing, H4 (`#84 × #53`, "shadow mode against live traffic"), was cut by cluster cap; #84 alone carried forward as the standalone finalist.
   - **Complexity:** S
   - **First step:** Add a dry-run flag/env var that runs the full check path but substitutes deny-with-log for actual blocking, paired with the persistent active-indicator noted below.
   - **Strongest surviving objection:** Per plan.md, it "ships only with the mandatory persistent active-indicator the kill pass required" — without an always-visible indicator, an operator could forget the mode is active and mistake "nothing got blocked" for a real pass.

9. **Docker destructive-ops pack** (Gap round, G1) — bead `icg-d3i`
   - **Pitch:** A new rule pack covering Docker/container destructive ops (`system prune -a`, `volume rm`, forced image removal), same shape as the `vault` pack.
   - **Why it won:** Beat its gap-round sibling G2 (Backblaze B2 CLI pack), which survived but wasn't selected specifically because of "genuine open uncertainty whether agents invoke a `b2` CLI directly versus only via library/API calls" — Docker CLI invocation carries no equivalent uncertainty in this environment.
   - **Complexity:** S
   - **First step:** Author the docker pack (`tool_keywords: [docker]`) covering `system prune -a`, `volume rm`, and forced image removal, following the same Pack/GuardedPattern shape as `vault`.
   - **Strongest surviving objection:** Confirm agents actually invoke `docker` directly (not just through a higher-level build tool) before assuming full coverage — the same class of assumption that sidelined G2, just far better-grounded here.

10. **Early showcase `updatedInput` rule — force-push stripping** (Lens 3/redirect-acceleration, #93, merged with #31) — not adopted as a bead
    - **Pitch:** Ship one concrete `updatedInput` case (stripping `--force`/`--force-with-lease` from a force-push) in Phase 1, ahead of Phase 3's general redirect-mechanism work, to de-risk the mechanism early.
    - **Why it won:** Absorbed #31 ("ship `updatedInput` for a small vetted set from day one"), which was merged directly into this entry rather than competing separately.
    - **Complexity:** S
    - **First step (if picked up):** Add `updatedInput` handling for the single force-push case as the intent-preserving rewrite.
    - **Strongest surviving objection:** Per plan.md Phase 3, this was "considered and deliberately deferred to notes only, not adopted as a bead" — winning the ideation round wasn't enough to override the sequencing decision to keep Phase 1 deny-only.

## 2026-08-13 — second run (POOL 100, KEEP 10)

Same GOAL/CONSTRAINTS as the first run. PRIOR = the first run's full 100
ideas above, actively used to steer generation into genuinely new
territory rather than re-litigating settled ground — confirmed no
resurrections were needed (none of the first run's kill objections had
stopped applying).

Grounding: `bf ready` showed all 9 first-run beads still `open`/unstarted;
`docs/plan/plan.md` now has Phase 4 listing them.

Stats: 100 generated (8 lenses) → 40 triage survivors (after a second
harsher trim on oversized clusters, matching run 1's pattern) + 4 crossover
hybrids → 30 advanced through pairwise ranking (cluster cap 2) → 25
survived the adversarial kill pass (5 killed) → 1 completeness gap round
added 2 more, both surviving → **10 finalists selected** from the combined
27.

### Lens 1 — invert-the-problem
1. Guard CI/build pods on iad-ci too, not just interactive sessions — ranked, **FINALIST**.
2. Separate detection from enforcement (audit-only mode) — triaged survivor, cut in the second trim pass (weakest of its cluster).
3. Protect the agent from a compromised/malicious repo's instructions — triaged out: adversarial-threat territory the stated threat model explicitly excludes.
4. Highest-severity rules require both wrapper and hook layers to concur — triaged out: over-engineered relative to the honest-mistake threat model, same objection class as run 1's two-person-rule kill.
5. Post-execution result scanning for cases pre-execution can't cover — triaged out: vague mechanism without per-MCP-server-specific logic; superseded by #47/H3's cleaner framing.
6. Agent self-authors proposed new rules from its own close calls — triaged out: overlaps the already-adopted auto-denial-becomes-test (icg-rri), which is the concrete version of the same instinct.
7. Time-of-day-aware strictness — triaged out: weak/gameable "operator known away" heuristic, real added complexity for a soft benefit.
8. Machine-readable "did you mean" field in the deny payload — ranked, **survived kill pass**, not selected in the final 10 (solid but speculative until a consumer exists).
9. Guard protects other agents from one agent's in-flight uncommitted shared-tree changes — ranked, **FINALIST**.
10. Friction-cost model instead of binary allow/deny — triaged out: a different interaction paradigm than the already-designed three-channel redirect model.
11. Guard inbound data for embedded prompt-injection-shaped instructions — triaged out: same adversarial-threat exclusion as #3.
12. Rule packs co-located per-repo, discovered by the engine — triaged out: directly reopens the agent-writable rule-source hole this whole project exists to close.
51. Marked-but-allowed output for explicit override cases — triaged out (not separately clustered; folded as a variant already covered by the emergency-bypass-token idea).
52. Self-update as required human push, not opt-in pull — triaged out: reverts the already-settled self-update automation design.
53. Guard's own actions carry an attributable service identity — triaged survivor, cut in the second trim pass.
69. Guard as a standing daemon, not a fresh process per check — triaged out: real architecture change for unclear benefit given the current per-invocation model already works within PreToolUse's shape.
70. Deny reason includes a ready-to-run safe-alternative script — ranked, **survived kill pass**, not selected (real value, scoped to the subset of rules where a generic safe alternative genuinely exists).
85. Logged retry-with-justification per denial — triaged survivor, cut in the second trim pass.
93. Split one tool's rules across multiple files by severity tier — ranked, cut by cluster cap.

### Lens 2 — adjacent-domain transplant
13. Reactor SCRAM: fleet-wide halt command — ranked, **survived kill pass** (must be human-only-triggerable), not selected standalone — its mechanism carries forward via finalist H1.
14. Legal "material adverse change": auto-void overrides on confirmed incident — ranked, cut in the second trim pass.
15. Continuous tamper-evident heartbeat logging — ranked, cut by cluster cap; mechanism carries forward via finalist H1.
16. Crash-survivable black-box decision log — ranked, **survived kill pass**, not selected (genuinely distinct from crash-detection, real value, lost out on priority in the final cut).
17. Periodic aggregate shrinkage-style pattern review — triaged out: overlaps existing dashboard/forensics territory, a cadence variant not a new mechanism.
18. Running uptime/coverage percentage — triaged out: near-duplicate of #15, weaker framing.
19. Cheap "proof of presence" pre-check before high-risk ops — triaged out at the pre-ranking trim: too weak a substitute for a real second party to merit a slot.
20. Three-tier forward-looking watch/warning/emergency alert — triaged out: requires stateful near-miss pattern tracking, same objection as run 1's killed risk-scoring cluster.
21. Chain-of-custody tracking for rule-pack handling — triaged out at the pre-ranking trim: overlaps release-integrity's existing content-verification layers without adding a distinct property.
22. Graceful load-shedding under extreme guard load — ranked, cut in the second trim pass.
54. Certificate-pinning-style exact signing-key trust for Layer 3 — ranked, **survived kill pass**, not selected (valuable but Layer 3 itself is already explicitly deferred — recorded as a design note for when it's picked up, not urgent now).
55. Certified-mail-style logged acknowledgment for overrides — ranked, **KILLED** in kill pass: redundant with the already-adopted per-repo override's existing Layer 1/2 review requirement, which already is the acknowledgment.
56. Separation of duties: rule-pack author and release-approver must be genuinely separate sessions — ranked, **FINALIST**.
71. Sealed, single-use emergency bypass token, human-only reachable — ranked, **survived kill pass**, not selected (strong, but more niche/rare-use than the top 10).
72. Data-driven "people who got denied here usually then ran X" — triaged out: requires learned aggregate data with a cold-start problem.
86. Sterile-cockpit: block concurrent rule-pack changes during release windows — ranked, cut in the second trim pass.
94. Named-peril vs. all-risk-adjacent operating posture toggle — triaged out: the all-risk mode reintroduces fuzzy non-deterministic matching.

### Lens 3 — remove-a-constraint
23. Per-tool micro-guard binaries instead of one engine — triaged out: fragments the already-settled one-engine architecture without a matching benefit.
24. Fully dynamic, context-aware deny reasons via live ACL reads — triaged out: reintroduces I/O in the hot path, same objection as run 1's killed medical-interlock idea.
25. Default extra scrutiny for unknown highly-privileged binaries — ranked, **survived kill pass**, not selected in the top 10 (solid, cheap, lost out on priority).
26. Ship Phase 1 packs independently as ready — ranked, **KILLED** in kill pass: doesn't add anything beyond the already-decided modular architecture; a scheduling detail, not a distinct capability.
27. Guard proactively suggests periodic hygiene review — ranked, **KILLED** in kill pass: requires the same persistent usage-history tracking already ruled out elsewhere in this pool for the same reason.
28. Different trust tiers of hosts pin to different release channels — ranked, **survived kill pass**, not selected (well-grounded, real value, lost out on priority).
57. Structured-data-only responses, presentation as a swappable layer — ranked, **survived kill pass**, not selected (cheap architectural hygiene, worth doing, lost out on priority).
58. Codex support ships as an independently-versioned sub-release — ranked, cut in the second trim pass.
73. Extend scope from commands to arbitrary tool_input (e.g. Terraform plan files) — ranked, cut by cluster cap.
74. Accept OPA/Rego-authored packs — triaged out: real new-dependency cost for a benefit that doesn't clearly apply to this project's actual solo-operator authoring context.
95. Independent per-pack versioning — triaged out: real version-matrix complexity, mostly subsumed by the simpler "ship packs independently" idea (itself also cut).

### Lens 4 — 10x-cheaper/simpler
29. Two-state block/warn cut — triaged out: minor variant of run 1's already-killed one-bucket idea.
30. No dedicated CLI at all, document raw layout — triaged out: undercuts already-adopted CLI-tooling finalists from run 1.
31. Single shell-alias-block distribution — triaged out: same weaker-mechanism objection as run 1's killed bash-functions idea.
32. `flock` lockfile instead of general state-store for Phase 2 — ranked, **KILLED** in kill pass: solves concurrent-access mutual exclusion, not the actual cross-invocation sequencing-over-time problem Phase 2 needs — doesn't actually solve the stated problem.
59. Ship every release to 100% of hosts immediately, no canary — triaged out: directly contradicts the already-adopted canary-rollout finalist from run 1.
60. Docs site generated directly from rule-pack source — ranked, **survived kill pass**, not selected (real value, complementary to the CLI tooling, lost out on priority).
75. Zero built-in rule packs, pure scaffolding — triaged out: removes the project's actual motivating value (Vault/force-push/etc. coverage).
76. Pre-commit-hook-only distribution, no pre-execution blocking — triaged out: removes exactly the pre-execution safety property the project is named for.
88. Quarterly manual Codex-compat check instead of automated CI — triaged out: directly undoes the already-adopted automated-matrix finalist without sufficient justification.

### Lens 5 — power-user workflow
33. `.icgignore`-style local-only suppress file — triaged out: real risk of confusion with the already-adopted, verified per-repo override.
34. Programmatic library API for other fleet tools — ranked, cut in the second trim pass.
35. Mine the entire git log to seed the regression suite with real historical incidents — ranked, **survived kill pass**, not selected (valuable one-time bootstrap, lost out on priority against cheaper wins).
36. Coverage-diff auto-posted as a PR comment — ranked, cut in the second trim pass.
37. `--explain-verbose` full decision trace — ranked, **survived kill pass**, not selected (overlaps `icg why-not`; recommended as one unified tool at implementation time).
61. Chat-native slash-command integration — ranked, cut in the second trim pass.
62. Export denial history to the existing VictoriaMetrics stack — ranked, **survived kill pass**, not selected (must stay async/fire-and-forget to respect the no-I/O-hot-path constraint; solid, lost out on priority).
77. `ICG_DRY_RUN=1` personal env-var escape hatch — ranked, cut in the second trim pass.
78. Portable rule-pack bundle for offline/air-gapped sync — triaged out at the pre-ranking trim: real but niche, lower priority than broadly-applicable ideas.
89. `icg why-not <command>`, inverse of explain — ranked, **survived kill pass**, not selected (real value for near-miss debugging, lost out on priority).
97. `--since <date>` time-bounded forensics query flag — triaged out at the pre-ranking trim: too small/dependent on unbuilt forensics tooling to merit its own slot.

### Lens 6 — failure-mode/reliability
38. CI check for catastrophic-backtracking (ReDoS) regexes in submitted packs — ranked, **FINALIST**.
39. Fuzz/property-test the shell segmentation/tokenization parser — ranked, **survived kill pass**, not selected standalone (strong; mechanism partially carries forward via H2).
40. Clock-skew detection against trust-pointer/override-expiry timestamps — triaged out: narrow edge case relative to stronger competing ideas.
41. Guard against its own dependency-supply-chain drift — triaged out: overlaps the already-deferred Layer 3 build-provenance work.
42. Explicit test/decision for guard being OOM-killed mid-check — ranked, **FINALIST**.
63. Test guard behavior under this environment's real cgroup/CPUQuota containment — ranked, **survived kill pass**, not selected (well-grounded, real incidents, lost out on priority).
79. Regex timeout on legitimately oversized input — triaged out: near-duplicate of #38, folded into the same testing effort.
80. Verify behavior with empty/malformed $PATH — triaged out: narrow edge case relative to stronger competing ideas.
90. Verify correctness across this environment's three actual host types — triaged out: near-duplicate/subsumed by #63's more concrete framing.
98. Chaos test: corrupt a rule-pack file, confirm self-health-check catches it — ranked, cut in the second trim pass.

### Lens 7 — novice-user/intuitiveness
43. Richer recovery flow on a user's first REAL denial — triaged out: overlaps already-adopted practice-mode and status-command territory.
44. Localization-ready deny templates — triaged out: subsumed by #57 (structured-data responses make this a natural consequence, not a separate deliverable).
45. Cookbook doc: 5 real before/after transcripts — ranked, cut in the second trim pass.
46. Explicit zero-config regression test — triaged out at the pre-ranking trim: folded into general Phase 0 hardening, not a distinct standalone idea.
65. One memorable line on first install — triaged out: too minor, thin territory versus run 1's tour concept.
66. Explicit non-goals stated up front in the README — ranked, **FINALIST**.
81. Live self-test: send a known-bad command through safely, confirm denial — ranked, **survived kill pass**, not selected (genuinely distinct from the static status report, lost out on priority).
82. Install-time smoke test confirming no conflict with `org-rule-guard.py` — ranked, **FINALIST**.
91. FAQ entry with a measured "does this slow me down" number — triaged out: minor documentation detail, not a distinct deliverable.
99. Glossary of project-specific jargon — triaged out: minor documentation polish.

### Lens 8 — what would a competitor ship first
47. Dedicated MCP-server-specific guardrail/proxy mode — ranked, **survived kill pass, scope-narrowed** — superseded by the more concrete finalist H3.
48. Open-core free/paid tiers — triaged out: doesn't apply to this environment's self-hosted, non-commercial use.
49. Zero-install GitHub App/bot — triaged out: substantial separate product surface, same objection class as run 1's killed VS Code extension.
50. Marketplace listings in Claude Code/Codex's own stores — triaged out: depends on external platforms maturing, not actionable now.
67. Data-backed "trust score" from real dogfooding — ranked, cut in the second trim pass.
68. Publish the adversarial kill-pass methodology as a reusable pattern — ranked, **KILLED** in kill pass: no clear urgency, loses the priority comparison against ideas with real functional value at this project's current stage.
83. Publish anonymized aggregate denial stats across adopters — triaged out: presupposes external adopters that don't exist yet.
84. "Certified safe for AI agents" checklist for other tools — triaged out: scope expansion into being a standards body, well beyond this project's current stage.
92. Migration guide from raw `org-rule-guard.py` usage — ranked, **survived kill pass**, not selected standalone (stronger combined with the smoke test as H4).
96. CVSS-style standardized severity scoring — triaged out: borrows the name without the methodology mapping cleanly; the existing bespoke taxonomy is already simple and adequate.
100. Public "why we didn't build X" page from the ledger — ranked, cut in the second trim pass.

### Crossover hybrids (Step 4)
- H1 (#13 SCRAM × #15 heartbeat, "dead-man's-switch SCRAM") — ranked, **FINALIST** (requires a debounce of N consecutive missed heartbeats, not a single miss, to avoid false-alarm disruption).
- H2 (#64 mismatch-alarm × #39 fuzz-testing, "differential fuzzing") — ranked, cut by cluster cap; both parents' value substantially carries forward independently.
- H3 (#34 library API × #47 MCP mode, "guard-as-MCP-server") — ranked, **FINALIST** (scoped to local-only/stdio transport, not network-exposed, to avoid expanding the attack surface).
- H4 (#92 migration guide × #82 smoke test, "guided coexistence migration tool") — ranked, cut by cluster cap; #82 selected standalone, #92 noted as stronger combined with it at implementation time.

### Completeness gap round (Step 7)
Identified gaps: nothing addressing interactions BETWEEN the 9 already-
adopted run-1 finalists once they're actually built; no fleet-wide,
single-rule (as opposed to per-repo) kill-switch for a misbehaving rule.

- G1. Cross-feature integration tests covering seams between the 9 run-1 finalists (e.g. practice mode vs. poison-pill deny-rate counting, per-repo override vs. canary channel) — survived, not selected in the top 10, explicitly sequenced as work for once those 9 are actually implemented.
- G2. Fleet-wide single-rule kill-switch, distinct from the per-repo-scoped override (different axis of granularity: which rules, not which repos) — **FINALIST**.

## Finalists (10) — second run

1. Guard CI/build pods on iad-ci too, including this project's own `icg-ci` (Lens 1, #1)
2. Shared-tree collision protection for concurrent agents (Lens 1, #9) — later revised by the user into a simpler mechanism (tracked-vs-actual remote git HEAD comparison before push) and shipped as bead `icg-2m8`; see plan.md Phase 5.
3. Dead-man's-switch SCRAM, debounced (Hybrid H1)
4. Guard-as-MCP-server, local-only (Hybrid H3)
5. Separation of duties: rule-pack author ≠ release-approver (Lens 2, #56)
6. Explicit fail-open/fail-closed decision for guard OOM-kill (Lens 6, #42)
7. CI check for catastrophic-backtracking (ReDoS) regexes in submitted packs (Lens 6, #38)
8. Fleet-wide single-rule kill-switch (Gap round, G2)
9. Explicit non-goals stated in the README (Lens 7, #66)
10. Install-time smoke test vs. `org-rule-guard.py` conflict/duplication (Lens 7, #82)

### Finalist dossiers

1. **Guard CI/build pods on iad-ci** (Lens 1, #1) — bead `icg-4p8`
   - **Pitch:** Extend guard coverage to CI/build pods on `iad-ci`, including this project's own `icg-ci` release pipeline, not just interactive agent sessions.
   - **Why it won:** Requires no architecture change, unlike lens-mate #69 (triaged out: "guard as a standing daemon... real architecture change for unclear benefit given the current per-invocation model already works") — it's the same per-invocation engine applied to a new execution context.
   - **Complexity:** M
   - **First step:** Identify where the PATH-wrapper/hook adapters need to be installed inside `icg-ci`'s own Argo Workflow pod image, starting with guarding `icg-ci`'s own release pipeline first.
   - **Strongest surviving objection:** CI pods are a different execution environment than an interactive shell — the concrete adapter shape has to stay a per-invocation check, not drift toward the "standing daemon" architecture #69 was killed for.

2. **Shared-tree collision protection for concurrent agents** (Lens 1, #9) — later revised into bead `icg-2m8` (see note)
   - **Pitch:** Protect other agents from one agent's in-flight, uncommitted shared-tree changes.
   - **Why it won:** Grounded directly in this environment's own recorded incident class — the shared-checkout git-index race and the `.beads/` file-vs-directory collision risk the Architecture section already treats as real.
   - **Complexity:** L (as originally proposed)
   - **First step (as originally proposed):** Prototype the collision check against a known concurrent-shared-checkout scenario to confirm the detection approach doesn't produce false positives under normal sequential ops.
   - **Strongest surviving objection:** The mechanism as originally proposed didn't ship as designed — see the revision note below.
   - **Note (Gap C):** Per plan.md Phase 5, this finalist was later revised by the user into a simpler mechanism — comparing tracked-vs-actual remote git HEAD before a `git push` — and shipped as bead `icg-2m8` ("stale-HEAD push guard"), not the collision-detection shape described above.

3. **Dead-man's-switch SCRAM, debounced** (Hybrid H1) — no bead listed in plan.md Phase 5
   - **Pitch:** A fleet-wide halt that triggers automatically after N consecutive missed heartbeats, rather than requiring a human to notice something's wrong.
   - **Why it won:** Combines #13 (reactor SCRAM, survived kill pass only on condition it stay "human-only-triggerable") with #15 (heartbeat logging, cut by cluster cap but its mechanism carried forward) — #15 gives #13 automatic failure detection, #13 gives #15's heartbeat a safe, human-gated action to trigger.
   - **Complexity:** M
   - **First step:** Define the heartbeat interval and the N-miss debounce threshold, then wire the halt mechanism to trigger only after N consecutive misses.
   - **Strongest surviving objection:** The debounce requirement exists precisely because a single missed heartbeat (e.g. a network blip) would otherwise trigger a disruptive fleet-wide halt — the finalist only survived the kill pass by building that safeguard in from the start.

4. **Guard-as-MCP-server, local-only** (Hybrid H3) — no bead listed in plan.md Phase 5
   - **Pitch:** Expose the guard's check engine as a local-only, stdio-transport MCP server for other fleet tools to call.
   - **Why it won:** #47 (dedicated MCP-server guardrail mode) survived the kill pass only in "scope-narrowed" form; combining it with #34 (programmatic library API) gave it the concrete local-only/stdio shape that satisfies that narrowing.
   - **Complexity:** M
   - **First step:** Expose the engine's check function via a local-only stdio MCP server binary, reusing the same engine as the PATH-wrapper/hook adapters, no network transport.
   - **Strongest surviving objection:** A network-exposed version would expand the attack surface beyond the stated honest-mistake threat model — which is exactly why this finalist is deliberately scoped to local-only/stdio and not network-exposed.

5. **Separation of duties: rule-pack author ≠ release-approver** (Lens 2, #56) — no bead listed in plan.md Phase 5
   - **Pitch:** The person who authors a rule-pack change and the person who approves its release must be genuinely separate sessions.
   - **Why it won:** Contrasts with killed lens-mate #55 ("certified-mail-style logged acknowledgment for overrides" — killed as "redundant with the already-adopted per-repo override's existing Layer 1/2 review requirement, which already is the acknowledgment"). #56 adds a property Layer 1/2 review doesn't already provide, where #55 didn't.
   - **Complexity:** M
   - **First step:** Define what makes two sessions "genuinely separate" (different NEEDLE identifiers, or simply distinct human-invoked sessions) and encode that as a release-approval precondition alongside the existing Layer 1/2 review.
   - **Strongest surviving objection:** Without a concrete mechanism for verifying separateness, it risks becoming exactly the kind of paperwork-only requirement that got #55 killed as redundant ceremony.

6. **Explicit fail-open/fail-closed decision for guard OOM-kill** (Lens 6, #42) — bead `icg-4bu`
   - **Pitch:** Force an explicit, tested decision for what happens when the guard itself is OOM-killed mid-check, rather than leaving it to the engine's general fails-open default.
   - **Why it won:** The Architecture's fails-open design ("a missed violation is recoverable, a stuck fleet is not") was written for parse failures and exceptions; this finalist closes the gap for the specific mid-check-death case, which isn't automatically the same failure mode.
   - **Complexity:** M
   - **First step:** Verify whether Claude Code's/Codex's own hook-timeout/hook-error configuration can substitute for detecting a dead guard process — if either harness supports "deny on hook failure," the fail-closed transition may just mean flipping that harness setting once `icg-2ck`'s poison-pill signal confirms reliability, with no new icg-side process needed. Only build a dedicated supervisor/watchdog if neither harness supports it — see `docs/plan/plan.md` Architecture, which flags this as unresolved and explicitly notes a watchdog is the same "standing daemon" shape ideation killed elsewhere (Lens 1 idea #69).
   - **Strongest surviving objection:** Per plan.md, it "fails open until the guard's reliability is validated" — during that interim window, an OOM-killed check silently lets a command through with no denial, the same accepted-but-real risk the Architecture's fails-open default already carries, now made an explicit, tracked instance of it.

7. **CI check for catastrophic-backtracking (ReDoS) regexes** (Lens 6, #38) — bead `icg-3xz`
   - **Pitch:** Catch pathological, catastrophically-backtracking regexes in submitted rule packs at CI time, before they ship.
   - **Why it won:** #79 ("regex timeout on legitimately oversized input") was folded into the same testing effort as a near-duplicate rather than competing separately — #38 became the umbrella for both the static CI-time check and the runtime-timeout angle.
   - **Complexity:** S
   - **First step:** Add an `icg-ci` lint step that runs each submitted pack's regexes against adversarial pathological-backtracking inputs with a timeout, failing the build on any regex that doesn't terminate quickly.
   - **Strongest surviving objection:** A CI-time static check alone doesn't cover a regex that only backtracks catastrophically on unusually large real input at runtime — #79's runtime-timeout half needs to genuinely be folded in, not just cut from the pool.

8. **Fleet-wide single-rule kill-switch** (Gap round, G2) — bead `icg-4mu`
   - **Pitch:** Disable one misbehaving rule fleet-wide, independent of the per-repo override — a different axis of granularity (which rules, not which repos).
   - **Why it won:** Identified explicitly in the second run's completeness-gap round as a real, uncovered axis — run 1's per-repo override (`icg-2i8`) scopes by repo, not by rule, so this fills a genuine gap rather than duplicating it.
   - **Complexity:** S
   - **First step:** Add a per-rule enabled/disabled boolean to the `GuardedPattern` data model, gated the same way as any other rule-pack change (Layer 1/2 review), not a separate fast path.
   - **Strongest surviving objection:** Per plan.md, the revised implementation "reuses the normal Layer 1/2 release pipeline instead of a dedicated fast path" — the tradeoff is explicit: it's "no longer sub-release-cycle-fast... flagged as a real, unresolved gap if true emergency speed is ever needed."

9. **Explicit non-goals stated in the README** (Lens 7, #66) — done directly, not a bead
   - **Pitch:** A "what this does not do" section in the README, stated up front.
   - **Why it won:** Beat lens-mate #65 ("one memorable line on first install" — triaged out as "too minor, thin territory versus run 1's tour concept") by adding genuinely new scope-boundary information instead of cosmetic value.
   - **Complexity:** S
   - **First step:** Already done — see `README.md`'s "What this does not do" section (per plan.md Phase 5).
   - **Strongest surviving objection:** None recorded — the lowest-risk finalist of the second run, consistent with it being shipped directly rather than tracked as a bead.

10. **Install-time smoke test vs. `org-rule-guard.py`** (Lens 7, #82) — bead `icg-53q`
    - **Pitch:** An install-time check that confirms the new guard doesn't conflict with or duplicate `org-rule-guard.py`'s existing denials.
    - **Why it won:** Its hybrid pairing, H4 (`#92` migration guide `×` `#82` smoke test), was cut by cluster cap; #82 was selected standalone, with #92 noted as "stronger combined with it at implementation time" rather than required now.
    - **Complexity:** S
    - **First step:** Write an install-time check that runs both `org-rule-guard.py` and icg's hook adapter against the same known-bad test command, confirming they agree (both deny, or both allow) — expected redundant double-deny is a pass, not a failure; only a *divergent* verdict (one denies, the other doesn't) fails the check. See `docs/plan/plan.md`'s Phase 5 entry for `icg-53q`.
    - **Strongest surviving objection:** Per the Overview, coexistence with `org-rule-guard.py` is "an interim state, not the intended end state" — this smoke test's relevance is temporary by design, valuable only until that hook is actually deprecated.
