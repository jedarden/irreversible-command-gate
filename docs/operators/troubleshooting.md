# Irreversible Command Gate (icg) - Troubleshooting Guide

## Overview

This guide helps operators diagnose and resolve common issues with icg, including false positives, rule conflicts, installation problems, and debugging procedures.

## Quick Reference: Common Issues

| Symptom | Likely Cause | Quick Fix |
|---------|--------------|-----------|
| Operations denied unexpectedly | False positive or outdated rule pack | Check `icg status --denials`, update rule pack |
| Hook not triggering | Misconfigured hook path | Verify `~/.claude/settings.json` or `~/.codex/hooks.json` |
| "Command not found" after install | PATH-wrapper symlinks missing | Recreate symlinks in `/usr/local/libexec/icg-wrappers/` |
| All operations denied | Fail-closed mode + guard crash | Check `ICG_FAIL_CLOSED`, verify rule pack integrity |
| Agent can't update icg | Permission denied | Use `sudo icg update` or fix `/etc/icg/` ownership |

## Diagnosis Procedures

### Step 1: Check Guard Health

```bash
# Basic health check
icg health

# Detailed diagnostics
icg health --verbose

# Check specific components
icg health --component engine
icg health --component rule-pack
icg health --component state-store
```

### Step 2: Review Recent Denials

```bash
# View recent denials
icg status --denials --since 1h

# View denials by rule pack
icg status --denials --group-by pack

# View denial patterns
icg status --denials --pattern-summary
```

### Step 3: Check Configuration

```bash
# View active configuration
icg config --show

# Verify rule pack is loaded
icg config --rule-pack

# Verify trust pointer
cat /etc/icg/trust-pointer.json | jq .
```

### Step 4: Test Individual Components

```bash
# Test engine directly
icg check --command "vault kv destroy secret/test"

# Test hook integration
echo '{"name":"bash","input":{"command":"vault kv destroy secret/test"}}' | icg hook

# Test PATH-wrapper
/usr/local/libexec/icg-wrappers/vault version
```

## Common Issues and Solutions

### Issue: False Positive - Operation Wrongly Denied

#### Symptom

A legitimate operation is denied with a generic reason like "Pattern matched: vault-destructive".

#### Diagnosis

```bash
# View the exact denial details
icg status --denials --last 1

# Check which pattern matched
icg status --denials --verbose
```

#### Solutions

**1. Update Rule Pack**

False positives are often fixed in newer rule pack versions:

```bash
# Check for updates
icg update --check-only

# Apply update if available
icg update
```

**2. Check for Pattern Refinement**

The pattern may be too broad. Check the rule pack:

```bash
# View the pattern that matched
cat /etc/icg/rule-pack.json | jq '.packs[] | select(.id=="vault") | .guarded_patterns[] | select(.id=="vault-destructive")'
```

**3. Create Repository Override (If Legitimate Use Case)**

If this is a legitimate, repo-specific exception:

```bash
# Create override file (requires Layer 1/2 approval)
icg override create --repo /path/to/repo \
  --pattern-id "vault-destructive" \
  --justification "Approved vault migration for legacy system"
```

**4. Report the Issue**

If it's a genuine false positive:

```bash
# Export denial details for bug report
icg status --denials --format json > false-positive-report.json

# File issue with the export
```

### Issue: Rule Conflicts - Multiple Patterns Match

#### Symptom

Unclear which rule caused denial, or conflicting reasons given.

#### Diagnosis

```bash
# View all matched patterns
icg status --denials --verbose --show-all-matches

# Check pattern precedence
icg config --show-precedence
```

#### Solutions

**1. Understand Precedence**

Precedence order (highest to lowest):
1. Safe patterns (explicitly allowed)
2. Guarded patterns (enabled, ordered by pack then by pattern ID)
3. Disabled patterns (ignored)

**2. Check for Overlapping Patterns**

```bash
# View pattern overlaps in rule pack
icg audit --pattern-overlaps
```

**3. Refine Patterns (If Rule Pack Author)**

Edit the rule pack to narrow pattern scopes or add safe patterns.

### Issue: Hook Not Triggering

#### Symptom

Commands execute without guard evaluation in Claude Code or Codex CLI.

#### Diagnosis

```bash
# Check hook configuration
cat ~/.claude/settings.json | jq '.hooks.PreToolUse'
cat ~/.codex/hooks.json

# Test hook directly
echo '{"name":"bash","input":{"command":"vault kv destroy secret/test"}}' | icg hook

# Check binary permissions
ls -la /usr/local/bin/icg
```

#### Solutions

**1. Fix Hook Configuration**

```bash
# For Claude Code
cat > ~/.claude/settings.json <<EOF
{
  "hooks": {
    "PreToolUse": {
      "bash": "/usr/local/bin/icg hook",
      "apply_patch": "/usr/local/bin/icg hook"
    }
  }
}
EOF

# For Codex CLI
cat > ~/.codex/hooks.json <<EOF
{
  "PreToolUse": {
    "bash": "/usr/local/bin/icg hook",
    "apply_patch": "/usr/local/bin/icg hook"
  }
}
EOF
```

