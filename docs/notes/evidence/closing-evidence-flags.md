# Closing-evidence flags — weak or unverifiable closures among the 41 inventory beads

Final step of irrevers-c52de1f2's chain (dispatched as irrevers-fa1b03f9).
Reviews the evidence in `docs/notes/evidence/consolidated-evidence-summary.md`
and the four per-category files, and flags every closed bead whose **closing
evidence** is weak, missing, or unverifiable, for downstream scrutiny. Written
2026-09-11 at HEAD `cc68127`. No edits to `docs/plan/plan.md`.

Scope: the 41-bead inventory in
`docs/notes/closed-beads-release-verification-inventory.md`. The other ~312
closed beads in the workspace (test/smoke, starvation-alert, genesis
duplicates, scenario housekeeping) were excluded as noise by the inventory
bead by design and are out of scope here.

Two distinctions the flags turn on, both established in the consolidated
summary and re-verified for this pass:

- **Evidence for the work** vs **evidence for the closure**. Several beads
  have verifiable work sitting under other IDs or landing after close; the
  closure record itself is still weak (bookkeeping close, or close predating
  the work). Those are flagged even though nothing is missing from the code.
- **Re-checkability**. Self-reported evidence was real when recorded but is
  not independently re-checkable from git; several such claims are also no
  longer re-checkable from the live systems they described.

## Tier 1 — FLAG: no verifiable closing evidence at all

- **irrevers-c87a3c50** (Base self-updater and trust pointer) — closed
  **1 second after creation** with no notes and no assignee, as a
  bead-forge→bead-rs rehydration artifact (rehydration commit `0941686` landed
  2 s later); no implementing commit is attributable to it, and the repo
  verifiably contained none of the described code at the close instant (tip
  was `200bb7e`; `src/trust_pointer.rs` first appears ~9.5 h later in
  `97b8eba`). Its scope later landed under successors irrevers-5fdc2e13 and
  irrevers-f59f9313, so treat this closure as void bookkeeping and reconcile
  the scope against those two — never reopened despite the identical
  false-closure shape `943b3ca` documented for siblings.

## Tier 2 — FLAG: self-reported only; close-time evidence not re-checkable

All five beads the consolidated summary classes SELF-REPORTED ONLY. In each
case the evidence existed and was plausibly genuine at close, but nothing in
git carries it, and for two of them the live systems described have since
changed under it.

- **irrevers-340ae322** (Prove argo-guarded-builder:0.1.0 published/pullable)
  — evidence is a manual `docker manifest inspect` plus pod states from
  workflow `icg-ci-rg9n7`; the pods are unrecoverable (podGC OnPodCompletion)
  and the registry now 401s anonymous manifest requests, so **neither half of
  the claim can be re-verified today** — only indirect git-side corroboration
  remains (VERSION and both `image:` pins read 0.1.0; later audits saw fresh
  0.1.0 pulls, e.g. `icg-ci-czdkx` 2026-09-11).
- **irrevers-eff8909f** (Inventory of shipped releases v0.1.0–v0.1.6) —
  deliverable is a tag table in its own notes with **no carrying commit**;
  the content does independently verify (all seven tags exist at the recorded
  SHAs, v0.1.0 `f0fe556` … v0.1.6 `aab687d`), but the bead's "zero GitHub
  Releases" line was wrong for the mirror even at close, and the stated range
  is now far stale (v0.1.37 Latest at this pass).
- **irrevers-93baa29a** (Verify icg policy status/reconcile as root) —
  verification-only bead (the expected shape for its type): the entire
  evidence is the 2026-09-07 manual root-run notes; no commit was in scope
  and none exists, so a downstream reviewer must either trust the notes or
  re-run the checks — nothing is independently checkable.
