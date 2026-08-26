# Rule-pack update runbook

This runbook governs publishing and deploying a new icg rule-pack release.
Rule packs are trusted release data, not mutable local configuration. The
operator must validate the candidate, complete Layer 1 and Layer 2 review, and
advance an exact trust pointer before installing it. A pack update changes
what operations are denied, redirected, or allowed, so treat it as a safety
release.

For the final human release gate, use the [release-cutting
runbook](release-cutting.md). If post-deployment deny rates spike, use the
[rollback runbook](rollback.md) rather than attempting another update.

## Preconditions and preparation

Before changing a host or channel:

- schedule a maintenance window and identify the operator, reviewer, candidate
  release tag, full candidate SHA, and target cohort;
- confirm the current trusted reference and pack are healthy;
- ensure the previous trusted release and its artifact are available for
  rollback;
- confirm root/admin-controlled ownership of the binary, pack, and trust
  pointer; and
- do not use `latest`, a mutable branch, or a hand-edited production JSON file.

Capture the baseline and create a recoverable backup:

```bash
icg trust show
icg status
icg health status
icg telemetry status
icg backup create --output /tmp/icg-before-<candidate>.tar.gz
icg backup verify /tmp/icg-before-<candidate>.tar.gz
```

Do not reset telemetry to make a candidate look healthy. Record the baseline
window and current deny rate with the change evidence.

## Build and validate the candidate (Layer 1)

Run the checks against the exact previous and current manifests that will be
used for the release. Review the diff as a security report, not merely as a
test artifact.

```bash
cargo test
icg redos-check current-release.json
icg regression-suite current-release.json \
  --output regression-current.json
icg coverage-diff previous-release.json current-release.json \
  --justification "<required justification for any reported regression>"
```

Layer 1 is complete only when the fixed deny-regression suite passes and the
`coverage-diff/v1` report has been produced by the same CI run. Investigate
every removed guarded pattern, newly disabled rule, widened safe pattern,
narrowed destructive pattern, and newly exempted override. A justification is
not permission to ignore a finding; eliminate unexplained regressions.

If the release contains a repository override, include both manifests in the
diff and exercise the approved override in the regression suite:

```bash
icg coverage-diff previous-release.json current-release.json \
  --previous-override overrides/example.previous.toml \
  --current-override overrides/example.toml \
  --justification "<why the exemption is necessary>"
icg regression-suite current-release.json \
  --override-file overrides/example.toml \
  --repository jedarden/example \
  --trusted-ref <candidate-release-ref>
```

Also check the pack for valid JSON/schema, expected pattern IDs and channels,
safe redirects, fixture coverage, performance, and false positives. Preserve
the complete CI run URL/artifact and the candidate SHA in the release record.

## Layer 2 review and release

Layer 2 reviews the exact `coverage-diff/v1` report from Layer 1. The reviewer
must verify:

1. the report and CI run refer to the candidate SHA;
2. every finding has an eliminated change or an explicit, technically sound
   disposition;
3. any new exemption has a bounded scope, future expiry, non-blank
   justification, and a 90-day re-justification plan; and
4. a second adversarial pass is recorded when the report contains findings.

Record reviewer identities, UTC time, decision, CI/report artifact, candidate
SHA, and a disposition for every finding. A missing record, unexplained
finding, stale report, or mismatched SHA blocks release. Complete the
[release-cutting runbook](release-cutting.md) and create a new, immutable tag;
never move or reuse a tag.

## Canary deployment

Deploy the release to a small, identified cohort using the channel-specific
trust pointer. The pointer must already be authorized by the release record.

```bash
sudo icg trust set <candidate-release-ref> \
  --channel canary \
  --justification "Layer 1/2 approved candidate <full-sha>; canary cohort <id>"
sudo icg update --channel canary
sudo icg trust show --channel canary
sudo icg status --channel canary
```

Run the canary smoke checks for allowed, redirected, and denied operations,
then observe the agreed window. Check health, telemetry, latency, error rate,
denial distribution, and hook behavior. Unknown or missing telemetry is not a
clean result. Stop and follow rollback if the pack fails to load, a fixed deny
case allows, safe behavior changes unexpectedly, or the deny rate is anomalous.

The updater fetches the exact `icg-packs.tar.gz` asset named by the trusted
release, validates every root-level JSON pack, and atomically exchanges the
installed directory. The previous tree is retained as `packs.previous`; packs
removed by the release do not survive activation. Verify the downloaded
directory and trust pointer after the update; do not manually copy a file over
the active pack.

## Stable rollout

Promote only after the canary evidence is complete and attached to the release
record. Advance the stable pointer to the same exact release reference, then
update cohorts in controlled batches:

```bash
sudo icg trust set <candidate-release-ref> \
  --justification "Canary passed; Layer 1/2 evidence <record-id>"
sudo icg update
sudo icg trust show
sudo icg status
sudo icg health status
```

After each batch, verify the active pack, one known deny, one safe redirect,
and the telemetry baseline before continuing. Keep the previous trust reference
and backup until the observation window closes.

## Stop conditions and completion

Stop the rollout immediately when any check fails, telemetry is incomplete,
the trust pointer does not equal the reviewed reference, or operators cannot
explain a new denial. Do not broaden the pack or disable telemetry to make the
rollout pass. Use the [rollback runbook](rollback.md), record the release and
reason, and reopen Layer 1/2 review for the corrected candidate.

The update is complete when every intended cohort reports the reviewed exact
reference, smoke checks pass, the observation window is clean, and the release
record contains the candidate SHA, CI evidence, Layer 2 review, canary/stable
rollout evidence, and final trust-pointer state.
