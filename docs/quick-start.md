# icg Quick Start Guide

Get started with icg in 5 minutes. This guide covers installation, basic usage, and common tasks for new users.

## What is icg?

**icg (irreversible-command-gate)** is a safety system for AI coding agents that blocks destructive operations before they can cause damage.

- **Protects against**: Vault data destruction, git force-pushes, deleting cluster resources, and more
- **Works with**: Claude Code and Codex CLI (local harnesses only)
- **Philosophy**: Every denial explains what to do instead (not just "blocked")
- **Design**: Fail-open by default — allows operations if rule packs aren't loaded or the tool isn't recognized

---

## Installation (2 minutes)

### Option 1: Download Binary (Recommended)

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
```

### Option 2: Build from Source

```bash
# Clone repository
git clone https://github.com/jedarden/irreversible-command-gate.git
cd irreversible-command-gate

# Build
cargo build --release

# Install
sudo cp target/release/icg /usr/local/bin/
sudo chmod +x /usr/local/bin/icg

# Verify
icg --version
```

---

## Initial Setup (3 minutes)

### Step 1: Install Rule Packs

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

# Verify rule packs are loaded
icg coverage --list
```

### Step 2: Configure Claude Code Hook

```bash
# Create Claude Code config directory
mkdir -p ~/.config/claude-code

# Configure hook
cat > ~/.config/claude-code/settings.json <<'EOF'
{
  "hooks": {
    "PreToolUse": {
      "command": "/usr/local/bin/icg",
      "args": ["hook"]
    }
  }
}
EOF

# Note: The hook reads PreToolUse JSON from stdin and responds with
# the decision envelope. No additional flags are needed.
```

### Step 3: Test Installation

```bash
# Test a dangerous command (should be denied)
echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  icg check --stdin

# Expected output:
DENIED: vault kv destroy is permanently destructive and cannot be undo
Pack: vault
Pattern: vault-kv-destroy
```

**How the test works**:
1. `icg check --stdin` reads PreToolUse JSON from stdin
2. Extracts the Bash command from `toolInput.command`
3. Evaluates it against rule pack patterns
4. Returns the denial decision with pack/pattern details

---

## Basic Usage

### Checking Commands

Test if a command would be allowed or denied:

```bash
# Check a specific command
icg check --command "git push --force origin main"

# Output:
DENIED: git push --force would rewrite public history
Pack: git
Pattern: git-force-push

# Check a safe command
icg check --command "git status"

# Output:
ALLOW: no configured rule matched
```

### Understanding Denials

Learn why a command was denied:

```bash
# Get detailed explanation
icg explain --pattern vault-kv-destroy

# Output:
Pattern: vault-kv-destroy
Pack: vault
Enabled: true
Tier: Tier1
Severity: Critical
Why: vault kv destroy is permanently destructive and cannot be undo
Redirect channel: Alternative
Alternative: Use vault kv patch to reconcile secrets without destroying versions
```

### Viewing Rule Pack Coverage

See what operations are protected:

```bash
# List all loaded rule packs
icg coverage --list

# Output:
✓ vault (8 patterns)
✓ git (12 patterns)
✓ image-tag (6 patterns)
```

---

## What Gets Protected

### Critical Operations (Always Blocked)

- **Vault**: `vault kv destroy`, `vault policy delete`
- **Git**: `git push --force`, `git push -f`, `git rebase` (on protected branches)
- **Kubernetes**: `kubectl delete pvc` (persistent data)
- **Images**: Using `:latest` tags in YAML files
- **Storage**: Using `ssd` storage class on Rackspace Spot

### Safe Operations (Always Allowed)

- **Vault**: `vault kv get`, `vault kv list`
- **Git**: `git status`, `git log`, `git diff`
- **Kubernetes**: `kubectl get`, `kubectl describe`, `kubectl logs`
- **Images**: Using semantic version tags (`v1.2.3`)
- **Storage**: Using `sata` or `sata-large`

---

## Common Tasks

### Task 1: Handling a Denied Command

When a command is denied, follow these steps:

