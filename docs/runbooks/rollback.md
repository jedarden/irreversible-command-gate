# Rollback runbook

Use this runbook when poison-pill telemetry identifies a rule-pack release
whose deny rate has spiked above the established baseline. The normal path is
automatic trust-pointer rollback to the previous known-good release. This
runbook covers verification, failed automatic rollback, and post-rollback
containment.

A single normal denial is not a poison pill. A poison pill is a release-bound
deny-rate anomaly after the configured minimum sample count. A release rollback
also differs from fail-closed policy demotion: reverting a bad pack does not
change the crash policy. In Fail-Closed state, the fleet remains Fail-Closed
after the pack is rolled back.

## Automatic trigger and invariants

After each evaluation, telemetry records the verdict and, when available, the
active release reference. Once the rolling window has enough samples, the
anomaly detector compares the current deny rate with its baseline and configured
spike threshold. On an armed anomaly it:

1. reads the current trusted reference and its stored previous reference;
2. checks that auto-rollback is enabled and not on cooldown;
3. atomically writes the previous exact reference to the trust pointer;
4. records the rollback time and anomaly evidence; and
5. reports the current release, previous release, severity, rates, and whether
   rollback succeeded.

If auto-rollback is disabled, the detector is on cooldown, or no previous exact
reference exists, the anomaly is not silently treated as healthy. Page the
operator and follow the manual path below. Never guess a previous release and
never use `latest`.

## Respond to a rollback alert

1. Freeze promotion of the triggering release and declare an incident if the
   anomaly affects production.
2. Capture the alert/report before resetting anything. Record detection time,
   active and previous references, current and baseline deny rates, threshold,
   sample count, severity, channel/cohort, and rollback result.
3. Inspect the local evidence:

   ```bash
   icg telemetry status
   icg trust show
   icg health status
   icg status
   ```

4. Confirm the trust pointer now names the previous exact release. Confirm that
   the installed artifact has been refreshed through the normal updater and
   that the active pack is readable and valid.
5. Run a known-safe check and a known guarded deny check. Monitor deny rate,
   health, hook errors, and service behavior through the agreed observation
   window.

Do not run `icg telemetry reset` during response. It destroys the baseline
needed to explain the trigger and does not fix the pack.

## Manual recovery when automatic rollback failed

Manual rollback is an administrator action and must be performed out-of-band
from the affected agent session. First establish that the previous reference
is the one recorded in the trust-pointer/runtime state and that its release
evidence is available. If that cannot be proven, stop and escalate rather than
selecting an arbitrary version.

```bash
icg trust check <previous-exact-release-ref>
sudo icg trust set <previous-exact-release-ref> \
  --justification "Poison-pill rollback for <triggering-ref>; incident <id>"
sudo icg update
icg trust show
icg status
```

If the pointer write or update fails, keep the rollout stopped. A cooldown is a
loop-prevention signal, not permission to force repeated pointer changes; use
incident command approval for any exceptional action and preserve the failure
evidence.

After a manual pointer change, verify the previous pack, regression/smoke
decisions, and cohort convergence before resuming operations. If the previous
artifact is unavailable, use the approved backup and deployment mechanism, not
an ad-hoc file edit.

## Contain and investigate the triggering release

Quarantine the triggering tag/reference so it cannot be promoted again. Keep
the release and telemetry immutable for analysis. Compare the previous and
triggering manifests and inspect:

- newly denied or widened patterns and changed redirects;
- repository overrides or expiry changes;
- pack parsing, regex/ReDoS, and performance behavior;
- cohort-specific rollout differences; and
- whether the anomaly is a legitimate traffic change rather than a pack
  regression.

The rollback is successful only if the exact previous release is trusted,
health is stable, and the deny-rate anomaly clears without hiding evidence.
Do not re-enable the triggering release merely because its tests pass after the
fact. A corrected candidate must complete the full [rule-pack update
runbook](rule-pack-updates.md), including fresh Layer 1 and Layer 2 evidence.

## Completion record

Attach the detector report, trust-pointer before/after values, updater output,
artifact identity, verification results, affected cohorts, and operator/reviewer
identities to the incident. Include whether rollback was automatic or manual,
whether it was cooldown-suppressed, and the root cause or next investigation
owner. Keep the previous release available until the post-rollback observation
window and incident review are complete.

If the incident is actually caused by guard disappearance or a fleet wedged in
fail-closed mode, stop using this release rollback path and switch to the
[incident response runbook](incident-response.md). Policy demotion is a
separate, explicit emergency action.