- **irrevers-b0a453b2** (Verify regression-suite gate fails the build) — the
  mutation experiment (deny→`additional_context` on `vault-kv-destroy` →
  `cargo test` 101 / gate exit 1) exists in **no commit**, and
  `e1aab5a` (2026-09-06) deliberately narrowed that exact edge — the same
  flip now exits 0 with a reasoned skip — so the behavior it verified **is no
  longer the behavior at HEAD**; re-anchor its claim to the pinned-corpus
  invariants (`no_deny_regex_rule_is_ever_skipped` etc.) before relying on it.
- **irrevers-29a9131c** (Verify coverage-diff gate blocks unjustified change)
  — the block-then-pass experiment lives only in its close notes; mitigated
  (identical independent re-reproduction 2026-09-10/11; real-pack regression
  detection pinned by `tests/release_gate_integrity_tests.rs`), but a
  residual gap stands: a pure redirect-channel flip is not flagged by
  coverage-diff, and channel enforcement rests on the pinned corpus.

## Tier 3 — FLAG: work verifiable, closure record defective

Five beads whose implementing evidence is real and verified at HEAD but whose
**closure** was a timing or bookkeeping event rather than a completion
record. Flagged so downstream reconciliation reads these closures correctly.

- **irrevers-0f49129d** (Poison-pill auto-rollback, original bundled) —
  **no commit or completion note lands under its own ID**; closed the same
  second its successor was split off, with delivery via irrevers-b6579270
  (`5b4d5df`, `721f3d9`) + irrevers-ff4f17da (`d653ade`) — a bookkeeping
  close, not a completion claim.
- **irrevers-b4b37bf0** (Layer 1: regression-suite CI gate) — closed at the
  instant its scope split, when **no regression-suite code existed**;
  delivered same-day-later by children irrevers-7684fa60/irrevers-69594753
  (`0970190`, declarative-config `0f3f5faf`).
- **irrevers-f59f9313** (icg update: self-updater command) — closed **~21–54
  minutes before its implementing commits landed** (`9c6951b`, `d1e2b38`),
  which were bundled under unrelated commit subjects; the close notes were an
  unanchored completion claim (no hash, no test names).
- **irrevers-f61efd80** (Layer 1: coverage-diff CI gate) — closed before its
  described report-format/justification scope existed; that scope landed
  **~9.6 h after close** (`48d5a60`), and its own close notes carry only a
  terminology reconciliation.
- **irrevers-6de781f4** (Canary rollout via NEEDLE --identifier) — the
  mechanism half is git-verified (`90a9653`) but the **operational half (an
  actually launched `canary-icg` worker) has no verifiable evidence** beyond
  a doc comment (`src/trust_pointer.rs:102`); half the closure is
  unsubstantiated.

## Confirmed solid (evidence verified, not merely unflagged)

The remaining 26 inventory beads have solid closing evidence: an implementing
(or verification-pass) commit that resolves at HEAD and whose content matches
the bead's scope, with tests present where the scope produced them. Three of
them carry a recorded caveat that does not weaken the closure — noted so the
confirmation is honest, not blanket.

Clean, no caveats (22):

