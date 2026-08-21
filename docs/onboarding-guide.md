# icg Onboarding Guide

Welcome to the irreversible command gate (icg)! This guide provides a structured path for learning how to use, operate, and extend icg. Whether you're an operator protecting production systems or a developer adding new protections, this guide will help you get started quickly and effectively.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Learning Path](#learning-path)
3. [Role-Specific Tracks](#role-specific-tracks)
4. [Quick Reference](#quick-reference)
5. [Common Tasks](#common-tasks)
6. [Getting Help](#getting-help)

---

## Getting Started

### What is icg?

**icg (irreversible-command-gate)** is a safety system for AI coding agents that intercepts commands before they execute and blocks operations that cause irreversible damage.

**Problem it solves**: AI agents have the power to execute destructive operations (delete secrets, force-push git, destroy data). Without protection, these operations can cause data loss, infrastructure damage, and security incidents.

**How it works**: icg sits between the AI agent and command execution, evaluates each operation against safety rules, and blocks dangerous operations while explaining safe alternatives.

**Key principle**: Redirect-not-just-block — every denial explains what to do instead, not just why something was blocked.

### Installation (5 minutes)

If you haven't installed icg yet, follow the Quick Start Guide:

```bash
# Download and install
wget https://github.com/jedarden/irreversible-command-gate/releases/download/v0.1.0/icg-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
tar -xzf icg-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
sudo cp icg /usr/local/bin/
sudo chmod +x /usr/local/bin/icg

# Verify
icg --version
```

Full installation instructions: **[Quick Start Guide](quick-start.md)**

### First Test (2 minutes)

Verify icg is working:

```bash
# Test a dangerous command (should be denied)
echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  icg check --stdin

# Expected: DENIED with explanation
```

---

## Learning Path

This is the recommended sequence for learning icg. Follow this path whether you're an operator or developer.

### Level 1: Core Concepts (30 minutes)

**Goal**: Understand what icg does and how it works.

**Read**:
1. **[Quick Start Guide](quick-start.md)** — 15 minutes
   - What icg protects
   - Installation and setup
   - Basic usage
   - Common tasks

2. **[Training Manual - Introduction](operators/training-manual.md#introduction-to-icg)** — 15 minutes
   - Problem statement
   - icg solution
   - Architecture overview
   - Core concepts

**Practice**:
- Run `icg health` to check your installation
- Test a few commands with `icg check --command "<cmd>"`
- Explore rule packs with `icg coverage --list`

**Assessment**: You should be able to answer:
- What problem does icg solve?
- How does icg intercept commands?
- What's the difference between command-mode and content-mode packs?

### Level 2: Using icg (1 hour)

**Goal**: Be able to use icg effectively in daily work.

**Read**:
1. **[Quick Start Guide - Basic Usage](quick-start.md#basic-usage)** — 15 minutes
   - Checking commands
   - Viewing denials
   - Understanding denials

2. **[Quick Start Guide - Example Workflows](quick-start.md#example-workflows)** — 15 minutes
   - Daily development workflow
   - Handling denials
   - Emergency bypass

3. **[Examples - Operator Scenarios](examples/README.md#operator-scenarios)** — 30 minutes
   - First-time installation
   - Daily operations
   - Handling denials
   - Emergency response

**Practice**:
- Monitor denials for a day: `icg status --denials --since 1d`
- Investigate a denial: `icg explain --pattern <pattern-id>`
- Practice the emergency bypass procedure (simulation only)

**Assessment**: You should be able to:
- Check if a command would be blocked
- Understand why a command was blocked
- Find safe alternatives to blocked operations
- Handle an emergency bypass correctly

### Level 3: Operating icg (2 hours)

**Goal**: Be able to install, configure, and maintain icg for a team.

**Read**:
1. **[Operator Documentation](operators/README.md)** — 30 minutes
   - Deployment model
   - Rule packs and release safety
   - Operator commands

2. **[Training Manual - Day-to-Day Operations](operators/training-manual.md#day-to-day-operations)** — 30 minutes
   - Morning routine
   - Monitoring denials
   - Updating rule packs

3. **[Fail-Closed Mode Guide](operators/fail-closed-mode.md)** — 30 minutes
   - Activation prerequisites and release qualification
   - Monitoring and alert interpretation
   - Emergency demotion and troubleshooting

4. **[Training Manual - Maintenance Procedures](operators/training-manual.md#maintenance-procedures)** — 30 minutes
   - Weekly maintenance
   - Monthly maintenance
   - Quarterly maintenance

5. **[Training Manual - Troubleshooting](operators/training-manual.md#troubleshooting)** — 30 minutes
   - Common issues
   - Debug mode
   - Getting help

**Practice**:
- Perform a health check: `icg health --verbose`
- Review denial trends: `icg status --denials --trend --since 7d`
- Update rule packs: `icg update --check-only`

**Assessment**: You should be able to:
- Install and configure icg for a team
- Monitor system health
- Perform regular maintenance
- Troubleshoot common issues

### Level 4: Extending icg (4 hours)

**Goal**: Be able to create custom rule packs and contribute to icg development.

**Read**:
1. **[Developer Guide](developers/README.md)** — 1 hour
   - Architecture overview
   - Development environment
   - Understanding rule packs
   - Front-end integration

2. **[Rule Pack Best Practices](developers/rule-pack-best-practices.md)** — 1 hour
   - Design principles
   - Pattern authoring
   - Testing and validation
   - Common pitfalls

3. **[Examples - Developer Scenarios](examples/README.md#developer-scenarios)** — 1 hour
   - Creating a new rule pack
   - Testing pattern changes
   - Debugging false positives

4. **[Examples - Integration Scenarios](examples/README.md#integration-scenarios)** — 1 hour
   - Migrating from org-rule-guard.py
   - Multi-harness support
   - Repository overrides

**Practice**:
- Scaffold a new pack: `icg new-pack --id my-tool --mode command`
- Write tests for your patterns
- Generate a regression suite

**Assessment**: You should be able to:
- Create a new rule pack
- Write effective patterns
- Test and validate your pack
- Contribute to icg development

---

## Role-Specific Tracks

### Track: System Operator

**Role**: You're responsible for installing and maintaining icg for a team or organization.

**Focus**: Installation, configuration, monitoring, maintenance, troubleshooting.

**Learning Path**:
1. Level 1: Core Concepts (30 min)
2. Level 2: Using icg (1 hour)
3. Level 3: Operating icg (2 hours)

**Additional Reading**:
- [Deployment Guide](operators/deployment-guide.md)
- [Monitoring and Deployment Guide](monitoring-deployment-guide.md)
- [Troubleshooting Guide](operators/troubleshooting.md)

**Practice Exercises**:
- Perform a complete installation from scratch
- Set up monitoring for denials
- Handle a simulated emergency
- Perform weekly maintenance

**Validation**:
- Can install icg without referencing docs
- Can troubleshoot common issues
- Can perform all maintenance procedures
- Can train other operators

### Track: Security Engineer

**Role**: You're responsible for ensuring icg provides adequate protection and reviewing rule packs.

**Focus**: Security coverage, pattern accuracy, threat modeling, risk assessment.

**Learning Path**:
1. Level 1: Core Concepts (30 min)
2. Level 2: Using icg (1 hour)
3. Level 3: Operating icg (2 hours)
4. Level 4: Extending icg (4 hours)

**Additional Reading**:
- [Rule Pack Best Practices](developers/rule-pack-best-practices.md)
- [Release Integrity Verification](notes/release-integrity-verification.md)
- [Fail-Closed Mode Guide](operators/fail-closed-mode.md)
- [Transition Design](design/fail-closed-transition.md)

**Practice Exercises**:
- Audit existing rule packs for coverage gaps
- Create a threat model for your environment
- Review a rule pack PR for security issues
- Design patterns for a new tool

**Validation**:
- Can identify security gaps in protection
- Can assess risk of operations
- Can review rule packs for accuracy
- Can design effective security patterns

### Track: Software Developer

**Role**: You're a developer who wants to protect your workflow with icg or contribute to icg development.

**Focus**: Daily usage, creating custom packs, contributing to core.

**Learning Path**:
1. Level 1: Core Concepts (30 min)
2. Level 2: Using icg (1 hour)
4. Level 4: Extending icg (4 hours)

**Additional Reading**:
- [Developer Guide](developers/README.md)
- [Rule Pack Best Practices](developers/rule-pack-best-practices.md)
- [Project README](../README.md)

**Practice Exercises**:
- Create a rule pack for your tool
- Write tests for your pack
- Contribute a pattern to existing pack
- Debug a false positive

**Validation**:
- Can create effective patterns
- Can test and validate packs
- Can debug pattern issues
- Can contribute to icg development

### Track: DevOps Engineer

**Role**: You're integrating icg into CI/CD pipelines and automated workflows.

**Focus**: Integration, automation, monitoring, incident response.

**Learning Path**:
1. Level 1: Core Concepts (30 min)
2. Level 2: Using icg (1 hour)
3. Level 3: Operating icg (2 hours)

**Additional Reading**:
- [Deployment Guide](operators/deployment-guide.md)
- [Monitoring and Deployment Guide](monitoring-deployment-guide.md)
- [Examples - Integration Scenarios](examples/README.md#integration-scenarios)

**Practice Exercises**:
- Integrate icg into CI pipeline
- Set up automated denial monitoring
- Create incident response procedures
- Test rollback procedures

**Validation**:
- Can integrate icg into automated systems
- Can monitor and alert on denials
- Can handle incidents involving icg
- Can maintain icg in production

---

## Quick Reference

### Essential Commands

```bash
# Health check
icg health

# Test a command
icg check --command "<cmd>"

# View recent denials
icg status --denials --since 1h

# Explain a pattern
icg explain --pattern <pattern-id>

# List rule packs
icg coverage --list

# Update rule packs
icg update --check-only
```

### Key Files

```
Binary:        /usr/local/bin/icg
Rule packs:    /etc/icg/packs/
State store:   /var/lib/icg/state.db
Denial log:    /var/log/icg/denials.log
```

### Severity Levels

| Severity | Description |
|----------|-------------|
| Critical | Irreversible damage (data destruction, history rewrite) |
| High | Significant damage (resource deletion, wrong config) |
| Medium | Moderate damage (service disruption) |

### Response Channels

| Channel | Behavior |
|---------|----------|
| deny | Block entirely (critical/high severity) |
| updated_input | Provide safe alternative (future) |
| additional_context | Warn without blocking (tier 3) |

---

## Common Tasks

### Task: Check if a Command Would Be Blocked

```bash
icg check --command "vault kv destroy secret/test"
```

### Task: View Recent Denials

```bash
icg status --denials --since 1h
```

### Task: Understand Why a Command Was Blocked

```bash
icg explain --pattern vault-kv-destroy
```

### Task: Update Rule Packs

```bash
icg update --check-only  # Check for updates
icg update              # Apply updates
```

### Task: Perform Health Check

```bash
icg health --verbose
```

### Task: Create a New Rule Pack

```bash
icg new-pack --id my-tool --mode command
```

### Task: Test a Rule Pack

```bash
icg check --command "my-tool dangerous" --pack /etc/icg/packs/my-tool.json
```

### Task: Generate Regression Suite

```bash
icg regression-suite /etc/icg/packs/vault.json --output vault-regression.json
```

---

## Getting Help

### Documentation

- **[Quick Start Guide](quick-start.md)** — Get started in 5 minutes
- **[Operator Documentation](operators/README.md)** — Comprehensive operator guide
- **[Developer Guide](developers/README.md)** — Developer documentation
- **[Examples](examples/README.md)** — Real-world scenarios
- **[Training Manual](operators/training-manual.md)** — Comprehensive training

### Troubleshooting

1. **Check health**: `icg health --verbose`
2. **Review logs**: `/var/log/icg/denials.log`
3. **Enable debug**: `export ICG_DEBUG=1`
4. **Read troubleshooting guide**: [Troubleshooting](operators/troubleshooting.md)

### Community

- **GitHub Issues**: https://github.com/jedarden/irreversible-command-gate/issues
- **Documentation**: https://github.com/jedarden/irreversible-command-gate/tree/main/docs

### When Asking for Help

Include this information:
1. icg version: `icg --version`
2. Command you're trying: `icg check --command "<cmd>"`
3. Error message: Full output
4. Context: What you're trying to accomplish

---

## Next Steps

### After Completing Level 1 (Core Concepts)

- Install icg on your system
- Test a few commands
- Explore the rule packs
- Read the operator documentation

### After Completing Level 2 (Using icg)

- Use icg in your daily workflow
- Monitor denials for a week
- Practice handling denials
- Help onboard others

### After Completing Level 3 (Operating icg)

- Set up icg for your team
- Configure monitoring and alerting
- Create maintenance procedures
- Document your setup

### After Completing Level 4 (Extending icg)

- Create custom rule packs for your tools
- Contribute patterns to existing packs
- Report bugs and feature requests
- Help review PRs

---

## Learning Resources

### Official Documentation

- **Project README**: [../README.md](../README.md)
- **Quick Start**: [quick-start.md](quick-start.md)
- **Operator Docs**: [operators/README.md](operators/README.md)
- **Developer Docs**: [developers/README.md](developers/README.md)
- **Examples**: [examples/README.md](examples/README.md)

### Training Materials

- **Training Manual**: [operators/training-manual.md](operators/training-manual.md)
- **Best Practices**: [developers/rule-pack-best-practices.md](developers/rule-pack-best-practices.md)
- **Deny Messages**: [operators/deny-messages.md](operators/deny-messages.md)

### Reference Materials

- **Troubleshooting**: [operators/troubleshooting.md](operators/troubleshooting.md)
- **Deployment**: [operators/deployment-guide.md](operators/deployment-guide.md)
- **Monitoring**: [monitoring-deployment-guide.md](monitoring-deployment-guide.md)

---

## Assessment Checklist

Use this checklist to track your progress:

### Level 1: Core Concepts

- [ ] Understand what problem icg solves
- [ ] Understand how icg intercepts commands
- [ ] Know the difference between command-mode and content-mode packs
- [ ] Can run basic icg commands
- [ ] Can interpret a denial message

### Level 2: Using icg

- [ ] Can check if a command would be blocked
- [ ] Can view recent denials
- [ ] Can understand why a command was blocked
- [ ] Can find safe alternatives
- [ ] Can handle an emergency (simulation)

### Level 3: Operating icg

- [ ] Can install icg from scratch
- [ ] Can configure hooks for AI harnesses
- [ ] Can monitor system health
- [ ] Can perform maintenance procedures
- [ ] Can troubleshoot common issues

### Level 4: Extending icg

- [ ] Can create a new rule pack
- [ ] Can write effective patterns
- [ ] Can test and validate packs
- [ ] Can debug pattern issues
- [ ] Can contribute to icg development

---

## Feedback and Contributions

### Reporting Issues

Found a bug or have a feature request?

```bash
# Create a bug report
icg bug-report --output /tmp/icg-bug-report.txt
gh issue create \
  --title "Bug: <description>" \
  --body "Attached bug report" \
  --repo jedarden/irreversible-command-gate
```

### Contributing

Want to contribute to icg?

1. Read the [Developer Guide](developers/README.md)
2. Read the [Best Practices](developers/rule-pack-best-practices.md)
3. Follow the [Contributing Guidelines](../README.md#contributing)
4. Submit a PR

### Improving Documentation

Found a gap in the documentation?

1. Identify what's missing
2. Create an issue describing the gap
3. Suggest where it should be documented
4. Optionally, submit a PR with the improvement

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

### Frequently Asked Questions

**Q: Will icg slow down my workflow?**

A: No. icg adds <1ms latency per command check. The evaluation is local and doesn't make network calls.

**Q: Can icg block all dangerous operations?**

A: No. icg protects against specific, known dangerous patterns. It's a safety layer, not a complete security solution.

**Q: What if icg blocks a legitimate operation?**

A: Follow the redirect suggestion, or use emergency bypass (ICG_DISABLED=1) if absolutely necessary. Report false positives to improve icg.

**Q: Do I need to be a Rust developer to use icg?**

A: No. Most operators and users never need to write Rust code. Rule packs are JSON and use regex patterns.

**Q: How often are rule packs updated?**

A: Rule packs are updated as new threats are identified and patterns are improved. Check for updates regularly.

### Time Estimates

| Activity | Time |
|----------|------|
| Installation | 5-10 min |
| Level 1: Core Concepts | 30 min |
| Level 2: Using icg | 1 hour |
| Level 3: Operating icg | 2 hours |
| Level 4: Extending icg | 4 hours |
| Total learning path | ~8 hours |

### Support Channels

- **Documentation**: Start here — most questions are answered in the docs
- **GitHub Issues**: For bugs, feature requests, and questions
- **Security**: security@company.com for security-related issues
- **Operations**: ops-team@company.com for operational issues

---

**Onboarding Guide Version**: 1.0
**Last Updated**: 2026-08-16
**For**: icg v0.1.0+

Welcome to icg! If you have questions, start with the documentation or file an issue. Happy protecting!
