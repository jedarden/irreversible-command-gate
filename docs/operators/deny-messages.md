# Deny Message Interpretation Guide

## Overview

This guide explains how to interpret icg denial messages and take appropriate corrective action. When icg denies an operation, it provides structured information about why the operation was blocked and what to do instead.

## Denial Message Structure

Every denial message includes:

- **Rule Pack ID**: Which rule pack caught the violation (e.g., `vault`, `git`, `image-tag`)
- **Pattern ID**: Which specific pattern matched (e.g., `vault-destructive`, `force-push`)
- **Severity**: How critical the violation is (`Critical`, `High`, `Medium`)
- **Explanation**: Why this operation is dangerous
- **Redirect**: What to do instead (corrective action)

### Example Denial Message

```
DENIED by icg
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Rule Pack:    vault
Pattern ID:   vault-destructive
Severity:     Critical
Explanation:  This operation would permanently destroy secret data and cannot be undone.
Redirect:     Use 'vault kv patch' to reconcile or 'vault kv delete' for versioned metadata.
Command:      vault kv destroy secret/app/api-key
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Rule Pack Specific Denials

### Vault/OpenBao Pack

#### Pattern: vault-destructive

**Message Example**:
```
DENIED: vault kv destroy would permanently destroy secret data
Severity: Critical
```

**What Was Blocked**:
- `vault kv destroy secret/<path>` - Permanent secret destruction
- `vault kv destroy -versions=<n> <path>` - Permanent version destruction
- `bao secrets disable <path>` - Disable secret engine
- `vault policy delete <name>` - Delete policy
- `vault token revoke` - Revoke token

**Why It's Dangerous**:
- Destroyed secrets cannot be recovered
- Permanent data loss
- No undo mechanism

**Corrective Action**:
1. **If you need to update a secret**: Use `vault kv patch` or `vault kv put`
2. **If you need to delete metadata**: Use `vault kv metadata delete` (safer than destroy)
3. **If you need to reconcile**: Use `vault kv patch` with reconciliation operations
4. **If you truly need to destroy**: This is a deliberate protection - consult your team lead

**Example Correction**:
```bash
# WRONG (blocked)
vault kv destroy secret/app/api-key

# RIGHT (allowed)
vault kv patch secret/app/api-key -remove=expired_field
```

---

### Git Pack

#### Pattern: force-push

**Message Example**:
```
DENIED: git push --force would rewrite public history
Severity: Critical
```

**What Was Blocked**:
- `git push --force` - Force push to any remote
- `git push -f` - Short form of force push
- `git push --force-with-lease` - Force with lease (still blocked in Phase 1)

**Why It's Dangerous**:
- Rewrites public git history
- Breaks other collaborators' repos
- Loses commits that others may have based work on
- Cannot be undone after others have pulled

**Corrective Action**:
1. **If you need to fix a recent commit**: Use `git commit --amend` (before pushing)
2. **If you need to rebase**: Use `git rebase -i` (before pushing)
3. **If you need to reconcile divergent history**: Use `git merge` instead
4. **If history is already corrupted**: Coordinate with team to recover

**Example Correction**:
```bash
# WRONG (blocked)
git push --force origin main

# RIGHT (allowed)
git push origin main
# If push fails, reconcile first:
git pull --rebase origin main
# Or:
git merge origin/main
git push origin main
```

---

#### Pattern: stale-HEAD-push

**Message Example**:
```
DENIED: git push would overwrite remote changes you don't have locally
Severity: High
```

**What Was Blocked**:
- `git push` when local HEAD is behind remote HEAD
- Indicates you haven't pulled recent changes

**Why It's Dangerous**:
- Overwrites remote commits you don't have locally
- Causes data loss for others' work
- Creates divergent history that's hard to recover

**Corrective Action**:
1. **Pull recent changes first**:
   ```bash
   git pull origin main
   ```
2. **Then push**:
   ```bash
   git push origin main
   ```
3. **If there are conflicts**: Resolve them before pushing

**Example Correction**:
```bash
# WRONG (blocked when HEAD is stale)
git push origin main

# RIGHT (allowed)
git pull --rebase origin main  # or: git merge origin/main
git push origin main
```

---

#### Pattern: commit-without-pathspec

**Message Example**:
```
DENIED: git commit without pathspec would commit entire staged index
Severity: High
```

**What Was Blocked**:
- `git commit -m "message"` with no trailing pathspec
- `git commit -a` (commits all modified tracked files)
- `git commit --all` (commits all modified tracked files)

**Why It's Dangerous**:
- Commits entire staged index, not just files you intended
- In shared checkouts, includes other workers' uncommitted files
- Violates CLAUDE.md requirement for precise `git add` usage

**Corrective Action**:
1. **Stage only the files you want to commit**:
   ```bash
   git add path/to/file1 path/to/file2
   ```
2. **Commit with explicit pathspec**:
   ```bash
   git commit -m "message" path/to/file1 path/to/file2
   ```
3. **Or use commit-after-add pattern**:
   ```bash
   git add file1 file2
   git commit -m "message" file1 file2  # Include pathspec anyway
   ```

**Example Correction**:
```bash
# WRONG (blocked)
git add src/main.rs
git commit -m "fix main.rs"  # Would commit entire index

