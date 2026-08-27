# Cascading Repair Strategies for Starvation Recovery

## Overview

The Cascading Repair Service is an escalation mechanism that triggers automatically when primary auto-repair fails (i.e., when the ready bead count remains at 0 after standard repair attempts). It implements a multi-stage recovery strategy to diagnose and resolve starvation conditions that standard repairs cannot fix.

## Problem Statement

When the bead frontier consistency service detects that open/in_progress beads exist but `bead list --ready` returns 0 results, the workspace is in a starvation state. Primary repairs (assignment clearing, bead doctor) may not resolve this if the underlying issues are more complex. The cascading repair service provides a systematic escalation path.

## Strategy Sequence

The cascading repair service executes strategies in sequence, stopping as soon as any strategy succeeds in making beads visible:

### 1. Aggressive Dependency Pruning (24+ hour stale blockers)

**When it runs:** First strategy, if enabled.

**What it does:**
- Identifies dependencies where the blocking bead hasn't been updated in >24 hours
- Removes these stale blocking dependencies
- Logs all dependency removals to `.beads/diagnostics/cascading-repair.jsonl`

**Why it helps:** Stale blockers often represent abandoned work or circular dependencies where the blocker will never complete. Removing these dependencies unblocks dependent beads.

**Safety considerations:**
- Only removes dependencies where the blocker is >24 hours stale
- Logs all actions before execution
- Supports dry-run mode for testing

**Configuration:**
- Enable/disable: `ICG_DEPENDENCY_PRUNING_ENABLED=true/false` (default: true)
- Threshold: `ICG_STALE_THRESHOLD_HOURS=N` (default: 24)

### 2. Emergency Assignee Clearing (Human-labeled beads)

**When it runs:** Second strategy, if enabled and strategy 1 didn't succeed.

**What it does:**
- Finds all open/in_progress beads with the 'human' label that have assignees
- Clears the assignee from these beads
- Logs all assignee clears to `.beads/diagnostics/cascading-repair.jsonl`

**Why it helps:** Beads labeled 'human' may be assigned to workers that are no longer active but not detected by the standard assignment repair (e.g., human workers, ad-hoc processes). Clearing these assignments returns the beads to the ready frontier.

**Safety considerations:**
- Only affects beads explicitly labeled 'human'
- Logs all actions before execution
- Supports dry-run mode for testing

**Configuration:**
- Enable/disable: `ICG_ASSIGNEE_CLEARING_ENABLED=true/false` (default: true)

### 3. Query Filter Relaxation (Diagnostic)

**When it runs:** Third strategy, if enabled and strategies 1-2 didn't succeed.

**What it does:**
- Queries all open/in_progress beads
- Compares against ready frontier
- Identifies beads excluded by label filters or other query constraints
- Logs recommendations for manual intervention

**Why it helps:** This is primarily a diagnostic strategy. It identifies why beads are being filtered out and recommends specific label removal or other manual actions.

**Safety considerations:**
- Diagnostic only - does not modify bead state
- Provides specific recommendations for each excluded bead
- Supports dry-run mode for testing

**Configuration:**
- Enable/disable: `ICG_FILTER_RELAXATION_ENABLED=true/false` (default: true)

### 4. Bead State Reset (48+ hour inactive beads)

**When it runs:** Fourth strategy (last resort), if enabled and strategies 1-3 didn't succeed.

**What it does:**
- Identifies beads with no activity (updated_at) in >48 hours
- Resets these beads to open/unassigned state
- Logs all state resets to `.beads/diagnostics/cascading-repair.jsonl`

**Why it helps:** Beads that haven't been touched in 48+ hours may be in a corrupted or stuck state. Resetting them to a clean open state can resolve visibility issues.

**Safety considerations:**
- Only affects beads >48 hours inactive
- Logs all actions before execution
- Supports dry-run mode for testing
- May lose some context (assignee, status)

**Configuration:**
- Enable/disable: `ICG_STATE_RESET_ENABLED=true/false` (default: true)
- Threshold: `ICG_INACTIVE_THRESHOLD_HOURS=N` (default: 48)

## Integration with Frontier Consistency Service

The cascading repair service is automatically triggered by the frontier consistency service when:

1. Primary auto-repair has completed
2. The ready bead count is 0
3. Discrepancies exist (open/in_progress beads not in ready frontier)

## Configuration

### Environment Variables

