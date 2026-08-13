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
7. Capability-grant requests instead of raw commands — ranked, **KILLED** in kill pass: inverts the "needs zero agent cooperation" strength; this is SEAM's shape, not this project's.
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
