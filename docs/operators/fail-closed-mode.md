# Fail-Closed Mode: activation and operations

This guide describes the implemented fail-closed guard-availability policy in
`icg`. It covers the native `icg hook` front end and the Unix PATH wrapper.
An ordinary rule denial is a successful guard decision in either mode.

Keep new installations in Fail-Open while the guard, harness, and telemetry
are being validated. Enable Fail-Closed only when a temporary workflow outage
is safer than one unvalidated operation passing the guard. Use the
administrator-controlled policy file; `ICG_FAIL_CLOSED=true` is only a stricter
compatibility override, not the durable activation or rollback mechanism.

## What the policy protects

| Situation | Fail-Open | Fail-Closed |
| --- | --- | --- |
| Guard evaluates normally | Normal allow/deny/warning/rewrite | Same |
| Individual evaluation or pack-loading failure | Allow and emit a failure event | Deny as `pack=fail-closed, pattern=guard-crash` |
| Tracked invocation disappears before clean exit | Record a crash; allow the next invocation | Record a crash; deny the next invocation |
| Poison-pill deny-rate anomaly | Roll back the release and reset open-mode qualification | Roll back the release and remain Fail-Closed |
| Policy file is missing or unreadable | Bootstrap Fail-Open and emit an error | Repair the policy; do not claim Fail-Closed |
| Approved `ICG_DISABLED=1` emergency invocation | Bypass this one invocation and record a secret-free audit event | Same; the explicit emergency bypass takes precedence |

The tracked disappearance is detected by the durable health run marker on the
next invocation. A harness that sees no process, timeout, or malformed output
must also be configured to deny hook failure if that is part of the deployed
Fail-Closed contract. An environment variable inside a process cannot react to
a process that never returns.

Fail-Closed is not a defense against an approved
[`ICG_DISABLED=1` emergency bypass](../runbooks/emergency-bypass.md), a deliberately modified binary, a
compromised host, direct library calls, absolute-path bypasses, or cloud-hosted
agent sessions. Keep root-owned deployment and harness controls in place.

## Architecture overview

```text
AI agent / automation
        |
        | PreToolUse JSON or wrapped argv
        v
icg hook or Unix PATH wrapper
        |
        +--> Engine --> rule packs --> allow / deny / warning / rewrite
        |       |
        |       +--> availability failure: allow in open, deny in closed
        |
        +--> HealthStore: run marker, crash history, uptime
        |       `-- stale marker on next start becomes one crash event
        |
        +--> StateStore: release evaluations, deny history,
        |       trust-pointer history, rollback metadata
        |
        +--> poison-pill detector --> exact previous trust pointer
        |
        `--> PolicyStore: mode, clean-release streak, generation, events
```

The stores have separate authority:

- `/etc/icg/fail-closed-policy.json` is the administrator-controlled mode
  decision. It is read at process start.
- `/etc/icg/trust-pointer.json` names the exact rule-pack release in use.
- The cache state store records observations and rollback metadata; it is
  evidence, not authority for Fail-Closed.
- Health state records process lifecycle evidence and syncs a snapshot into
  telemetry at lifecycle boundaries.
- Poison-pill rollback owns release rollback. Policy reconciliation consumes
  its durable rollback count and does not rewrite poison-pill data.

The current implementation has two durable modes, `FailOpen` and `FailClosed`.
The `Graduating` state in the historical design document is a deployment/canary
procedure, not a third enum value in the current policy file. Use a separate
policy path and cohort for independent canary state.

## When to enable Fail-Closed

Record these checks in the change or release record before enabling a cohort:

1. The cohort has an approved immutable trust pointer and reviewed rule pack.
   The binary, `/etc/icg`, and policy file are administrator-owned and not
   writable by the guarded agent.
2. The native hook is installed for every intended harness. A PATH wrapper is
   Unix-only, must find the real binary later in `PATH`, and does not cover
   absolute-path invocations.