```bash
# Core settings
ICG_WORKSPACE_PATH=/path/to/workspace
ICG_DRY_RUN=true/false  # Default: false

# Strategy enable/disable
ICG_DEPENDENCY_PRUNING_ENABLED=true/false    # Default: true
ICG_ASSIGNEE_CLEARING_ENABLED=true/false     # Default: true
ICG_FILTER_RELAXATION_ENABLED=true/false    # Default: true
ICG_STATE_RESET_ENABLED=true/false           # Default: true

# Thresholds
ICG_STALE_THRESHOLD_HOURS=24     # Default: 24
ICG_INACTIVE_THRESHOLD_HOURS=48   # Default: 48
```

### Configuration File

The service can also be configured programmatically:

```rust
use icg::cascading_repair::{CascadingRepairConfig, CascadingRepairService};

let config = CascadingRepairConfig {
    workspace_path: PathBuf::from("/path/to/workspace"),
    dependency_pruning_enabled: true,
    assignee_clearing_enabled: true,
    filter_relaxation_enabled: true,
    state_reset_enabled: true,
    stale_threshold_hours: 24,
    inactive_threshold_hours: 48,
    dry_run: false,
};

let mut service = CascadingRepairService::new(config);
let report = service.execute_cascading_repair()?;
```

## Logging and Monitoring

### Diagnostic Log

All cascading repair actions are logged to `.beads/diagnostics/cascading-repair.jsonl`:

```json
{
  "timestamp": "2026-08-26T21:30:00.000Z",
  "ready_beads_before": 0,
  "ready_beads_after": 5,
  "duration_seconds": 12.5,
  "strategies": [
    {
      "strategy_name": "dependency_pruning",
      "timestamp": "2026-08-26T21:30:05.000Z",
      "executed": true,
      "beads_affected": 3,
      "actions": [
        "Removing dependency: bead-123 blocked by bead-456 (blocker last updated: 2026-08-25T10:00:00Z)",
        "Removing dependency: bead-789 blocked by bead-456 (blocker last updated: 2026-08-25T10:00:00Z)"
      ],
      "success": true,
      "newly_visible_beads": 5,
      "error": null
    }
  ],
  "overall_success": true,
  "recommendations": []
}
```

### Events Log

Cascading repair events are also emitted to `.beads/events.jsonl` for monitoring dashboards:

```json
{
  "issue_id": "cascading-repair",
  "kind": "cascading_repair_execution",
  "actor": "icg-cascading-repair-service",
  "time": "2026-08-26T21:30:00.000Z",
  "detail": {
    "ready_beads_before": 0,
    "ready_beads_after": 5,
    "duration_seconds": 12.5,
    "strategies_executed": 1,
    "overall_success": true
  }
}
```

## Deployment

The cascading repair service is integrated into the frontier consistency service and runs automatically when needed. No separate deployment is required.

## Testing

### Dry Run Mode

For testing without making changes:

```bash
ICG_DRY_RUN=true cargo run --bin frontier-consistency-service
```

### Unit Tests

```bash
cargo test --lib cascading_repair
```

### Integration Tests

The service can be tested manually:

```bash
# Trigger a cascading repair cycle
cargo run --bin frontier-consistency-service -- --once

# Check the logs
cat .beads/diagnostics/cascading-repair.jsonl | jq .
```

## Troubleshooting

### Cascading repair didn't trigger

**Check:**
- Verify ready bead count was 0 after primary repairs
- Verify discrepancies were found
- Check logs for "STARVATION DETECTED" message

### Strategy failed to execute

**Check:**
- Verify the strategy is enabled in configuration
- Check logs for specific error messages
- Verify database access and permissions

### Beads still invisible after cascading repair

**Check:**
- Review the recommendations in the cascading repair report
- Run `bead doctor` manually on specific beads
- Check for systemic issues (database corruption, CLI version mismatch)

## Safety and Recovery

### Reversibility

All cascading repair actions are reversible:
- Dependency removal: Can be re-added with `bead dep add`
- Assignee clearing: Bead can be reassigned
- State reset: Bead history is preserved in the database

### Monitoring

Monitor cascading repair frequency:
- Frequent triggers may indicate underlying systemic issues
- Review logs for patterns in which strategies succeed
- Consider adjusting thresholds if strategies are too aggressive

## Future Enhancements

Potential improvements:
- More sophisticated dependency cycle detection
- Machine learning for predicting optimal strategy order
- Integration with external monitoring systems
- Custom strategy plugins
