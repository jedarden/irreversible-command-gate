# Automated Checkpoint Flush Verification and Repair

## Overview

This document describes the automated checkpoint verification and repair system that prevents bead starvation caused by checkpoint desynchronization between `beads.db` and `.beads/checkpoint/`.

## Problem Statement

Bead starvation occurs when `bead list --ready` returns zero candidates despite open beads existing in the database. A common root cause is checkpoint sync failure where the database state diverges from the durable checkpoint.

### The Starvation Scenario

1. **Checkpoint becomes stale**: The `.beads/checkpoint/current.json` file is not updated after database mutations
2. **Drift accumulates**: Beads exist in the database but are missing from the checkpoint, or vice versa
3. **Queries fail**: `bead list --ready` relies on checkpoint data; stale checkpoint = invisible beads
4. **Workers starve**: No beads are returned from the ready frontier, workers spin idle

## Solution Architecture

The checkpoint monitor implements **bead-level drift detection** with automatic repair:

### 1. Independent Checkpoint Parsing

The monitor parses checkpoint files directly from disk, independent of the bead CLI:

```rust
// Extract bead IDs from forensic.jsonl (authoritative checkpoint)
fn extract_checkpoint_bead_ids(&self) -> Result<Vec<String>>

// Extract bead IDs from database via bead CLI
fn extract_database_bead_ids(&self) -> Result<Vec<String>>
```

### 2. Bead-Level Comparison

Instead of just comparing **counts** (which misses drift), the monitor compares **individual bead IDs**:

```rust
// Find beads in checkpoint but missing in database
beads_missing_in_database: Vec<String>

// Find beads in database but missing in checkpoint  
beads_missing_in_checkpoint: Vec<String>

// Total drift count
drift_count: usize
```

### 3. Automatic Repair Actions

The monitor performs deterministic repairs based on detected issues:

#### Priority 1: Corrupted Database
- **Detection**: Database exists but is unreadable or schema-invalid
- **Repair**: `bead doctor --repair`
- **Safety**: Uses bead-rs's built-in recovery mechanisms

#### Priority 2: Corrupted or Missing Checkpoint
- **Detection**: Checkpoint file missing, JSON invalid, or forensic.jsonl corrupted
- **Repair**: `bead sync import-only --restore-into-empty --input .beads/checkpoint/forensic.jsonl --actor checkpoint-monitor`
- **Safety**: Rebuilds from authoritative checkpoint, never loses data

#### Priority 3: Stale Checkpoint
- **Detection**: Checkpoint timestamp exceeds threshold (default: 5 minutes)
- **Repair**: `bead sync flush-only`
- **Safety**: Brings checkpoint up-to-date with database state

#### Priority 4: Bead-Level Drift
- **Detection**: Beads present in checkpoint but missing in database (or vice versa)
- **Repair**: `bead sync flush-only` (same as stale checkpoint)
- **Safety**: Flush re-synchronizes checkpoint from current database state

### 4. Diagnostic Reporting

Every check publishes a structured report to `.beads/diagnostics/checkpoint-report.jsonl`:

```json
{
  "timestamp": "2026-08-26T22:36:50.569413001+00:00",
  "health_status": "healthy",
  "checkpoint_sync": {
    "sync_status": "desynchronized",
    "drift_count": 107,
    "beads_missing_in_database": ["irrevers-01096272", "irrevers-012be0c8", ...],
    "beads_missing_in_checkpoint": []
  },
  "database_health": {
    "exists": true,
    "readable": true,
    "corrupted": false
  },
  "repair_triggered": false,
  "recommended_actions": [
    "107 beads in checkpoint but missing in database. Database may be out of sync. Run: bead sync flush-only"
  ]
}
```

## Usage

### Manual Verification

Run a single check with verbose output:

```bash
/home/coding/target/release/checkpoint-monitor --once --verbose
```

Output includes:
- Overall health status (healthy, degraded, critical)
- Checkpoint sync status with drift details
- Individual bead IDs that differ (first 5 shown in verbose mode)
- Recommended repair actions

### Continuous Monitoring

Run the monitor in a loop with 5-minute intervals:

```bash
/home/coding/target/release/checkpoint-monitor --interval-secs 300
```

### Systemd Service

The weekly automated service is configured in `systemd/icg-checkpoint-monitor.{service,timer}`:

```bash
# Enable the weekly timer
sudo systemctl enable icg-checkpoint-monitor.timer
sudo systemctl start icg-checkpoint-monitor.timer

# Check status
systemctl status icg-checkpoint-monitor.timer
systemctl status icg-checkpoint-monitor.service

# View logs
journalctl -u icg-checkpoint-monitor.service -f
```

**Schedule**: Weekly on Sunday at 3 AM (`OnCalendar=Sun 03:00`)

## Verification vs Repair Modes

### Verification-Only Mode

For diagnostic purposes without automatic repair:

```bash
/home/coding/target/release/checkpoint-monitor --once --no-repair --verbose
```

This mode:
- Detects all issues (drift, staleness, corruption)
- Generates recommendations instead of performing repairs
- Exits with code 1 if critical issues found

### Auto-Repair Mode

Default mode that performs automatic repairs:

```bash
/home/coding/target/release/checkpoint-monitor --once
```

This mode:
- Detects issues and repairs them automatically
- Logs all repairs to `.beads/events.jsonl` for audit trail
- Continues monitoring until interrupted

## Safety Guarantees

### Data Loss Prevention

1. **Never rebuild database unless corrupted**: The monitor only runs `bead sync import-only --restore-into-empty` if the database is confirmed corrupted or schema-invalid
2. **Checkpoint is authoritative**: When rebuilding is necessary, the checkpoint (git-tracked) is the source of truth
3. **Idempotent repairs**: All repair actions are safe to run multiple times
4. **Audit trail**: Every repair is logged to events.jsonl with timestamp and actor

