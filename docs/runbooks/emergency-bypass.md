# Emergency bypass runbook

Use this runbook only when a critical service or recovery action is blocked by
icg and there is no safe alternative. `ICG_DISABLED=1` is an incident escape
hatch, not a faster approval path or a substitute for a repository override.
Every bypass must be narrow, time-bounded, and followed by an incident review.

If the guard has crashed or is returning `guard-crash` denials, first follow
the [incident response runbook](incident-response.md). If a rule-pack release
is causing a deny-rate spike, follow the [rollback runbook](rollback.md).

## Enforcement semantics and audit trail

`ICG_DISABLED=1` is read before rule-pack loading, hook payload parsing, and
guard-availability handling. It applies consistently to `icg check`, the
native PreToolUse hook, and a PATH-shadowed wrapper binary. The hook returns a
normal JSON `allow` response with the emergency warning in `systemMessage`; a
wrapper writes the warning to stderr before it executes the real binary.

Each activation emits an `icg_emergency_bypass` event and a bounded telemetry
record containing only the timestamp and front end (`check`, `hook`, or
`wrapper`). It deliberately does **not** contain the command, arguments, tool
payload, working directory, or environment values, because those can include
credentials or other secrets. A telemetry-cache write failure is itself
reported as an `icg_emergency_bypass` stderr event, but cannot block the
approved emergency operation.

The explicit bypass takes precedence over both Fail-Open and Fail-Closed
guard-availability behavior for that single process invocation. In other
words, a graduated Fail-Closed `guard-crash` denial does not override an
approved emergency bypass. Once `ICG_DISABLED` is absent, the configured
Fail-Open/Fail-Closed policy resumes unchanged.

## Activation criteria

An emergency bypass is justified only when all of the following are true:

- a production or security incident is actively impacting users, data,
  recovery, or service availability;
- icg is the confirmed blocker;
- the redirect or other safe alternative has been considered and cannot meet
  the incident objective in time; and
- an operator has recorded the risk and obtained incident-command approval, or
  has recorded why approval was unavailable.

Do not use this procedure for a false positive that can wait for normal review,
to test a command, or to avoid a release-bound override. A bypass does not make
the command safe and does not grant permission to run additional commands.

## Procedure

### 1. Confirm and declare the emergency

Capture the denial before changing the environment. Record the incident ID,
operator, host, harness, repository, service, exact command, rule-pack and
pattern IDs, denial/telemetry ID, timestamp, expected impact, and the risk of
executing the command.

```bash
icg status
icg health status
icg explain --denial <telemetry-id> --show-redirect
icg explain --pattern <pattern-id> --show-redirect
```

Confirm that the command is the smallest operation that will restore service.
Prefer a reversible repair, a backup, or a safe redirect. For destructive
operations, use a second operator to verify the target and arguments.

Create an incident record before execution. Store it in the incident system;
the following is a minimal local template when the incident system is
unavailable:

```text
EMERGENCY BYPASS RECORD
Incident: <incident-id>
Timestamp (UTC): <time>
Operator: <identity>
Approver or approval exception: <identity/reason>
Service and repository: <service>, <repo>
Blocked command: <exact command>
Rule pack / pattern / denial ID: <values>
Why the safe alternative cannot be used: <reason>
Expected result: <verification>
Risk and blast radius: <assessment>
```

### 2. Execute the smallest possible bypass

Prefer a one-command environment override so the disabled state cannot leak to
later commands:

```bash
ICG_DISABLED=1 <exact-dangerous-command>
```

Examples from the operator training scenarios:

```bash
ICG_DISABLED=1 vault policy write auth-policy /backups/auth-policy.hcl
ICG_DISABLED=1 git push --force origin corrected-branch
ICG_DISABLED=1 kubectl delete pvc stuck-pvc
```

If one recovery requires several commands, do not leave a shell-wide export in
place longer than necessary. Apply the variable to each explicitly reviewed
command, or use a dedicated, audited subshell and verify its environment before
running the command. Never put credentials or secret values in the incident
record or command transcript.

Do not edit `/etc/icg/rule-pack.json`, the trust pointer, or an override file to
achieve an emergency bypass. Do not use a mutable `latest` reference. Those
changes bypass the release and review controls and make recovery evidence
ambiguous.

### 3. Verify recovery and re-enable protection

Verify the operation's result and the affected service before closing the
emergency window:

```bash
<service-specific-verification>
unset ICG_DISABLED
icg health status
icg status
```

If `ICG_DISABLED` was exported by an existing shell, run `unset ICG_DISABLED`
and start a fresh shell or process for the verification. Confirm that a known
safe check and a known guarded destructive check return the expected decisions.
For example, verify the restored Vault policy, corrected remote branch, or PVC
state before allowing normal traffic.

If the bypassed command failed, do not repeatedly retry it with the guard
disabled. Stop, preserve the evidence, and escalate to incident response.

### 4. Close out the bypass

Within the incident's response window:

1. Attach the bypass record, command output, service verification, and icg
   health/telemetry evidence to the incident.
2. Record the exact start and end time of the bypass and confirm the variable
   is absent from the operator and service environments.
3. Explain why the rule or redirect did not support the recovery path.
4. Decide whether the durable fix is a rule-pack change, a safe redirect, or a
   release-bound repository override. Route that change through the relevant
   [rule-pack update](rule-pack-updates.md) or
   [override approval](override-approval.md) runbook.
5. Schedule a postmortem for a critical bypass and include a prevention owner
   and due date.

An emergency bypass is complete only when icg is active again, the service is
verified, and the evidence is retained.
