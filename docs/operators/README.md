# Irreversible Command Gate (icg) - Operator Documentation

Welcome to the operator documentation for the Irreversible Command Gate (icg). This documentation is designed for operators who will deploy, maintain, and troubleshoot icg in production environments.

## Quick Start

1. **New Installation**: Start with [Deployment Guide](deployment-guide.md)
2. **Migrating from org-rule-guard.py**: See [Migration Guide](migration-from-org-rule-guard.md)
3. **Troubleshooting Issues**: See [Troubleshooting Guide](troubleshooting.md)
4. **Understanding Denials**: See [Deny Message Interpretation](deny-messages.md)

## Documentation Index

### Deployment Guide (`deployment-guide.md`)

**Who should read**: Operators installing icg for the first time

**Covers**:
- System requirements and dependencies
- Step-by-step installation procedures
- Configuration options and environment variables
- Hook integration (Claude Code and Codex CLI)
- PATH-wrapper setup
- Deployment patterns (single host, fleet-wide)
- Maintenance procedures
- Backup and recovery
- Security considerations
- Performance optimization

**Time to read**: 30 minutes
**Time to complete installation**: 15 minutes

---

### Troubleshooting Guide (`troubleshooting.md`)

**Who should read**: Operators experiencing issues with icg

**Covers**:
- Quick reference for common issues
- Diagnosis procedures
- False positives and rule conflicts
- Installation problems
- Hook and PATH-wrapper issues
- State store corruption
- Performance issues
- Debugging procedures
- Emergency rollback procedures
- Prevention and monitoring

**Time to read**: 20 minutes
**Use as reference**: Look up specific issues as they occur

---

### Migration Guide (`migration-from-org-rule-guard.md`)

**Who should read**: Operators currently using `org-rule-guard.py`

**Covers**:
- What's changing from org-rule-guard.py to icg
- Pre-migration checklist
- Step-by-step migration process
- Coexistence configuration (both hooks running together)
- Verification procedures
- Rollback procedures
- Common migration issues
- Success criteria

**Time to read**: 25 minutes
**Time to complete migration**: 30 minutes

---

### Deny Message Interpretation (`deny-messages.md`)

**Who should read**: All operators and users working with icg

**Covers**:
- Denial message structure and format
- Rule pack specific denials (vault, git, image-tag, etc.)
- Corrective actions for each denial type
- General workflow for handling denials
- Escalation procedures for false positives
- Emergency operations
- Common denial scenarios
- Training guidance for users
- Quick reference card

**Time to read**: 35 minutes
**Use as reference**: Look up specific denial types as they occur

---

## Additional Resources

### Architecture and Design

- **Plan**: `../plan/plan.md` - Complete application architecture and implementation phases
- **Design Notes**: `../notes/` - Individual design decisions and rationale
- **Fail-Closed Transition**: `../design/fail-closed-transition.md` - Detailed fail-closed policy design

### Developer Documentation

- **README**: `../../README.md` - Project overview and developer introduction
- **Source Code**: `../../src/` - Implementation details

### External References

- **GitHub Issues**: https://github.com/jedarden/irreversible-command-gate/issues
- **Existing Infrastructure Analysis**: `../notes/existing-enforcement-infrastructure.md`

## Reading Paths

### For New Installations

1. Read this README
2. Read [Deployment Guide](deployment-guide.md)
3. Perform installation
4. Read [Deny Message Interpretation](deny-messages.md) (for reference)
5. Keep [Troubleshooting Guide](troubleshooting.md) bookmarked

### For Migrations from org-rule-guard.py

1. Read this README
2. Read [Migration Guide](migration-from-org-rule-guard.md)
3. Perform migration
4. Read [Deny Message Interpretation](deny-messages.md) (for reference)
5. Keep [Troubleshooting Guide](troubleshooting.md) bookmarked

### For Troubleshooting

1. Go directly to [Troubleshooting Guide](troubleshooting.md)
2. Look up your specific issue
3. Follow diagnostic procedures
4. If needed, consult [Deployment Guide](deployment-guide.md) for configuration details

### For Understanding Denials

1. Go directly to [Deny Message Interpretation](deny-messages.md)
2. Look up your specific denial type
3. Follow corrective action
4. If needed, file an issue or request override

## Key Concepts

### What icg Does

icg is a safety system that intercepts commands before they execute and blocks operations that could cause irreversible or hard-to-reverse damage. It provides:

- **Protection**: Blocks destructive operations (vault destroy, git force-push, etc.)
- **Guidance**: Every denial explains why and what to do instead
- **Visibility**: Logs all denials for auditing and trend analysis

### What icg Does NOT Do

- **Not adversarial protection**: icg protects against honest mistakes, not malicious actors
- **Not Codex cloud coverage**: Only local Claude Code and Codex CLI are covered
- **Not prompt injection defense**: Does not protect against malicious repositories
- **Not complete kubectl coverage**: Mutating kubectl remains with org-rule-guard.py

### Architecture Overview

icg has two front-ends:

