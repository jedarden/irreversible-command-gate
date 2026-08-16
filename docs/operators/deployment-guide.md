# Irreversible Command Gate (icg) - Operator Guide

## Overview

The Irreversible Command Gate (icg) is a safety system that intercepts commands before they execute and blocks operations that could cause irreversible or hard-to-reverse damage. This guide is for operators who will deploy, maintain, and troubleshoot icg in production environments.

## What icg Protects Against

icg guards against operations that can cause irreversible damage, including:

- **Vault/OpenBao destructive operations**: `vault kv destroy`, `vault policy delete`, token revocation
- **Git force-pushes**: Accidental repository history destruction
- **Unsafe image tags**: Using `:latest` or bare git SHAs for container images
- **Improper storage classes**: Using `ssd`/`ssd-large` on Rackspace Spot
- **Bead state corruption**: Hand-editing `.beads/` directories in shared checkouts
- **Deprecated tool usage**: Invoking deprecated bead CLIs (`br`, `bf`)
- **Credential leakage**: Writing credential values to commits or files

## Architecture

### Front-Ends

icg provides two independent, complementary front-ends:

1. **PATH-Wrapper Binary**: Shadows guarded binaries (`vault`, `git`, etc.) via symlinks earlier in `$PATH` than the real binaries
2. **Native PreToolUse Hooks**: Integrates directly with Claude Code and Codex CLI hook systems

### Components

- **Engine**: Evaluation logic that runs rule packs against commands
- **Rule Packs**: Modular, per-tool domain rules (`vault`, `git`, `image-tag`, etc.)
- **State Store**: Minimal persistent state for cross-invocation rules
- **Trust Pointer**: Tracks the current trusted rule pack release
- **Self-Updater**: User-triggered update mechanism (no automatic polling)

## System Requirements

### Supported Platforms

- Linux (kernel 2.6.32+)
- x86_64 and ARM64 architectures

### Dependencies

- **Runtime**: None (static binary)
- **Hook Integration**: Claude Code 1.0+ or Codex CLI with hook support
- **Network**: GitHub Releases API access (for updates only, not for command evaluation)

### Permissions

- Binary installation: `root` access for `/usr/local/bin/icg`
- Rule pack installation: `root` access for `/etc/icg/`
- Hook configuration: User-level `~/.claude/settings.json` or `~/.codex/hooks.json` modification

## Installation

### Step 1: Download and Install Binary

```bash
# Download latest release from GitHub
wget https://github.com/jedarden/irreversible-command-gate/releases/latest/download/icg-linux-x86_64.tar.gz

# Extract and install
tar -xzf icg-linux-x86_64.tar.gz
sudo cp icg /usr/local/bin/icg
sudo chmod 0755 /usr/local/bin/icg
sudo chown root:root /usr/local/bin/icg

# Verify installation
icg --version
```

### Step 2: Install Rule Pack

```bash
# Create rule pack directory
sudo mkdir -p /etc/icg

# Download latest rule pack
sudo wget -O /etc/icg/rule-pack.json https://github.com/jedarden/irreversible-command-gate/releases/latest/download/rule-pack.json

# Set permissions
sudo chmod 0644 /etc/icg/rule-pack.json
sudo chown root:root /etc/icg/rule-pack.json
```

### Step 3: Initialize Trust Pointer

```bash
# Create trust pointer pointing to current release
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

### Step 4: Configure Hook Integration

#### For Claude Code

```bash
# Edit Claude Code settings
cat >> ~/.claude/settings.json <<EOF

{
  "hooks": {
    "PreToolUse": {
      "bash": "/usr/local/bin/icg hook",
      "apply_patch": "/usr/local/bin/icg hook"
    }
  }
}
EOF
```

#### For Codex CLI

```bash
# Create Codex hooks configuration
mkdir -p ~/.codex
cat > ~/.codex/hooks.json <<EOF
{
  "PreToolUse": {
    "bash": "/usr/local/bin/icg hook",
    "apply_patch": "/usr/local/bin/icg hook"
  }
}
EOF
```

### Step 5: Set Up PATH-Wrapper Symlinks (Optional)

```bash
# Create wrapper directory
sudo mkdir -p /usr/local/libexec/icg-wrappers

