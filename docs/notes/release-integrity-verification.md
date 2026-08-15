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

- **Fixed regression suite.** Every enabled `guarded_pattern` in every rule pack
  has a paired test case — a real example command that must come back
  `deny`. The release candidate runs the full suite; if anything
  previously caught is no longer caught, the release doesn't get built.
  This is the core mechanism: it doesn't depend on anyone noticing a
  diff, it's a pass/fail gate on actual behavior.
- **Structured coverage-diff.** Diff the new rule-pack manifest against
  the last release's, mechanically, at the data level: which
  `guarded_pattern` IDs were removed, which guarded patterns were newly
  disabled, which `guarded_pattern` regexes got *narrower* (especially those where `destructive: true`), and which `safe_pattern` regexes got *wider*. Any of these
  four changes silently reduce coverage without necessarily failing the
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
field. Each of the four sections is always present, even when it contains
`None`:

- `Removed guarded_patterns` lists the pattern ID, its previous check value,
  and `current: <removed>`.
- `Disabled guarded_patterns` lists each pattern whose `enabled` flag changed
  from `true` to `false`, with `previous: true` and `current: false`.
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

### Layer 2 review procedure

Layer 2 is a release-candidate review, not another regex-diff implementation.
The reviewer must receive the `coverage-diff/v1` report produced by the same
Layer 1 CI run that tested the candidate. The report is the review input; the
raw rule-pack diff is not a substitute for it.

The reviewer performs these checks, in order:

1. **Bind the report to the candidate.** Confirm that the CI run passed on the
   exact candidate commit, that `format: coverage-diff/v1` is present, and
   that the report's previous/current manifests are the releases being
   compared. A missing, stale, or unbound report is a review failure.
2. **Read all four sections.** Check `Removed guarded_patterns`, `Disabled
   guarded_patterns`, `Widened safe_patterns`, and `Narrowed guarded_patterns (destructive: true)`,
   including every `pattern_id` and its `previous`/`current` values. A clean
   report must say `status: no_regressions` and show `None` in each section.
3. **Resolve every finding.** If the report says
   `status: regressions_detected`, the reviewer must decide separately for
   each finding whether it is an intentional, documented change or an
   unexplained loss of coverage. The report must contain a non-blank
   `justification:`; its presence makes the finding reviewable, but does not
   make the change approved. Unexplained findings require changes before
   release.
4. **Use a second pass for adversarial review.** A reviewer other than the
   author, or a fresh review session, should try to show that each accepted
   removal, widening, or narrowing weakens protection. The second pass reads
   the same report and findings rather than starting over from raw regex.

The reviewer records the decision with the candidate commit, the report
artifact or CI-run URL, reviewer identity, review time, decision, and the
disposition of every finding. A minimal record is:

```text
candidate_commit: <full commit SHA>
coverage_report: <artifact or CI-run URL>
report_format: coverage-diff/v1
reviewer: <human or review-session identity>
decision: approve | request_changes
findings: <none, or one disposition per pattern_id>
justification_verified: yes | no | not_required
reviewed_at: <UTC timestamp>
```

Approval is valid only when the report is bound to the candidate, Layer 1 has
passed, and every reported regression is either absent or explicitly accepted
with a recorded disposition. The `--justification` value is evidence for the
review, not evidence that the reviewer approved it. The human may inspect the
raw rule-pack expressions to understand a finding, but a raw regex diff alone
cannot replace the generated coverage-diff report or the review record.

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
