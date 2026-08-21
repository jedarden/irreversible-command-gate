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

---

## Operator Scenarios

### Scenario 1: First-time Installation

**Context**: You're a new operator installing icg for the first time on a development server.

#### Step 1: Download and Install

```bash
# Download the release binary
wget https://github.com/jedarden/irreversible-command-gate/releases/download/v0.1.0/icg-v0.1.0-x86_64-unknown-linux-gnu.tar.gz

# Extract
tar -xzf icg-v0.1.0-x86_64-unknown-linux-gnu.tar.gz

# Install to system directory
sudo cp icg /usr/local/bin/
sudo chmod +x /usr/local/bin/icg

# Verify installation
icg --version
# Output: icg v0.1.0
```

#### Step 2: Install Rule Packs

```bash
# Create rule pack directory
sudo mkdir -p /etc/icg/packs

# Download default rule packs
sudo curl -o /etc/icg/packs/vault.json \
  https://raw.githubusercontent.com/jedarden/irreversible-command-gate/v0.1.0/packs/vault.json

sudo curl -o /etc/icg/packs/git.json \
  https://raw.githubusercontent.com/jedarden/irreversible-command-gate/v0.1.0/packs/git.json

sudo curl -o /etc/icg/packs/image-tag.json \
  https://raw.githubusercontent.com/jedarden/irreversible-command-gate/v0.1.0/packs/image-tag.json

# Verify rule packs
icg health --check-packs
# Output: ✓ All rule packs valid
```

#### Step 3: Configure Claude Code Hook

```bash
# Edit Claude Code settings
mkdir -p ~/.config/claude-code
cat > ~/.config/claude-code/settings.json <<'EOF'
{
  "hooks": {
    "PreToolUse": {
      "command": "/usr/local/bin/icg",
      "args": ["check", "--stdin", "--harness", "claude-code"]
    }
  }
}
EOF

# Verify hook configuration
icg health --check-hooks
# Output: ✓ Claude Code hook configured
```

#### Step 4: Test Installation

```bash
# Test a dangerous command (should be denied)
echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  icg check --stdin

# Expected output:
{
  "verdict": "deny",
  "packId": "vault",
  "patternId": "vault-kv-destroy",
  "severity": "Critical",
  "reason": "vault kv destroy is permanently destructive and cannot be undone",
  "rewrite": null,
  "telemetryId": "den-abc123"
}

# Test a safe command (should be allowed)
echo '{"toolName":"Bash","toolInput":{"command":"vault kv get secret/test"}}' | \
  icg check --stdin

# Expected output:
{
  "verdict": "allow",
  "telemetryId": "all-def456"
}
```

#### Step 5: Review Setup

```bash
# Run full health check
icg health --verbose

# Output:
# ✓ icg binary: /usr/local/bin/icg v0.1.0
# ✓ Rule packs: 3 packs loaded
#   - vault (8 patterns)
#   - git (12 patterns)
#   - image-tag (6 patterns)
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
# 2026-08-16 10:23:45     vault       vault-kv-destroy    Critical
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
# vault-kv-destroy          23      31%          ↗ Increasing
# git-force-push            18      24%          → Stable
# latest-tag                15      20%          ↘ Decreasing
# commit-without-pathspec   12      16%          → Stable
# storage-class-ssd          7       9%          → Stable
```

#### Step 3: Investigate Anomalies

```bash
# Export details for a specific denial
icg status --denials --since 1h --format json > denials.json
cat denials.json | jq '.[] | select(.patternId == "vault-kv-destroy")'

# Output:
# {
#   "timestamp": "2026-08-16T10:23:45Z",
#   "packId": "vault",
#   "patternId": "vault-kv-destroy",
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
  --title "False positive: vault-kv-destroy" \
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
Pattern ID:   vault-kv-destroy
Severity:     Critical
Explanation:  This operation would permanently destroy secret data and cannot be undone.
Redirect:     Use 'vault kv patch' to reconcile or 'vault kv delete' for versioned metadata.
Command:      vault kv destroy secret/app/api-key
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

#### Step 2: Understand the Pattern

```bash
# Look up the pattern documentation
icg explain --pattern vault-kv-destroy

# Output:
# Pattern: vault-kv-destroy
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
# Last denial: vault-policy-delete (Critical)
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
  --title "Incident: Emergency bypass of vault-policy-delete" \
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
# ✓ Rule packs: 5 packs, all valid
# ✓ State store: Healthy
# ✓ Denial log: 1,234 entries
# ✓ Disk space: 45MB used (limit: 500MB)
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
# 145 denials   132 denials   118 denials   126 denials
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

**Context**: Users report that a legitimate command is being blocked incorrectly.

#### Step 1: Reproduce the Issue

```bash
# Get the exact command from the user
COMMAND="kubectl delete pod $(kubectl get pods -o json | jq -r '.items[0].name')"

# Test locally
echo '{"toolName":"Bash","toolInput":{"command":"'$COMMAND'"'}}' | \
  icg check --stdin

# Output:
# DENIED: kubectl delete pvc is permanently destructive.
```

#### Step 2: Analyze the Match

```bash
# Check which pattern matched
icg check --command "$COMMAND" --debug

# Output:
# DEBUG: Pattern matching trace
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Packed dispatched: kubectl (matched keyword "kubectl")
#
# Safe patterns checked: 0 matches
#
# Guarded patterns checked:
#   kubectl-delete-pvc: NO MATCH
#   kubectl-delete-deployment: NO MATCH
#   kubectl-delete-pod: MATCH (regex: "kubectl delete")
#
# Final verdict: DENY
# Pattern: kubectl-delete-pod
# Reason: Deleting pods can cause service disruption
```

