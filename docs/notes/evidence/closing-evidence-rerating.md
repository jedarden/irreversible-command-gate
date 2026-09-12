# Evidence-strength re-rating — enumerated inventory beads

Working deliverable for **irrevers-92165552** (child 4 of the
irrevers-2c3b4637 auto-split; depends on child 3). Rated 2026-09-12 at repo
HEAD `afaf476`.

**Inputs**

- Child 2 — `first-half-closing-evidence-verification.md` (irrevers-98e9188e,
  commit `9fd67b6`): enumerated-list rows 1–21 (§A 1–16 + §B 1–5). Result:
  19 verified · 2 corrected · 0 unverifiable.
- Child 3 — `second-half-closing-evidence-verification.md`
  (irrevers-25063548, commit `8408665`): §B 6–21 + §C 1–5, plus
  irrevers-84b36e47 re-verified explicitly (overlap with child 2's row 1;
  both stand). Result: 19 verified · 3 corrected · 0 unverifiable.
- Baseline for the diff — `docs/notes/closed-beads-compiled-inventory.md`
  as of commit `b15d587`: **5 weak-unverifiable + 6 partial** (+ 31 solid,
  5 of them with caveat), 42 beads.

**Method** — each bead's child-2/3 verification verdict (VERIFIED /
CORRECTED-minor) is combined with the compiled inventory's class (GIT /
SELF / NONE) into a rating of **strong** (= the compiled inventory's
"solid"), **partial**, or **weak-unverifiable**, using that document's
definitions: strong = evidence verified at HEAD; partial = substance real
and verifiable but the closure record itself defective (split/bookkeeping
close, close predating the work, half-unsubstantiated scope);
weak-unverifiable = closing evidence not re-checkable from the repo (or
never existed). A CORRECTED verdict moves a rating only if the correction
touches the rating's basis — substance, closure-record shape, or
re-checkability — not if it fixes a citation figure in a prior pass's
prose. Every corrected row was assessed individually (see Diff).

**Result: all 42 ratings confirm — the diff against the compiled inventory
is empty.** 5 weak + 6 partial + 31 strong (5 with caveat), matching
`b15d587` count for count. No new unverifiable evidence was found by either
verification pass, and no pass contradicted any rating's basis.

## Rating table (all 42 enumerated beads, child-1 order)

Verdict = the covering verification pass's result for the bead. Ratings
below are unchanged from the compiled inventory, so they are listed without
comment (per the task); the five CORRECTED rows are annotated only because
their corrections are assessed in the Diff section.

