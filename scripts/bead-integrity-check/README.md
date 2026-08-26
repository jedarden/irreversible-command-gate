# Bead Database Integrity Checker

A verification tool that compares three views of bead state to detect divergences and provide automated repair recommendations.

## Overview

The checker compares bead state across:
1. **SQLite database** (`beads.db`) - the authoritative live store
2. **Checkpoint files** (`current.json`, `forensic.jsonl`, `objects/`) - git-tracked durable checkpoint
3. **Git-tracked checkpoint** - last committed checkpoint state

## Installation

Build from source:

```bash
cd scripts/bead-integrity-check
cargo build --release
```

The binary will be available at `/home/coding/target/release/bead-integrity-check`.

## Usage

Run from any directory within a bead workspace:

```bash
bead-integrity-check
```

The tool automatically searches upward for `.beads/` directory to find the workspace root.

## Exit Codes

- `0` - All views consistent (no divergences)
- `1` - Critical divergences detected (repair required)
- `2` - Non-critical divergences detected (review recommended)

## Divergence Types

### `database_ahead_of_checkpoint` (HIGH)
Beads exist in database but not in checkpoint. The checkpoint is stale and needs synchronization.

**Repair:** Run `bead sync flush-only`

### `checkpoint_ahead_of_database` (HIGH)
Beads exist in checkpoint but not in database. Possible database corruption or sync failure.

**Repair:** 
```bash
bead doctor --repair
bead sync import-only --input .beads/checkpoint/forensic.jsonl --restore-into-empty --actor <you>
```

### `git_ahead_of_checkpoint` (MEDIUM)
Beads exist in git-tracked checkpoint but not in current checkpoint. Uncommitted changes or stale checkpoint.

**Repair:** Review uncommitted changes and run `bead sync flush-only` if needed.

### `stale_assignees` (MEDIUM)
Open beads have assignees but should be unassigned (possible reopen bug).

**Repair:** Run `bead update <id> --clear-assignee` for each affected bead.

### `count_mismatch_*` (LOW/MEDIUM)
View counts differ without specific bead mismatches.

**Repair:** Run `bead sync flush-only` to resynchronize.

## Output Format

The tool outputs both human-readable and machine-readable formats:

### Human-Readable Summary
```
╔════════════════════════════════════════════════════════════╗
║         Bead Integrity Check Report                       ║
╚════════════════════════════════════════════════════════════╝

📊 View Summary:
   Database: 168 beads (2026-08-26 13:32:40 UTC)
   Checkpoint: 168 beads (2026-08-26 13:32:40 UTC)
   Git (HEAD): 146 beads (commit  /home/coding/irreversible-command-gate)
```

### Machine-Readable JSON
```json
{
  "database_view": { "bead_count": 168, ... },
  "checkpoint_view": { "bead_count": 168, ... },
  "git_view": { "bead_count": 146, ... },
  "divergences": [...],
  "recommendations": [...]
}
```

## Integration with CI/CD

Add to your CI pipeline to catch checkpoint drift before it causes issues:

```bash
#!/bin/bash
bead-integrity-check || exit_code=$?
if [ $exit_code -eq 1 ]; then
    echo "Critical divergences detected - running repair..."
    bead sync flush-only
    bead-integrity-check
fi
```

## Architecture

### Database View
- Direct SQLite query: `SELECT id, base_status, assignee, created_at, updated_at FROM issues`
- Cross-verification with `bead list --json` (JSONL format)
- Most authoritative view (live data)

### Checkpoint View
- Reads `current.json` for metadata
- Reads `forensic.jsonl` for bead records (JSONL format)
- Parses nested `issue` objects with `record_type: "issue"`
- Should match database after `bead sync flush-only`

### Git View
- Reads checkpoint files from git HEAD commit
- Uses `git2` crate to access blob objects
- Detects uncommitted checkpoint changes
- Baseline for what's actually tracked in version control

## Preventing Divergences

**Always run `git pull` before `bead sync flush-only`** to avoid conflicts between local and remote checkpoint states.

## See Also

- [CLAUDE.md - Beads (bead-rs CLI)](../../CLAUDE.md)
- [`bead doctor`](https://github.com/jedarden/bead-rs) for safe auto-repair
- [`bead sync flush-only`](https://github.com/jedarden/bead-rs) for database→checkpoint synchronization

## License

MIT License - same as parent project.
