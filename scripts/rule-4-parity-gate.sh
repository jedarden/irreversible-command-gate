#!/bin/sh
# Rule 4 parity gate: decide whether org-rule-guard.py's blanket kubectl
# mutation rule may retire from a host's live hook chain.
#
# ADR-001 (docs/adr/001-kubectl-mutation-pack.md) records that the legacy
# rule "can retire ... on the same terms as rules 1-3" once the kubectl pack
# covers it. The terms are DEPLOYED parity on the host doing the retiring,
# not code parity in this repository: the registered hook front-end loads
# packs only from the deployed pack directory (/etc/icg/packs), so a pack
# that ships here but is not deployed yet leaves the legacy rule as the only
# mechanical enforcement of the deny surface. Verified live on codinghome
# 2026-09-25: with the trust pointer still at v0.1.61 and no kubectl pack
# under /etc/icg/packs, the deployed front-end allowed every deny-class
# probe in the corpus below while org-rule-guard.py denied it
# (irrevers-c609004e recorded the same finding when it declined to retire
# the rule; irrevers-47f004b4 turned the retry gate into this script).
#
# What it checks, in order:
#   1. Preconditions: both hooks present, the deployed front-end is not
#      emergency-disabled, and a kubectl pack exists in the deployed pack
#      directory. The trust pointer reference is printed for the audit
#      trail but not version-compared: pack presence plus the behavioral
#      corpus below is the authoritative signal, and an old binary under a
#      new pointer is caught behaviorally too (a pre-sh -c engine allowed
#      payload probes even with the pack present).
#   2. Corpus: every probe below is piped as an identical Bash PreToolUse
#      payload to BOTH live hooks from a neutral working directory (hook
#      mode never resolves packs from the CWD, and the neutral directory
#      keeps it that way even if that ever changes). Verdict classes:
#        shared-deny   both hooks must deny
#        shared-allow  both hooks must allow
#        icg-stricter  icg denies, legacy allows -- the documented ADR-001
#                      deltas (run/expose, wrapper prefixes, sh -c payloads,
#                      the flag-order create carve-out). Tolerated; counted.
#      The gate FAILS on any shared-class divergence and on any probe where
#      the legacy hook denies and icg allows (an enforcement gap).
#
# Probe commands never appear in the caller's own command line -- they live
# in this file and travel to the hooks as stdin JSON -- so running the gate
# does not trip the guards it is probing.
#
# Usage: scripts/rule-4-parity-gate.sh
# Environment overrides:
#   ICG_LEGACY_HOOK  legacy hook path   (default ~/.claude/hooks/org-rule-guard.py)
#   ICG_HOOK_BIN     icg binary         (default /usr/local/bin/icg)
#   ICG_PACK_DIR     deployed packs dir (default /etc/icg/packs)
# Exit codes:
#   0  gate PASSED -- the legacy rule may be retired per
#      docs/operators/rule-4-parity-gate.md
#   1  corpus FAILED -- verdict divergence on the shared surface
#   2  preconditions not met -- the deployed stack does not carry the
#      kubectl pack yet (takes precedence over 1)
#   3  harness error -- a hook is missing, exited nonzero, hung, or
#      produced an unparseable verdict
#
# Requires python3 for payload construction and verdict parsing.
set -u

case "${1:-}" in
  --help)
    cat <<'USAGE'
Usage: scripts/rule-4-parity-gate.sh
See the file header and docs/operators/rule-4-parity-gate.md for the gate
definition, corpus classes, and exit codes.
USAGE
    exit 0
    ;;
  "")
    ;;
  *)
    echo "usage: $0 [--help]" >&2
    exit 3
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  echo "harness error: python3 is required but not on PATH" >&2
  exit 3
fi

exec python3 - <<'PYEOF'
import json
import os
import subprocess
import sys

legacy_hook = os.path.expanduser(
    os.environ.get("ICG_LEGACY_HOOK", "~/.claude/hooks/org-rule-guard.py")
)
icg_bin = os.environ.get("ICG_HOOK_BIN", "/usr/local/bin/icg")
pack_dir = os.environ.get("ICG_PACK_DIR", "/etc/icg/packs")

