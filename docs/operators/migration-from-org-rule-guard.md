# Migration Guide: org-rule-guard.py to icg

## Overview

This guide helps operators migrate from the existing `org-rule-guard.py` hook to the new `irreversible-command-gate (icg)` system. The migration is designed to be non-disruptive, with a coexistence period where both systems run in parallel.

## What's Changing

### org-rule-guard.py Coverage

The existing hook enforces 5 rules:
1. No `.github/workflows/*` writes
2. No `kind: Job`/`kind: CronJob` in YAML writes
3. No `:latest` image tags in YAML writes
4. No mutating `kubectl` verbs in Bash calls
5. No committed credential values (Write/Edit only, not Bash)

### icg Coverage

icg supersedes most of these rules and adds new coverage:

| org-rule-guard.py Rule | icg Equivalent | Status |
|------------------------|----------------|--------|
| No `.github/workflows/*` | Not yet implemented (Phase 4) | Remains with org-rule-guard.py |
| No `kind: Job`/`CronJob` | Not yet implemented (Phase 4) | Remains with org-rule-guard.py |
| No `:latest` image tags | `image-tag` pack (includes bare-SHA) | **MIGRATED** |
| No mutating `kubectl` | Explicitly not implemented | Remains with org-rule-guard.py |
| No credential values (Write/Edit) | Remains with org-rule-guard.py | Coexistence |
| (New) Credential values in Bash | `secrets` pack | **NEW** |
| (New) Vault/OpenBao destructive ops | `vault` pack | **NEW** |
| (New) Git force-push | `git` pack | **NEW** |
| (New) Beads protection | `beads` pack | **NEW** |

### Migration Timeline

**Phase 1: Coexistence** (Current)
- Both hooks active
- Double denials are expected and harmless
- icg handles new rules (vault, git, beads, secrets in Bash)
- org-rule-guard.py handles its original 5 rules

**Phase 2: Gradual Transition** (Future)
- Migrate `:latest` rule to icg
- Keep org-rule-guard.py for kubectl, workflows, and credential rules

**Phase 3: Deprecation** (Future)
- org-rule-guard.py reduced to kubectl-only
- icg handles all other rules

## Pre-Migration Checklist

### System Requirements

- [ ] Linux system with x86_64 or ARM64 architecture
- [ ] Root access for installation
- [ ] Claude Code 1.0+ or Codex CLI with hook support
- [ ] Network access to GitHub Releases API

### Backup Current Configuration

```bash
# Backup org-rule-guard.py
cp ~/.claude/hooks/org-rule-guard.py ~/backups/org-rule-guard.py.backup

# Backup hook configuration
cp ~/.claude/settings.json ~/backups/settings.json.backup

# Note current org-rule-guard.py version
head -5 ~/.claude/hooks/org-rule-guard.py
```

### Verify Current Installation

```bash
# Check org-rule-guard.py is working
echo '{"name":"Write","input":{"path":"test.yaml","content":"kind: Job\n"}}' | \
  ~/.claude/hooks/org-rule-guard.py
# Should exit non-zero and output denial message

# Check current hook configuration
cat ~/.claude/settings.json | jq '.hooks.PreToolUse'
```

### Review Known Limitations

- [ ] Understood that `kubectl` mutations remain with org-rule-guard.py
- [ ] Understood that `.github/workflows` rules remain with org-rule-guard.py
- [ ] Understood that double denials are expected during coexistence
- [ ] Understood that icg does not defend against adversarial agents

## Migration Procedure

### Step 1: Install icg

Follow the installation guide in `deployment-guide.md`. Summary:

```bash
# Download and install binary
wget https://github.com/jedarden/irreversible-command-gate/releases/latest/download/icg-linux-x86_64.tar.gz
tar -xzf icg-linux-x86_64.tar.gz
sudo cp icg /usr/local/bin/icg
sudo chmod 0755 /usr/local/bin/icg
sudo chown root:root /usr/local/bin/icg

# Install rule pack
sudo mkdir -p /etc/icg
sudo wget -O /etc/icg/rule-pack.json \
  https://github.com/jedarden/irreversible-command-gate/releases/latest/download/rule-pack.json
sudo chmod 0644 /etc/icg/rule-pack.json
sudo chown root:root /etc/icg/rule-pack.json

# Initialize trust pointer
sudo tee /etc/icg/trust-pointer.json > /dev/null <<EOF
{
  "schema_version": 1,
  "trusted_release": "v0.1.0",
  "last_updated": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "update_source": "github-releases"
}
EOF
sudo chmod 0644 /etc/icg/trust-pointer.json
sudo chown root:root /etc/icg/trust-pointer.json
```