# RIGHT (allowed)
git add src/main.rs
git commit -m "fix main.rs" src/main.rs  # Explicit pathspec
```

---

### Image Tag Pack

#### Pattern: latest-tag

**Message Example**:
```
DENIED: image tag :latest is not pinned to a specific version
Severity: High
```

**What Was Blocked**:
- `image: foo:latest` in YAML files
- `image: foo@sha256:...` (bare git SHA) in YAML files

**Why It's Dangerous**:
- `:latest` doesn't pin to a specific version
- Breaks reproducibility (different versions over time)
- Bare git SHA is unmaintainable (no semantic version)

**Corrective Action**:
1. **Pin to a semantic version tag**:
   ```yaml
   image: foo:v1.2.3
   ```
2. **Or use a specific digest**:
   ```yaml
   image: foo@sha256:abc123...  # Only if you have the digest
   ```
3. **Best practice**: Use `containers/<name>/VERSION` file for the canonical version

**Example Correction**:
```yaml
# WRONG (blocked)
image: ronaldraygun/myapp:latest

# RIGHT (allowed)
image: ronaldraygun/myapp:v1.2.3
```

---

### Storage Class Pack

#### Pattern: ssd-storage

**Message Example**:
```
DENIED: storageClassName ssd is not allowed on Rackspace Spot
Severity: High
```

**What Was Blocked**:
- `storageClassName: ssd` in YAML manifests
- `storageClassName: ssd-large` in YAML manifests

**Why It's Dangerous**:
- Rackspace Spot's default is wrong (ssd)
- SSD storage cannot be expanded or reclassed in place
- Higher cost without benefit for most workloads

**Corrective Action**:
1. **Use sata storage class**:
   ```yaml
   storageClassName: sata
   ```
2. **Or sata-large for larger volumes**:
   ```yaml
   storageClassName: sata-large
   ```

**Example Correction**:
```yaml
# WRONG (blocked)
storageClassName: ssd

# RIGHT (allowed)
storageClassName: sata
```

---

### Beads Pack

#### Pattern: beads-write-in-shared-checkout

**Message Example**:
```
DENIED: writing to .beads/ in a shared checkout risks concurrent corruption
Severity: Critical
```

**What Was Blocked**:
- Any `Write` or `Edit` operation targeting a path under `.beads/`
- Only in shared/primary checkouts (`.git` is a directory)
- NOT blocked in linked worktrees (`.git` is a file)

**Why It's Dangerous**:
- Multiple workers can write to `.beads/` concurrently
- Risk of corrupting bead state
- bead-rs lacks multi-writer concurrency control

**Corrective Action**:
1. **Use a throwaway worktree for bead conflicts**:
   ```bash
   git worktree add ../beads-fix -b beads-fix
   cd ../beads-fix
   # Make your changes to .beads/
   git checkout -
   ```
2. **Or use the bead CLI instead of direct editing**:
   ```bash
   bead update <id> --notes "new notes"
   ```

**Example Correction**:
```bash
# WRONG (blocked in shared checkout)
echo "test" > .beads/checkpoint/current.json

# RIGHT (allowed)
git worktree add ../beads-fix -b beads-fix
cd ../beads-fix
echo "test" > .beads/checkpoint/current.json
# Or:
bead update <id> --notes "new notes"
```

---

### Secrets Pack

#### Pattern: credential-value-in-bash

**Message Example**:
```
DENIED: writing credential value to file or command
Severity: Critical
```

**What Was Blocked**:
- `echo "ghp_..." >> file.txt` - GitHub token in file
- `curl -d "token=sk-..."` - API key in command
- `export AWS_ACCESS_KEY_ID=...` - Credential in environment
- Any credential-like value in Bash commands

**Why It's Dangerous**:
- Credentials end up in shell history, logs, or files
- Can be leaked in commits, logs, or monitoring
- Violates security best practices

**Corrective Action**:
1. **Use OpenBao/Vault for credential storage**:
   ```bash
   vault kv get -field=api_key secret/app/production
   ```
2. **Use environment files with proper permissions**:
   ```bash
   echo "API_KEY=$(vault kv get -field=api_key secret/app)" > .env.local
   chmod 600 .env.local
   ```
3. **Use secret management tools**:
   - Kubernetes: External Secrets Operator
   - Docker: Docker Secrets
   - Terraform: Terraform Cloud/Enterprise

**Example Correction**:
```bash
# WRONG (blocked)
echo "ghp_test_token" > github-token.txt