# (class, command). Keep this table identical to the corpus documented in
# docs/operators/rule-4-parity-gate.md.
CORPUS = [
    # --- shared-deny: the legacy rule's deny surface, in the shapes both
    # implementations model (plain verbs, flag order, sudo/env prefixes,
    # absolute path, chain position, downstream pipe) ---
    ("shared-deny", "kubectl delete pod foo"),
    ("shared-deny", "kubectl apply -f deploy.yaml"),
    ("shared-deny", "kubectl patch deployment web -p '{\"spec\":{\"replicas\":2}}'"),
    ("shared-deny", "kubectl edit deploy/web"),
    ("shared-deny", "kubectl replace -f x.yaml"),
    ("shared-deny", "kubectl set env deploy/web K=V"),
    ("shared-deny", "kubectl annotate pod foo key=val"),
    ("shared-deny", "kubectl label pod foo tier=web"),
    ("shared-deny", "kubectl scale deploy/web --replicas=3"),
    ("shared-deny", "kubectl autoscale deploy/web --min=1 --max=3"),
    ("shared-deny", "kubectl cordon node-a"),
    ("shared-deny", "kubectl uncordon node-a"),
    ("shared-deny", "kubectl drain node-a"),
    ("shared-deny", "kubectl taint node-a key=val:NoSchedule"),
    ("shared-deny", "kubectl evict pod foo"),
    ("shared-deny", "kubectl rollout restart deploy/api"),
    ("shared-deny", "kubectl rollout undo deploy/api"),
    ("shared-deny", "kubectl rollout pause deploy/api"),
    ("shared-deny", "kubectl rollout resume deploy/api"),
    ("shared-deny", "kubectl -n prod delete pod foo"),
    ("shared-deny", "sudo kubectl delete pod foo"),
    ("shared-deny", "FOO=1 kubectl delete pod foo"),
    ("shared-deny", "git status && kubectl delete pod foo"),
    ("shared-deny", "/usr/local/bin/kubectl delete pod foo"),
    ("shared-deny", "kubectl delete pod foo | grep foo"),
    # --- shared-allow: read-only surface, both carve-outs, lookalikes, and
    # the payload negative arm ---
    ("shared-allow", "kubectl get pods"),
    ("shared-allow", "kubectl describe pod foo"),
    ("shared-allow", "kubectl logs deploy/api -f"),
    ("shared-allow", "kubectl top nodes"),
    ("shared-allow", "kubectl version --short"),
    ("shared-allow", "kubectl rollout status deploy/api"),
    ("shared-allow", "kubectl rollout history deploy/api"),
    ("shared-allow", "kubectl create -f wf.yaml -n argo-workflows"),
    ("shared-allow", "kubectl create -f wf.yaml -n iad-ci"),
    ("shared-allow", "kubectl exec -it pod/foo -- sh"),
    ("shared-allow", "kubectl cp pod/foo:/tmp/a ./a"),
    ("shared-allow", "kubectl port-forward pod/foo 8080:80"),
    ("shared-allow", "kubectl auth can-i delete pods"),
    ("shared-allow", "kubectl get pods -l app=delete-me"),
    ("shared-allow", "kubectl get pods | grep web"),
    ("shared-allow", "echo kubectl-apply-is-blocked"),
    ("shared-allow", "sh -c \"kubectl get pods\""),
    # --- icg-stricter: documented ADR-001 deltas; icg must deny, the
    # legacy hook's allow is the accepted divergence ---
    ("icg-stricter", "kubectl run demo --image=nginx"),
    ("icg-stricter", "kubectl expose deployment web --port=80"),
    ("icg-stricter", "kubectl -n argo-workflows create -f wf.yaml"),
    ("icg-stricter", "timeout 300 kubectl apply -f deploy.yaml"),
    ("icg-stricter", "nice -n 5 kubectl delete pod foo"),
    ("icg-stricter", "sh -c \"kubectl delete pod foo\""),
    ("icg-stricter", "bash -c 'kubectl apply -f x.yaml'"),
]


def harness_error(message):
    print(f"harness error: {message}", file=sys.stderr)
    sys.exit(3)


