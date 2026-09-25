# Rule 4 parity gate

The gate that decides when `org-rule-guard.py`'s blanket kubectl-mutation
rule (rule 4) may be retired from a host's live hook chain. Run it with
`scripts/rule-4-parity-gate.sh`; this page defines what the gate measures,
why it must be measured against the deployed stack, and exactly what passing
authorizes.

[ADR-001](../adr/001-kubectl-mutation-pack.md) records the retirement as a
consequence: rule 4 "can retire from `org-rule-guard.py` on the same terms
as rules 1–3, leaving only its Write/Edit credential-value rule." This page
is those terms.

## Why deployed parity, not code parity

Code-level coexistence — the shipped `packs/kubectl.json` agrees with the
Python hook on their shared verdict surface — is pinned in-repo by
`tests/kubectl_pack_tests.rs`,
`tests/kubectl_shell_payload_hook_integration_tests.rs`, and
`tests/coexistence_org_rule_guard_tests.rs`. Code parity is necessary but
not sufficient, because the registered hook front-end
(`icg hook` in `~/.claude/settings.json`) loads packs **only** from the
deployed pack directory (`/etc/icg/packs`): `--rule-pack` and
`ICG_RULE_PACK` exist, but the registered invocation carries neither, and
repository-relative pack discovery exists only in operator subcommands
(`icg check`, `icg coverage`) — never in hook mode. A pack that ships in
this repository but is not deployed to the host leaves the legacy rule as
the host's only mechanical enforcement of the deny surface.

This is not hypothetical. State found on codinghome, 2026-09-25: trust
pointer at v0.1.61 (set 2026-09-18; the kubectl pack first shipped in
v0.1.62 on 2026-09-19), ten packs under `/etc/icg/packs` with no kubectl
pack, and the deployed front-end allowing **every** deny-class probe while
`org-rule-guard.py` denied it. The same finding stopped the first
retirement attempt (`irrevers-c609004e`); the second (`irrevers-47f004b4`)
turned that bead's retry gate into this page and the script. Retiring rule
4 in that state would leave 25 deny shapes with zero enforcement.

The gate is behavioral on purpose, not a pack-presence check alone: an old
binary under a new pointer under-covers too. The deployed 0.1.63 binary
with the kubectl pack injected would still have allowed the `sh -c` payload
probes — the engine's payload handling (`irrevers-a4779a37`) is newer than
that binary. Only the corpus catches that.

## Preconditions

All of these must hold before the corpus is even meaningful. The script
checks them and exits 2 when they fail:

1. Both hooks are present: the legacy Python hook and the registered
   `icg` binary.
2. The deployed front-end is not emergency-disabled (`ICG_DISABLED` unset).
   Verdicts from a disabled guard prove nothing.
3. A `kubectl.json` pack exists in the deployed pack directory
   (`/etc/icg/packs` by default). This is the deployment prerequisite;
   advancing it is an operator act under the
   [rule-pack update runbook](../runbooks/rule-pack-updates.md) — never a
   hand-copied file.

The trust pointer reference is printed for the audit trail but is not
version-compared: pack presence plus the behavioral corpus is the
authoritative signal, and version arithmetic in a shell script is a
failure mode of its own.

## Corpus and pass criteria

The script pipes each probe, as an identical Bash PreToolUse payload, to
**both live hooks** from a neutral working directory (`/`), and compares
verdicts. Probe commands live in the script and travel to the hooks as
stdin JSON — never through the caller's own command line — so running the
gate does not trip the guards it probes. The corpus is 49 probes in three
classes:

- **shared-deny (25)** — the legacy rule's deny surface in the shapes both
  implementations model: every mutating verb including all four mutating
  `rollout` subcommands, flag-before-verb order, `sudo` and environment
  prefixes, absolute path, `&&`-chain position, downstream pipe, and a
  mutating verb inside a longer command. Both hooks must deny.
- **shared-allow (18)** — read-only verbs, `rollout status|history`, both
  Argo Workflow create carve-outs (`argo-workflows`, `iad-ci`), `exec`/`cp`
  /`port-forward`, `auth can-i`, a lookalike label value
  (`app=delete-me`), a mention in `echo`, and a read-only `sh -c` payload.
  Both hooks must allow.
- **icg-stricter (7)** — the deltas ADR-001 documents: `run` and `expose`
  (pack-only verbs), the flag-order create carve-out (the pack's safe
  pattern requires `create` *before* the namespace token, the Python hook
  matches the namespace anywhere in the segment), and the wrapper/payload
  prefixes the engine normalizes and the Python hook does not (`timeout`,
  `nice`, `sh -c`, `bash -c`). icg must deny; the legacy hook's allow is
  the accepted, counted divergence.

The gate **fails** on:

- any shared-class divergence; and
- any probe where the legacy hook denies and icg allows — an enforcement
  gap, wherever it appears.

Any other divergence not in the icg-stricter table is a corpus bug or a
behavior change: do not widen a class to absorb it, investigate it.

Exit codes: `0` pass, `1` corpus failure, `2` precondition failure
(takes precedence), `3` harness error — a hook missing, nonzero exit, hang,
or unparseable verdict. A nonzero hook exit is a harness error on purpose:
a fail-closed or broken hook is not producing parity verdicts, and that
condition is an
[incident](../runbooks/incident-response.md), not a gate result.

## What passing authorizes

Passing authorizes exactly one change, on that host:

1. Back up the current hook first
   (`cp ~/.claude/hooks/org-rule-guard.py ~/.claude/hooks/org-rule-guard.py.bak-pre-rule4-retirement-$(date -u +%Y%m%dT%H%M%SZ)`)
   — this backup is the rollback arm in the
   [rollback runbook](../runbooks/rollback.md).
2. Remove **only** rule 4's Bash kubectl arm from `check_bash`: the
   `MUTATING`/`ROLLOUT_SAFE` sets and the `exe == "kubectl"` branch. The
   rump that must remain: rules 1–3 (Write/Edit manifests), rule 5
   (credential values, **both** channels — it is the last rule with no
   absorbed Write/Edit channel), and rule 6 (git commit pathspec).
3. Flip the coexistence documentation to the post-retirement rump: the
   migration guide's rule table row and
   `tests/coexistence_org_rule_guard_tests.rs`'s header currently describe
   rule 4 as double-denied, which stops being true the moment the rule is
   removed. (Note that the doc-consistency test requires the migration
   guide to record the double-deny while it is true — the two edits land in
   the same commit as the removal or not at all.)
4. Record the gate output, the backup path, and the commit on the tracking
   bead.
5. Re-run the gate after the removal: the legacy column should now allow
   everything while icg still denies every shared-deny and icg-stricter
   probe — that is the post-retirement steady state the gate also
   documents.

If any step cannot be completed, stop with the rule still in place: an
unenforced policy must never be the residue of a half-finished edit.

## What to do when the gate fails

Nothing to the hook. Record the gate output on the tracking bead with the
trust pointer and pack listing; the fix is deploying a pack-bearing release
to the host, which is an operator action under the
[rule-pack update runbook](../runbooks/rule-pack-updates.md) — scheduled,
reviewed, pointer-advanced, never a manual copy into `/etc/icg/packs`.
Re-run the gate after that deployment lands.
