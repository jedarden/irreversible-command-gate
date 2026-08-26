# Bead Dependency Validator

## Overview

The Bead Dependency Validator is a standalone tool that detects and fixes circular dependencies and orphaned references in bead-rs databases. These issues are common causes of **bead starvation** - when `bead list --ready` returns no results even though open beads exist.

## Problem Description

Bead starvation occurs when the dependency graph becomes malformed, preventing any beads from appearing on the ready frontier. Common causes:

### 1. Circular Dependencies
```
A blocks B → B blocks C → C blocks A
```
In this cycle, every bead is blocked by another, so none become ready.

### 2. Orphaned Dependencies
A bead is blocked by a bead that:
- Doesn't exist in the database (deleted externally)
- Has been closed (but the dependency wasn't cleaned up)

### 3. Assigned-but-Open Beads
A bead has `status=open` but a non-null `assignee` field. These beads are invisible to `bead list --ready` because the filter excludes assigned beads, but they're not actually being worked on.

## Usage

### Build
```bash
cd /home/coding/irreversible-command-gate
cargo build --bin bead-dependency-validator
```

### Run (from any bead workspace)
```bash
# Dry run - detect issues without fixing them
/home/coding/target/debug/bead-dependency-validator --dry-run

# Fix issues
/home/coding/target/debug/bead-dependency-validator

# With custom paths
/home/coding/target/debug/bead-dependency-validator \
  --db-path /path/to/.beads/beads.db \
  --events-path /path/to/.beads/events.jsonl
```

### Integration with Starvation Alerts
When a starvation alert is detected, run this tool automatically:

```bash
# In a script that monitors for starvation
if bead list --ready | grep -q "No beads"; then
    /home/coding/target/debug/bead-dependency-validator
    bead sync flush-only
fi
```

## What It Does

### 1. Loads All Beads and Dependencies
- Queries the `issues` table for all beads
- Queries the `dependencies` table for all blocking relationships

### 2. Detects Circular Dependencies
- Uses DFS cycle detection to find cycles in the dependency graph
- For each cycle, identifies the "youngest" bead (by creation time)
- Removes the blocking edge pointing to the youngest bead
- Logs the fix to `events.jsonl`

### 3. Detects Orphaned Dependencies
- Finds beads blocked by non-existent beads
- Finds beads blocked by closed beads
- Removes the orphaned dependency
- Logs the fix to `events.jsonl`

### 4. Logs All Fixes
All fixes are written to `events.jsonl` with:
- Event type: `dependency_fix`
- Actor: `bead-dependency-validator`
- Detail: Full JSON of the issue and fix applied

## Exit Codes

- `0`: Success (no issues found)
- `1`: Error occurred during validation/fix
- `2`: Issues found and fixed (informational, indicates fixes were applied)

## Examples

### Before Fix
```bash
$ bead list --ready
No beads match the specified criteria

$ /home/coding/target/debug/bead-dependency-validator
=== Bead Dependency Validator Summary ===
Total beads checked: 166
Open beads: 3
Issues found: 533
Fixes applied: 533
✅ All issues fixed successfully!
```

### After Fix
```bash
$ bead list --ready | jq length
13

$ /home/coding/target/debug/bead-dependency-validator
=== Bead Dependency Validator Summary ===
Total beads checked: 166
Open beads: 3
Issues found: 0
Fixes applied: 0
✅ No issues found - dependency graph is healthy!
```

## Technical Details

### Database Schema
The tool reads from the bead-rs SQLite database:

**Table: `issues`**
```sql
CREATE TABLE issues (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    base_status TEXT NOT NULL CHECK (base_status IN ('open', 'in_progress', 'deferred', 'closed')),
    created_at TEXT NOT NULL,
    assignee TEXT,
    manual_blocked INTEGER NOT NULL DEFAULT 0,
    ...
);
```

**Table: `dependencies`**
```sql
CREATE TABLE dependencies (
    blocked_issue_id TEXT NOT NULL,
    blocker_issue_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('blocks', 'relates_to')),
    PRIMARY KEY (blocked_issue_id, blocker_issue_id, kind),
    FOREIGN KEY (blocked_issue_id) REFERENCES issues(id) ON DELETE CASCADE,
    FOREIGN KEY (blocker_issue_id) REFERENCES issues(id) ON DELETE CASCADE
);
```

### Circular Dependency Algorithm
```rust
// Pseudocode
for each bead in database:
    if not visited:
        dfs_cycle_detect(bead, graph, visited, recursion_stack, path, cycles)

for each cycle in cycles:
    youngest = find_youngest_bead(cycle)
    blocker_to_remove = find_who_blocks_youngest(cycle, youngest)
    delete_dependency(youngest, blocker_to_remove)
```

### Orphaned Dependency Algorithm
```rust
// Pseudocode
for each dependency in dependencies:
    blocker = beads.get(dependency.blocker_issue_id)
    if blocker == None or blocker.status == "closed":
        delete_dependency(dependency)
```

## Integration with CI/CD

This tool can be integrated as a cron job or Argo Workflow:

### Cron Job (run daily)
```cron
0 2 * * * cd /path/to/workspace && /home/coding/target/debug/bead-dependency-validator
```

### Argo Workflow (run on starvation alert)
```yaml
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: bead-dependency-validator-
spec:
  entrypoint: validate
  templates:
    - name: validate
      container:
        image: ronaldraygun/icg:latest
        command: ["/usr/local/bin/bead-dependency-validator"]
```

## Related Tools

- **bead-sync**: Flushes database state to checkpoint (`bead sync flush-only`)
- **bead-doctor**: Safe auto-repair for stale temp files and checkpoint views
- **bead-list**: Query beads (including `--ready` for workable frontier)

## See Also

- CLAUDE.md: "Beads (bead-rs CLI)" section
- NEEDLE/AGENTS.md: "Working Safely" section
- ADR-015: "Concurrent Same-Repo Worker Isolation" (worktree discussion)

## Author

Generated by the NEEDLE unravel strand as an automated alternative to manual starvation investigation.

## Version

Part of icg v0.1.0 - bead dependency validator module
