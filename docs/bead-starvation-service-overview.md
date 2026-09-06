# Bead Starvation Detection and Auto-Repair Service

## Overview

Automated service for detecting bead starvation and repairing stuck assignment states. Runs as a continuous Kubernetes Deployment with internal scheduling loop.

## Problem Statement

Bead starvation occurs when `bead list --ready` returns empty results even though open beads exist. This happens due to:

1. **Stuck assigned-but-open beads**: Beads remain assigned to inactive workers (583 beads stuck on 2026-08-16)
2. **Query filter issues**: Beads not appearing in ready frontier despite being eligible
3. **Assignment state corruption**: Improper state transitions leaving beads unclaimable

## Solution

The service combines three core capabilities:

### 1. Starvation Detection
- Monitors for zero ready beads despite open beads existing
- Queries database for stuck assigned-but-open states
- Verifies worker liveness before taking action

### 2. Automated Repair
- Clears stale assignments from dead workers
- Applies `bead update --clear-assignee` for stuck beads
- Uses `bead release` for appropriate cases
- Respects assignment timeout thresholds (default: 4 hours)

### 3. Diagnostic Publishing
- Writes repair events to `.beads/diagnostics/assignment-repair.jsonl`
- Publishes metrics to Prometheus endpoint
- Maintains audit trail of all repairs

## Architecture

### Components

1. **assignment-repair-monitor**: Core monitoring service
   - Runs on configurable interval (default: 5 minutes)
   - Detects stuck assigned-but-open beads
   - Verifies worker liveness via process table
   - Auto-repairs stuck assignments

2. **bead-integrity-check**: Verification layer
   - Compares database, checkpoint, and git views
   - Detects divergences and corruption
   - Provides repair recommendations

### Deployment

**Namespace**: `assignment-repair-monitor`

**Image**: `ronaldraygun/assignment-repair-monitor:0.1.0`

**Environment Variables**:
- `ICG_WORKSPACE_PATH`: `/workspace`
- `ICG_CHECK_INTERVAL_SECONDS`: `300`
- `ICG_AUTO_REPAIR_ENABLED`: `true`
- `ICG_MONITOR_HOST`: `0.0.0.0`
- `ICG_MONITOR_PORT`: `9096`
- `DB_PATH`: `/workspace/.beads/beads.db`
- `ASSIGNMENT_TIMEOUT_HOURS`: `4`

**Storage**:
- Persistent volume claim: `bead-workspace-pvc` (10Gi, sata)
- EmptyDir for diagnostics: 100Mi

## Implementation Details

### Detection Logic

```sql
-- Find stuck assigned-but-open beads
SELECT id, assignee, updated_at 
FROM issues 
WHERE base_status = 'open' 
  AND assignee IS NOT NULL
  AND updated_at < datetime('now', '-4 hours');
```

### Worker Liveness Check

```bash
# Check if worker process is still running
ps aux | grep -q "needle.*${assignee}"
```

### Repair Actions

1. **Verify worker is dead**: Confirm process not running
2. **Clear assignment**: `bead update <id> --clear-assignee`
3. **Log event**: Write to `.beads/diagnostics/assignment-repair.jsonl`
4. **Publish metrics**: Update Prometheus counters

## Monitoring

### Health Endpoints

- `/health/live`: Liveness probe
- `/health/ready`: Readiness probe
- `/metrics`: Prometheus metrics

### Key Metrics

- `assignment_repair_total`: Total repairs performed
- `assignment_repair_success`: Successful repairs
- `stuck_assignments_current`: Current stuck count
- `starvation_detected`: Starvation events

## Files

- `containers/assignment-repair-monitor/Dockerfile`: Container definition
- `containers/bead-starvation-repair/Dockerfile`: Enhanced container with integrity checks
- `declarative-config/k8s/rs-manager/assignment-repair-monitor/deployment.yaml`: Kubernetes deployment
- `declarative-config/k8s/rs-manager/assignment-repair-monitor/namespace.yaml`: Namespace definition

## Usage

### Deploy to Cluster

```bash
kubectl apply -f declarative-config/k8s/rs-manager/assignment-repair-monitor/
```

### Manual Testing

```bash
# Run single check
assignment-repair-monitor --once

# With custom interval
assignment-repair-monitor --check-interval 600

# Dry run (no changes)
assignment-repair-monitor --dry-run
```

## Safety Features

- **Worker verification**: Only clears assignments from confirmed dead workers
- **Timeout thresholds**: Respects assignment age limits (4 hour default)
- **Idempotent operations**: Safe to run multiple times
- **Audit logging**: Complete repair history in `.beads/diagnostics/`
- **Dry-run mode**: Test without making changes

## Related Services

- **bead-integrity-monitor**: Database integrity verification
- **starvation-diagnostic**: Diagnostic tool for starvation root cause analysis
- **frontier-consistency-check**: Bead frontier consistency verification

## Pattern Reference

This service follows the established pattern from commit `deab209` (automated bead frontier consistency checker and repair service), using the same deployment model and internal scheduling loop approach.
