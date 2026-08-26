# icg Quick Start Guide

Get started with icg in about 5 minutes. This guide covers installation,
basic usage, and common tasks for new users. Every command and output below
was verified against the shipped `icg` v0.1.0 surface.

## What is icg?

**icg (irreversible-command-gate)** is a safety system for AI coding agents
that blocks or rewrites destructive operations before they can cause damage.

- **Protects against**: irreversible OpenBao operations, git force-pushes,
  credential literals in commands and files, banned image tags and storage
  classes, and a few fleet-specific footguns (see
  [What Gets Protected](#what-gets-protected))
- **Works with**: Claude Code and local Codex CLI harnesses, through their
  `PreToolUse` hook systems
- **Philosophy**: every denial explains what to do instead — not just
  "blocked"
- **Design**: fail-open by default — if no rule packs are loaded or the tool
  is not recognized, the operation is allowed rather than blocked

icg is a backstop for honest mistakes, not a boundary against a malicious
process. Keep the harness's own approval and sandbox controls enabled.

### What icg does NOT cover

- **kubectl.** There is deliberately no kubectl pack and there will not be
  one: mutating-verb blocking (`kubectl delete`, `patch`, `apply`, …) stays
  with the existing org-level hook (`org-rule-guard.py`), per the plan's
  "Explicitly not attempted" decision. `icg check --command "kubectl delete
  pvc data-volume"` returns `ALLOW: no configured rule matched` — that is
  expected, not a gap.
- **`.github/workflows/*` creation and `kind: Job`/`CronJob` manifests** —
  also owned by the org-level hook today.
- **Cloud-hosted agent sessions** (ChatGPT web, Claude.ai). Only local
  harnesses invoke local hooks.

## Installation (2 minutes)

### Option 1: Build from Source (currently the only path)

**No GitHub release has been cut yet** — the release pipeline exists but has
not produced a verified end-to-end release (tracked as `irrevers-84b36e47`).
Until one exists, build from source:

```bash
# Clone repository
git clone https://git.ardenone.com/jedarden/irreversible-command-gate.git
cd irreversible-command-gate

# Build
cargo build --release

# Install to the root-owned system location
sudo install -o root -g root -m 0755 target/release/icg /usr/local/bin/icg

# Verify installation
icg --version
# icg 0.1.0
```

Once releases exist, prefer downloading the release binary. For the full
production procedure (build verification, ownership model, trust pointers),
see `docs/operators/deployment-guide.md`.

---

## Initial Setup (3 minutes)

### Step 1: Install Rule Packs

The hook loads every JSON manifest in `/etc/icg/packs/` by default. Copy the
pack files from your checkout — the repo is the source of truth; there is no
tagged release to download packs from yet.

```bash
# Create the root-owned pack directory
sudo install -d -o root -g root -m 0755 /etc/icg /etc/icg/packs

# Install the pack files
sudo install -o root -g root -m 0644 packs/*.json /etc/icg/packs/

# Verify rule packs are loaded
icg coverage --list
```

The pack directory must stay root-owned; the guarded agent must not be able
to edit policy. `icg update` (see
[Update rule packs](#task-4-update-rule-packs)) is the sanctioned way to
change its contents later.

### Step 2: Configure the Claude Code Hook

Add a `PreToolUse` command hook to `~/.claude/settings.json`. Merge this
into your existing `hooks` object — do not overwrite unrelated settings:

```json
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
```

- The command must be an **absolute path** — it cannot depend on the agent's
  working directory or `PATH`.
- The `Bash` matcher supplies command-mode input. `Write` and `Edit`
  supply file content for the content-mode packs (`image-tag`,
  `storage-class`, `secrets`).
- Confirm the hook appears in Claude Code's hook inspection UI before
  relying on it. Matchers are case-sensitive.
- For a local Codex CLI, use the equivalent shape in its hook configuration
  — see `docs/operators/deployment-guide.md`.

### Step 3: Smoke-Test the Installation

```bash
# 1. All nine packs should be listed
icg coverage --list
# ✓ pack beads (3 patterns)
# ✓ pack docker (3 patterns)
# ✓ pack git (3 patterns)
# ✓ pack image-tag (2 patterns)
# ✓ pack misc (2 patterns)
# ✓ pack openbao (3 patterns)
# ✓ pack secrets (6 patterns)
# ✓ pack storage-class (1 patterns)
# ✓ pack tmux (1 patterns)

# 2. A destructive command must be denied
icg check --command "bao kv destroy secret/app/key"
# DENIED by icg
# Reason: This is an irreversible OpenBao operation. 'kv delete' soft-deletes and is recoverable; ...
# Pack: openbao
# Pattern: openbao-destructive-verb
# Severity: Critical
# ... (Explanation and Redirect lines continue with the full operator guidance)

# 3. A force-push is rewritten, not denied
icg check --command "git push --force origin main"
# REWRITE: Removed --force/-f/--force-with-lease from git push; force-pushing can rewrite remote history
# and lose commits. Retrying as a normal push preserves the requested commits without rewriting the remote.
# Suggested input: git push origin main
# Pack: git
# Pattern: git-force-push

# 4. A safe command must pass
icg check --command "git status"
# ALLOW: no configured rule matched
```

Notes on reading these results:

- `icg check` communicates the decision on **stdout**; its exit code is `0`
  for `ALLOW`, `REWRITE`, and `DENIED` alike. Scripts must parse the output,
  not the exit status.
- Warnings on **stderr** about `/var/cache/icg` mean telemetry could not be
  persisted (the directory is missing or not writable by the hook identity).
  The decision on stdout is unaffected. The deployment guide covers the
  cache-directory ownership model.
- `icg check` and `icg coverage` exit `1` with
  `Error: no rule packs found; pass --pack <path>` when run outside a
  checkout with no packs installed. The **hook**, by contrast, fails open:
  with `/etc/icg/packs` absent it silently allows everything. Always run
  `icg coverage --list` after installing to confirm the hook will actually
  load policy.

---

## Basic Usage

### Checking commands

`icg check` evaluates a command, file, or PreToolUse request without
executing it:

```bash
# A command string
icg check --command "git push --force origin main"

# A PreToolUse JSON document from stdin
echo '{"toolName":"Bash","toolInput":{"command":"bao kv destroy secret/test"}}' \
  | icg check --stdin

# File content (content-mode packs: image-tag, storage-class, secrets)
printf 'image: ronaldraygun/armor:latest\n' | icg check --file -
# DENIED by icg
# Reason: The :latest image tag is banned — it silently changes what runs and makes rollback impossible. ...
# Pack: image-tag
# Pattern: image-tag-latest
# Severity: High

# Extra evaluation detail while debugging
icg check --command "..." --debug
```

The three outcomes:

- `ALLOW: no configured rule matched` — nothing to say about this input.
- `REWRITE: <why> … Suggested input: <replacement>` — the guard produced a
  safe alternative (redirect channel `UpdatedInput`).
- `DENIED by icg` with `Reason`, `Pack`, `Pattern`, `Severity`,
  `Explanation`, and `Redirect` lines — blocked, with the operator
  explanation and what to do instead.

### Understanding a decision

Every pattern has a standing explanation:

```bash
icg explain --pattern git-force-push --show-redirect
# Pattern: git-force-push
# Pack: git
# Enabled: true
# Tier: Tier1
# Severity: Critical
# Why: Force-push flags can rewrite git history and lose commits
# Redirect channel: UpdatedInput
# Alternative: Removed --force/-f/--force-with-lease from git push; ...
# Replacement: {command_without_force}
```

`icg explain --pattern <id> --show-regex` adds the raw matcher.
`icg explain --denial <telemetry-id>` explains a recorded denial instead of
a pattern.

### Viewing rule pack coverage

```bash
# List all loaded rule packs
icg coverage --list

# List packs from an explicit file or directory
icg coverage --list --pack /etc/icg/packs
```

`check`, `explain`, and `coverage` take `--pack <path>` (defaulting to the
installed pack plus the repository's `packs/` directory when present). The
`hook` subcommand's equivalent flag is `--rule-pack` — see
[Hook mode vs check mode](#hook-mode-vs-check-mode).

---

## What Gets Protected

Nine packs ship today. Pattern IDs below are the IDs `icg explain` accepts.

| Pack | Patterns | What it blocks |
| --- | --- | --- |
| `openbao` | 3 | Irreversible verbs (`kv destroy`, `metadata delete`, policy/mount deletion, rekey) — `openbao-destructive-verb` (Critical); secret literals passed as arguments — `openbao-inline-secret-literal` (Critical); `kv get` dumped to stdout — `openbao-kv-get-to-stdout` (Medium) |
| `git` | 3 | Force-push flags (rewritten to a plain push) — `git-force-push` (Critical); committing without explicit pathspecs — `git-commit-without-pathspec` (High); pushing when the remote head is stale — `git-stale-remote-head-push` (High) |
| `secrets` | 6 | Credential literals in commands and file content: GitHub tokens and PATs, AWS access keys, Slack tokens, Anthropic API keys, PEM private-key blocks (all Critical) |
| `image-tag` | 2 | `:latest` and bare-SHA image references in file content (High) |
| `storage-class` | 1 | `ssd`/`ssd-large` storage classes in manifests — use `sata`/`sata-large` (High) |
| `docker` | 3 | `docker system prune --all`, `docker volume rm`, `docker image rm --force` (Critical) |
| `tmux` | 1 | Sending input to the operator's bare NATO tmux sessions — `bare-nato-session` (Medium) |
| `beads` | 3 | Hand-editing the shared `.beads` store (`beads-shared-checkout-write`, Critical); recovery misordering (`beads-repair-requires-flush`, `beads-flush-requires-pull`, High) |
| `misc` | 2 | `needle cleanup` against a live fleet (`needle-cleanup`, Critical); deprecated bead CLIs `bf`/`br` (`deprecated-bead-cli`, Medium) |

**Not covered by icg** (see [What icg does NOT cover](#what-icg-does-not-cover)):
kubectl mutations, `.github/workflows/*` creation, and `kind: Job`/`CronJob`
manifests remain the org-level hook's job.

**Safe operations** are not enumerated in a blocklist-facing doc — anything
no pattern matches is allowed (`git status`, `kubectl get`, `bao kv get -field=…`
into a config, semver image tags, `sata` storage classes, …). The
`openbao` and `git` packs additionally carry explicit safe-pattern lists that
keep read-only verbs fast.

---

## Integration with AI Harnesses

Claude Code and local Codex CLIs invoke `icg hook` as a `PreToolUse` command
hook (configuration in
[Step 2](#step-2-configure-the-claude-code-hook)).

**Request** (JSON on the hook's stdin):

```json
{
  "toolName": "Bash",
  "toolInput": {
    "command": "bao kv destroy secret/test"
  },
  "toolUseId": "toolu_0123456789"
}
```

**Response** (JSON on stdout). A denial:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "This is an irreversible OpenBao operation. ... [pack=openbao, pattern=openbao-destructive-verb]"
  }
}
```

A rewrite returns the safe alternative for the harness to retry with (the
`Reason` text is elided here for brevity; the real response carries the full
explanation):

```json
{
  "hookSpecificOutput": {
    "additionalContext": "Removed --force/-f/--force-with-lease from git push; ... [pack=git, pattern=git-force-push]",
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "updatedInput": {
      "command": "git push origin main"
    }
  }
}
```

An unmatched input returns `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}`.

### Hook mode vs check mode

- **Hook mode** (`icg hook`): used by harnesses; reads one PreToolUse JSON
  from stdin, emits one decision envelope, exits. Loads `/etc/icg/packs` by
  default (the legacy `/etc/icg/rule-pack.json` when the directory is
  absent). Override with `--rule-pack <path>` or `ICG_RULE_PACK`.
- **Check mode** (`icg check`): manual testing with `--command`, `--stdin`,
  or `--file`; prints human-readable decisions. Loads the installed pack
  plus the repository's `packs/` directory when present; override with
  `--pack`.

Both evaluate the same rule packs. Never rely on the hook until
`icg coverage --list` proves the packs load — an empty pack directory makes
the hook fail open.

---

## Common Tasks

### Task 1: Handle a denied command

1. **Read the denial message** — it names the pack and pattern and states
   what to do instead.
2. **Get the full explanation**:
   ```bash
   icg explain --pattern openbao-destructive-verb --show-redirect
   ```
3. **Use the suggested alternative** — e.g. for `bao kv destroy`, use the
   soft-delete (`kv delete`) path the redirect describes, or have a human
   run the destructive operation if it is genuinely intended.
4. **See the incident history** for context:
   ```bash
   icg status --denials --since 1h
   ```

### Task 2: Review denial history

```bash
# Recent denials
icg status --denials --since 1h

# Grouped by pattern
icg status --denials --pattern-summary --since 1d

# Machine-readable
icg status --denials --since 7d --format json
```

### Task 3: Health check

```bash
# Validate the configured Claude Code hook and every rule pack
icg health --check-hooks

# Complete operator health inventory
icg health --verbose

# Health/crash status only
icg health status
```

### Task 4: Update rule packs

```bash
# See what an update would do
icg update --check-only

# Download and atomically activate the modular pack archive
sudo icg update
```

`icg update` downloads the exact `icg-packs.tar.gz` release asset, validates
every manifest, atomically swaps the whole `/etc/icg/packs` directory, and
retains the previous one at `/etc/icg/packs.previous/` for rollback. It
requires a trust pointer set to the approved release — the full procedure is
in `docs/operators/deployment-guide.md`.

---

## Example Workflows

### Workflow 1: Daily development

```bash
# Morning: confirm the guard is armed
icg coverage --list
icg health --check-hooks

# During work: test a borderline command before running it
icg check --command "docker volume rm pgdata"

# End of day: skim what was blocked
icg status --denials --since 1h --pattern-summary
```

### Workflow 2: Handling a force-push rewrite

```bash
# The agent tries:
git push --force origin main

# The hook returns updatedInput and the harness retries:
git push origin main

# If the push is rejected because the remote is ahead, reconcile —
# never force-push:
git pull --no-rebase
git push origin main
```

### Workflow 3: Emergency bypass

```bash
# Last resort, one invocation only:
ICG_DISABLED=1 <dangerous-command>
```

`ICG_DISABLED` is an operator-controlled escape hatch that disables
enforcement for a single invocation. It prints a warning and must be
justified afterward — export the denial record for the review:

```bash
icg export-denial <telemetry-id> > incident.txt
```

---

## Troubleshooting

### icg doesn't seem to be running

```bash
# Validate the configured hook
icg health --check-hooks

# Test the hook exactly as the harness invokes it
echo '{"toolName":"Bash","toolInput":{"command":"bao kv destroy secret/test"}}' \
  | icg hook
```

If the hook returns `"permissionDecision":"allow"` for that input, the pack
directory is not loading: the hook fails open when `/etc/icg/packs` is
absent. Check `icg coverage --list` and
[Step 1](#step-1-install-rule-packs).

### A command was wrongly denied

```bash
icg explain --pattern <pattern-id> --show-redirect
```

If it is a false positive, file an issue:

```bash
gh issue create \
  --title "False positive: <pattern-id>" \
  --body "Command was: <command>" \
  --repo jedarden/irreversible-command-gate
```

### Rule packs not loading

```bash
# Verify the directory and its ownership
ls -la /etc/icg/packs/

# List what icg can actually see
icg coverage --list --pack /etc/icg/packs

# Fix drifted ownership; the pack directory stays root-owned
sudo chown -R root:root /etc/icg/packs && sudo chmod -R a=rX /etc/icg/packs
```

More depth: `docs/operators/troubleshooting.md`.

---

## Quick Reference

```bash
icg --version                     # icg 0.1.0
icg coverage --list               # list loaded rule packs
icg check --command "<cmd>"       # test a command string
icg check --stdin                 # test a PreToolUse JSON document
icg check --file <file-or-dash>   # test file content ('-' reads stdin)
icg explain --pattern <id>        # explain a pattern (--show-redirect, --show-regex)
icg hook                          # hook mode (harnesses; --rule-pack)
icg status --denials --since 1h   # denial history (--pattern-summary, --format json)
icg health --check-hooks          # validate hook + packs (--verbose)
icg update --check-only           # check for pack updates
```

Outcomes: `ALLOW` (no rule matched) · `REWRITE` (safe alternative supplied) ·
`DENIED` (blocked with explanation). Emergency escape hatch:
`ICG_DISABLED=1`.

---

## Next Steps

- **Deployment**: `docs/operators/deployment-guide.md` — the full production
  procedure (ownership model, trust pointers, updater, offline bootstrap)
- **Operator guide**: `docs/operators/README.md`
- **Denial messages**: `docs/operators/deny-messages.md`
- **Troubleshooting**: `docs/operators/troubleshooting.md`
- **Training**: `docs/operators/training-manual.md`
- **Examples**: `docs/examples/README.md`
- **Onboarding**: `docs/onboarding-guide.md`

Advanced surfaces, each with its own subcommand help: repository exceptions
(`icg override`), authoring packs (`icg new-pack`, plus
`docs/developers/rule-pack-best-practices.md`), trusted release references
(`icg trust`), and PATH-wrapper symlinks (`icg install`).

## Support

- **Documentation**: `docs/` directory
- **Issues**: https://github.com/jedarden/irreversible-command-gate/issues

Before asking: check the denial message (it names pack and pattern), run
`icg explain --pattern <id> --show-redirect`, and gather the version, OS,
and exact command.

---

**Quick Start Guide Version**: 3.0
**Last Updated**: 2026-08-25
**For**: icg v0.1.0