3. The harness behavior for process error, timeout, missing output, and invalid
   output is known. Configure deny-on-hook-failure where the harness offers it.
4. Health state and telemetry are writable, and an operator can query
   `icg health status`, `icg telemetry status`, and `icg policy status`.
5. The poison-pill detector is enabled, has an exact previous trusted release,
   and has a complete baseline. Missing telemetry is unknown, not clean.
6. The cohort has no unexplained guard crashes, timeouts, stale run markers, or
   telemetry gaps during the review window.
7. On-call operators have practiced the emergency demotion below from an
   administrator shell or deployment controller, outside the blocked agent
   session.

The automatic release qualification defaults are:

- three unique, fresh trusted release references;
- at least 100 observations for the current release;
- at least three prior releases and 300 prior-release observations;
- no more than 1,000 current-release observations when the decision is made;
- an enabled detector and no concerning deviation; and
- no new rollback count, stale evidence, or incomplete observation.

The code reconciles poison-pill and release evidence; it does not automatically
turn every health metric into a clean-release failure. Reject promotion when
health evidence shows an availability fault even if reconciliation reports a
clean release.

## Activation procedure

Run these checks as the administrator who owns the deployment. Replace paths
only when the service explicitly uses the corresponding environment override.

### 1. Inspect the current state

```bash
sudo icg policy status
sudo icg health status
sudo icg telemetry status
sudo icg trust show
sudo icg status
```

Confirm `FailOpen`, the intended exact trust reference, no recent unexplained
crashes, and sufficient telemetry. Save the outputs with the change record
after removing sensitive command payloads.

### 2. Set the qualification threshold

The default is three unique eligible clean releases. Set a different positive
threshold before the final release is counted; changing it while open resets
the streak:

```bash
sudo icg policy configure --threshold 3
sudo icg policy status
```

Do not lower it during an incident to recover from failed qualification.

### 3. Reconcile after each approved release

Normal hook and wrapper paths reconcile after evaluation. Operators can inspect
or reconcile explicitly:

```bash
sudo icg policy reconcile
sudo icg policy status
```

`Pending` means evidence is missing or incomplete, not that the release is
clean. `CleanRelease` advances once for that exact reference; repeating the
command is idempotent. `Graduated` means the policy file committed `FailClosed`
and a new generation.

The CLI reconciliation command uses default poison-pill settings. The normal
hook path maps configured telemetry controls into the reaction, so when those
controls are customized its automatic reconciliation is authoritative.

### 4. Canary the committed policy

Use a small isolated cohort with its own administrator-controlled policy path,
for example `/etc/icg/fail-closed-policy-canary.json`. Set
`ICG_FAIL_CLOSED_POLICY` to that path in the canary service/hook environment,
deploy the committed policy, and verify the mode and generation. Keep the
stable cohort on its existing path. `--channel canary` selects a trust pointer
and artifact path; it does not create a separate fail-closed policy.

Prefer normal reconciliation. Use the manual override only with explicit
change approval:

```bash
sudo env ICG_FAIL_CLOSED_POLICY=/etc/icg/fail-closed-policy-canary.json \
  icg policy force-graduate \
  --reason "CHG-1234: approved canary fail-closed activation"
```

Before expansion, require a complete canary observation window with zero
availability faults, no telemetry gaps, and no poison-pill event.

### 5. Expand and verify convergence

Roll out the committed policy through the deployment system. For every cohort:

```bash
sudo icg policy status
sudo icg health status
sudo icg telemetry status
```

Record mode, generation, trusted release, binary identity, and policy path.
Mixed generations are an incident. Keep the previous policy snapshot until the
observation window and change review are complete.

## Configuration reference

### Policy controls