1. **PATH-Wrapper**: Shadows binaries via symlinks (e.g., `vault`, `git`)
2. **Native Hooks**: Integrates with Claude Code and Codex CLI hook systems

Both front-ends share the same evaluation engine and rule packs.

### Rule Packs

Rules are organized into per-tool packs:
- `vault` - Vault/OpenBao destructive operations
- `git` - Force-push, stale-HEAD, commit-without-pathspec
- `image-tag` - `:latest` and bare SHA tags
- `storage-class` - `ssd`/`ssd-large` on Rackspace Spot
- `beads` - `.beads/` protection in shared checkouts
- `secrets` - Credential values in Bash commands
- `misc` - Deprecated tools and other misc rules
- `tmux` - Bare NATO session targeting

### Fail-Open vs Fail-Closed

icg starts in **fail-open mode** (allows operations if guard crashes). After 90+ days of crash-free production operation, it can graduate to **fail-closed mode** (denies operations if guard crashes). See `../design/fail-closed-transition.md` for details.

## Common Tasks

### Checking Health

```bash
icg health
```

### Viewing Recent Denials

```bash
icg status --denials --since 1h
```

### Updating Rule Pack

```bash
icg update --check-only  # Check for updates
icg update               # Apply updates
```

### Testing a Command

```bash
icg check --command "vault kv destroy secret/test"
```

### Running Diagnostics

```bash
icg health --verbose
icg status --denials --pattern-summary
```

## Support

### Before Requesting Help

Gather the following information:

```bash
# Version information
icg --version

# Health status
icg health --verbose > icg-health.txt

# Recent denials
icg status --denials --since 1h --format json > denials.json

# Configuration
icg config --show > icg-config.txt
```

### File Issues

File issues at: https://github.com/jedarden/irreversible-command-gate/issues

Include:
- icg version
- Rule pack version
- OS and kernel version
- Exact command that was denied
- Full denial message
- Health and configuration exports

### Emergency Contacts

For fleet-wide emergencies or critical incidents:
- Consult your organization's incident response procedures
- Use emergency rollback procedures from [Troubleshooting Guide](troubleshooting.md)

## Best Practices

### Installation

- ✅ Install to root-owned directories (`/usr/local/bin`, `/etc/icg`)
- ✅ Verify installation with `icg health`
- ✅ Test hook integration before relying on it
- ✅ Keep backups of known-good rule packs

### Operation

- ✅ Monitor denial rates for anomalies
- ✅ Review denial patterns weekly
- ✅ Keep rule packs updated
- ✅ Document repository overrides
- ✅ Train users on denial messages

### Maintenance

- ✅ Run health checks weekly
- ✅ Review denial patterns monthly
- ✅ Test rollback procedures quarterly
- ✅ Update documentation after changes

### Security

- ✅ Never manually edit `/etc/icg/trust-pointer.json`
- ✅ Keep backups of rule packs
- ✅ Monitor for unauthorized rule pack changes
- ✅ Use repository overrides sparingly
- ✅ Follow release pipeline for rule changes

## FAQ

### Q: Can I disable icg for a specific command?

**A**: Yes, but only for genuine emergencies:
```bash
ICG_DISABLED=1 <command>
```
Document why you needed to disable the guard and follow up with a review.

### Q: Why do I get double denials during migration?

**A**: This is expected. Both org-rule-guard.py and icg may deny the same operation during coexistence. They're both working correctly. See [Migration Guide](migration-from-org-rule-guard.md).

### Q: How do I request a repository override?

**A**: Use the override command (requires Layer 1/2 approval):
```bash
icg override create --repo /path/to/repo \
  --pattern-id "<pattern-id>" \
  --justification "<explanation>"
```

### Q: What's the difference between fail-open and fail-closed?

**A**: Fail-open allows operations if the guard crashes. Fail-closed denies operations if the guard crashes. icg starts in fail-open mode and can graduate to fail-closed after reliability validation. See `../design/fail-closed-transition.md`.

### Q: Can icg protect against malicious agents?

**A**: No. icg protects against honest mistakes, not adversarial actors. A truly malicious agent could bypass icg or attack its deployment location. See `../../README.md` "What this does not do".

### Q: Why doesn't icg cover all kubectl mutations?

**A**: Accurately distinguishing ArgoCD-managed resources requires live cluster state, which would break icg's zero-I/O determinism. The blanket kubectl mutation block remains with org-rule-guard.py.

### Q: How often should I update rule packs?

**A**: Check weekly, apply when:
- New rules are needed
- False positives are fixed
- Security issues are addressed

Updates are user-triggered, not automatic, so you control when they apply.

### Q: What happens if icg crashes?

**A**: In fail-open mode (default), operations are allowed. In fail-closed mode, operations are denied. You'll see a guard-crash denial message and should investigate the crash.

## Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0 | 2026-08-16 | Initial operator documentation |

---

**Documentation Version**: 1.0
**Last Updated**: 2026-08-16
**For**: icg v0.1.0+