1. **Read the denial message**
   ```
   DENIED: vault kv destroy is permanently destructive and cannot be undo
   Pack: vault
   Pattern: vault-kv-destroy
   ```

2. **Get more details**
   ```bash
   icg explain --pattern vault-kv-destroy --show-redirect
   ```

3. **Use the suggested alternative**
   ```bash
   vault kv patch secret/app/api-key -remove=expired_field
   ```

4. **Verify the fix**
   ```bash
   vault kv get secret/app/api-key
   ```

### Task 2: Checking Rule Pack Coverage

See what operations are protected:

```bash
# List all rule packs
icg coverage --list

# Check specific pack directory
icg coverage --list --pack /etc/icg/packs
```

### Task 3: Health Check

Verify icg is working correctly:

```bash
# Check health status
icg health status

# Output:
# Guard Health Status
#
# **Path:** /var/cache/icg/health-state.json
#
# ## Health Status
# **Status:** Running
# **Running:** true
# **Healthy:** true
# **Stable:** true
```

### Task 4: Viewing Denial Logs

See what operations have been blocked:

```bash
# Denials are logged to the state store
# View recent denials (if state store exists)
cat /var/lib/icg/state.json | jq '.denials[] | {timestamp, pack_id, pattern_id, reason}'
```

---

## Integration with AI Harnesses

### Claude Code Integration

Claude Code invokes icg through the PreToolUse hook system. The hook protocol:

1. **Request Format** (JSON from stdin):
```json
{
  "toolName": "Bash",
  "toolInput": {
    "command": "vault kv destroy secret/test"
  },
  "toolUseId": "toolu_0123456789"
}
```

2. **Response Format** (JSON to stdout):
```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "vault kv destroy is permanently destructive [pack=vault, pattern=vault-kv-destroy]"
  }
}
```

3. **Decision Types**:
   - `allow` — Operation permitted
   - `deny` — Operation blocked
   - `updatedInput` — Safe alternative provided
   - `additionalContext` — Warning, operation allowed

### Codex CLI Integration

Codex CLI uses the same PreToolUse hook protocol. Configure it in your Codex settings:

```json
{
  "hooks": {
    "PreToolUse": {
      "command": "/usr/local/bin/icg",
      "args": ["hook"]
    }
  }
}
```

**Important**: icg only protects **local** harnesses (Claude Code CLI, Codex CLI). Cloud-hosted sessions (ChatGPT web, Claude.ai) are not covered.

### Hook Mode vs Check Mode

- **Hook mode** (`icg hook`): Used by AI harnesses, reads PreToolUse JSON from stdin
- **Check mode** (`icg check`): Manual testing, accepts `--stdin`, `--command`, or `--file`

Both evaluate the same rule packs, but hook mode returns the harness-specific response envelope.

---

## Example Workflows

### Workflow 1: Daily Development

```bash
# Morning: Verify rule packs are loaded
icg coverage --list

# During work: Test commands before running
icg check --command "kubectl delete pvc data-volume"

# If denied: Get details
icg explain --pattern kubectl-delete-pvc --show-redirect
```

### Workflow 2: Handling a Denial

```bash
# Step 1: See the denial
# Agent output: DENIED: git push --force would rewrite public history

# Step 2: Understand why
icg explain --pattern git-force-push --show-redirect

# Step 3: Use the alternative
git merge origin/main
git push origin main
```

### Workflow 3: Emergency Bypass

```bash
# ONLY for genuine emergencies:
ICG_DISABLED=1 <dangerous-command>

# Follow up: Document why
echo "$(date): Emergency bypass for <reason>" >> /var/log/icg/emergency.log
```

---

## Troubleshooting

### Problem: icg Doesn't Seem to Be Running

**Solution**: Check if the hook is configured

```bash
# Verify Claude Code config
cat ~/.config/claude-code/settings.json

# Test icg directly
icg check --command "echo test"
```

### Problem: A Command Was Wrongly Denied

**Solution**: Check the pattern explanation

```bash
icg explain --pattern <pattern-id> --show-redirect
```

If it's a false positive, file an issue:

```bash
gh issue create \
  --title "False positive: <pattern-id>" \
  --body "Command was: <command>" \
  --repo jedarden/irreversible-command-gate
```