# RIGHT (allowed)
vault kv get -field=token secret/github > github-token.txt
chmod 600 github-token.txt
```

---

### Misc Pack

#### Pattern: deprecated-bead-cli

**Message Example**:
```
DENIED: deprecated bead CLI 'br' is no longer supported
Severity: Medium
```

**What Was Blocked**:
- `br` command (beads_rust, deprecated)
- `bf` command (bead-forge, deprecated as of 2026-08-14)

**Why It's Dangerous**:
- Deprecated tools may have bugs or security issues
- No longer maintained or supported
- May produce incompatible output

**Corrective Action**:
1. **Use the canonical bead CLI**:
   ```bash
   bead list
   bead show <id>
   bead create --title "..."
   ```
2. **Check which CLI is canonical**:
   - As of 2026-08-14: `bead` (bead-rs) is canonical
   - `br` and `bf` are deprecated

**Example Correction**:
```bash
# WRONG (blocked)
bf list

# RIGHT (allowed)
bead list
```

---

#### Pattern: needle-cleanup

**Message Example**:
```
DENIED: needle cleanup would SIGHUP live workers
Severity: Critical
```

**What Was Blocked**:
- `needle cleanup` command

**Why It's Dangerous**:
- SIGHUPs live workers, interrupting active work
- Can cause data corruption or incomplete operations
- No graceful shutdown

**Corrective Action**:
1. **Let workers finish naturally**: No cleanup needed
2. **If you must clean up**: Wait for workers to be idle first
3. **For stuck workers**: Investigate why they're stuck instead of force-cleaning

**Example Correction**:
```bash
# WRONG (blocked)
needle cleanup

# RIGHT (no cleanup needed - workers exit naturally)
# Or: wait for workers to finish, then cleanup is safe
```

---

### Tmux Pack

#### Pattern: bare-nato-session

**Message Example**:
```
DENIED: targeting bare NATO tmux session interferes with operator's session
Severity: Medium
```

**What Was Blocked**:
- `tmux send-keys -t alpha ...` - Targeting bare NATO session
- `tmux send-keys -t bravo ...` - Targeting bare NATO session
- Any tmux command targeting a bare NATO session name

**Why It's Dangerous**:
- Bare NATO sessions (`alpha`, `bravo`, etc.) are the operator's personal sessions
- Interfering with them disrupts operator workflow
- May send keys to the wrong session

**Corrective Action**:
1. **Use named NEEDLE worker sessions instead**:
   ```bash
   tmux send-keys -t needle-worker-001 ...  # Worker session, not operator's
   ```
2. **Or use session identifiers instead of bare names**:
   ```bash
   tmux send-keys -t @session-id ...  # Use session ID
   ```

**Example Correction**:
```bash
# WRONG (blocked)
tmux send-keys -t alpha "vim ~/.bashrc" Enter