| Control | Default | Meaning |
| --- | --- | --- |
| `ICG_FAIL_CLOSED_POLICY` | `/etc/icg/fail-closed-policy.json` | Administrator-controlled policy path. |
| `graduation_threshold` | `3` | Unique eligible clean releases required to switch modes. Must be positive; changing it while open resets qualification. |
| `mode` | `fail_open` | Durable posture: `fail_open` or `fail_closed`. |
| `generation` | `0` initially | Monotonic transition version; use it to detect mixed cohorts. |
| `clean_release_streak` | `0` | Current count of unique eligible clean releases. |
| `counted_releases` | empty | References already counted; duplicates do not advance the streak. |
| `last_poison_pill_event` | none | Last rollback/deviation event consumed by policy reconciliation. |
| `events` | empty | Bounded structured transition-event tail; at most 256 events are retained. |

Policy writes are locked, validated, atomic, and backed up as
`fail-closed-policy.json.bak`. Do not edit the JSON by hand during an incident;
use the CLI or the deployment controller.

`ICG_FAIL_CLOSED=true` (also `1`) is a legacy local/test override. It can make
the current process stricter than the durable policy. `false`, an unset
variable, or a stale environment value cannot demote a durable `FailClosed`
policy. Never use `ICG_FAIL_CLOSED=false` as rollback.

### Poison-pill and release-health controls

These are the defaults in `PoisonPillConfig` and `DenyRatePolicy`:

| Control | Default | Meaning |
| --- | --- | --- |
| `enabled` / `auto_rollback_enabled` | `true` | Anomaly reaction is armed. Disabled detection invalidates qualification. |
| `minimum_baseline_releases` | `3` | Prior release aggregates needed for a baseline. |
| `minimum_current_evaluations` | `100` | Current release observations before comparison. |
| `minimum_baseline_evaluations` | `300` | Observations represented by prior releases. |
| `minimum_absolute_deviation` | `0.05` | Current deny rate must be five percentage points above the prior mean. |
| `baseline_sigma_multiplier` | `3.0` | Current rate must also exceed mean + three population standard deviations. |
| `max_current_evaluations` | `1000` | Automatic action is limited to the early observation window. |
| `rollback_cooldown` | `3600s` | Minimum time between automatic trust-pointer rollbacks. |

The release baseline is made from per-release aggregates in
`session-state.json`, not from the legacy evaluation window alone. Ordinary
denials contribute to the rate but are not themselves poison pills.

The operator-facing telemetry file is controlled by `ICG_TELEMETRY_PATH` and
defaults to `/var/cache/icg/telemetry.json`. Its `icg telemetry configure`
knobs are:

| Option | Default | Meaning |
| --- | --- | --- |
| `--window-size` | `1000` | Legacy evaluation-window capacity used by telemetry status. |
| `--spike-threshold` | `3.0` | Sigma multiplier mapped into poison-pill detection on normal hook reconciliation. |
| `--minimum-samples` | `100` | Current-release minimum mapped into poison-pill detection. |
| `--cooldown-seconds` | `3600` | Rollback cooldown in seconds. |
| `--auto-rollback true\|false` | `true` | Enables the trust-pointer reaction; disabling it invalidates qualification. |

### Health controls

Health state uses `ICG_HEALTH_PATH` or the cache default
`/var/cache/icg/health-state.json`. OOM evidence uses `ICG_CGROUP_MEMORY_EVENTS`
or `/sys/fs/cgroup/memory.events` when present.

| Health setting | Default | Meaning |
| --- | --- | --- |
| `max_crash_history` | `100` | Crash records retained. |
| `stability_threshold` | `300s` | Running invocation is stable after five minutes. |
| `healthy_consecutive_runs` | `5` | Clean runs required for `Healthy`. |
| `max_crashes_per_hour` | `10.0` | More recent crashes yields `Unstable`. |

The shared runtime state defaults to `/var/cache/icg/session-state.json` and
retains at most 10,000 denial records and 32 release aggregates. These limits
are implementation constants, not policy switches. Trust pointers default to
`/etc/icg/trust-pointer.json`; channel pointers use
`/etc/icg/trust-pointer-<channel>.json`.

## Monitoring guide

