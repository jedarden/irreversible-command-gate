# icg Example Scenarios

This document provides realistic, step-by-step scenarios demonstrating how icg works in practice. Each scenario shows the exact commands, outputs, and decision points you'll encounter when working with icg.

## Table of Contents

1. [Operator Scenarios](#operator-scenarios)
   - First-time Installation
   - Daily Operations
   - Handling Denials
   - Emergency Response
   - Maintenance Tasks
2. [Developer Scenarios](#developer-scenarios)
   - Creating a New Rule Pack
   - Testing Pattern Changes
   - Debugging False Positives
   - Adding Custom Predicates
3. [Integration Scenarios](#integration-scenarios)
   - Migrating from org-rule-guard.py
   - Setting up Multi-Harness Support
   - Configuring Repository Overrides
4. [Coverage Audit](#coverage-audit)

---

## Operator Scenarios

### Scenario 1: First-time Installation

**Context**: You're a new operator installing icg for the first time on a development server.

#### Step 1: Download and Install

```bash
# Release binary and packs (v0.1.3, linux x86_64)
BASE=https://github.com/jedarden/irreversible-command-gate/releases/download/v0.1.3
curl -fsSLO "$BASE/icg" && curl -fsSLO "$BASE/icg-packs.tar.gz"

sudo install -o root -g root -m 0755 icg /usr/local/bin/icg
sudo install -d -o root -g root -m 0755 /etc/icg
sudo tar -xzf icg-packs.tar.gz -C /etc/icg
sudo chown -R root:root /etc/icg/packs

# Verify
icg --version          # icg 0.1.3
icg coverage --list    # all ten packs
```

> Building from source is equally supported and needs only a Rust
> toolchain — see the [Quick Start Guide](../quick-start.md).

#### Step 2: Install Rule Packs

Step 1's tarball already placed all ten under `/etc/icg/packs/`. Confirm
they load, and that they are byte-identical to the reviewed release:

```bash
icg coverage --list
# ✓ pack argocd-topology (1 patterns)
# ✓ pack beads (3 patterns)
# ... ten packs

icg health --check-packs
icg pack-manifest --verify pack-manifest.json --pack-dir /etc/icg/packs
# Pack directory matches manifest (10 packs)
```

Cherry-picking individual packs out of the tree is not an install path: the
manifest covers the directory as a whole, and a partial set silently
narrows coverage without failing anything. Install the release tarball, or
install `packs/*.json` from a checkout in one go.

#### Step 3: Configure Claude Code Hook

```bash
# Merge into ~/.claude/settings.json -- do not overwrite unrelated settings.
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "/usr/local/bin/icg hook",
            "timeout": 10
          }
        ]
      }
    ]
  }
}

# Verify hook
icg health --check-hooks
```

> The canonical hook contract is in
> [deployment-guide.md](../operators/deployment-guide.md). `icg hook` reads one
> PreToolUse JSON document from stdin and writes one decision envelope --
> `icg check --stdin` is the human-facing tester, not the hook entry point.

#### Step 4: Test Installation

```bash
# Test a dangerous command (should be denied)
echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  icg check --stdin

# Expected output:
# DENIED by icg
# Reason: vault kv destroy is permanently destructive and cannot be undone
# Pack: vault
# Pattern: openbao-destructive-verb
# Severity: Critical
# Explanation: vault kv destroy is permanently destructive and cannot be undone
# Redirect: Use 'vault kv patch' to reconcile or 'vault kv delete' for versioned metadata.

# Test a safe command (should be allowed)
echo '{"toolName":"Bash","toolInput":{"command":"vault kv get secret/test"}}' | \
  icg check --stdin

# Expected output:
# ALLOW: no configured rule matched
```

#### Step 5: Review Setup

```bash
# Run full health check
icg health --verbose

# Output:
# ✓ icg binary: /usr/local/bin/icg v0.1.0
# ✓ Rule packs: 3 packs loaded
#   - git (1 patterns)
#   - image-tag (1 patterns)
#   - vault (1 patterns)
# ✓ Claude Code hook: Configured
# ✓ State store: /var/lib/icg/state.db
# ✓ Denial log: /var/log/icg/denials.log
```

---

### Scenario 2: Daily Operations

**Context**: You're monitoring icg during normal operations and notice an unusual pattern of denials.

#### Step 1: Check Recent Denials

```bash
# View denials from the last hour
icg status --denials --since 1h

# Output:
# DENIALS (last 1h)
# ════════════════════════════════════════════════════════════════
# Time                    Pack        Pattern              Severity
# ────────────────────────────────────────────────────────────────
# 2026-08-16 10:23:45     vault       openbao-destructive-verb    Critical
# 2026-08-16 10:15:32     git        git-force-push       Critical
# 2026-08-16 09:58:17     image-tag  latest-tag           High
```

#### Step 2: Analyze Patterns

```bash
# View denial pattern summary
icg status --denials --pattern-summary --since 7d

# Output:
# DENIAL PATTERNS (last 7d)
# ════════════════════════════════════════════════════════════════
# Pattern ID                Count   % of Total   Trend
# ───────────────────────────────────────────────────────────────────
# git-force-push            1       33%          → Stable
# latest-tag                1       33%          → Stable
# openbao-destructive-verb          1       33%          → Stable
```

#### Step 3: Investigate Anomalies

```bash
# Export details for a specific denial
icg status --denials --since 1h --format json > denials.json
cat denials.json | jq '.[] | select(.patternId == "openbao-destructive-verb")'

# Output:
# {
#   "timestamp": "2026-08-16T10:23:45Z",
#   "packId": "vault",
#   "patternId": "openbao-destructive-verb",
#   "severity": "Critical",
#   "command": "vault kv destroy secret/app/api-key",
#   "reason": "vault kv destroy is permanently destructive and cannot be undone",
#   "sessionId": "session-456",
#   "telemetryId": "den-abc123"
# }
```

#### Step 4: Take Action

```bash
# If this is a training issue, review documentation
cat docs/operators/deny-messages.md | grep -A 20 "vault-destructive"

# If this is a false positive, file an issue
icg export-denial den-abc123 > false-positive-report.txt
gh issue create \
  --title "False positive: openbao-destructive-verb" \
  --body "Attached denial report. Command was legitimate." \
  --repo jedarden/irreversible-command-gate
```

---

### Scenario 3: Handling Denials

**Context**: An agent you're working with gets denied. You need to understand why and what to do.

#### Step 1: Read the Denial Message

```bash
# The agent receives this denial:
DENIED by icg
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Rule Pack:    vault
Pattern ID:   openbao-destructive-verb
Severity:     Critical
Explanation:  This operation would permanently destroy secret data and cannot be undone.
Redirect:     Use 'vault kv patch' to reconcile or 'vault kv delete' for versioned metadata.
Command:      vault kv destroy secret/app/api-key
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

#### Step 2: Understand the Pattern

```bash
# Look up the pattern documentation
icg explain --pattern openbao-destructive-verb

# Output:
# Pattern: openbao-destructive-verb
# Severity: Critical
# Matches: vault kv destroy, vault kv destroy -versions=<n>
# Why: Permanently destroys secret data versions
# Alternative: vault kv patch (safe reconcile), vault kv delete (versioned metadata only)
```

#### Step 3: Follow the Redirect

```bash
# The agent tries the safe alternative
vault kv patch secret/app/api-key -remove=expired_field

# This command is allowed and executes successfully
# Success! Metadata updated
```

#### Step 4: Verify the Fix

```bash
# Check that the secret is still accessible
vault kv get secret/app/api-key

# Output:
# ========= Secrets Path =========
# secret/app/api-key
# ======= Metadata =======
# key                           value
# ---                           -----
# active_field                 some-value
# (expired_field is gone)
```

---

### Scenario 4: Emergency Response

**Context**: A critical service is down and you need to bypass icg to fix it immediately.

#### Step 1: Assess the Situation

```bash
# Check if icg is the blocker
icg status --health

# Output:
# ✓ icg is healthy and running
# Recent denials: 3 in last 5m
# Last denial: openbao-destructive-verb (Critical)
```

#### Step 2: Document the Emergency

```bash
# Create an incident record
cat > /tmp/emergency-$(date +%s).txt <<EOF
EMERGENCY BYPASS RECORD
======================
Timestamp: $(date)
Service: auth-api
Issue: Vault policy deleted, breaking authentication
Action: Bypassing icg to restore policy
Justification: Service down, users affected
EOF
```

#### Step 3: Bypass the Guard

```bash
# Use emergency disable (one command only)
ICG_DISABLED=1 vault policy write auth-policy auth-policy.hcl

# Output:
# WARNING: icg guard disabled for this command
# Success! Policy written
```

#### Step 4: Verify and Restore

```bash
# Verify the service is restored
curl https://auth-api.example.com/health
# Output: {"status":"healthy"}

# Re-enable icg protection (remove environment variable)
unset ICG_DISABLED

# Verify icg is active again
icg status --health
# Output: ✓ icg is active and protecting
```

#### Step 5: Follow Up

```bash
# File an incident report
gh issue create \
  --title "Incident: Emergency bypass of openbao-destructive-verb" \
  --body "Attached incident record. Need to review why legitimate operation was blocked." \
  --label incident \
  --repo jedarden/irreversible-command-gate

# Schedule a postmortem
echo "Postmortem scheduled for: $(date -d '+2 days')" >> /tmp/emergency-*
```

---

### Scenario 5: Maintenance Tasks

**Context**: Regular maintenance of icg to ensure continued reliability.

#### Step 1: Weekly Health Check

```bash
# Run comprehensive health check
icg health --verbose > /tmp/icg-health-$(date +%Y%m%d).txt

# Review the output
cat /tmp/icg-health-$(date +%Y%m%d).txt

# Output:
# ✓ Binary: /usr/local/bin/icg v0.1.0
# ✓ Rule packs: 3 packs loaded
#   - git (1 patterns)
#   - image-tag (1 patterns)
#   - vault (1 patterns)
# ✓ Claude Code hook: Configured
# ✓ State store: /var/lib/icg/state.db
# ✓ Denial log: /var/log/icg/denials.log
```

#### Step 2: Monthly Review

```bash
# Review denial trends
icg status --denials --trend --since 30d

# Output:
# DENIAL TRENDS (last 30d)
# ════════════════════════════════════════════════════════════════
# Week 1        Week 2        Week 3        Week 4
# ─────────────────────────────────────────────────────────────────
# 3             3             3             3
# Trend: ↘ Decreasing (good - users learning safe patterns)
```

#### Step 3: Rule Pack Updates

```bash
# Check for updates
icg update --check-only

# Output:
# Updates available:
#   vault: v0.1.0 → v0.1.1 (fixes false positive in kv patch)
#   git: v0.1.0 → v0.1.2 (adds stale-HEAD-before-push check)

# Schedule update window (not automatic!)
echo "Rule pack updates scheduled for: $(date -d 'Saturday 2am')" >> /tmp/maintenance.txt
```

#### Step 4: Quarterly Testing

```bash
# Test rollback procedures
icg backup create --output /tmp/icg-backup-$(date +%Y%m%d).tar.gz

# Verify backup works
icg backup verify /tmp/icg-backup-$(date +%Y%m%d).tar.gz

# Output:
# ✓ Backup verified successfully
#   Contains: 5 rule packs, state.db, denial log
```

---

## Developer Scenarios

### Scenario 6: Creating a New Rule Pack

**Context**: You want to protect against destructive kubectl operations.

#### Step 1: Scaffold the Pack

```bash
# Use the scaffolding tool
cargo run --bin icg -- new-pack kubectl \
  --pack-type command \
  --output-dir packs/kubectl

# Output:
# ✓ Pack scaffold created: packs/kubectl/kubectl.json
# ✓ Test stub created: packs/kubectl/kubectl_pack_tests.rs
```

#### Step 2: Define Safe Patterns

```bash
# Edit the pack manifest
cat > packs/kubectl/kubectl.json <<'EOF'
{
  "id": "kubectl",
  "tool_keywords": ["kubectl", "kubecfg"],
  "applies_to": [],
  "safe_patterns": [
    {
      "id": "safe-get",
      "type": "command_regex",
      "regex": "^kubectl get"
    },
    {
      "id": "safe-describe",
      "type": "command_regex",
      "regex": "^kubectl describe"
    },
    {
      "id": "safe-logs",
      "type": "command_regex",
      "regex": "^kubectl logs"
    }
  ],
  "guarded_patterns": []
}
EOF
```

#### Step 3: Define Guarded Patterns

```bash
# Add destructive operations
cat >> packs/kubectl/kubectl.json <<'EOF'
{
  "guarded_patterns": [
    {
      "id": "kubectl-delete-deployment",
      "type": "command_regex",
      "regex": "kubectl delete deployment",
      "tier": "tier1",
      "severity": "High",
      "explanation": "Deleting a deployment removes all running pods",
      "destructive": true,
      "redirect": {
        "channel": "deny",
        "reason_template": "kubectl delete deployment is destructive. Use 'kubectl scale deployment --replicas=0' instead to preserve the deployment object.",
        "rewrite_template": null
      }
    },
    {
      "id": "kubectl-delete-pvc",
      "type": "command_regex",
      "regex": "kubectl delete pvc",
      "tier": "tier1",
      "severity": "Critical",
      "explanation": "Deleting a PVC destroys persistent data",
      "destructive": true,
      "redirect": {
        "channel": "deny",
        "reason_template": "kubectl delete pvc is permanently destructive. Data cannot be recovered.",
        "rewrite_template": null
      }
    }
  ]
}
EOF
```

#### Step 4: Write Tests

```bash
# Create test file
cat > packs/kubectl/kubectl_pack_tests.rs <<'EOF'
#[cfg(test)]
mod tests {
    use crate::rule_pack::load_pack;

    #[test]
    fn test_safe_patterns() {
        let pack = load_pack("packs/kubectl/kubectl.json").unwrap();
        assert!(pack.allows("kubectl get pods"));
        assert!(pack.allows("kubectl describe deployment myapp"));
        assert!(pack.allows("kubectl logs -f pod/mypod"));
    }

    #[test]
    fn test_guarded_patterns() {
        let pack = load_pack("packs/kubectl/kubectl.json").unwrap();
        assert!(pack.blocks("kubectl delete deployment myapp"));
        assert!(pack.blocks("kubectl delete pvc data-pvc"));
    }

    #[test]
    fn test_chaining_support() {
        let pack = load_pack("packs/kubectl/kubectl.json").unwrap();
        assert!(pack.allows("kubectl get pods && kubectl describe deployment myapp"));
        assert!(pack.blocks("kubectl get pods && kubectl delete deployment myapp"));
    }
}
EOF
```

#### Step 5: Test Locally

```bash
# Run tests
cargo test kubectl

# Test specific command
cargo run --bin icg -- check \
  --command "kubectl delete deployment myapp" \
  --pack packs/kubectl/kubectl.json

# Output:
# DENIED: kubectl delete deployment is destructive. Use 'kubectl scale deployment --replicas=0' instead.
```

#### Step 6: Generate Regression Suite

```bash
# Generate regression suite
cargo run --bin icg -- regression-suite \
  packs/kubectl/kubectl.json \
  --output tests/fixtures/kubectl-regression.json

# Verify regression suite
cat tests/fixtures/kubectl-regression.json | jq '.cases | length'
# Output: 2 (one per destructive pattern)
```

---

### Scenario 7: Testing Pattern Changes

**Context**: You need to modify an existing pattern and want to ensure you don't introduce regressions.

#### Step 1: Generate Baseline

```bash
# Generate regression suite before changes
icg regression-suite \
  /etc/icg/packs/git.json \
  --output git-baseline.json

# Save baseline
cp git-baseline.json ~/backups/git-baseline-$(date +%Y%m%d).json
```

#### Step 2: Make Your Changes

```bash
# Edit the pattern
# Change from: "regex": "git push.*--force"
# Change to: "regex": "git push (--force|-f)"
```

#### Step 3: Test Against Baseline

```bash
# Generate new regression suite
icg regression-suite \
  /etc/icg/packs/git.json \
  --output git-new.json

# Compare
cargo run --bin icg -- coverage-diff \
  git-baseline.json \
  git-new.json

# Output:
# ✓ No coverage narrowing detected
# ✓ All destructive patterns still protected
# ⚠ Pattern regex changed (semantic equivalence verified)
```

#### Step 4: Manual Verification

```bash
# Test edge cases
icg check --command "git push --force origin main"
icg check --command "git push -f origin main"
icg check --command "git push --force-with-lease origin main"

# Verify:
# --force: BLOCKED
# -f: BLOCKED
# --force-with-lease: ALLOWED (different pattern)
```

#### Step 5: Deploy to Test Environment

```bash
# Copy to test server
scp /etc/icg/packs/git.json test-server:/tmp/

# Install on test server
ssh test-server "sudo cp /tmp/git.json /etc/icg/packs/git.json"

# Verify health
ssh test-server "icg health --check-packs"
```

---

### Scenario 8: Debugging False Positives

**Context**: A developer reports that icg is complaining about a secret read
they believe is already safe. Every transcript below is real output from
`icg` run inside a checkout (`--pack` defaults to `packs/`).

#### Step 1: Reproduce the report

```bash
icg check --command "bao kv get -field=password secret/app/db"

# WARNING: This read prints a secret value to stdout, where it enters the transcript.
# Prefer redirecting to a mode-600 destination (`bao kv get -field=<k> <path> > ~/.config/<app>/creds`),
# or consuming it inline for one command via an environment assignment. To check that a
# path exists without revealing the value, use `bao kv metadata get`.
# Pack: openbao
# Pattern: openbao-kv-get-to-stdout
```

First thing to establish: this is a `WARNING`, not a denial. The command
was never blocked — `icg hook` returned `permissionDecision: "allow"` with
the caution in `additionalContext`. "icg blocked me" reports are often
this channel being read as a block.

#### Step 2: See which rule matched, and which safe patterns were tried

`--debug` writes the full evaluation trace to **stderr**, with the decision
still on stdout:

```bash
icg check --command "bao kv get -field=password secret/app/db" --debug
```

```text
Pack dispatched: openbao (input: bao kv get -field=password secret/app/db)
Safe patterns checked:
  safe-bao-status: NO MATCH (check: command regex "(?i)^(bao|vault)\s+status\b")
  ...
  safe-bao-kv-get-redirected: NO MATCH (check: command regex "(?i)\b(bao|vault)\s+kv\s+get\b[^\n]*>")
Guarded patterns checked:
  openbao-inline-secret-literal: NO MATCH (...)
  openbao-destructive-verb: NO MATCH (...)
  openbao-kv-get-to-stdout: MATCH (check: command regex "(?i)\b(bao|vault)\s+kv\s+get\b")
Final verdict: WARNING (openbao/openbao-kv-get-to-stdout)
```

The trace names the safe pattern that *would* have suppressed this —
`safe-bao-kv-get-redirected` — and shows it did not fire. That is the
answer to "why me": the read has no destination.

#### Step 3: Read the rule's standing explanation

```bash
icg explain --pattern openbao-kv-get-to-stdout --show-redirect

# Pattern: openbao-kv-get-to-stdout
# Pack: openbao
# Enabled: true
# Tier: Tier1
# Severity: Medium
# Why: Reading a secret to stdout puts its value in the agent transcript and any
#      log capturing it. Reads should land in a destination, not the terminal.
# Redirect channel: AdditionalContext
# Alternative: This read prints a secret value to stdout ... Prefer redirecting to a
#      mode-600 destination, or consuming it inline for one command via an environment
#      assignment. To check that a path exists without revealing the value, use
#      `bao kv metadata get`.
```

Add `--show-regex` to see the matcher itself.

#### Step 4: Decide whether it is actually a false positive

Test the forms the redirect recommends before touching the pack:

```bash
# Redirected to a file -- allowed by safe-bao-kv-get-redirected
icg check --command 'bao kv get -field=password secret/app/db > ~/.config/app/creds'
# ALLOW: no configured rule matched

# Consumed inline for one command -- allowed
icg check --command 'TOKEN=$(bao kv get -field=token secret/app/x) curl -H "Authorization: Bearer $TOKEN" https://api.example'
# ALLOW: no configured rule matched
```

Both sanctioned forms pass. The original command was not a false positive:
it really does print a secret to the terminal, and the warning is the rule
doing its job. Most "false positive" reports resolve here.

#### Step 5: If it *is* a false positive, widen a safe pattern — not the guarded one

A genuine false positive means a safe form is missing from `safe_patterns`.
Widening the guarded regex instead is how coverage silently disappears.
Add the safe pattern to a copy of the pack, and prove both directions:

```bash
cp packs/openbao.json /tmp/openbao-candidate.json
# ...add the new entry to "safe_patterns" in /tmp/openbao-candidate.json...

# The reported command is now clean
icg check --pack /tmp/openbao-candidate.json --command "<the reported command>"

# ...and the rule still fires on the case it exists for
icg check --pack /tmp/openbao-candidate.json --command "bao kv get secret/app/db"
# WARNING: This read prints a secret value to stdout ...
```

Then run the release gate before proposing the change — `coverage-diff`
reports a safe-pattern addition that swallows an existing deny case as a
regression:

```bash
icg regression-suite packs/openbao.json --output /tmp/openbao-suite.json
icg coverage-diff packs/openbao.json /tmp/openbao-candidate.json
```

See [rule-pack-best-practices.md](../developers/rule-pack-best-practices.md)
for the full authoring contract.

---

### Scenario 9: Adding Custom Predicates

**Context**: You need to check state that can't be determined from command syntax alone.

#### Step 1: Identify the Need

```bash
# Example: Beads pack needs to check if .git is a directory
# This determines if we're in a shared checkout (dangerous) or worktree (safe)

# Command regex can't do this
# We need a predicate: "is_shared_checkout"
```

#### Step 2: Define the Predicate

```rust
// src/predicates.rs

use std::path::Path;

/// Check if .git is a directory (shared checkout) vs a file (worktree)
pub fn is_shared_checkout() -> bool {
    Path::new(".git").is_dir()
}

/// Check if we have uncommitted changes
pub fn has_uncommitted_changes() -> bool {
    use std::process::Command;

    let output = Command::new("git")
        .args(&["status", "--porcelain"])
        .output();

    match output {
        Ok(o) => !o.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Check if HEAD is stale (behind remote)
pub fn is_head_stale() -> bool {
    use std::process::Command;

    let output = Command::new("git")
        .args(&["rev-parse", "@{u}"])
        .output();

    // If we have an upstream, check if we're behind
    if output.is_ok() {
        let status = Command::new("git")
            .args(&["rev-list", "--count", "@{u}..HEAD"])
            .output();

        match status {
            Ok(o) => {
                let count = String::from_utf8_lossy(&o.stdout).trim();
                count == "0" // We're behind if count is 0
            }
            Err(_) => false,
        }
    } else {
        false
    }
}
```

#### Step 3: Register the Predicate

```rust
// src/engine.rs

use crate::predicates::{is_shared_checkout, has_uncommitted_changes, is_head_stale};

fn evaluate_predicate(name: &str) -> bool {
    match name {
        "is_shared_checkout" => is_shared_checkout(),
        "has_uncommitted_changes" => has_uncommitted_changes(),
        "is_head_stale" => is_head_stale(),
        _ => {
            eprintln!("Warning: Unknown predicate '{}'", name);
            false
        }
    }
}
```

#### Step 4: Use in Rule Pack

```json
{
  "id": "beads-shared-checkout-write",
  "type": "predicate",
  "predicate_name": "is_shared_checkout",
  "tier": "tier1",
  "severity": "Critical",
  "explanation": "Writing to .beads/ in a shared checkout risks concurrent corruption",
  "destructive": true,
  "redirect": {
    "channel": "deny",
    "reason_template": "Writing to .beads/ in a shared checkout risks concurrent corruption. Use a worktree instead.",
    "rewrite_template": null
  }
}
```

#### Step 5: Test the Predicate

```bash
# Test in shared checkout (should deny)
cd /home/coding/shared-repo
echo "test" > .beads/checkpoint/current.json

# icg intercepts:
# DENIED: Writing to .beads/ in a shared checkout risks concurrent corruption

# Test in worktree (should allow)
cd /home/coding/worktree-repo
echo "test" > .beads/checkpoint/current.json

# icg allows:
# ✓ Write succeeded
```

---

## Integration Scenarios

### Scenario 10: Migrating from org-rule-guard.py

**Context**: You're currently using `org-rule-guard.py` and want to migrate to icg.

#### Step 1: Inventory Existing Rules

```bash
# List what org-rule-guard.py currently protects
cat ~/.claude/hooks/org-rule-guard.py | grep "BLOCKED"

# Output:
# BLOCKED: .github/workflows/
# BLOCKED: kind: Job
# BLOCKED: kind: CronJob
# BLOCKED: :latest image tags
# BLOCKED: mutating kubectl verbs
# BLOCKED: credential values in Write/Edit
```

#### Step 2: Compare with icg Coverage

```bash
# Check what icg covers
icg coverage --list

# Output:
# ✓ vault (destructive operations)
# ✓ git (force-push, stale-HEAD, commit-without-pathspec)
# ✓ image-tag (:latest, bare SHA)
# ✓ storage-class (ssd, ssd-large)
# ✓ beads (.beads/ protection)
# ✓ secrets (credential values in Bash)
# ✓ misc (deprecated tools, needle cleanup)
# ✓ tmux (bare NATO sessions)
#
# ❌ NOT COVERED:
#   - .github/workflows/
#   - kind: Job/CronJob
#   - mutating kubectl verbs
```

#### Step 3: Plan Migration Strategy

```bash
# Coverage gap analysis
cat > migration-plan.md <<'EOF'
# Migration Plan: org-rule-guard.py → icg

## Phase 1: Coexistence (Week 1-2)
- Keep org-rule-guard.py active
- Install icg alongside
- Both hooks running (double denials expected)
- Verify no conflicts

## Phase 2: Migrate Overlapping Rules (Week 3-4)
- image-tag: org-rule-guard.py → icg
- secrets: org-rule-guard.py → icg
- Remove rules from org-rule-guard.py
- Test thoroughly

## Phase 3: Keep org-rule-guard.py for Uncovered Rules (Ongoing)
- .github/workflows/ (no icg equivalent planned)
- kind: Job/CronJob (no icg equivalent planned)
- mutating kubectl (permanent exclusion)
EOF
```

#### Step 4: Configure Coexistence

Both guards run as ordinary `PreToolUse` command hooks. Claude Code runs
every hook whose matcher fits, so listing both in one matcher block keeps
them side by side during the overlap window:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Write|Edit",
        "hooks": [
          { "type": "command", "command": "/home/coding/.claude/hooks/org-rule-guard.py", "timeout": 10 },
          { "type": "command", "command": "/usr/local/bin/icg hook", "timeout": 10 }
        ]
      }
    ]
  }
}
```

Expect double denials for any rule both guards cover — that is the
intended, visible signal during coexistence, and the cue to remove the
rule from `org-rule-guard.py`. See
[migration-from-org-rule-guard.md](../operators/migration-from-org-rule-guard.md)
for the ordered cutover, and
[deployment-guide.md](../operators/deployment-guide.md) for the canonical
hook contract.

#### Step 5: Verify and Monitor

```bash
# Run for 2 weeks, collect data
icg status --denials --since 14d --format json > coexistence-data.json

# Analyze
cat coexistence-data.json | jq '[.[] | .packId] | group_by | map({pack: .[0], count: length})'

# Output:
# [
#   {"pack": "vault", "count": 23},
#   {"pack": "git", "count": 18},
#   {"pack": "image-tag", "count": 15}
# ]
```

---

### Scenario 11: Setting up Multi-Harness Support

**Context**: You need to protect both Claude Code and Codex CLI agents.

#### Step 1: Test Claude Code Integration

```bash
# Merge into ~/.claude/settings.json -- do not overwrite unrelated settings.
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "/usr/local/bin/icg hook",
            "timeout": 10
          }
        ]
      }
    ]
  }
}

# Verify hook
icg health --check-hooks
```

> The canonical hook contract is in
> [deployment-guide.md](../operators/deployment-guide.md). `icg hook` reads one
> PreToolUse JSON document from stdin and writes one decision envelope --
> `icg check --stdin` is the human-facing tester, not the hook entry point.

#### Step 2: Test Codex CLI Integration

```bash
# Codex CLI reads ~/.codex/hooks.json (or a repo's .codex/hooks.json).
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|apply_patch",
        "hooks": [
          {
            "type": "command",
            "command": "/usr/local/bin/icg hook",
            "timeout": 10
          }
        ]
      }
    ]
  }
}

# Verify hook
icg health --check-hooks
```

> Use the file and schema documented by the installed Codex CLI version --
> that hook surface is young and still moving; see
> [multi-harness-integration.md](../notes/multi-harness-integration.md).

#### Step 3: Verify Both Harnesses

```bash
# Test Claude Code-specific features (apply_patch)
echo '{"toolName":"apply_patch","toolInput":{"command":"*** Begin Patch\n*** Update File: deployment.yaml\n+storageClassName: ssd\n*** End Patch"}}' | \
  icg check --stdin --harness claude-code

# Output:
# DENIED: storageClassName: ssd is prohibited on Rackspace Spot

# Test Codex CLI-specific features (same format)
echo '{"toolName":"apply_patch","toolInput":{"command":"*** Begin Patch\n*** Update File: deployment.yaml\n+image: app:latest\n*** End Patch"}}' | \
  icg check --stdin --harness codex-cli

# Output:
# DENIED: image tag :latest is not pinned to a specific version
```

#### Step 4: Monitor Both Harnesses

```bash
# Check recent denials from the shared hook telemetry
icg status --denials --since 1d --format json

# Output:
# [
#   {"packId":"git","patternId":"git-force-push", ...},
#   {"packId":"storage-class","patternId":"storage-class-ssd", ...}
# ]
```

The shared `hookSpecificOutput.permissionDecision` envelope is the stable
cross-harness contract. Harness-specific counts should be grouped from the
caller telemetry; `icg status` intentionally reports the shared denial log and
does not provide a `--by-harness` flag.

---

### Scenario 12: Configuring Repository Overrides

**Context**: A specific repository needs an exception to a rule.

#### Step 1: Identify the Need

```bash
# Repository: legacy-app
# Issue: Uses bare git SHA in image tags (historical reason)
# Rule: image-tag pack blocks "image: ronaldraygun/legacy-app:<git-sha>"
# Need: Override for this specific repo
```

#### Step 2: Request Override

```bash
# Create override request
icg override create \
  --repo /home/coding/legacy-app \
  --pattern-id "image-tag-bare-sha" \
  --justification "Legacy app uses immutable SHA-based tags for audit compliance. SHA is sourced from build system and never manually specified. Approved by security@company.com." \
  --output /tmp/override-request-legacy-app.json

# Output:
# Override request created: /tmp/override-request-legacy-app.json
# Requires Layer 1/2 approval via release pipeline.
```

#### Step 3: Get Approval

```bash
# Submit for review
cat /tmp/override-request-legacy-app.json | \
  jq '{repo: .repo, pattern: .patternId, justification: .justification}'

# Email to security team with:
# - Override request JSON
# - Repository context
# - Security review approval
# - Timeline for eventual migration
```

#### Step 4: Apply Approved Override

```bash
# After approval, apply the override
icg override approve \
  --request /tmp/override-request-legacy-app.json \
  --approver security-team-lead \
  --expiration 2026-12-31 \
  --release-ref v0.1.0-integration-test \
  --pack /etc/icg/packs/image-tag.json \
  --output /etc/icg/overrides/legacy-app.toml

# Output:
# ✓ Override approved and installed
# Repository: /home/coding/legacy-app
# Pattern: image-tag-bare-sha
# Expires: 2026-12-31
# Stored in: /etc/icg/overrides/legacy-app.toml
```

#### Step 5: Verify Override

```bash
# Test in the repository
cd /home/coding/legacy-app
cat deployment.yaml | grep "image:"

# Output:
# image: ronaldraygun/legacy-app:0123456789abcdef0123456789abcdef01234567

# Test through the release-bound hook contract
printf '%s\n' '{"toolName":"Write","toolInput":{"filePath":"deployment.yaml","content":"image: ronaldraygun/legacy-app:0123456789abcdef0123456789abcdef01234567\n"}}' | \
  icg hook \
    --rule-pack /etc/icg/packs/image-tag.json \
    --override-file /etc/icg/overrides/legacy-app.toml \
    --repository legacy-app \
    --trusted-ref v0.1.0-integration-test

# Output:
# {"hookSpecificOutput":{"permissionDecision":"allow", ...}}

# Test outside repository
printf '%s\n' '{"toolName":"Write","toolInput":{"filePath":"deployment.yaml","content":"image: ronaldraygun/legacy-app:0123456789abcdef0123456789abcdef01234567\n"}}' | \
  icg hook --rule-pack /etc/icg/packs/image-tag.json

# Output:
# {"hookSpecificOutput":{"permissionDecision":"deny", ...}}
```

#### Step 6: Monitor and Review

```bash
# Check active overrides
icg override list

# Output:
# ACTIVE OVERRIDES
# ════════════════════════════════════════════════════════════════
# Repository              Pattern               Expires
# ────────────────────────────────────────────────────────────────
# legacy-app             image-tag-bare-sha    2026-12-31
# test-env               openbao-destructive-verb   2026-09-30

# Review quarterly
echo "Override review scheduled: $(date -d '+3 months')" >> calendar.txt
```

---

## Coverage Audit

Audited 2026-08-21 against the executable examples suite. Every scenario below
has a stable fixture or an explicit test input, a named scenario test, and a
documented expected outcome. The links point to the exact fixture directory and
test source so a future example change can be checked without guessing which
test owns it.

| Scenario | Fixture(s) | Scenario test | Expected outcome |
|---|---|---|---|
| 1. First-time Installation | [`installation.json`](../../tests/fixtures/operator-scenarios/installation.json), [`installation-packs/`](../../tests/fixtures/operator-scenarios/installation-packs/) | `first_time_installation_validates_documented_commands_and_outputs` in [`operator_scenarios.rs`](../../tests/operator_scenarios.rs) | Version and health checks pass; the Vault destroy request is denied with `openbao-destructive-verb`; the safe get request is allowed; missing hook configuration fails. |
| 2. Daily Operations | [`daily-operations.json`](../../tests/fixtures/operator-scenarios/daily-operations.json) | `daily_operations_queries_fixture_for_tables_json_and_reports` in [`operator_scenarios.rs`](../../tests/operator_scenarios.rs) | The 1-hour table, 7-day summary, JSON history, and `den-abc123` report match the fixture; an unknown denial ID fails. |
| 3. Handling Denials | [`handling-denials.json`](../../tests/fixtures/operator-scenarios/handling-denials.json), [`handling-denials-pack.json`](../../tests/fixtures/operator-scenarios/handling-denials-pack.json) | `handling_denials_checks_format_redirect_and_safe_alternatives` in [`operator_scenarios.rs`](../../tests/operator_scenarios.rs) | The Vault destroy denial exposes severity, explanation, and redirect; `explain` finds the pattern; both safe alternatives allow; an unknown pattern fails. |
| 4. Emergency Response | [`emergency-response.json`](../../tests/fixtures/operator-scenarios/emergency-response.json), [`daily-operations.json`](../../tests/fixtures/operator-scenarios/daily-operations.json) | `emergency_response_records_state_bypasses_once_and_restores_protection` in [`operator_scenarios.rs`](../../tests/operator_scenarios.rs) | The incident record is persisted; `ICG_DISABLED=1` allows once with a warning; protection denies again after removal; health and denial export succeed. |
| 5. Maintenance Tasks | [`maintenance.json`](../../tests/fixtures/operator-scenarios/maintenance.json), [`installation-packs/`](../../tests/fixtures/operator-scenarios/installation-packs/) | `maintenance_commands_validate_health_trends_updates_and_backup` in [`operator_scenarios.rs`](../../tests/operator_scenarios.rs) | Verbose health, trend, update-check, backup creation, and backup verification succeed; a corrupt archive is rejected. |
| 6. Creating a New Rule Pack | [`creating-rule-pack-new.json`](../../tests/fixtures/developer-scenarios/creating-rule-pack-new.json) | `scenario_6_new_pack_scaffold_and_local_validation` in [`developer_scenarios.rs`](../../tests/developer_scenarios.rs) | The scaffold is loadable; a safe `kubectl get` allows; PVC deletion denies; a duplicate scaffold refuses to overwrite. |
| 7. Testing Pattern Changes | [`testing-pattern-changes-baseline.json`](../../tests/fixtures/developer-scenarios/testing-pattern-changes-baseline.json), [`testing-pattern-changes-updated.json`](../../tests/fixtures/developer-scenarios/testing-pattern-changes-updated.json), [`regression-suite-baseline.json`](../../tests/fixtures/developer-scenarios/regression-suite-baseline.json), [`regression-suite-updated.json`](../../tests/fixtures/developer-scenarios/regression-suite-updated.json) | `scenario_7_regression_generation_verification_and_coverage_diff` in [`developer_scenarios.rs`](../../tests/developer_scenarios.rs) | Regression suites verify; the narrowed diff is rejected without justification and accepted with one; missing cases and changed inputs fail verification. |
| 8. Debugging False Positives | the shipped [`packs/openbao.json`](../../packs/openbao.json), plus [`debugging-false-positives-overly-broad.json`](../../tests/fixtures/developer-scenarios/debugging-false-positives-overly-broad.json) and [`debugging-false-positives-fixed.json`](../../tests/fixtures/developer-scenarios/debugging-false-positives-fixed.json) | `scenario_8_documented_walkthrough_runs_against_the_shipped_openbao_pack` and `scenario_8_debug_trace_reproduce_fix_and_verify_false_positive` in [`developer_scenarios.rs`](../../tests/developer_scenarios.rs) | The documented walkthrough reproduces on the real pack: the reported read warns rather than denies, both redirect-recommended forms allow, and the `--debug` trace names `safe-bao-kv-get-redirected` as the safe pattern that did not fire. On synthetic fixtures the same loop shows a broad rule denying every delete form and the narrowed rules still denying PVC deletion; malformed packs fail. |
| 9. Adding Custom Predicates | [`adding-custom-predicates.json`](../../tests/fixtures/developer-scenarios/adding-custom-predicates.json) | `scenario_9_custom_predicates_evaluate_shared_checkout_scope` in [`developer_scenarios.rs`](../../tests/developer_scenarios.rs), with predicate cases in [`custom_predicates_tests.rs`](../../tests/custom_predicates_tests.rs) | A `.beads/` write in the shared checkout denies, an unrelated write allows, and both decisions work through the stdin CLI path. |
| 10. Migrating from org-rule-guard.py | [`scenario-10-migration.json`](../../tests/fixtures/integration-scenarios/scenario-10-migration.json) | `scenario_10_compares_org_guard_overlap_and_coverage_gaps` in [`integration_scenarios.rs`](../../tests/integration_scenarios.rs) | Overlapping latest-image decisions agree; org-only probes remain allowed by icg; the icg-only OpenBao destructive probe denies; the live org hook is compared when present. |
| 11. Setting up Multi-Harness Support | [`scenario-11-multi-harness.json`](../../tests/fixtures/integration-scenarios/scenario-11-multi-harness.json) | `scenario_11_parses_and_runs_both_harness_wire_formats` in [`integration_scenarios.rs`](../../tests/integration_scenarios.rs) | CamelCase and snake_case payloads parse to the same decisions; `check --stdin` and native `hook` return the shared deny envelope with the expected pack and pattern. |
| 12. Configuring Repository Overrides | [`scenario-12-repository-overrides.json`](../../tests/fixtures/integration-scenarios/scenario-12-repository-overrides.json) | `scenario_12_runs_override_request_approval_verification_and_expiry` in [`integration_scenarios.rs`](../../tests/integration_scenarios.rs) | Request/approval creates a release-bound TOML artifact; only the exact repository and release can use it; dangerous content is otherwise denied; listing shows fresh/expired status and expiry is enforced. |

The suite intentionally does not execute external setup or coordination commands
such as `wget`, `scp`, `ssh`, `gh`, Vault itself, or email. Those steps remain
operator actions; the fixture-backed tests exercise every corresponding `icg`
command, hook decision, and expected allow/deny boundary. Run the complete
examples coverage with:

```text
cargo test --test operator_scenarios --test developer_scenarios \
  --test developer_scenarios_cli_tests --test integration_scenarios \
  --test examples_coverage
```

---

## Summary

These scenarios cover the most common workflows for both operators and developers working with icg. Key takeaways:

1. **Operators**: Use health checks, monitor denials, follow redirects, document emergencies
2. **Developers**: Start with scaffold, test thoroughly, generate regression suites, verify coverage
3. **Both**: Understand the architecture, read documentation, ask questions

For more information:
- **Operator Guide**: `docs/operators/README.md`
- **Developer Guide**: `docs/developers/README.md`
- **Denial Messages**: `docs/operators/deny-messages.md`

---

**Example Scenarios Version**: 1.0
**Last Updated**: 2026-08-21
**For**: icg v0.1.0+
