# icg Operator Training Manual

Comprehensive training guide for operators working with the irreversible command gate (icg). This manual covers everything from basic concepts to advanced operations.

## Table of Contents

1. [Introduction to icg](#introduction-to-icg)
2. [Core Concepts](#core-concepts)
3. [Installation and Setup](#installation-and-setup)
4. [Day-to-Day Operations](#day-to-day-operations)
5. [Monitoring and Alerting](#monitoring-and-alerting)
6. [Maintenance Procedures](#maintenance-procedures)
7. [Emergency Response](#emergency-response)
8. [Troubleshooting](#troubleshooting)
9. [Advanced Topics](#advanced-topics)
10. [Practical Exercises](#practical-exercises)

---

## Introduction to icg

### What is icg?

**icg (irreversible-command-gate)** is a safety system for AI coding and automation agents that intercepts commands before they execute and blocks operations that cause irreversible or hard-to-reverse damage.

### Problem Statement

AI coding agents have the power to execute destructive operations:
- Delete secret data from Vault
- Force-push to git repositories
- Destroy Kubernetes resources
- Purge persistent volumes
- Write corrupt data to shared state

Without protection, these operations can cause:
- **Data loss**: Secrets, configurations, or application data
- **Infrastructure damage**: Cluster state, network resources
- **Workflow disruption**: Broken deployments, failed builds
- **Security incidents**: Exposed credentials, compromised systems

### icg Solution

icg provides:
1. **Pre-execution interception**: Catches dangerous commands before they run
2. **Pattern-based blocking**: Uses regex patterns to identify destructive operations
3. **Redirect-not-just-block**: Every denial explains what to do instead
4. **Graduated availability policy**: Fail-Open is the default; approved cohorts can use Fail-Closed when guard availability is more important than workflow continuity. See the [Fail-Closed mode guide](fail-closed-mode.md).
5. **Zero network dependency**: Core evaluation doesn't require external calls

### What icg Protects

| Domain | Protected Operations | Severity |
|--------|---------------------|----------|
| **Vault** | `vault kv destroy`, `vault policy delete` | Critical |
| **Git** | `git push --force`, `git push -f` | Critical |
| **Kubernetes** | `kubectl delete pvc` | Critical |
| **Container Images** | `:latest` tags, bare SHA | High |
| **Storage** | `ssd`/`ssd-large` on Rackspace Spot | High |
| **Beads** | Writing `.beads/` in shared checkouts | Critical |
| **Secrets** | Credential values in commands | Critical |

### What icg Does NOT Protect

- **Prompt injection**: Malicious repositories tricking agents (out of scope)
- **Cloud-hosted agents**: Only local CLI installations are covered
- **Authorized destruction**: If you disable icg, it can't stop you
- **Non-command operations**: File edits outside hooks, library calls

Fail-Closed changes guard-availability failures, not ordinary rule outcomes.
An ordinary denial is not a poison pill. For activation, monitoring, and
emergency demotion, use the [Fail-Closed mode guide](fail-closed-mode.md)
rather than setting `ICG_FAIL_CLOSED=false` or editing policy JSON by hand.

---

## Core Concepts

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                      AI Agent                                │
│                    (Claude/Codex)                            │
└───────────────────────────┬─────────────────────────────────┘
                            │ attempts operation
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                   AI Harness                                  │
│              (Claude Code/Codex CLI)                          │
│            Sends PreToolUse JSON to hook                    │
└───────────────────────────┬─────────────────────────────────┘
                            │ stdin: tool invocation
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                    icg Hook                                   │
│           (/usr/local/bin/icg check --stdin)                │
└───────────────────────────┬─────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                 Evaluation Engine                            │
│  ┌──────────────────────────────────────────────────────┐ │
│  │  1. Parse PreToolUse JSON                             │ │
│  │  2. Dispatch to matching rule pack                    │ │
│  │  3. Check safe_patterns (allow if match)              │ │
│  │  4. Check guarded_patterns (deny if match)            │ │
│  │  5. Generate decision (deny/allow/warning)            │ │
│  └──────────────────────────────────────────────────────┘ │
└───────────────────────────┬─────────────────────────────────┘
                            │ stdout: JSON decision
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                   AI Harness                                  │
│         Blocks operation if verdict=deny                    │
└─────────────────────────────────────────────────────────────┘
```

### Rule Pack Structure

A **rule pack** is a JSON file defining protection patterns for a specific tool:

```json
{
  "id": "vault",
  "tool_keywords": ["vault"],
  "safe_patterns": [
    {
      "id": "safe-read",
      "type": "command_regex",
      "regex": "^vault kv get"
    }
  ],
  "guarded_patterns": [
    {
      "id": "vault-kv-destroy",
      "type": "command_regex",
      "regex": "vault kv destroy",
      "tier": "tier1",
      "severity": "Critical",
      "explanation": "Permanently destroys secret data",
      "destructive": true,
      "redirect": {
        "channel": "deny",
        "reason_template": "vault kv destroy is permanently destructive",
        "rewrite_template": null
      }
    }
  ]
}
```

### Pack Modes

**Command-Mode Packs**:
- Match shell command invocations
- Use `tool_keywords` to identify relevant commands
- Work in both hook and wrapper frontends
- Examples: vault, git, kubectl

**Content-Mode Packs**:
- Match file content being written
- Use `applies_to` globs to match file paths
- Hook-frontend only (Write/Edit operations)
- Examples: storage-class, image-tag, beads

### Evaluation Flow

```
Command received
       │
       ▼
Dispatch to pack (by tool_keyword or file glob)
       │
       ├─→ No pack matches → ALLOW (fail-open)
       │
       ├─→ Pack found
       │     │
       │     ▼
       │ Check safe_patterns
       │     │
       │     ├─→ Match → ALLOW
       │     │
       │     └─→ No match
       │           │
       │           ▼
       │     Check guarded_patterns
       │           │
       │           ├─→ Match → DENY (with redirect)
       │           │
       │           └─→ No match → ALLOW
       │
       └─→ Parse error → ALLOW (fail-open)
```

### Severity Levels

- **Critical**: Immediate, irreversible damage
  - Data destruction (vault kv destroy)
  - History rewriting (git push --force)
  - State corruption (writing .beads/ in shared checkout)

- **High**: Significant damage or hard to reverse
  - Resource deletion (kubectl delete pvc)
  - Wrong configuration (ssd storage on Spot)
  - Unpinned images (:latest tags)

- **Medium**: Moderate damage with workarounds
  - Service disruption (kubectl delete pod)
  - Deprecated tool usage

### Response Channels

- **deny**: Block the operation entirely (critical/high severity)
- **updated_input**: Provide a safe alternative (future feature)
- **additional_context**: Warn without blocking (tier 3 only)

---

## Installation and Setup

### Prerequisites

Before installing icg, ensure you have:

- **Operating System**: Linux (x86_64)
- **AI Harness**: Claude Code or Codex CLI installed
- **Permissions**: sudo access for system directories
- **Network**: Internet connection for downloading releases

### Installation Methods

#### Method 1: Download Release Binary (Recommended)

```bash
# Download the latest release
wget https://github.com/jedarden/irreversible-command-gate/releases/download/v0.1.0/icg-v0.1.0-x86_64-unknown-linux-gnu.tar.gz

# Extract
tar -xzf icg-v0.1.0-x86_64-unknown-linux-gnu.tar.gz

# Install to system directory
sudo cp icg /usr/local/bin/
sudo chmod +x /usr/local/bin/icg

# Verify installation
icg --version
# Expected output: icg v0.1.0
```

#### Method 2: Build from Source

```bash
# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Clone repository
git clone https://github.com/jedarden/irreversible-command-gate.git
cd irreversible-command-gate

# Build release binary
cargo build --release

# Install
sudo cp target/release/icg /usr/local/bin/
sudo chmod +x /usr/local/bin/icg

# Verify
icg --version
```

### Rule Pack Installation

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

sudo curl -o /etc/icg/packs/storage-class.json \
  https://raw.githubusercontent.com/jedarden/irreversible-command-gate/v0.1.0/packs/storage-class.json

sudo curl -o /etc/icg/packs/beads.json \
  https://raw.githubusercontent.com/jedarden/irreversible-command-gate/v0.1.0/packs/beads.json

# Set permissions
sudo chmod 644 /etc/icg/packs/*.json

# Verify rule packs
icg health --check-packs
```

### Hook Configuration

#### Claude Code Hook

```bash
# Create Claude Code config directory
mkdir -p ~/.config/claude-code

# Configure hook
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

# Verify hook
icg health --check-hooks
```

#### Codex CLI Hook

```bash
# Create Codex CLI config directory
mkdir -p ~/.config/codex-cli

# Configure hook
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

# Verify hook
icg health --check-hooks
```

### Verification Testing

```bash
# Test 1: Dangerous command (should be denied)
echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  icg check --stdin --harness claude-code

# Expected output:
# {
#   "verdict": "deny",
#   "packId": "vault",
#   "patternId": "vault-kv-destroy",
#   "severity": "Critical",
#   "reason": "vault kv destroy is permanently destructive and cannot be undone",
#   "rewrite": null,
#   "telemetryId": "den-abc123"
# }

# Test 2: Safe command (should be allowed)
echo '{"toolName":"Bash","toolInput":{"command":"vault kv get secret/test"}}' | \
  icg check --stdin --harness claude-code

# Expected output:
# {
#   "verdict": "allow",
#   "telemetryId": "all-def456"
# }

# Test 3: Run full health check
icg health --verbose

# Expected output:
# ✓ icg binary: /usr/local/bin/icg v0.1.0
# ✓ Rule packs: 5 packs loaded
#   - vault (8 patterns)
#   - git (12 patterns)
#   - image-tag (6 patterns)
#   - storage-class (4 patterns)
#   - beads (2 patterns)
# ✓ Claude Code hook: Configured
# ✓ State store: /var/lib/icg/state.db
# ✓ Denial log: /var/log/icg/denials.log
```

---

## Day-to-Day Operations

### Morning Routine

Start each day with a quick health check:

```bash
# Check icg health
icg health

# Check recent denials (overnight activity)
icg status --denials --since 12h

# Review any critical denials
icg status --denials --severity Critical --since 12h
```

### Monitoring Denials

Throughout the day, periodically check denials:

```bash
# View denials from the last hour
icg status --denials --since 1h

# View denial pattern summary
icg status --denials --pattern-summary --since 1d

# Export denials for analysis
icg status --denials --since 1d --format json > daily-denials.json
```

### Understanding Denial Trends

Analyze patterns to identify training needs:

```bash
# Weekly trend analysis
icg status --denials --trend --since 7d

# Top denied patterns
icg status --denials --pattern-summary --since 7d | head -20

# Denials by severity
icg status --denials --by-severity --since 7d
```

### Handling User Questions

When users ask about denials:

1. **Get the denial details**:
   ```bash
   # Ask user for the telemetry ID from the denial message
   icg explain --denial <telemetry-id>
   ```

2. **Explain the pattern**:
   ```bash
   icg explain --pattern <pattern-id>
   ```

3. **Provide alternatives**:
   ```bash
   # Check if rewrite is available
   icg explain --pattern <pattern-id> --show-redirect
   ```

### Updating Rule Packs

Check for and apply updates:

```bash
# Check for updates
icg update --check-only

# If updates are available, review the changes
icg update --dry-run

# Apply updates (during maintenance window)
icg update

# Verify after update
icg health --check-packs
```

### End-of-Day Review

```bash
# Daily summary
icg status --denials --since 1d --summary

# Check for any anomalies
icg status --denials --since 1d --severity Critical

# Export daily data
icg export --denials --since 1d --output denials-$(date +%Y%m%d).json
```

---

## Monitoring and Alerting

### Key Metrics to Monitor

1. **Denial Count**: Number of blocked operations
2. **Denial Rate**: Denials per hour/day
3. **Pattern Distribution**: Which patterns are triggering most
4. **Severity Breakdown**: Critical vs High vs Medium
5. **Trend Analysis**: Increasing or decreasing over time

### Monitoring Commands

```bash
# Real-time denial monitoring (watch mode)
watch -n 60 'icg status --denials --since 5m'

# Hourly denial rate
icg status --denials --rate --since 24h

# Pattern hot spots
icg status --denials --pattern-summary --since 24h | grep -E "Increasing|Critical"

# Weekly comparison
icg status --denials --compare-weeks 2
```

### Setting Up Alerts

Create alert thresholds:

```bash
# Alert if critical denials exceed threshold
icg alert create \
  --name critical-denials \
  --condition "denials > 10 AND severity = 'Critical'" \
  --period 1h \
  --action email \
  --recipient ops-team@company.com

# Alert if denial rate spikes
icg alert create \
  --name denial-spike \
  --condition "rate > 50/hour" \
  --period 1h \
  --action slack \
  --channel #ops-alerts
```

### Dashboard Setup

Recommended dashboard panels:

1. **Denials Over Time**: Line chart showing denial count per hour
2. **Top Patterns**: Bar chart of most-triggered patterns
3. **Severity Distribution**: Pie chart of Critical/High/Medium
4. **Denial Rate**: Single-stat showing current rate
5. **Recent Criticals**: Table of last 10 critical denials

---

## Maintenance Procedures

### Weekly Maintenance

```bash
# Health check
icg health --verbose > /var/log/icg/health-$(date +%Y%m%d).txt

# Disk space check
df -h /var/log/icg
df -h /var/lib/icg

# Log rotation (if needed)
sudo logrotate /etc/logrotate.d/icg

# Review rule pack versions
icg status --rule-packs
```

### Monthly Maintenance

```bash
# Full denial analysis
icg status --denials --since 30d --report > /tmp/monthly-report-$(date +%Y%m).txt

# Rule pack update check
icg update --check-only

# Review and document any false positives
icg status --denials --since 30d --tag false-positive

# Backup rule packs
sudo tar -czf /tmp/icg-packs-backup-$(date +%Y%m).tar.gz /etc/icg/packs/
```

### Quarterly Maintenance

```bash
# Full system backup
icg backup create --output /tmp/icg-full-backup-$(date +%Y%m%d).tar.gz

# Review all overrides
icg override list --include-expired > /tmp/overrides-review-$(date +%Y%m).txt

# Audit critical denials
icg audit --since 90d --severity Critical

# Performance check
icg benchmark --duration 60s
```

### Log Rotation

Configure logrotate:

```bash
sudo cat > /etc/logrotate.d/icg <<'EOF'
/var/log/icg/denials.log {
    daily
    rotate 30
    compress
    delaycompress
    missingok
    notifempty
    create 0644 root root
    postrotate
        icg telemetry reopen > /dev/null 2>&1 || true
    endscript
}

/var/log/icg/health.log {
    weekly
    rotate 52
    compress
    delaycompress
    missingok
    notifempty
    create 0644 root root
}
EOF
```

---

## Emergency Response

### Identifying Emergencies

An icg-related emergency is when:
- A legitimate critical operation is blocked
- The blocking causes service outage
- No safe alternative exists
- Manual intervention is required

### Emergency Bypass Procedure

#### Step 1: Verify Emergency

```bash
# Confirm icg is blocking the operation
icg status --health

# Check the specific denial
icg explain --denial <telemetry-id>

# Confirm no alternative exists
icg explain --pattern <pattern-id> --show-redirect
```

#### Step 2: Document Decision

```bash
# Create incident record
cat > /tmp/icg-emergency-$(date +%s).txt <<EOF
EMERGENCY BYPASS RECORD
======================
Timestamp: $(date)
Operator: $(whoami)
Service: <affected-service>
Issue: <description>
Command: <blocked-command>
Pattern: <pattern-id>
Justification: <why this is necessary>
Risk Assessment: <potential damage>
Approved By: <approver-name>
EOF
```

#### Step 3: Execute Bypass

**Option A: Single-command bypass**
```bash
ICG_DISABLED=1 <dangerous-command>
```

**Option B: Temporary disable**
```bash
# Disable icg temporarily
export ICG_DISABLED=1

# Execute necessary operations
<command-1>
<command-2>

# Re-enable
unset ICG_DISABLED
```

**Option C: Pattern-specific override (if supported)**
```bash
icg override temporary \
  --pattern <pattern-id> \
  --duration 15m \
  --reason "<incident-description>"
```

#### Step 4: Verify and Monitor

```bash
# Verify operation succeeded
<verification-command>

# Re-enable icg
unset ICG_DISABLED

# Verify icg is active
icg health

# Monitor for follow-up issues
icg status --denials --since 5m
```

#### Step 5: Follow Up

```bash
# File incident report
gh issue create \
  --title "Incident: Emergency bypass of <pattern-id>" \
  --body "Attached emergency record. Review needed." \
  --label incident \
  --repo jedarden/irreversible-command-gate

# Schedule postmortem
echo "Postmortem: $(date -d '+2 days')" | mail -s "Postmortem Required" ops-team@company.com

# Update runbook
# Document why this happened and how to prevent future emergencies
```

### Common Emergency Scenarios

#### Scenario 1: Vault Policy Deleted

**Symptom**: Authentication failing after vault policy deletion

**Emergency**:
```bash
# Restore policy
ICG_DISABLED=1 vault policy write auth-policy /backups/auth-policy.hcl

# Verify
vault policy read auth-policy
```

**Follow-up**: Review why policy deletion was blocked and if rule needs adjustment

#### Scenario 2: Git Force-Push Required

**Symptom**: Repository history corrupted, needs force-push to fix

**Emergency**:
```bash
# Force-push to fix history
ICG_DISABLED=1 git push --force origin corrected-branch

# Verify
git log --oneline origin/corrected-branch
```

**Follow-up**: Document why this was necessary and review git workflow

#### Scenario 3: PVC Deletion Required

**Symptom**: Stuck PVC blocking deployment

**Emergency**:
```bash
# Delete stuck PVC
ICG_DISABLED=1 kubectl delete pvc stuck-pvc

# Verify
kubectl get pvc
```

**Follow-up**: Review why PVC got stuck and prevent recurrence

---

## Troubleshooting

### Common Issues

#### Issue: icg Not Intercepting Commands

**Symptoms**: Commands execute without icg checking them

**Diagnosis**:
```bash
# Check hook configuration
icg health --check-hooks

# Verify binary exists
which icg
ls -la /usr/local/bin/icg

# Test manually
echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  icg check --stdin
```

**Solutions**:
- Reinstall hook configuration
- Verify harness is calling the hook
- Check file permissions

#### Issue: False Positive

**Symptoms**: Legitimate command being blocked

**Diagnosis**:
```bash
# Get denial details
icg explain --denial <telemetry-id>

# Test the pattern
icg check --command "<exact-command>" --debug

# Review pattern regex
icg explain --pattern <pattern-id> --show-regex
```

**Solutions**:
- Report false positive to maintainers
- Request pattern refinement
- Use repository override (if approved)

#### Issue: Rule Pack Won't Load

**Symptoms**: Health check shows pack errors

**Diagnosis**:
```bash
# Check pack syntax
icg health --check-packs --verbose

# Validate JSON
jq empty /etc/icg/packs/problem-pack.json

# Check regex syntax
icg validate-pack /etc/icg/packs/problem-pack.json
```

**Solutions**:
- Fix JSON syntax errors
- Validate regex patterns
- Re-download pack from upstream

#### Issue: High Denial Rate

**Symptoms**: Unusual spike in denials

**Diagnosis**:
```bash
# Check denial trends
icg status --denials --trend --since 1h

# Identify top patterns
icg status --denials --pattern-summary --since 1h

# Check for specific agent
icg status --denials --by-session --since 1h
```

**Solutions**:
- Identify triggering agent/session
- Review with agent operator
- Check for pattern changes
- Provide additional training

### Debug Mode

Enable debug output:

```bash
# Enable debug logging
export ICG_DEBUG=1
export ICG_LOG_LEVEL=debug

# Run with debug
icg check --command "vault kv destroy secret/test"

# Check debug log
tail -f /var/log/icg/debug.log
```

### Getting Help

When stuck:

1. **Check documentation**:
   ```bash
   man icg
   icg --help
   ```

2. **Search existing issues**:
   ```bash
   gh issue list --search "<error-message>"
   ```

3. **Create bug report**:
   ```bash
   icg bug-report --output /tmp/icg-bug-report.txt
   gh issue create \
     --title "Bug: <description>" \
     --body "Attached bug report" \
     --repo jedarden/irreversible-command-gate
   ```

---

## Advanced Topics

### Multi-Harness Support

Protect both Claude Code and Codex CLI:

```bash
# Configure both harnesses
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

# Verify both
icg health --check-hooks
```

### Repository Overrides

Allow exceptions for specific repositories:

```bash
# Request override
icg override create \
  --repo /path/to/repo \
  --pattern <pattern-id> \
  --justification "<reason>"

# Get approval
# Submit for security review

# Apply approved override
icg override approve \
  --request /tmp/override-request.json \
  --approver <approver> \
  --expiration <date>
```

### Custom Rule Packs

Create organization-specific packs:

```bash
# Scaffold new pack
icg new-pack --id my-tool --mode command

# Edit pack.json
vim /etc/icg/packs/my-tool.json

# Test locally
icg check --command "my-tool dangerous-action" \
  --pack /etc/icg/packs/my-tool.json

# Install
sudo cp /etc/icg/packs/my-tool.json /etc/icg/packs/
sudo chmod 644 /etc/icg/packs/my-tool.json

# Verify
icg health --check-packs
```

### Performance Tuning

Optimize for high-throughput environments:

```bash
# Benchmark current performance
icg benchmark --iterations 1000

# Adjust cache size
export ICG_CACHE_SIZE=1000

# Adjust worker threads
export ICG_WORKERS=4

# Re-benchmark
icg benchmark --iterations 1000
```

---

## Practical Exercises

### Exercise 1: Basic Installation

**Objective**: Install icg and verify it's working

**Steps**:
1. Download and install icg binary
2. Install default rule packs
3. Configure Claude Code hook
4. Test with a dangerous command
5. Verify the command is blocked

**Validation**:
```bash
icg health
# Should show all checks passing
```

### Exercise 2: Pattern Understanding

**Objective**: Understand how patterns work

**Steps**:
1. List all patterns in the vault pack
2. Test safe commands (vault kv get)
3. Test dangerous commands (vault kv destroy)
4. Examine the denial messages
5. Identify the redirect suggestions

**Validation**:
```bash
icg explain --pattern vault-kv-destroy
# Should show pattern details and alternatives
```

### Exercise 3: Denial Investigation

**Objective**: Practice investigating denials

**Steps**:
1. Generate a test denial
2. Note the telemetry ID
3. Look up the denial details
4. Understand why it was blocked
5. Identify the safe alternative

**Validation**:
```bash
icg status --denials --since 1h
# Should show the test denial
```

### Exercise 4: Emergency Response

**Objective**: Practice emergency bypass procedure

**Steps**:
1. Simulate an emergency (legitimate command blocked)
2. Document the decision
3. Execute emergency bypass
4. Verify operation succeeded
5. Re-enable icg
6. File incident report

**Validation**:
```bash
# Check emergency documentation exists
ls -la /tmp/icg-emergency-*
```

### Exercise 5: Monitoring Setup

**Objective**: Set up basic monitoring

**Steps**:
1. Create a daily denial summary report
2. Set up a weekly trend analysis
3. Identify top 5 denied patterns
4. Check for any critical denials
5. Document findings

**Validation**:
```bash
icg status --denials --since 7d --report
# Should generate comprehensive report
```

---

## Assessment Checklist

Use this checklist to assess operator readiness:

### Basic Competency

- [ ] Can explain what icg does and why it's needed
- [ ] Can install icg from scratch
- [ ] Can configure hooks for AI harnesses
- [ ] Can interpret denial messages
- [ ] Can find safe alternatives to blocked operations

### Intermediate Competency

- [ ] Can troubleshoot hook failures
- [ ] Can investigate false positives
- [ ] Can analyze denial trends
- [ ] Can update rule packs safely
- [ ] Can perform emergency bypass

### Advanced Competency

- [ ] Can create custom rule packs
- [ ] Can tune icg performance
- [ ] Can set up monitoring and alerting
- [ ] Can train other operators
- [ ] Can contribute to rule pack development

---

## Resources

### Internal Resources

- **Quick Start Guide**: `docs/quick-start.md`
- **Operator Documentation**: `docs/operators/README.md`
- **Denial Messages**: `docs/operators/deny-messages.md`
- **Troubleshooting**: `docs/operators/troubleshooting.md`
- **Examples**: `docs/examples/README.md`

### External Resources

- **GitHub Repository**: https://github.com/jedarden/irreversible-command-gate
- **Issue Tracker**: https://github.com/jedarden/irreversible-command-gate/issues
- **Documentation**: https://github.com/jedarden/irreversible-command-gate/tree/main/docs

### Getting Help

1. **Documentation first**: Check the relevant doc file
2. **GitHub Issues**: Search existing issues or create new one
3. **Security Team**: security@company.com for security-related questions
4. **Ops Team**: ops-team@company.com for operational issues

---

## Appendix

### Glossary

- **icg**: irreversible-command-gate
- **Rule Pack**: JSON file defining protection patterns
- **Pattern**: Regex rule matching dangerous operations
- **Hook**: Integration point with AI harness
- **Harness**: AI coding system (Claude Code, Codex CLI)
- **Denial**: Decision to block an operation
- **Redirect**: Suggested safe alternative
- **Telemetry ID**: Unique identifier for a denial event
- **Fail-open**: Availability failures allow operations and emit health evidence
- **Fail-closed**: Availability failures deny operations until the guard recovers

### Command Reference

| Command | Description |
|---------|-------------|
| `icg health` | Check system health |
| `icg check` | Test a command |
| `icg status` | View denials and statistics |
| `icg policy status` | View durable Fail-Open/Fail-Closed state and generation |
| `icg health status` | View crash, uptime, and stability metrics |
| `icg telemetry status` | View deny-rate baseline and rollback state |
| `icg explain` | Get pattern details |
| `icg update` | Update rule packs |
| `icg override` | Manage repository overrides |
| `icg backup` | Create backup |
| `icg benchmark` | Performance test |

### Severity Matrix

| Severity | Description | Examples |
|----------|-------------|----------|
| Critical | Irreversible damage | vault kv destroy, git push --force |
| High | Significant damage | kubectl delete pvc, :latest tags |
| Medium | Moderate damage | kubectl delete pod |

### File Locations

| File | Location |
|------|----------|
| Binary | `/usr/local/bin/icg` |
| Rule packs | `/etc/icg/packs/` |
| State store | `/var/cache/icg/session-state.json` |
| Denial log | `/var/log/icg/denials.log` |
| Health log | `/var/log/icg/health.log` |
| Debug log | `/var/log/icg/debug.log` |

---

**Training Manual Version**: 1.0
**Last Updated**: 2026-08-16
**For**: icg v0.1.0+