**2. Restart Claude Code/Codex CLI**

Hook configuration is read at startup. Restart your session.

**3. Check File Permissions**

```bash
# Ensure hook is executable
sudo chmod 0755 /usr/local/bin/icg

# Ensure user can read hook
ls -la /usr/local/bin/icg
```

### Issue: PATH-Wrapper Not Working

#### Symptom

Real binary executes instead of wrapper (e.g., real `vault` runs instead of icg).

#### Diagnosis

```bash
# Check if symlinks exist
ls -la /usr/local/libexec/icg-wrappers/

# Check PATH order
echo $PATH | tr ':' '\n' | grep -n icg-wrappers

# Test wrapper directly
/usr/local/libexec/icg-wrappers/vault version
```

#### Solutions

**1. Recreate Symlinks**

```bash
# Ensure wrapper directory exists
sudo mkdir -p /usr/local/libexec/icg-wrappers

# Recreate symlinks
sudo ln -sf /usr/local/bin/icg /usr/local/libexec/icg-wrappers/vault
sudo ln -sf /usr/local/bin/icg /usr/local/libexec/icg-wrappers/git
sudo ln -sf /usr/local/bin/icg /usr/local/libexec/icg-wrappers/bao
```

**2. Fix PATH Order**

```bash
# Add to beginning of PATH in ~/.bashrc
echo 'export PATH="/usr/local/libexec/icg-wrappers:$PATH"' >> ~/.bashrc

# Reload shell
source ~/.bashrc
```

**3. Verify Symlinks**

```bash
# Should point to /usr/local/bin/icg
ls -la /usr/local/libexec/icg-wrappers/vault
```

### Issue: All Operations Denied

#### Symptom

Every operation is denied, including safe ones.

#### Diagnosis

```bash
# Check if fail-closed mode is enabled
echo $ICG_FAIL_CLOSED

# Check for guard crashes
icg health --component engine

# Check rule pack integrity
sha256sum /etc/icg/rule-pack.json
```

#### Solutions

**1. Check for Guard Crash**

If `ICG_FAIL_CLOSED=true` and the guard crashed:

```bash
# View crash logs
journalctl -u icg --since 1h | grep -i crash

# Restart guard (clears fail-open state)
sudo systemctl restart icg  # if running as service
# Or just start a new session (per-invocation model)
```

**2. Disable Fail-Closed (Emergency Only)**

```bash
# Emergency rollback to fail-open
unset ICG_FAIL_CLOSED
# Or
export ICG_FAIL_CLOSED=false
```

**3. Restore Rule Pack**

If rule pack is corrupted:

```bash
# Restore from backup
sudo cp /backups/icg-config-$(date +%F).tar.gz /etc/icg/rule-pack.json

# Or download known-good version
sudo wget -O /etc/icg/rule-pack.json \
  https://github.com/jedarden/irreversible-command-gate/releases/download/v0.0.9/rule-pack.json
```

### Issue: Rule Pack Update Fails

#### Symptom

`icg update` fails with network or permission errors.

#### Diagnosis

```bash
# Check network connectivity
curl -I https://github.com

# Check permissions
ls -la /etc/icg/

# Check disk space
df -h /etc/
```

#### Solutions

**1. Fix Permissions**

```bash
# Ensure root ownership
sudo chown root:root /etc/icg/
sudo chmod 0755 /etc/icg/

# Ensure files are root-owned
sudo chown root:root /etc/icg/*.json
sudo chmod 0644 /etc/icg/*.json
```

**2. Fix Network Issues**

```bash
# Test GitHub connectivity
curl -I https://github.com

# Use proxy if needed
export HTTPS_PROXY=http://your-proxy:port
icg update
```

**3. Free Disk Space**

```bash
# Clean up old rule pack backups
sudo rm /etc/icg/rule-pack.json.old.*

# Check disk space
df -h /etc/
```

### Issue: State Store Corruption

#### Symptom

Tier 2 rules (cross-invocation state) behave incorrectly or fail.

#### Diagnosis

```bash
# Check state store
icg health --component state-store

# View state file
sudo cat /var/lib/icg/state.json | jq .
```

#### Solutions

**1. Restore from Backup**

```bash
# Stop any active sessions
# Restore state
sudo cp /backups/icg-state-$(date +%F).json /var/lib/icg/state.json

# Verify
icg health --component state-store
```

**2. Clear State (Last Resort)**

```bash
# WARNING: This loses all Tier 2 rule state
sudo rm /var/lib/icg/state.json
icg health --component state-store  # Will recreate empty state
```

### Issue: Coexistence with org-rule-guard.py

#### Symptom

Double denials (both icg and org-rule-guard.py deny same operation).

#### Diagnosis

```bash
# Check if both hooks are registered
cat ~/.claude/settings.json | jq '.hooks.PreToolUse'

# Test with icg disabled temporarily
ICG_DISABLED=1 vault kv destroy secret/test
```

#### Solutions

**1. This is Expected During Migration**

Per `migration-from-org-rule-guard.md`, double denials are expected and harmless during coexistence.

**2. Verify Consistent Verdicts**

Both should deny or both should allow. Divergent verdicts indicate a problem:

```bash
# Run smoke test
icg smoke-test-vs-org-rule-guard
```

**3. Remove org-rule-guard.py After Migration**

Once migration is complete:

```bash
# Remove old hook
rm ~/.claude/hooks/org-rule-guard.py

# Update settings to remove reference
# Edit ~/.claude/settings.json to remove org-rule-guard.py
```

## Debugging Procedures

### Enable Debug Logging

```bash
# Set debug level
export ICG_LOG_LEVEL=debug

# Run command with debug output
icg check --command "vault kv destroy secret/test" 2>&1 | tee /tmp/icg-debug.log
```

### Test Pattern Matching

```bash
# Test if a specific pattern matches
icg test-pattern --pack vault --pattern-id vault-destructive \
  --command "vault kv destroy secret/test"

# Test all patterns in a pack
icg test-pattern --pack vault --all \
  --command "vault kv destroy secret/test"
```

### Export Rule Pack for Analysis

```bash
# Export entire rule pack
cat /etc/icg/rule-pack.json | jq . > /tmp/rule-pack-export.json

# Export specific pack
cat /etc/icg/rule-pack.json | jq '.packs[] | select(.id=="vault")' > /tmp/vault-pack.json
```

### Trace Evaluation Path

```bash
# Enable trace logging
export ICG_LOG_LEVEL=trace

# Run command and trace evaluation
icg check --command "vault kv destroy secret/test" 2>&1 | grep -i trace
```

## Performance Issues

### Issue: Slow Command Evaluation

#### Symptom

Noticeable delay before command executes or is denied.

#### Diagnosis

```bash
# Measure evaluation time
time icg check --command "vault status"

# Check rule pack size
cat /etc/icg/rule-pack.json | jq '.packs | length'

# Check for complex regexes
cat /etc/icg/rule-pack.json | jq '.packs[].guarded_patterns[].check' | grep -c '.*.*'
```

#### Solutions

**1. Reduce Rule Pack Size**

Disable unused rule packs (if rule pack author supports it):

```bash
# Request smaller pack or create custom pack
```

**2. Simplify Regex Patterns**

Work with rule pack author to simplify overly complex patterns.

**3. Use PATH-Wrapper for Frequently-Used Commands**

PATH-wrapper has lower overhead than hook for raw command execution.

### Issue: High Memory Usage

#### Symptom

icg process uses more memory than expected (>50MB).

#### Diagnosis

```bash
# Check memory usage
ps aux | grep icg

# Check rule pack size
ls -la /etc/icg/rule-pack.json

# Check state store size
ls -la /var/lib/icg/state.json
```

#### Solutions

**1. Rotate State Store**

```bash
# Backup and clear old state
sudo cp /var/lib/icg/state.json /backups/icg-state-old.json
sudo tee /var/lib/icg/state.json > /dev/null <<EOF
{"schema_version":1,"state":{}}
EOF
```

**2. Reduce Rule Pack Size**

Work with rule pack author to split large packs.

## Emergency Procedures

### Emergency Rollback

```bash
# Immediate rollback to previous release
icg update --rollback

# Rollback to specific version
icg update --rollback-to v0.0.9

# Emergency disable (last resort)
ICG_DISABLED=1 vault kv destroy secret/test
```

### Emergency Rollback from Fail-Closed

```bash
# 1. Immediately disable fail-closed
unset ICG_FAIL_CLOSED

# 2. Investigate crash
icg health --component engine

# 3. Fix issue (update rule pack, fix regex, etc.)

# 4. Re-qualify before re-enabling fail-closed
# (See fail-closed-transition.md)
```

### Fleet-Wide Emergency

```bash
# On orchestration host
for host in workstation-001 workstation-002 workstation-003; do
  ssh $host "unset ICG_FAIL_CLOSED && icg update --rollback"
done
```

## Getting Help

### Before Requesting Help

1. Run health check: `icg health --verbose`
2. Export denial history: `icg status --denials --format json > denial-report.json`
3. Export configuration: `icg config --show > config-report.txt`
4. Export logs: `journalctl -u icg --since 1h > icg-journal.log`

### Information to Include

- icg version: `icg --version`
- Rule pack version: `cat /etc/icg/trust-pointer.json | jq .trusted_release`
- OS and kernel version: `uname -a`
- Exact command that was denied
- Full denial message
- Relevant logs from the time of the issue

### Resources

- **Issues**: https://github.com/jedarden/irreversible-command-gate/issues
- **Documentation**: `docs/` directory in repository
- **Architecture**: See `docs/plan/plan.md` for detailed design
- **Design Notes**: See `docs/notes/` for individual design decisions

## Prevention

### Regular Maintenance

```bash
# Weekly update check
icg update --check-only

# Monthly rule pack review
icg audit --pattern-overlaps
icg status --denials --since 30d

# Quarterly health review
icg health --verbose
icg compliance-report --period 90d
```

### Monitoring Setup

Set up monitoring for:
- Guard uptime and crash rate
- Denial rate (alert if >1%)
- Rule pack version drift across fleet
- State store corruption

See `deployment-guide.md` for monitoring configuration.