| # | Bead | § | Verdict | Class | Rating |
|---|---|---|---|---|---|
| 1 | irrevers-84b36e47 | A | VERIFIED (both passes; release re-checked live) | GIT | strong (caveat: Argo run TTL-reaped) |
| 2 | irrevers-e77615c8 | A | VERIFIED | GIT | strong |
| 3 | irrevers-37eb1100 | A | VERIFIED | GIT | strong |
| 4 | irrevers-340ae322 | A | VERIFIED as self-reported | SELF | weak-unverifiable |
| 5 | irrevers-e2bb8fbf | A | CORRECTED (minor) | GIT | strong |
| 6 | irrevers-6de781f4 | A | VERIFIED | GIT | partial |
| 7 | irrevers-b6579270 | A | VERIFIED | GIT | strong |
| 8 | irrevers-eff8909f | A | VERIFIED (substance) | SELF | partial |
| 9 | irrevers-2cb3dbd2 | A | VERIFIED | GIT | strong (caveat: sub-clause skipped at close) |
| 10 | irrevers-c87a3c50 | A | CORRECTED (minor) | NONE | weak-unverifiable |
| 11 | irrevers-5fdc2e13 | A | VERIFIED | GIT | strong |
| 12 | irrevers-f59f9313 | A | VERIFIED | GIT | partial |
| 13 | irrevers-96594031 | A | VERIFIED | GIT | strong |
| 14 | irrevers-ca79d63a | A | VERIFIED | GIT | strong |
| 15 | irrevers-e00a5381 | A | VERIFIED (cross-repo + live) | GIT | strong |
| 16 | irrevers-075634b8 | A | VERIFIED | GIT | strong |
| 17 | irrevers-8d2d4a73 | B | VERIFIED | GIT | strong |
| 18 | irrevers-aab3854c | B | VERIFIED | GIT | strong |
| 19 | irrevers-cd3f4c44 | B | VERIFIED | GIT | strong |
| 20 | irrevers-8a24ad8d | B | VERIFIED | GIT | strong |
| 21 | irrevers-019c36d3 | B | VERIFIED | GIT | strong |
| 22 | irrevers-fffef435 | B | VERIFIED | GIT | strong |
| 23 | irrevers-0f49129d | B | VERIFIED as bookkeeping close | GIT VIA | partial |
| 24 | irrevers-ff4f17da | B | CORRECTED (minor) | GIT | strong |
| 25 | irrevers-3fc4bdde | B | VERIFIED | GIT | strong |
| 26 | irrevers-f891f555 | B | VERIFIED | GIT | strong |
| 27 | irrevers-edb5c4ca | B | VERIFIED | GIT | strong |
| 28 | irrevers-9eb4de16 | B | VERIFIED | GIT | strong (caveat: manual negative control) |
| 29 | irrevers-93baa29a | B | VERIFIED as self-reported | SELF | weak-unverifiable |
| 30 | irrevers-3e6c6fde | B | VERIFIED | GIT | strong |
| 31 | irrevers-50077acb | B | VERIFIED | GIT | strong |
| 32 | irrevers-ffdc924b | B | VERIFIED (count note) | GIT | strong |
| 33 | irrevers-9007792b | B | CORRECTED (minor) | GIT | strong |
| 34 | irrevers-0d710c9a | B | VERIFIED | GIT | strong |
| 35 | irrevers-1517a263 | B | VERIFIED | GIT | strong |
| 36 | irrevers-0aa08f4e | B | VERIFIED | GIT | strong |
| 37 | irrevers-49dbb095 | B | VERIFIED (clarification + nit) | GIT | strong (caveat: record linkage + bundled scope) |
| 38 | irrevers-b4b37bf0 | C | VERIFIED as bookkeeping close | GIT VIA | partial |
| 39 | irrevers-b0a453b2 | C | VERIFIED as self-reported | SELF | weak-unverifiable |
| 40 | irrevers-f61efd80 | C | CORRECTED (minor) | GIT | partial |
| 41 | irrevers-29a9131c | C | VERIFIED as self-reported | SELF | weak-unverifiable |
| 42 | irrevers-ed77224f | C | VERIFIED | GIT | strong (caveat: Argo-API drive operational-only) |

## Diff vs the compiled-inventory ratings

**Changed ratings: none.** The re-rating reproduces the compiled inventory's
classification exactly — 42/42 unchanged, and the weak/partial counts match
the `b15d587` baseline (5 weak-unverifiable + 6 partial). There is therefore
no per-change justification to give; the compiled inventory's weak-evidence
flags remain current as of this pass and need no refresh.

For completeness, the five CORRECTED verdicts that could in principle have
moved a rating, each with the one-line reason it does not:

- **irrevers-e2bb8fbf** (strong) — the correction reclassifies `0fb164d` as
  an *earlier* touch, not a later one; the closing evidence itself
  (`c7e9df5`, test at :450, 2 m 02 s before close) verified exactly, so the
  rating's basis is untouched.
- **irrevers-c87a3c50** (weak-unverifiable) — the gap to `97b8eba` is
  ~12.4 h, not ~9.5 h; the direction is unchanged and the longer gap makes
  the void-rehydration record *more* hollow, not less.
- **irrevers-ff4f17da** (strong) — four conservative-trigger tests, not
  five (`:212` is the `release_is_fresh` helper, re-confirmed at this
  pass's HEAD); four exact-line tests plus an exact `+371` commit are more
  than enough for strong.
- **irrevers-9007792b** (strong) — the compiled inventory's `:1153` anchor
  is blank at every recorded HEAD, but the redaction test exists (see
  refinement below) and every other anchor and the `+1268 −40` stat are
  exact; a drifting line anchor is not an evidence-strength defect.
- **irrevers-f61efd80** (partial) — corrected timings (36 m 07 s before
  close for `97b8eba`; 10 h 01 m after close for `48d5a60`) preserve the
  T3 shape exactly — close-time anchor covered only half the described
  scope, and the rest postdates the close — so partial stands.

**Refinement found at this pass (third-order, no rating impact):** child 3's
suggested replacement anchor for the redaction test — ":1157 at HEAD" — is
itself off by one. `redacts_payloads_when_full_content_logging_is_disabled`
sits at **src/denial_log.rs:1158** at both the child-3 pass HEAD (`a7d6541`)
and this pass's HEAD (`afaf476`; no committed drift in the file between
them). The substantive half of that correction — the compiled inventory's
`:1153` anchor is blank at `e6771f0`, `c4385ac`, and HEAD — was re-verified
here directly. Cite **:1158** downstream.