#### Step 3: Identify the Problem

```bash
# View the pattern
icg explain --pattern kubectl-delete-pod

# Output:
# Pattern: kubectl-delete-pod
# Regex: kubectl delete
# Issue: Too broad! Matches "kubectl delete pod" AND "kubectl delete deployment"
#
# The pattern "kubectl delete" is too permissive.
# It matches:
#   - kubectl delete pod (legitimate)
#   - kubectl delete deployment (legitimate)
#   - kubectl delete pvc (dangerous)
```

#### Step 4: Fix the Pattern

```bash
# Edit the pack to be more specific
cat > /tmp/kubectl-fix.json <<'EOF'
{
  "id": "kubectl-delete-pod",
  "type": "command_regex",
  "regex": "kubectl delete pod",
  "tier": "tier1",
  "severity": "Medium",
  "explanation": "Deleting pods causes service disruption but pods can be recreated",
  "destructive": false,
  "redirect": {
    "channel": "deny",
    "reason_template": "kubectl delete pod disrupts service. Use 'kubectl rollout restart' instead.",
    "rewrite_template": null
  }
}
EOF
```

#### Step 5: Verify the Fix

```bash
# Test the new pattern
icg check --command "kubectl delete pod myapp-abc123" \
  --pack /tmp/kubectl-fix.json

# Output:
# DENIED: kubectl delete pod disrupts service. Use 'kubectl rollout restart' instead.

# Test that deployment deletion is still caught
icg check --command "kubectl delete deployment myapp" \
  --pack /tmp/kubectl-fix.json

# Output:
# ALLOW: No patterns matched (kubectl-delete-pod doesn't match "delete deployment")
```

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

```bash
# Edit Claude Code settings for both hooks
cat > ~/.config/claude-code/settings.json <<'EOF'
{
  "hooks": {
    "PreToolUse": [
      {
        "command": "~/.claude/hooks/org-rule-guard.py",
        "args": []
      },
      {
        "command": "/usr/local/bin/icg",
        "args": ["check", "--stdin", "--harness", "claude-code"]
      }
    ]
  }
}
EOF

# Test coexistence
echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  ~/.claude/hooks/org-rule-guard.py

# Output:
# BLOCKED: vault kv destroy (org-rule-guard.py doesn't catch this)
# ALLOW: (org-rule-guard.py doesn't protect vault)

echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  icg check --stdin

# Output:
# DENIED: vault kv destroy is permanently destructive (icg catches this)
```

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
# Configure Claude Code hook
mkdir -p ~/.config/claude-code
cat > ~/.config/claude-code/settings.json <<'EOF'
{
  "hooks": {
    "PreToolUse": {
      "command": "/usr/local/bin/icg",
      "args": ["check", "--stdin", "--harness", "claude-code"]
    }
  }
}
EOF

# Test with Claude Code
echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  icg check --stdin --harness claude-code

# Output:
# DENIED: vault kv destroy is permanently destructive
```

#### Step 2: Test Codex CLI Integration

```bash
# Configure Codex CLI hook
mkdir -p ~/.config/codex-cli
cat > ~/.config/codex-cli/settings.json <<'EOF'
{
  "hooks": {
    "PreToolUse": {
      "command": "/usr/local/bin/icg",
      "args": ["check", "--stdin", "--harness", "codex-cli"]
    }
  }
}
EOF

# Test with Codex CLI
echo '{"toolName":"Bash","toolInput":{"command":"git push --force origin main"}}' | \
  icg check --stdin --harness codex-cli

# Output:
# DENIED: git push --force would rewrite public history
```

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
# Check denials by harness
icg status --denials --by-harness --since 1d

# Output:
# DENIALS BY HARNESS (last 1d)
# ════════════════════════════════════════════════════════════════
# Harness         Denials   % of Total
# ────────────────────────────────────────────────────────────────
# claude-code     45        68%
# codex-cli       21        32%
```

---

### Scenario 12: Configuring Repository Overrides

**Context**: A specific repository needs an exception to a rule.

#### Step 1: Identify the Need

```bash
# Repository: legacy-app
# Issue: Uses bare git SHA in image tags (historical reason)
# Rule: image-tag pack blocks "image: app@sha256:..."
# Need: Override for this specific repo
```

#### Step 2: Request Override

```bash
# Create override request
icg override create \
  --repo /home/coding/legacy-app \
  --pattern-id "image-tag-bare-sha" \
  --justification "Legacy app uses immutable SHA-based tags for audit compliance. SHA is sourced from build system and never manually specified. Approved by security@company.com."

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
  --expiration 2026-12-31

# Output:
# ✓ Override approved and installed
# Repository: /home/coding/legacy-app
# Pattern: image-tag-bare-sha
# Expires: 2026-12-31
# Stored in: /etc/icg/overrides/legacy-app-image-tag-bare-sha.json
```

#### Step 5: Verify Override

```bash
# Test in the repository
cd /home/coding/legacy-app
cat deployment.yaml | grep "image:"

# Output:
# image: app@sha256:abc123...

# Test if blocked
icg check --file deployment.yaml

# Output:
# ALLOW: Repository override in effect (image-tag-bare-sha)

# Test outside repository
cd /tmp
echo "image: app@sha256:abc123..." | icg check --file -

# Output:
# DENIED: image tag bare SHA is not allowed
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
# test-env               vault-policy-delete   2026-09-30

# Review quarterly
echo "Override review scheduled: $(date -d '+3 months')" >> calendar.txt
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
**Last Updated**: 2026-08-16
**For**: icg v0.1.0+