### Step 2: Configure icg Hook

**IMPORTANT**: This step adds icg alongside org-rule-guard.py, not replacing it.

```bash
# Backup current settings
cp ~/.claude/settings.json ~/settings.json.before-icg

# Add icg hook (keeps org-rule-guard.py)
# Edit ~/.claude/settings.json to add icg to PreToolUse hooks
# The configuration should now have BOTH hooks registered
```

Expected `~/.claude/settings.json` after this step:

```json
{
  "hooks": {
    "PreToolUse": {
      "bash": [
        "~/.claude/hooks/org-rule-guard.py",
        "/usr/local/bin/icg hook"
      ],
      "apply_patch": [
        "~/.claude/hooks/org-rule-guard.py",
        "/usr/local/bin/icg hook"
      ]
    }
  }
}
```

**Note**: Some hook systems may not support arrays. If yours doesn't, see "Alternative Configuration" below.

### Step 3: Verify Coexistence

```bash
# Restart Claude Code or Codex CLI to reload hook configuration

# Test that both hooks are active
# Try a :latest violation (should be denied by BOTH hooks)
# In a Claude Code session, try to write a YAML file with "image: foo:latest"

# Verify icg is working
icg status

# Verify health
icg health
```

### Alternative Configuration: Single Combined Hook

If your harness doesn't support multiple hooks, create a wrapper script:

```bash
# Create combined hook script
cat > ~/.claude/hooks/combined-guard.sh <<'EOF'
#!/bin/bash
# Run org-rule-guard.py first
~/.claude/hooks/org-rule-guard.py "$@"
RESULT=$?

# If org-rule-guard.py denied, stop there
if [ $RESULT -ne 0 ]; then
  exit $RESULT
fi

# Otherwise, run icg
/usr/local/bin/icg hook "$@"
EOF

chmod +x ~/.claude/hooks/combined-guard.sh

# Update settings to use combined hook
# Edit ~/.claude/settings.json:
# "bash": "~/.claude/hooks/combined-guard.sh"
# "apply_patch": "~/.claude/hooks/combined-guard.sh"
```

### Step 4: Configure PATH-Wrapper (Optional but Recommended)

```bash
# Create wrapper directory
sudo mkdir -p /usr/local/libexec/icg-wrappers

# Add to PATH
echo 'export PATH="/usr/local/libexec/icg-wrappers:$PATH"' >> ~/.bashrc
source ~/.bashrc

# Create symlinks
sudo ln -sf /usr/local/bin/icg /usr/local/libexec/icg-wrappers/vault
sudo ln -sf /usr/local/bin/icg /usr/local/libexec/icg-wrappers/git
sudo ln -sf /usr/local/bin/icg /usr/local/libexec/icg-wrappers/bao

# Test wrapper
vault status  # Should be evaluated by icg
```

### Step 5: Install-Time Smoke Test

Run the coexistence smoke test to verify both hooks give consistent verdicts:

```bash
# Built-in smoke test (if available)
icg smoke-test-vs-org-rule-guard

# Or manual verification
# Test case 1: :latest tag (should be denied by BOTH)
echo '{"name":"Write","input":{"path":"test.yaml","content":"image: foo:latest\n"}}' | \
  ~/.claude/hooks/org-rule-guard.py
echo '{"name":"Write","input":{"path":"test.yaml","content":"image: foo:latest\n"}}' | \
  /usr/local/bin/icg hook
# Both should exit non-zero

# Test case 2: Safe operation (should be allowed by BOTH)
echo '{"name":"Write","input":{"path":"test.txt","content":"hello world\n"}}' | \
  ~/.claude/hooks/org-rule-guard.py
echo '{"name":"Write","input":{"path":"test.txt","content":"hello world\n"}}' | \
  /usr/local/bin/icg hook
# Both should exit zero
```

## Verification Procedures

### Post-Installation Verification

```bash
# Verify icg is installed correctly
icg --version
icg health --verbose

# Verify hook configuration
cat ~/.claude/settings.json | jq '.hooks.PreToolUse'

# Verify rule pack is loaded
icg config --rule-pack

# Verify trust pointer
cat /etc/icg/trust-pointer.json | jq .
```

### Coexistence Verification

```bash
# Test that both hooks are firing
# In a Claude Code session, try a denied operation:
# - Write YAML with "kind: Job"
# - Write YAML with "image: foo:latest"
# - Run "kubectl delete pod test"

# Each should produce TWO denial messages (one from each hook)
# This is expected and harmless

# Verify consistent verdicts
icg smoke-test-vs-org-rule-guard
```