| Bead | Anchor evidence (re-verified at HEAD) |
|---|---|
| irrevers-e77615c8 | `2c541ec` + `c2fcfd9`; `rule-pack.json` live asset since v0.1.1 |
| irrevers-37eb1100 | `436bdce` added the runbook exactly as the close notes describe |
| irrevers-e2bb8fbf | `c7e9df5` matches the close-note claim incl. `maintenance_scenario_trust_channel_roundtrip` (line 450 at HEAD, verified) |
| irrevers-b6579270 | `5b4d5df` + `721f3d9`; both named tests at HEAD |
| irrevers-5fdc2e13 | `97b8eba`; all six close-note tests still at HEAD |
| irrevers-96594031 | `d1e2b38` + `a03a7e6` both verify; scoped check `verify_artifact_directory_security` present |
| irrevers-ca79d63a | `a03a7e6` matches the close notes line for line |
| irrevers-e00a5381 | declarative-config `122623ae`/`0f3f5faf`/`d11a6472`/`b2c2a0e5` all resolve; template live on iad-ci |
| irrevers-075634b8 | `409ca42`; both end-to-end tests at HEAD |
| irrevers-8d2d4a73 | `48acfc1`; both acceptance fixtures + `layer_one_malformed_stdin_fails_open` at HEAD |
| irrevers-aab3854c | `16f84c1` design doc covers every criterion |
| irrevers-cd3f4c44 | `bb362fb` + chain; threshold resolves at `src/fail_closed.rs` |
| irrevers-8a24ad8d | `17971b7`; acceptance-matrix tests at HEAD |
| irrevers-019c36d3 | strongest record in its section: notes name all four commits + exact verify commands; all resolve |
| irrevers-fffef435 | `3f0f00d`; named test at HEAD |
| irrevers-ff4f17da | `d653ade`; five conservative-trigger tests at HEAD |
| irrevers-3fc4bdde | `20808e9` (+`9ec6848`); named tests at HEAD, `#[ignore]` stripped |
| irrevers-f891f555 | `0a5faa9` names the bead ID in its subject |
| irrevers-edb5c4ca | same changeset `0a5faa9`, closed 36 s later; call-site comments at HEAD |
| irrevers-3e6c6fde | `0e669f2` names the bead ID; regression tests at cited lines |
| irrevers-50077acb | `c38b0cd`; Dockerfile-only by nature, build-time assertion is the standing check |
| irrevers-1517a263 | `0c062e3` names the bead ID; pollution-guard tests at HEAD |

Solid with a recorded caveat (4) — closure evidence is verifiable and
sufficient; the caveat is scope/verification-shape, not missing evidence:

| Bead | Caveat |
|---|---|
| irrevers-84b36e47 | the green Argo Workflow object itself is TTL-reaped; the surviving GitHub release object is the evidence (releases confirmed live again at this pass) |
| irrevers-2cb3dbd2 | one sub-clause explicitly skipped: deny-regression generation runs on current packs, not prior-release packs |
| irrevers-9eb4de16 | the "fails without the fix" criterion rests on the commit body's manual negative control (v0.1.3 warns), not a recorded pre-fix test run |
| irrevers-ed77224f | area (1) — driving the actual Argo API — is operational-only evidence (green four-asset releases), not an in-repo test |

Count check: 1 (Tier 1) + 5 (Tier 2) + 5 (Tier 3) + 22 clean-solid +
4 caveat-solid = **41**, matching the inventory row for row. The Tier 2 set
is exactly the summary's five SELF-REPORTED ONLY beads; the Tier 1 bead is
exactly its one NO VERIFIABLE EVIDENCE bead; every Tier 3 bead is a
summary GIT-VERIFIED entry carrying an explicit via-successors /
split-bookkeeping / late-landing / mechanism-only caveat — this pass
promotes those caveats to closure-evidence flags.

## How this was verified (this pass, 2026-09-11)

Independent spot-checks against HEAD `cc68127`; the rest is taken from the
consolidated summary, whose own verification pass (recorded on
irrevers-c52de1f2) this pass did not repeat in full:

- Every cited in-repo commit hash resolves via `git cat-file -e` (~45 checked,
  including every hash named above); declarative-config anchors `122623ae`,
  `0f3f5faf`, `d11a6472`, `b2c2a0e5`, `083fd82e` resolve in that repo.
- Eight named tests grepped at HEAD at exactly their cited lines.
- Close-instant arithmetic recomputed from bead timestamps vs commit author
  dates (c87a3c50, f59f9313, f61efd80, b4b37bf0, 6de781f4).
- v0.1.0–v0.1.6 tags resolve at the recorded SHAs; GitHub mirror releases
  confirmed live (v0.1.37 Latest at check time).
- `e1aab5a`'s message confirms the reasoned-skip narrowing behind the
  irrevers-b0a453b2 staleness flag; gate wiring confirmed at
  `containers/argo-guarded-builder/icg-ci-guarded-workflowtemplate.yml:130,
  149, 156`.
- Full (untruncated) notes read for all five Tier 2 beads — the evidence the
  summary quotes is intact on each bead; nothing was lost to the notes-update
  clobber hazard.
