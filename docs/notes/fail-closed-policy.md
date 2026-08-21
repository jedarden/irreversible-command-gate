# Fail-Closed policy notes

This note records the implemented policy boundary and the design decisions
from the fail-closed work. It is not the activation runbook. For current
commands, defaults, alerting, and emergency procedures, use the
[Fail-Closed Mode guide](../operators/fail-closed-mode.md).

## Implemented boundary

`icg` distinguishes a normal rule decision from guard availability:

- A normal allow, deny, warning, or rewrite is returned in both modes.
- An in-process engine/pack-loading failure is allowed in Fail-Open and denied
  as `pack=fail-closed, pattern=guard-crash` in Fail-Closed.
- `GuardLifecycle` persists a run marker before hook/wrapper work. A stale
  marker on the next invocation records one crash and applies the durable
  policy. This is the available process-disappearance evidence in the current
  per-invocation architecture.
- A harness must still deny process errors, timeouts, missing output, and
  malformed output if those failures occur outside the process's response
  boundary.

The default is Fail-Open. The administrator policy file is
`/etc/icg/fail-closed-policy.json`, or the path in `ICG_FAIL_CLOSED_POLICY`.
Missing or unreadable policy state bootstraps Fail-Open so a configuration
problem does not wedge a fleet before it can be repaired. The legacy
`ICG_FAIL_CLOSED=true` setting can only make the current process stricter; it
does not demote a durable Fail-Closed policy when set to false.

## State and graduation

The implemented durable state has `FailOpen` and `FailClosed` modes. The
historical transition design also describes `Graduating`; in this repository
that is a deployment/canary procedure rather than a persisted enum value.

`PolicyStore` uses the existing poison-pill evidence rather than creating a
second deny-rate detector. With the defaults, reconciliation requires a fresh
trusted release with at least 100 observations, a baseline of at least three
prior releases and 300 observations, a current observation count no greater
than 1,000, and no concerning deviation. Three unique eligible releases are
required by default to commit Fail-Closed. Release references are counted at
most once.

The policy consumes rollback evidence in this order:

```text
release-bound evaluations
        |
        v
prior-release deny-rate baseline --> poison-pill anomaly
        |                                      |
        |                                      v
        +------------------------------> exact trust-pointer rollback
                                               |
                                               v
                                  PolicyStore qualification observer
```

An ordinary denial is not a poison pill. A poison-pill rollback resets an
open-mode clean streak. Once Fail-Closed is committed, the bad rule-pack
release is rolled back but the guard-availability policy remains Fail-Closed.
Policy demotion is an explicit emergency action and records a reason and
generation.

## Persistence and safety

Policy state is separate from the cache state store, trust pointer, telemetry,
and health state. Policy writes are locked, validated, atomically replaced, and
backed up to `fail-closed-policy.json.bak`. Transition events are structured
and bounded. The cache state store is durable evidence but is not an authority
for changing Fail-Closed mode.

The implementation incorporates the predecessor designs as follows:

- the state-machine boundary keeps guard availability separate from ordinary
  rule outcomes;
- health tracking persists crashes, uptime, clean runs, stale-run detection,
  and cgroup-aware OOM evidence;
- runtime enforcement is explicit and remains Fail-Open by default; and
- poison-pill graduation reads rollback evidence without mutating the
  poison-pill store, while force-graduate and force-revert are audited manual
  controls.

See the [historical transition design](../design/fail-closed-transition.md)
for the rationale and the [operator guide](../operators/fail-closed-mode.md)
for procedures.