# Add to PATH before system binaries
# Add to /etc/environment or ~/.bashrc:
export PATH="/usr/local/libexec/icg-wrappers:$PATH"

# Create symlinks for guarded binaries
sudo ln -sf /usr/local/bin/icg /usr/local/libexec/icg-wrappers/vault
sudo ln -sf /usr/local/bin/icg /usr/local/libexec/icg-wrappers/git
sudo ln -sf /usr/local/bin/icg /usr/local/libexec/icg-wrappers/bao
```

### Step 6: Verify Installation

```bash
# Test basic functionality
icg check --command "vault status"

# Test hook integration (in a Claude Code or Codex session)
# Try a denied operation:
#vault kv destroy secret/test
```

## Configuration

### Environment Variables

| Variable | Purpose | Default | Notes |
|----------|---------|---------|-------|
| `ICG_FAIL_CLOSED` | Enable fail-closed mode | `false` | Only enable after reliability validation |
| `ICG_RULE_PACK_PATH` | Override rule pack location | `/etc/icg/rule-pack.json` | For testing only |
| `ICG_STATE_PATH` | Override state store location | `/var/lib/icg/state.json` | For testing only |
| `ICG_LOG_LEVEL` | Logging verbosity | `info` | Options: `error`, `warn`, `info`, `debug`, `trace` |

### Fail-Closed Mode

**WARNING**: Only enable after 90+ days of crash-free production operation.

```bash
# Enable fleet-wide
sudo tee /etc/icg/fail-closed.conf > /dev/null <<EOF
ICG_FAIL_CLOSED=true
EOF

# Enable per-session
export ICG_FAIL_CLOSED=true
```

See `fail-closed-transition.md` for graduation criteria and procedures.

### Rule Pack Overrides

For repository-specific overrides (e.g., allowing a pattern in one repo):

```bash
# Create override file (requires Layer 1/2 approval via release pipeline)
icg override create --repo /path/to/repo \
  --pattern-id "vault-destructive" \
  --justification "Legacy vault migration in progress"
```

## Maintenance Procedures

### Daily Operations

```bash
# Check guard health
icg status

# View recent denials
icg status --denials --since 24h

# Check for updates
icg update --check-only
```

### Updating Rule Packs

```bash
# Check for available updates
icg update --check-only

# Preview what would change
icg update --dry-run

# Apply update
icg update

# Verify after update
icg regression-suite verify /etc/icg/rule-pack.json
```

### Monitoring

#### Key Metrics

- **Guard uptime**: Time since last guard crash/restart
- **Denial rate**: Percentage of operations denied (alert if >1%)
- **Rule pack version**: Currently active release
- **Health status**: Overall system health

#### Health Checks

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

### Log Analysis

icg logs to stderr by default. Configure log aggregation:

```bash
# Journal-based logging (systemd)
journalctl -u icg -f

# File-based logging
icg check --command "test" 2> /var/log/icg.log
```

## Deployment Patterns

### Single Host Deployment

Simplest pattern for individual workstations:

```bash
# Install as above
# No additional coordination needed
```

### Fleet Deployment

For coordinated fleet rollouts:

#### 1. Canary Deployment

```bash
# Deploy to canary subset first
export ICG_CANARY_ID="workstation-001"
icg update --canary

# Monitor for 7 days
icg status --health --since 7d

# Rollout to rest of fleet if successful
icg update --fleet-wide
```

#### 2. NEEDLE Worker Integration

For NEEDLE fleet deployments:

```bash
# Set worker identifier
export NEEDLE_WORKER_ID="$(hostname)"

# Deploy with canary identifier
icg update --identifier "$NEEDLE_WORKER_ID"
```

#### 3. Coordinated Rollout

```bash
# On orchestration host
for host in workstation-001 workstation-002 workstation-003; do
  ssh $host "icg update && icg status"
done
```

## Troubleshooting Installation

### Installation Fails

**Symptom**: Permission denied during installation

**Solution**:
```bash
# Ensure running with sudo
sudo -i