# RIGHT (allowed)
tmux send-keys -t needle-worker-001 "cargo test" Enter
```

---

## Taking Corrective Action

### General Workflow

When you receive a denial:

1. **Read the denial message carefully**
   - Identify the rule pack and pattern
   - Read the explanation
   - Read the redirect (corrective action)

2. **Understand why it was blocked**
   - Is this a critical operation that needs special approval?
   - Is there a safer alternative?
   - Did you make a mistake in the command?

3. **Follow the redirect**
   - Use the suggested alternative command
   - Or follow the documented corrective action

4. **If the denial is incorrect** (false positive):
   - Document what you were trying to do
   - Check if there's a repository override needed
   - File an issue with the denial details

### Escalation Procedures

#### For False Positives

If you believe an operation was wrongly denied:

1. **Gather information**:
   ```bash
   icg status --denials --last 1 --format json > false-positive.json
   ```

2. **Check for known issues**:
   - Review GitHub issues for similar reports
   - Check if there's a newer rule pack version

3. **Request a repository override** (if legitimate, repo-specific case):
   ```bash
   icg override create --repo /path/to/repo \
     --pattern-id "<pattern-id>" \
     --justification "<detailed explanation>"
   ```
   Note: Requires Layer 1/2 approval via release pipeline

4. **File an issue** (if genuine false positive):
   - Include the false-positive.json export
   - Describe what you were trying to do
   - Explain why the denial is incorrect

#### For Emergency Operations

If you need to bypass the guard for an emergency:

1. **Assess the risk**:
   - Is this truly an emergency?
   - Can it wait for proper approval?

2. **Document the emergency**:
   - Record what you're doing and why
   - Note the time and context

3. **Use emergency disable** (last resort):
   ```bash
   ICG_DISABLED=1 <command>
   ```
   Note: This completely disables the guard for one command

4. **Follow up**:
   - File an incident report
   - Review why the guard blocked a legitimate emergency operation
   - Update rule pack or procedures to handle this case better

## Common Denial Scenarios

### Scenario 1: Accidental Destructive Command

**Situation**: You accidentally type `vault kv destroy` instead of `vault kv delete`

**Denial Message**:
```
DENIED: vault kv destroy would permanently destroy secret data
```

**What to Do**:
1. Recognize you made a mistake (the guard caught it!)
2. Use the correct command: `vault kv delete` (for versioned metadata)
3. If you truly need to destroy: Consult your team lead first

### Scenario 2: Force Push After Rebase

**Situation**: You rebased your feature branch and try to force push

**Denial Message**:
```
DENIED: git push --force would rewrite public history
```

**What to Do**:
1. Don't force push to shared branches (main, master, etc.)
2. For feature branches: Coordinate with team to force push
3. Consider using merge commits instead of rebase for shared branches

### Scenario 3: Commit Without Pathspec

**Situation**: You stage files with `git add` then run `git commit -m "message"`

**Denial Message**:
```
DENIED: git commit without pathspec would commit entire staged index
```

**What to Do**:
1. Include the pathspec in the commit command:
   ```bash
   git commit -m "message" file1 file2
   ```
2. This ensures you only commit what you intended, even if the index has other files

### Scenario 4: Using Latest Tag

**Situation**: You're deploying and use `image: app:latest`

**Denial Message**:
```
DENIED: image tag :latest is not pinned to a specific version
```

**What to Do**:
1. Check the canonical version:
   ```bash
   cat containers/app/VERSION
   ```
2. Use that version:
   ```yaml
   image: app:v1.2.3
   ```

### Scenario 5: Accidental Beads Edit

**Situation**: You try to manually edit `.beads/checkpoint/current.json`

**Denial Message**:
```
DENIED: writing to .beads/ in a shared checkout risks concurrent corruption
```

**What to Do**:
1. Use a worktree for this operation:
   ```bash
   git worktree add ../beads-fix -b beads-fix
   ```
2. Or use the bead CLI:
   ```bash
   bead update <id> --notes "new notes"
   ```

## Training for Users

### Key Concepts for Users

1. **Denials are protective**: The guard is preventing a mistake, not obstructing legitimate work
2. **Read the message**: Every denial explains why and what to do instead
3. **Follow the redirect**: The corrective action is in the denial message
4. **False positives happen**: If you think it's wrong, follow escalation procedures
5. **Emergency disable exists**: But use it only for genuine emergencies

### Common Misconceptions

**Misconception**: "The guard is blocking my work"

**Reality**: The guard is preventing a mistake that could cause serious damage. Read the denial message to understand why.

**Misconception**: "I can just disable the guard"

**Reality**: You can, but you should document why and follow up. Emergency disable is for genuine emergencies, not convenience.

**Misconception**: "All denials are false positives"

**Reality**: Most denials are legitimate. Read the explanation before assuming it's wrong.

## Monitoring and Trends

### Reviewing Denial Patterns

Regularly review denial patterns to identify:

1. **Frequent false positives**: May indicate rule pack issues
2. **Training gaps**: Users making the same mistakes repeatedly
3. **Workflow issues**: Processes that require unsafe operations

```bash
# View denial patterns
icg status --denials --pattern-summary --since 7d

# View denial trends
icg status --denials --trend --since 30d
```

### Acting on Trends

**If false positives are frequent**:
1. Check for rule pack updates
2. File issues with specific examples
3. Request repository overrides if needed

**If legitimate mistakes are frequent**:
1. Provide additional training to users
2. Update documentation to clarify common issues
3. Consider improving command-line tools to make safe operations easier

## Quick Reference Card

### Vault
- `vault kv destroy` → Use `vault kv patch` or `vault kv delete`
- `vault policy delete` → Coordinate with team
- `vault token revoke` → Use token renewal instead

### Git
- `git push --force` → Use `git merge` instead
- `git commit -m` (no pathspec) → Add pathspec: `git commit -m "msg" file1 file2`
- `git push` (stale HEAD) → Pull first: `git pull origin main`

### Images
- `image: foo:latest` → Use `image: foo:v1.2.3`
- `image: foo@sha256:...` → Use semantic version instead

### Beads
- Editing `.beads/` directly → Use worktree or `bead` CLI

### Secrets
- Writing credentials to files → Use Vault/OpenBao
- Credentials in commands → Use secret management tools

### Misc
- `needle cleanup` → Let workers finish naturally
- Targeting NATO sessions → Use worker sessions instead

---

**Document Version**: 1.0
**Last Updated**: 2026-08-16
**For**: icg v0.1.0+
