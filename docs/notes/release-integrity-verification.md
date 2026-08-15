# Confirming a release isn't nerfed

`docs/notes/self-update-and-release-gating.md` establishes that
release-cutting needs a human gate, separate from routine CI-on-push. This
note answers the next question: a human clicking approve isn't itself
proof the content wasn't weakened — especially reviewing a rule-pack diff
by eye is exactly the kind of task humans are bad at catching subtle
regressions in. Four layers, each catching a different way a release could
be nerfed without anyone noticing.

## Layer 1 — deterministic CI gate (load-bearing; Phase 0, required)

Two automated checks, both run by the `icg-ci` release pipeline, both
**fail the build** (not just warn) on regression:

- **Fixed regression suite.** Every `guarded_pattern` in every rule pack
  has a paired test case — a real example command that must come back
  `deny`. The release candidate runs the full suite; if anything
  previously caught is no longer caught, the release doesn't get built.
  This is the core mechanism: it doesn't depend on anyone noticing a
  diff, it's a pass/fail gate on actual behavior.
- **Structured coverage-diff.** Diff the new rule-pack manifest against
  the last release's, mechanically, at the data level: which
  `guarded_pattern` IDs were removed, which `guarded_pattern` regexes
  got *narrower* (especially those where `destructive: true`), which `safe_pattern` regexes got *wider*. Any of these
  three changes silently reduces coverage without necessarily failing the
  regression suite (a narrowed regex can still pass every existing test
  case while missing new variants the tests don't happen to cover) — so
  this check exists specifically to catch what the behavioral suite can't.
  Any hit here blocks the release pending explicit justification, not an
  automatic fail — additions and genuine deprecations are legitimate,
  silent narrowing isn't.

### Coverage-diff report contract

The coverage-diff command emits the versioned Markdown format
`coverage-diff/v1` for the Layer 2 reviewer. The header contains the previous
and current manifest paths, a `status` field, and a required `justification`
field. Each of the three sections is always present, even when it contains
`None`:

- `Removed guarded_patterns` lists the pattern ID, its previous check value,
  and `current: <removed>`.
- `Widened safe_patterns` lists the pattern ID plus `previous` and `current`
  check values.
- `Narrowed guarded_patterns (destructive: true)` lists the pattern ID plus
  `previous` and `current` check values. Only guarded patterns marked
  `destructive: true` in both manifests are included in this section.

When any regression is present, the report's `justification` field must be
populated with a non-blank release-approval rationale using
`--justification <rationale>`. Without it, the command exits non-zero after
printing the report with `justification: REQUIRED`. A clean report records
that justification is not required because no coverage regression was found.

## Layer 2 — review, informed by Layer 1's output (Phase 0, required)

The human gate from `self-update-and-release-gating.md`, refined: the
reviewer looks at Layer 1's generated coverage-diff report — "these 3
patterns were removed, this regex got narrower" — not a raw 500-line
regex diff. That's the difference between a review that can actually catch
something and one that's a rubber stamp. Worth adding an adversarial
second pass distinct from whoever authored the change — a fresh agent
session specifically prompted to argue *why* a given diff might be a
weakening, mirroring the adversarial-verify pattern this environment
already uses elsewhere for code review — since even a careful single pass,
human or agent, tends to miss what a second pass looking specifically for
problems catches.

## Layer 3 — build provenance/signing (later hardening, not Phase 0)

Addresses a *different* threat than Layers 1-2: not "the reviewed content
was weakened," but "the published binary doesn't actually match what was
reviewed" — substitution or tampering between review and publish.
Sigstore/cosign-style attestation, or simpler (the Argo Workflow signs
what it builds), binds the artifact to a specific commit. Valuable, but
Layers 1-2 already address the threat this whole design thread started
from (an agent weakening rules during authoring); this is complementary
supply-chain hardening, not required to close that specific gap. Deferred.

## Layer 4 — deploy-side decoupling (minimal version: Phase 0; staged rollout: later)

**Minimal version, Phase 0:** the self-updater doesn't trust bare "latest
GitHub release." It tracks a specific, separately-advancing pointer (a
tag or small manifest file) that only moves forward after Layers 1-2 pass.
This decouples "CI produced an artifact" from "this artifact is trusted
for fleet-wide auto-deploy" — a second choke point, and cheap: it's one
more indirection, not new infrastructure.

**Staged/canary rollout, later:** a new release goes to one instance
first, gets exercised against the regression suite live there, before
other hosts adopt it. Deferred because it needs multiple live guard
instances to be meaningful — not useful until this project actually has a
fleet of deployments, which Phase 0/1 doesn't yet.

## How to apply

Phase 0's release-gating requirement is satisfied by Layers 1, 2, and the
minimal form of Layer 4 (trust pointer, not "latest"). Layer 3 (signing)
and full staged rollout are real, worthwhile hardening but not blocking —
track them as later work, not as part of what makes the initial deploy
trustworthy.
