# Incident response runbook

Use this runbook when icg crashes, exits unexpectedly, times out, emits an
invalid hook response, or denies work because it is in fail-closed mode. It
also covers repeated fail-open guard faults that could permit an unvalidated
operation. A normal rule denial is not an icg availability incident.

The current policy distinguishes an individual evaluation error from guard
process disappearance. An in-process evaluation error remains fail-open and
must be reported. A deployed fail-closed front end denies when it cannot obtain
a trustworthy guard decision. `ICG_FAIL_CLOSED=true` is not by itself a
harness-level process-disappearance detector; the hook or wrapper must also be
configured to deny on guard error or timeout as described in the
[fail-closed transition design](../design/fail-closed-transition.md).

## Detect and classify

Page the on-call operator for any of the following:

- `guard-crash` or equivalent fail-closed denials;
- a guard process exit, OOM kill, segfault, abort, or repeated timeout;
- missing or malformed hook responses;
- more than one unexplained fail-open guard fault in a day; or
- a fleet-wide increase in failures after a binary or rule-pack deployment.

Do not classify a deny-rate spike as a crash. A deny-rate anomaly is a
poison-pill release event and belongs to the [rollback runbook](rollback.md).

## Immediate response

1. Declare an incident and assign an incident commander. Stop broad destructive
   operations and pause any rollout that is in progress. Do not reset telemetry
   or health state while investigating.
2. Identify the affected host(s), harness(es), channel, active trusted release,
   and whether the failure is fail-open or fail-closed.
3. Preserve the first failure's timestamp, request/denial ID, stderr, exit
   status, timeout, and relevant service logs.

```bash
icg status
icg health status
icg telemetry status
icg trust show
icg bug-report --output /tmp/icg-incident-<incident-id>.json \
  --pack /etc/icg/rule-pack.json
```

Copy the telemetry, health, denial, and runtime-state files to the incident
evidence store using the site's approved collection method. Treat those files
as evidence; do not expose command payloads, credentials, or secret values in
the incident ticket.

## Triage the failure

### Guard process or host failure

Check the service supervisor and host evidence for OOM kills, signals, resource
exhaustion, deployment changes, and filesystem/permission errors. If the
deployment provides a watchdog, record the event without rewriting history:

```bash
icg health record-crash \
  --crash-type <oom|segfault|abort|timeout|unknown> \
  --context "<incident-id and short context>"
```

Keep the original crash type and timestamp in the incident record even if the
watchdog reports it later. A clean restart is recovery, not proof that the
cause is fixed.

### Fail-closed denial

If the denial says `pack=fail-closed, pattern=guard-crash`, the hook received no
trustworthy guard result. Confirm whether the guard is running and whether the
harness's deny-on-hook-error/timeout setting is behaving as configured. Do not
loosen the policy by editing a rule pack or trust pointer.

If a critical recovery operation must proceed before the guard is repaired,
obtain incident-command approval and use the
[emergency bypass runbook](emergency-bypass.md). The bypass is out-of-band and
must be independently recorded.

### Fail-open fault

In fail-open mode, an availability fault allows the operation but emits a
high-priority health event. Treat the affected operation as unvalidated:

- identify what ran during the fault and assess whether it needs containment or
  compensating action;
- check for a shared cause across hosts, harnesses, and rule-pack versions;
- keep the fleet fail-open while reliability is investigated unless the
  approved transition state already provides a working enforcement point; and
- do not count the window toward fail-closed graduation.

## Recover safely

1. Quarantine a bad host or cohort if the deployment mechanism supports it.
2. Restart the guard through the normal supervisor, or redeploy the known-good
   binary and exact trusted rule-pack artifact. Do not install an unreviewed
   local pack as a test in production.
3. Check the release and pack before resuming traffic:

   ```bash
   icg trust show
   icg health status
   icg coverage --list --pack /etc/icg/rule-pack.json
   icg check --command "<safe-read-only-command>"
   icg check --command "<known-guarded-destructive-command>"
   ```

   The first check should allow or warn as expected; the second must retain its
   deny decision. Use an approved canary before returning a cohort to service.
4. Watch health and telemetry for a defined observation window. Verify that
   crashes, timeouts, and invalid responses remain at zero and that no new
   deny-rate anomaly appears.
5. If the evidence points to a rule-pack release, stop here and execute the
   [rollback runbook](rollback.md). If it points to the binary, harness,
   resource limits, or host, open the corresponding remediation work and do
   not re-enable fail-closed until the transition criteria are met.

## Fleet-wide fail-closed recovery

If fail-closed has wedged the fleet and normal operator actions cannot get a
trustworthy guard decision, policy demotion is an explicit, authenticated,
out-of-band change. It must not be performed through the blocked agent session.

1. Record the incident, reason, operator, time, affected generation, and expiry
   (if the emergency demotion is time-bounded).
2. Through the deployment mechanism, set `ICG_FAIL_CLOSED=false` or remove it
   for the affected cohort, and propagate the new policy generation. Confirm
   cohort convergence; mixed generations are an active incident.
3. Reset the clean-release qualification streak. Demotion never qualifies the
   fleet for automatic re-promotion.
4. Repair and validate the guard, harness, or host. Re-qualify with the
   configured clean-release threshold and a fresh canary before enabling
   fail-closed again.

This action reduces protection. It is a policy rollback, not a poison-pill
release rollback; use it only for guard or harness recovery.

## Resolution and postmortem

Close the incident only after the affected cohort is healthy, the intended
fail-open/fail-closed state is durable and converged, and monitoring is green.
The postmortem must include the failure class, release/trust reference,
availability evidence, operations permitted or denied during the fault,
containment, root cause, and a prevention action. A crash-free restart alone is
not sufficient evidence for graduation.