# Verify directories exist
sudo mkdir -p /usr/local/bin /etc/icg

# Check disk space
df -h /usr/local /etc
```

### Hook Not Triggering

**Symptom**: Commands execute without guard evaluation

**Solution**:
```bash
# Verify hook configuration
cat ~/.claude/settings.json | grep -A5 hooks
cat ~/.codex/hooks.json

# Test hook directly
echo '{"name":"bash","input":{"command":"vault kv destroy secret/test"}}' | icg hook

# Check binary is executable
which icg
ls -la /usr/local/bin/icg
```

### PATH-Wrapper Not Working

**Symptom**: Real binary executes instead of wrapper

**Solution**:
```bash
# Check symlink exists
ls -la /usr/local/libexec/icg-wrappers/git

# Verify PATH order
echo $PATH | tr ':' '\n' | grep icg-wrappers

# Test wrapper directly
/usr/local/libexec/icg-wrappers/vault version
```

## Backup and Recovery

### Backup Configuration

```bash
# Backup rule pack and trust pointer
sudo tar -czf /backups/icg-config-$(date +%F).tar.gz /etc/icg/

# Backup state (if using Tier 2 rules)
sudo cp /var/lib/icg/state.json /backups/icg-state-$(date +%F).json
```

### Recovery Procedures

#### Restore from Backup

```bash
# Stop any active sessions
# Extract backup
sudo tar -xzf /backups/icg-config-2026-08-16.tar.gz -C /

# Verify restoration
icg status
icg health
```

#### Emergency Rollback

```bash
# Rollback to previous trusted release
icg update --rollback-to v0.0.9

# If rule pack is corrupted
sudo wget -O /etc/icg/rule-pack.json \
  https://github.com/jedarden/irreversible-command-gate/releases/download/v0.0.9/rule-pack.json
```

## Security Considerations

### Deployment Security

- **Binary ownership**: Must be `root:root` to prevent agent modification
- **Rule pack ownership**: Must be `root:root` to prevent agent modification
- **Trust pointer**: Only updated via release pipeline, not manual edits
- **State directory**: Should be root-owned and mode `0700`

### Update Security

- Updates are user-triggered, not automatic
- Only adopts releases verified by release-integrity pipeline
- Trust pointer ensures untrusted releases can't be loaded
- Poison-pill mechanism auto-rolls back bad releases

### Operational Security

- Monitor denial rate for anomalies (may indicate compromised rule pack)
- Review trust pointer changes in audit logs
- Never manually edit `/etc/icg/trust-pointer.json` (use `icg update`)
- Keep backup of known-good rule pack releases

## Performance Considerations

### Overhead

- **Per-command overhead**: ~1-5ms for typical command evaluation
- **PATH-wrapper overhead**: ~1ms additional process spawn time
- **Memory footprint**: ~10-20MB resident set (no daemon)

### Optimization

```bash
# Use PATH-wrapper for frequently-used binaries (faster than hook for raw commands)
# Disable debug logging in production
export ICG_LOG_LEVEL=error

# Use state store only when Tier 2 rules are needed
```

## Compliance and Auditing

### Audit Log

```bash
# View denial history
icg audit --since 30d

# Export for compliance
icg audit --since 90d --format json > /compliance/icg-audit-90d.json
```

### Compliance Reports

```bash
# Generate compliance summary
icg compliance-report --period 90d
```

## Next Steps

After completing installation:

1. Read `troubleshooting.md` for common issues and solutions
2. Review `deny-messages.md` to understand how to interpret and respond to denials
3. If migrating from `org-rule-guard.py`, see `migration-from-org-rule-guard.md`
4. Configure monitoring and alerting for guard health
5. Set up regular rule pack updates (weekly or bi-weekly recommended)

## Support and Resources

- **Issues**: https://github.com/jedarden/irreversible-command-gate/issues
- **Documentation**: `docs/` directory in repository
- **Architecture**: See `docs/plan/plan.md` for detailed design
- **Design Notes**: See `docs/notes/` for individual design decisions