### Problem: Rule Packs Not Loading

**Solution**: Check pack directory

```bash
# Verify packs exist
ls -la /etc/icg/packs/

# Test loading
icg coverage --list --pack /etc/icg/packs
```

---

## Next Steps

### Learn More

- **Full Operator Guide**: `docs/operators/README.md` — Comprehensive operations manual
- **Training Manual**: `docs/operators/training-manual.md` — 8-hour learning path
- **Examples**: `docs/examples/README.md` — Real-world scenarios
- **Onboarding Guide**: `docs/onboarding-guide.md` — Structured learning path

### Advanced Setup

- **Multi-harness support**: Protect both Claude Code and Codex CLI
- **Repository overrides**: Allow exceptions for specific repos (see `icg override`)
- **Custom rule packs**: Create rules for your tools (see `icg new-pack`)
- **Trust pointer management**: Track trusted rule pack versions (see `icg trust`)

### Contribute

- **Report issues**: https://github.com/jedarden/irreversible-command-gate/issues
- **Contributing guide**: See `docs/developers/README.md`
- **Rule pack authoring**: See `docs/developers/rule-pack-best-practices.md`

---

## Quick Reference Card

### Essential Commands

```bash
icg --version                    # Show version
icg coverage --list              # List rule packs
icg check --command "<cmd>"     # Test a command
icg check --stdin                # Test via PreToolUse JSON
icg explain --pattern <id>      # Explain a pattern
icg health status                # Check health
```

### Denial Response

When you see a denial:

1. **Read the message** — It explains why and what to do
2. **Use the alternative** — Follow the redirect suggestion
3. **Ask for help** — If unsure, check docs or file an issue

### Response Types

- **ALLOW** — Operation permitted
- **DENIED** — Operation blocked (critical/high severity)
- **REWRITE** — Safe alternative provided
- **WARNING** — Allowed with context (tier 3 patterns)

### Emergency Disable

```bash
# Last resort for genuine emergencies
ICG_DISABLED=1 <command>
```

Document why you needed this and follow up with a review.

---

## Support

### Getting Help

- **Documentation**: `docs/` directory
- **Issues**: https://github.com/jedarden/irreversible-command-gate/issues
- **Training Manual**: `docs/operators/training-manual.md`

### Before Asking

1. Check the denial message explanation
2. Review the operator documentation
3. Search existing GitHub issues
4. Gather information: version, OS, exact command

---

**Quick Start Guide Version**: 2.0
**Last Updated**: 2026-08-17
**For**: icg v0.1.0+

Welcome to icg! You're now protected against destructive operations. If you have questions, start with the documentation or file an issue.

---

## Common Tasks

### Task 1: Fixing a Denied Command

When a command is denied, follow these steps:

1. **Read the denial message**
   ```
   DENIED: vault kv destroy is permanently destructive
   Redirect: Use 'vault kv patch' to reconcile
   ```

2. **Use the suggested alternative**
   ```bash
   vault kv patch secret/app/api-key -remove=expired_field
   ```

3. **Verify the fix**
   ```bash
   vault kv get secret/app/api-key
   ```

### Task 2: Checking Rule Pack Coverage

See what operations are protected:

```bash
# List all rule packs
icg coverage --list

# Output:
# ✓ vault (8 patterns)
# ✓ git (12 patterns)
# ✓ image-tag (6 patterns)
# ✓ storage-class (4 patterns)
# ✓ beads (2 patterns)
```

### Task 3: Updating Rule Packs

Check for and apply updates:

```bash
# Check for updates
icg update --check-only

# Apply updates (when ready)
icg update
```

### Task 4: Health Check

Verify icg is working correctly:

```bash
# Run health check
icg health --verbose

# Output:
# ✓ icg binary: /usr/local/bin/icg v0.1.0
# ✓ Rule packs: 5 packs loaded
# ✓ Claude Code hook: Configured
# ✓ State store: /var/lib/icg/state.db
```

---

## What Gets Protected

### Critical Operations (Always Blocked)