def verdict(argv, command, label):
    payload = json.dumps(
        {"tool_name": "Bash", "tool_input": {"command": command}}
    )
    try:
        proc = subprocess.run(
            argv, input=payload, capture_output=True, text=True,
            cwd="/", timeout=30,
            env={k: v for k, v in os.environ.items() if k != "ICG_RULE_PACK"},
        )
    except subprocess.TimeoutExpired:
        harness_error(f"{label} timed out on {command!r}")
    except OSError as error:
        harness_error(f"{label} could not be executed: {error}")
    if proc.returncode != 0:
        harness_error(
            f"{label} exited {proc.returncode} on {command!r}; a fail-closed "
            "or broken hook is not a parity verdict -- see "
            "docs/runbooks/incident-response.md"
        )
    if not proc.stdout.strip():
        # The legacy hook prints nothing on allow; icg prints an allow JSON,
        # but silence is this hook's documented allow shape either way.
        return "allow"
    try:
        decision = json.loads(proc.stdout)["hookSpecificOutput"][
            "permissionDecision"
        ]
    except Exception:
        harness_error(
            f"{label} produced an unparseable verdict on {command!r}: "
            f"{proc.stdout[:120]!r}"
        )
    if decision not in ("allow", "deny", "warning", "rewrite"):
        harness_error(f"{label} returned unknown verdict {decision!r}")
    return decision


# --- preconditions ---------------------------------------------------------
if os.environ.get("ICG_DISABLED"):
    print(
        "precondition NOT met: ICG_DISABLED is set; the deployed front-end "
        "is emergency-disabled, so its verdicts prove nothing."
    )
    sys.exit(2)
if not os.path.isfile(legacy_hook):
    harness_error(f"legacy hook not found at {legacy_hook}")
kubectl_pack = os.path.join(pack_dir, "kubectl.json")
pack_present = os.path.isfile(kubectl_pack)
pointer = os.path.join(os.path.dirname(pack_dir), "trust-pointer.json")
pointer_ref = "unreadable"
try:
    with open(pointer) as handle:
        pointer_ref = json.load(handle).get("trusted_ref", pointer_ref)
except OSError:
    pass
print(f"trust pointer: {pointer_ref}")
print(f"deployed kubectl pack: {kubectl_pack} -> "
      f"{'present' if pack_present else 'ABSENT'}")
if not pack_present:
    print(
        "precondition NOT met: the deployed pack set predates the kubectl "
        "pack (first shipped v0.1.62). The legacy rule keeps denying; "
        "advancing the deployment is an operator act -- see "
        "docs/runbooks/rule-pack-updates.md. Corpus results follow for the "
        "record."
    )

# --- corpus ----------------------------------------------------------------
failures = []
stricter = 0
for expected_class, command in CORPUS:
    legacy = verdict([legacy_hook], command, "legacy hook")
    deployed = verdict([icg_bin, "hook"], command, "deployed icg hook")
    line = (f"{expected_class:13} legacy={legacy:8} icg={deployed:8}  {command}")
    if expected_class == "shared-deny":
        expected = {"legacy": "deny", "icg": "deny"}
    elif expected_class == "shared-allow":
        expected = {"legacy": "allow", "icg": "allow"}
    else:  # icg-stricter
        expected = {"legacy": "allow", "icg": "deny"}
    if legacy == "deny" and deployed == "allow":
        failures.append(f"ENFORCEMENT GAP (legacy denies, icg allows): {line}")
    elif (legacy, deployed) != (expected["legacy"], expected["icg"]):
        failures.append(f"DIVERGENCE: {line}")
    else:
        if expected_class == "icg-stricter":
            stricter += 1
        print(f"ok          {line}")

print()
print(f"corpus: {len(CORPUS)} probes, {len(failures)} failures, "
      f"{stricter} tolerated icg-stricter deltas")
for failure in failures:
    print(failure)

if not pack_present:
    sys.exit(2)
if failures:
    sys.exit(1)
print(
    "gate PASSED: deployed parity holds. The legacy rule may now be "
    "retired -- follow docs/operators/rule-4-parity-gate.md, and keep the "
    "pre-retirement hook backup for the rollback arm in "
    "docs/runbooks/rollback.md."
)
PYEOF