### Rollback Safety

- Checkpoint rebuild uses `forensic.jsonl` which contains the complete bead history
- Database rebuild uses checkpoint which is git-tracked in `.beads/checkpoint/`
- Both operations are standard bead-rs recovery procedures

## Integration with Bead-Rs

This monitor integrates with bead-rs's checkpoint system:

- **R026 activation** (2026-08-21): Bead-rs automatically publishes checkpoint after every mutation
- **Manual flush**: `bead sync flush-only` forces database → checkpoint synchronization
- **Standard recovery**: `bead sync import-only --restore-into-empty` rebuilds from checkpoint

The monitor augments these mechanisms with:
- Proactive detection (before starvation occurs)
- Bead-level granularity (finds drift that count-based checks miss)
- Automated remediation (no human intervention required)

## Monitoring and Alerting

### Prometheus Metrics

The monitor exports Prometheus metrics for monitoring:

```bash
# Health status
icg_checkpoint_monitor_healthy{status="healthy|degraded|critical"} 1

# Sync status
icg_checkpoint_sync_status{status="synchronized|stale|desynchronized"} 1
icg_checkpoint_stale 0 or 1
icg_checkpoint_stale_minutes 5

# Drift metrics
icg_checkpoint_issue_count 207
icg_database_issue_count 100
icg_checkpoint_corrupted 0 or 1
icg_database_corrupted 0 or 1

# Repair metrics
icg_checkpoint_repair_triggered 0 or 1
icg_checkpoint_repairs_performed 2
```

### Log Monitoring

Monitor logs for repair events:

```bash
journalctl -u icg-checkpoint-monitor.service -f | grep -E "(🔧|✅|❌)"
```

### Alert Thresholds

Recommended alerting thresholds:
- **Warning**: Drift count > 10 beads
- **Critical**: Drift count > 50 beads OR checkpoint corrupted
- **Emergency**: Database corrupted

## Troubleshooting

### Checkpoint Shows Drift But No Repairs Occurred

**Cause**: Auto-repair disabled or monitor in verification-only mode

**Solution**: 
```bash
# Check if auto-repair is enabled
/home/coding/target/release/checkpoint-monitor --once --verbose

# Run manual repair if needed
bead sync flush-only
```

### Monitor Reports "Corrupted Database"

**Cause**: Database schema invalid or file corrupted

**Solution**:
```bash
# Attempt automatic repair via bead doctor
bead doctor --repair

# If that fails, rebuild from checkpoint
bead sync import-only --restore-into-empty \
  --input .beads/checkpoint/forensic.jsonl \
  --actor <your-name>
```

### Monitor Reports "Corrupted Checkpoint"

**Cause**: Checkpoint JSON invalid or forensic.jsonl corrupted

**Solution**:
```bash
# Rebuild checkpoint from forensic.jsonl
bead sync import-only --restore-into-empty \
  --input .beads/checkpoint/forensic.jsonl \
  --actor checkpoint-monitor
```

### Weekly Timer Not Running

**Cause**: Systemd timer not enabled or service failed

**Solution**:
```bash
# Check timer status
systemctl status icg-checkpoint-monitor.timer

# Enable if not enabled
sudo systemctl enable icg-checkpoint-monitor.timer
sudo systemctl start icg-checkpoint-monitor.timer

# Check service logs
journalctl -u icg-checkpoint-monitor.service -n 50
```

## Implementation Details

### Bead-Level Comparison Algorithm

```rust
// Extract bead IDs from both sources
let checkpoint_bead_ids = extract_checkpoint_bead_ids()?;
let database_bead_ids = extract_database_bead_ids()?;

// Find symmetric difference
let beads_missing_in_database: Vec<String> = checkpoint_bead_ids
    .iter()
    .filter(|id| !database_bead_ids.contains(id))
    .cloned()
    .collect();

let beads_missing_in_checkpoint: Vec<String> = database_bead_ids
    .iter()
    .filter(|id| !checkpoint_bead_ids.contains(id))
    .cloned()
    .collect();

let drift_count = beads_missing_in_database.len() + beads_missing_in_checkpoint.len();
```

### Repair Priority System

```rust
if database_health.corrupted {
    // Priority 1: Fix database first
    repair_database_from_checkpoint();
} else if checkpoint_sync.corrupted {
    // Priority 2: Fix checkpoint
    repair_checkpoint_from_forensic();
} else if checkpoint_sync.stale || checkpoint_sync.drift_count > 0 {
    // Priority 3: Flush to resync
    repair_stale_checkpoint();
}
```

## Future Enhancements

Potential improvements to consider:

1. **Configurable drift threshold**: Allow users to set acceptable drift limits before auto-repair
2. **Differential repair**: Only sync the specific beads that differ, not full flush
3. **Historical drift tracking**: Track drift patterns over time to identify root causes
4. **Multi-repo support**: Monitor multiple workspaces from a single service
5. **Webhook alerts**: Send alerts to external systems on critical issues

## References

- **Bead-rs checkpoint system**: `.beads/checkpoint/` directory structure
- **Events log**: `.beads/events.jsonl` for audit trail
- **Diagnostic reports**: `.beads/diagnostics/checkpoint-report.jsonl`
- **Systemd units**: `systemd/icg-checkpoint-monitor.{service,timer}`

## Version History

- **2026-08-26**: Enhanced with bead-level drift detection (previously count-based only)
- **2026-08-21**: Initial implementation with basic checkpoint monitoring