### Operator checks

Run these on a schedule and after every binary, rule-pack, or policy change:

```bash
icg policy status
icg health status
icg telemetry status
icg trust show
icg status --health
```

Inspect policy `Mode`, `Generation`, `Clean Release Streak`, `Last Poison-Pill
Event`, `Last Transition`, and the last event type. Inspect health `Status`,
`Total Crashes`, `Recent Crashes (1h)`, `Crash Rate`, `Consecutive Clean Runs`,
`Stable`, and `Last Crash`. Also compare the exact trust reference with the
artifact actually loaded by the hook.

### HTTP and Prometheus endpoints

When an embedding service starts `HealthServer`, scrape these endpoints from
its configured bind address (the library default is `127.0.0.1:8080`):

| Endpoint | Use |
| --- | --- |
| `GET /health` or `/health/status` | JSON status and durable health metrics. |
| `GET /health/ready` | `503` when the server is not ready. |
| `GET /health/live` | `503` when the server or durable state is not alive. |
| `GET /metrics` | Prometheus text exposition. |

Health metrics include `icg_total_crashes`, `icg_recent_crashes`,
`icg_crash_rate`, `icg_consecutive_clean_runs`, `icg_health_status`,
`icg_is_stable`, and `icg_uptime_seconds`. Health status codes are
`0=Unknown`, `1=Healthy`, `2=Recovering`, `3=Unstable`, `4=Degraded`, and
`5=Dead`. Telemetry metrics include baseline/current deny rates and rollback
cooldown state when a complete snapshot is available.

### Alert meanings

Page immediately for:

- any `guard-crash` denial in a Fail-Closed cohort;
- any recent crash, OOM evidence, timeout, invalid hook response, or dead health
  status in a Fail-Closed cohort;
- `POISON-PILL AUTO-ROLLBACK FAILED`, a missing exact previous release, or an
  anomaly suppressed by cooldown while it remains present;
- a policy parse/permission failure, unexpected mode/generation, or mixed
  generations; and
- missing health/telemetry scrapes or stale state that makes evidence
  incomplete.

Warn and investigate for any Fail-Open guard fault (the operation may have run
without validation), `Recovering`/`Degraded` health, rising recent crashes, a
streak that resets or remains `Pending`, and a rising deny-rate trend below the
poison-pill threshold. Do not page on a normal expected rule denial by itself.

## Emergency rollback: Fail-Closed to Fail-Open

This is a policy rollback, not a rule-pack rollback. Use it only when guard or
harness unavailability is wedging a critical recovery path, from an
administrator shell or deployment controller outside the blocked agent
session.

1. Declare the incident. Record policy path, mode, generation, host/cohort,
   trusted release, first failure, and approval. Do not reset health or
   telemetry.
2. Demote the exact policy store. `force-revert` is the explicit emergency
   control; `demote` is equivalent:

   ```bash
   sudo icg policy force-revert \
     --reason "INC-1234: guard availability incident; restore operations"
   # For a non-default policy path:
   sudo env ICG_FAIL_CLOSED_POLICY=/etc/icg/fail-closed-policy-canary.json \
     icg policy force-revert \
     --reason "INC-1234: canary guard recovery"
   ```

3. Propagate the resulting snapshot and generation through the approved
   deployment mechanism. If the hook process cannot run, deploy the file
   directly through the administrator-controlled channel; do not edit it from
   the agent session.
4. Confirm every cohort has `Mode: FailOpen`, the expected higher generation,
   and the intended trust pointer. Mixed generations remain an incident.
5. Verify health and telemetry, repair the guard/harness, and run safe and
   known-guarded smoke checks. Treat operations allowed during the fault as
   unvalidated and assess them in the incident.
6. Keep the clean-release streak at zero. Re-enable Fail-Closed only after
   fresh qualification and a canary; emergency demotion never qualifies a
   fleet for automatic re-promotion.