- **Vault**: `vault kv destroy`, `vault policy delete`
- **Git**: `git push --force`, `git push -f`
- **Kubernetes**: `kubectl delete pvc` (persistent data)
- **Images**: Using `:latest` tags
- **Storage**: Using `ssd` storage class on Rackspace Spot

### Safe Operations (Always Allowed)

- **Vault**: `vault kv get`, `vault kv list`
- **Git**: `git status`, `git log`, `git diff`
- **Kubernetes**: `kubectl get`, `kubectl describe`, `kubectl logs`
- **Images**: Using semantic version tags (`v1.2.3`)
- **Storage**: Using `sata` or `sata-large`

---

## Example Workflows

### Workflow 1: Daily Development

```bash
# Morning: Check health
icg health

# During work: View denials periodically
icg status --denials --since 1h

# Evening: Review patterns
icg status --denials --pattern-summary --since 1d
```

### Workflow 2: Handling a Denial

```bash
# Step 1: See the denial
# Agent output: DENIED: git push --force would rewrite public history

# Step 2: Understand why
icg explain --pattern git-force-push

# Step 3: Use the alternative
git merge origin/main
git push origin main
```

### Workflow 3: Emergency Bypass

```bash
# ONLY for genuine emergencies:
ICG_DISABLED=1 <dangerous-command>

# Follow up: File incident report
icg export-denial <telemetry-id> > incident.txt
```

---

## Troubleshooting

### Problem: icg Doesn't Seem to Be Running

**Solution**: Check if the hook is configured

```bash
icg health --check-hooks
```

If not configured, see "Initial Setup" above.

### Problem: A Command Was Wrongly Denied

**Solution**: Check the pattern explanation

```bash
icg explain --pattern <pattern-id>
```

If it's a false positive, file an issue:

```bash
gh issue create \
  --title "False positive: <pattern-id>" \
  --body "Command was: <command>" \
  --repo jedarden/irreversible-command-gate
```

### Problem: Can't Update Rule Packs

**Solution**: Check permissions

```bash
# Rule pack directory should be writable
sudo ls -la /etc/icg/packs

# Fix permissions if needed
sudo chown -R $USER:$USER /etc/icg/packs
```

---

## Next Steps

### Learn More

- **Full Operator Guide**: `docs/operators/README.md` - Comprehensive operations manual
- **Denial Messages**: `docs/operators/deny-messages.md` - Complete denial reference
- **Examples**: `docs/examples/README.md` - Real-world scenarios

### Advanced Setup

- **Multi-harness support**: Protect both Claude Code and Codex CLI
- **Repository overrides**: Allow exceptions for specific repos
- **Custom rule packs**: Create rules for your tools

### Contribute

- **Report issues**: https://github.com/jedarden/irreversible-command-gate/issues
- **Contributing guide**: See `docs/developers/README.md`
- **Rule pack authoring**: See `docs/developers/rule-pack-best-practices.md`

---

## Quick Reference Card

### Common Commands

```bash
icg --version                    # Show version
icg health                      # Check health
icg check --command "<cmd>"     # Test a command
icg status --denials            # View recent denials
icg explain --pattern <id>      # Explain a pattern
icg update --check-only         # Check for updates
```

### Denial Response

When you see a denial:

1. **Read the message** - It explains why and what to do
2. **Follow the redirect** - Use the suggested alternative
3. **Ask for help** - If unsure, check docs or file an issue

### Emergency Disable

```bash
# Last resort for genuine emergencies
ICG_DISABLED=1 <command>
```

Document why you needed this and follow up with a review.

---

## Support

### Getting Help

- **Documentation**: `docs/` directory
- **Issues**: https://github.com/jedarden/irreversible-command-gate/issues
- **Operator Guide**: `docs/operators/README.md`

### Before Asking

1. Check the denial message explanation
2. Review the operator documentation
3. Search existing GitHub issues
4. Gather information: version, OS, exact command

---

**Quick Start Guide Version**: 1.0
**Last Updated**: 2026-08-16
**For**: icg v0.1.0+

Welcome to icg! You're now protected against destructive operations. If you have questions, start with the documentation or file an issue.