### Functional Verification

Test that icg's new rules work:

```bash
# Test vault pack
vault kv destroy secret/test  # Should be denied

# Test git pack
git push --force origin main  # Should be denied

# Test beads pack
echo "test" > .beads/test.json  # Should be denied in shared checkout

# Test secrets pack (Bash channel)
echo "ghp_test_token" >> test.txt  # Should be denied
```

### Rollback Test

Test that rollback works if needed:

```bash
# Test icg rollback
icg update --rollback

# Re-apply update
icg update

# Test emergency disable
ICG_DISABLED=1 vault kv destroy secret/test  # Should succeed
```

## Rollback Procedures

### Partial Rollback: icg Only

If you need to remove icg but keep org-rule-guard.py:

```bash
# Remove icg from hook configuration
# Edit ~/.claude/settings.json to remove icg from PreToolUse hooks

# Restore original settings
cp ~/settings.json.before-icg ~/.claude/settings.json

# Remove PATH-wrapper symlinks (if installed)
sudo rm /usr/local/libexec/icg-wrappers/vault
sudo rm /usr/local/libexec/icg-wrappers/git
sudo rm /usr/local/libexec/icg-wrappers/bao

# Remove icg binary
sudo rm /usr/local/bin/icg

# Remove rule pack
sudo rm -rf /etc/icg/

# Restart Claude Code/Codex CLI
```

### Full Rollback: Both Systems

If you need to revert to a pre-icg state entirely:

```bash
# Remove icg (as above)

# Restore org-rule-guard.py
cp ~/backups/org-rule-guard.py.backup ~/.claude/hooks/org-rule-guard.py

# Restore settings
cp ~/backups/settings.json.backup ~/.claude/settings.json

# Restart Claude Code/Codex CLI
```

### Emergency Rollback

If icg is causing operational issues:

```bash
# Emergency disable icg
ICG_DISABLED=1

# Or remove from hook path temporarily
sudo mv /usr/local/bin/icg /usr/local/bin/icg.disabled

# Restart Claude Code/Codex CLI
# Fix the issue
# Restore icg
sudo mv /usr/local/bin/icg.disabled /usr/local/bin/icg
```

## Coexistence Considerations

### Expected Behavior During Coexistence

**Double Denials**: Operations that violate rules covered by BOTH hooks will be denied twice.

**Example**:
```
User: Write YAML file with "image: foo:latest"
org-rule-guard.py: DENIED (Rule 3: :latest image tags)
icg: DENIED (image-tag pack: :latest image tag)
Result: Operation denied (expected)
```

**Single Denials**: Operations that violate rules covered by only ONE hook will be denied once.

**Example**:
```
User: vault kv destroy secret/app-key
org-rule-guard.py: ALLOW (no vault rule)
icg: DENIED (vault pack: destructive operation)
Result: Operation denied (expected)
```

**Consistent Verdicts**: Both hooks should agree (both deny or both allow). Divergent verdicts indicate a problem.

### Performance Impact

Two hooks add minimal overhead:
- Per-hook overhead: ~1-2ms
- Total overhead: ~2-4ms per operation
- Negligible for interactive use

### Log Volume

Double denials produce more log messages. Configure log aggregation to handle this.

### Monitoring Changes

Monitor denial rates for BOTH systems during coexistence:
- org-rule-guard.py denials should remain stable
- icg denials should be visible for new rules only

## Post-Migration: Next Steps

### Immediate (Day 1)

- [ ] Verify both hooks are firing correctly
- [ ] Check for consistent verdicts
- [ ] Monitor for unexpected denials
- [ ] Train users on new denial messages

### Short-Term (Week 1)

- [ ] Review denial patterns from icg
- [ ] Adjust rule pack if needed (submit issues)
- [ ] Update documentation to reflect coexistence
- [ ] Set up monitoring for icg health

### Medium-Term (Month 1)

- [ ] Evaluate icg's effectiveness
- [ ] Plan migration of remaining rules (if applicable)
- [ ] Consider deprecating org-rule-guard.py for non-kubectl rules
- [ ] Document lessons learned

### Long-Term (Quarter 1)

- [ ] Complete migration of all planned rules
- [ ] Reduce org-rule-guard.py to kubectl-only
- [ ] Archive migration documentation
- [ ] Establish icg-only operational procedures

## Common Migration Issues

### Issue: Hook Not Firing

**Symptom**: icg doesn't seem to be evaluating operations.