`ICG_FAIL_CLOSED=false` does not override durable `FailClosed`. Do not change
the trust pointer as part of this procedure unless the incident is also a bad
rule-pack release; use the [release rollback runbook](../runbooks/rollback.md)
for that separate case.

## Troubleshooting

### `pack=fail-closed, pattern=guard-crash`

This is an availability failure, not an ordinary rule match. Check stderr,
`icg health status`, service supervisor/OOM evidence, and the policy generation.
Confirm that the harness did not turn a malformed response into a different
error. Restart only through the normal supervisor, then run a safe check and a
known guarded check. Do not solve it by changing a rule pack.

### The clean streak will not advance

Run `icg policy status`, `icg policy reconcile`, `icg trust show`, and
`icg telemetry status`. Common reasons are no trust pointer, telemetry that
predates pointer adoption, fewer than 100 current observations, fewer than
three prior releases/300 baseline observations, a disabled detector, a current
release beyond 1,000 observations, or a newly observed rollback count. Repair
the evidence source and start a fresh release observation; do not hand-edit
`counted_releases`.

### The policy unexpectedly starts Fail-Open

The runtime intentionally bootstraps Fail-Open when the policy file is missing
or unreadable. Check `ICG_FAIL_CLOSED_POLICY`, ownership and permissions of
`/etc/icg`, JSON validity, and the `.json.bak` recovery copy. Capture the error,
restore the last known-good administrator-controlled file, and verify the
generation. An unset or false legacy variable cannot demote a valid durable
Fail-Closed state; a true legacy variable can only make the current process
stricter.

### Health reports `Recovering`, `Unstable`, or `Dead`

`Recovering` means the current run has passed five minutes but has fewer than
five clean runs. `Unstable` means more than ten crashes were recorded in the
last hour. `Dead` means a prior run exists but no current run marker is active.
Inspect `last_crash_at`, crash type, signal/exit code, cgroup `oom_kill`, and
service logs. Preserve evidence before `icg health reset --force`; a reset
removes history and does not fix the cause.

### A poison pill did not roll back the release

Check whether auto-rollback is disabled, the one-hour cooldown is active, the
current release is outside the first 1,000 observations, or the exact previous
trust reference is missing. These are unsafe-to-ignore conditions: freeze the
rollout and follow the [rollback runbook](../runbooks/rollback.md). Never guess
the previous reference.

### Telemetry or health cannot be written

Check `ICG_TELEMETRY_PATH`, `ICG_HEALTH_PATH`, parent directories, disk space,
and the hook identity's narrow write permission. Telemetry and health are
operational evidence, not a reason to grant the agent write access to `/etc/icg`.
If evidence is unavailable, stop promotion and treat the release as pending.

### PATH wrapper does not behave as expected

The wrapper is Unix-only and must find the real tool later in `PATH`; inspect
the symlink, `PATH`, `ICG_RULE_PACK`, and `icg coverage --list`. It does not
cover absolute paths or direct library calls. If it cannot find the real binary,
repair the deployment and use the native hook for harness calls.

## Related procedures and onboarding

- [Incident response runbook](../runbooks/incident-response.md): crash,
  timeout, invalid-response, and fail-closed incidents.
- [Release rollback runbook](../runbooks/rollback.md): poison-pill or bad
  rule-pack rollback; it does not demote the policy.
- [Emergency bypass runbook](../runbooks/emergency-bypass.md): separately
  approved recovery when a rule denial, rather than guard availability, blocks
  a critical operation.
- [Operator onboarding](../onboarding-guide.md): learning path and first-day
  checks.
- [Historical transition design](../design/fail-closed-transition.md): design
  rationale and boundaries; this guide is authoritative for current behavior.

Before handing this feature to a new operator, ask them to explain why a
normal denial is not a poison pill, why `ICG_FAIL_CLOSED=false` does not perform
rollback, and which out-of-band command demotes a durable Fail-Closed policy.
If they cannot answer all three, stop the activation review and walk through
this guide with them.