## Explicit weak / unverifiable-evidence section (current)

Ratings unchanged from the compiled inventory; each entry now additionally
carries its covering pass's confirmation. 5 of 42 weak-unverifiable, 11 of
42 weak-or-partial.

### Weak-unverifiable (5)

- **irrevers-c87a3c50** — closed ~1.2 s after creation with a supersede-only
  close reason; no commit, no test; repo verifiably held none of the
  described code at close (tip `200bb7e`; `src/trust_pointer.rs` first
  appears ~12.4 h later in `97b8eba`). Void rehydration bookkeeping —
  child 2 confirmed the record exactly, correcting only the gap figure.
- **irrevers-340ae322** — close rests on a manual `docker manifest inspect`
  plus CI pod states; pods are podGC-deleted and the registry 401s anonymous
  manifest requests, so neither half is re-checkable today. Child 2
  re-verified the surviving git-side corroboration (0.1.0 pins at the close
  tip) and confirmed the weak-unverifiable class is correct.
- **irrevers-93baa29a** — entire evidence is 2026-09-07 manual root-run
  notes on an installed host; nothing in the repo can reproduce a
  host-level root run. Child 3 confirmed no commit cites the ID and that
  trust-the-notes-or-rerun is the accurate classification (expected shape
  for a verification-only bead, still unverifiable).
- **irrevers-b0a453b2** — the mutation experiment exists in no commit, and
  `e1aab5a` deliberately narrowed the exact behavior it verified. Child 3
  re-confirmed both halves and the re-anchor tests
  (regression_suite_scope_tests.rs :139/:83/:43).
- **irrevers-29a9131c** — block-then-pass experiment recorded only as prose
  (no hash, no test name); mitigated by the 2026-09-10 independent
  re-reproduction and release_gate_integrity_tests pins, but the closing
  record itself is unreproducible; residual redirect-channel-flip gap
  stands. Child 3 verified the mitigation artifact exists as claimed.

Unverifiable items found by the verification passes: **none new** — both
passes report 0 unverifiable, and the three inherently unre-checkable
closures above are exactly the ones the inventories had already flagged.

### Partial (6) — substance real, closure record defective

irrevers-eff8909f (self-authored tag table, two known-wrong claims,
substance re-verifies) · irrevers-f59f9313 (close predates implementing
commits by ~21–54 min) · irrevers-0f49129d (bookkeeping close, nothing
under its own ID) · irrevers-b4b37bf0 (no gate code at the close instant;
children delivered same day) · irrevers-f61efd80 (report-format scope
landed ~10 h after close) · irrevers-6de781f4 (launched-canary half rests
on a doc comment only). Every partial record re-verified against git
history by the covering pass.

### Strong with caveat (5) — not weak, listed for scrutiny

irrevers-84b36e47 (Argo run TTL-reaped; release object re-verified live by
both passes) · irrevers-2cb3dbd2 (sub-clause explicitly skipped at close) ·
irrevers-9eb4de16 ("fails without the fix" rests on the commit body's
manual negative control) · irrevers-ed77224f (Argo-API-drive area
operational-only) · irrevers-49dbb095 (linkage rests on the close note;
bundled redirect-message scope — `f0e0f9db` is a **bead ID**, not a SHA).

## Count check

42 beads = **31 strong** (26 clean + 5 with caveat) + **6 partial** +
**5 weak-unverifiable** = 42, matching the compiled inventory row for row
(A: 11/3/2, B: 19/1/1, C: 1/2/2; sections 16 + 21 + 5). Verdict roll-up
across the two verification passes (43 rows, 84b36e47 double-covered):
38 verified, 5 corrected (all minor), 0 unverifiable.

`docs/plan/plan.md` untouched by this pass (last touch `d26a83a`,
2026-09-11, bead irrevers-a39bdf35).