**Solution**:
```bash
# Verify hook configuration
cat ~/.claude/settings.json | jq '.hooks.PreToolUse'

# Test hook directly
echo '{"name":"bash","input":{"command":"vault status"}}' | /usr/local/bin/icg hook

# Restart Claude Code/Codex CLI
```

See `troubleshooting.md` for more details.

### Issue: Inconsistent Verdicts

**Symptom**: org-rule-guard.py denies but icg allows (or vice versa).

**Solution**:
```bash
# Identify which rule is diverging
icg status --denials --verbose

# Run smoke test
icg smoke-test-vs-org-rule-guard

# If divergence is on :latest rule, this is expected during migration
# Both hooks deny but with different messages - this is OK

# If divergence is on a rule that should be consistent, file an issue
```

### Issue: Performance Degradation

**Symptom**: Noticeable slowdown in command execution.

**Solution**:
```bash
# Measure overhead
time echo '{"name":"bash","input":{"command":"vault status"}}' | /usr/local/bin/icg hook

# If >10ms, check rule pack complexity
cat /etc/icg/rule-pack.json | jq '.packs | length'

# Consider disabling unused rule packs (if supported)
```

### Issue: Log Volume Increase

**Symptom**: Too many log messages from double denials.

**Solution**:
```bash
# Configure log aggregation to deduplicate
# Or configure icg to log only denials (not allows)
export ICG_LOG_LEVEL=warn
```

## Support and Resources

### Documentation

- **Deployment Guide**: `deployment-guide.md`
- **Troubleshooting Guide**: `troubleshooting.md`
- **Deny Message Guide**: `deny-messages.md`
- **Architecture**: `docs/plan/plan.md`
- **Design Notes**: `docs/notes/`

### Getting Help

Before requesting help, gather:
- icg version: `icg --version`
- org-rule-guard.py version: `head -5 ~/.claude/hooks/org-rule-guard.py`
- Hook configuration: `cat ~/.claude/settings.json`
- Recent denials: `icg status --denials --since 1h`
- Health status: `icg health --verbose`

File issues at: https://github.com/jedarden/irreversible-command-gate/issues

## Success Criteria

Migration is successful when:

1. ✅ Both hooks are configured and firing
2. ✅ icg denies operations covered by its new rules (vault, git, beads)
3. ✅ Verdicts are consistent (both deny or both allow) for shared rules
4. ✅ Performance impact is acceptable (<5ms overhead)
5. ✅ No unexpected operational disruptions
6. ✅ Users are trained on new denial messages
7. ✅ Monitoring is configured for icg health

## Appendix: Configuration Examples

### Example 1: Dual Hook Configuration (Supported Harness)

```json
{
  "hooks": {
    "PreToolUse": {
      "bash": [
        "~/.claude/hooks/org-rule-guard.py",
        "/usr/local/bin/icg hook"
      ],
      "apply_patch": [
        "~/.claude/hooks/org-rule-guard.py",
        "/usr/local/bin/icg hook"
      ],
      "Write": "~/.claude/hooks/org-rule-guard.py",
      "Edit": "~/.claude/hooks/org-rule-guard.py"
    }
  }
}
```

### Example 2: Combined Wrapper Script

```bash
#!/bin/bash
# ~/.claude/hooks/combined-guard.sh

set -euo pipefail

# Run org-rule-guard.py first
~/.claude/hooks/org-rule-guard.py "$@"
ORG_RESULT=$?

# If org-rule-guard.py denied, stop there
if [ $ORG_RESULT -ne 0 ]; then
  exit $ORG_RESULT
fi

# Otherwise, run icg
/usr/local/bin/icg hook "$@"
```

### Example 3: Migration Checklist Template

```markdown
## Migration Checklist for [HOSTNAME]

### Pre-Migration
- [ ] Backup org-rule-guard.py
- [ ] Backup settings.json
- [ ] Verify current hook configuration
- [ ] Document current org-rule-guard.py version

### Installation
- [ ] Download icg binary
- [ ] Install to /usr/local/bin/icg
- [ ] Install rule pack to /etc/icg/
- [ ] Initialize trust pointer
- [ ] Configure hook (dual or combined)

### Verification
- [ ] Run icg health check
- [ ] Run smoke test vs org-rule-guard.py
- [ ] Test new rules (vault, git, beads)
- [ ] Verify consistent verdicts

### Post-Migration
- [ ] Monitor for 24 hours
- [ ] Review denial patterns
- [ ] Update documentation
- [ ] Train users
```

---

**Migration Date**: ___________
**Migrated By**: ___________
**Verified By**: ___________
**Rollback Tested**: [ ] Yes [ ] N/A
**Notes**: _____________________
