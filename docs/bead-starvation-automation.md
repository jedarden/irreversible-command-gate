# Bead Starvation Automation

This directory contains automated tools for detecting and repairing bead starvation issues in NEEDLE workspaces.

## Problem Background

On 2026-08-16, a fleet-wide sweep found **583 beads stuck** in the "assigned-but-open" state across 47 of 66 workspaces, with ten workspaces fully starved (`bead ready --status open` returning zero while live workers spun).

### Root Cause

Beads in the "assigned-but-open" state (`status=open` AND `assignee` is set) are invisible to the ready frontier because `bead ready --status open` filters out assigned beads, but they're not actually `in_progress`. This happens when:

1. A worker claims a bead (`bead claim`)
2. The worker crashes or exits without releasing the bead
3. The bead remains assigned but not in progress, permanently invisible to `bead ready`

### Automation Strategy

The automation follows a three-tier approach:

1. **Detection** (`bead-starvation-alert-generator.sh`) - Creates alert beads with embedded diagnostics
2. **Analysis** (`bead-starvation-diagnostic.sh`) - Direct database scan to categorize issues  
3. **Repair** (`bead-assignee-stuck-sweeper.sh`) - Automated sweeper with worker liveness detection

## Scripts

### `bead-assignee-stuck-sweeper.sh` (NEW)

**Purpose:** Automated assignee-stuck-bead sweeper with worker liveness verification

**Features:**
- Scans all open beads for assigned-but-open state
- Verifies assignee worker liveness by checking recent event activity
- Clears stale assignees automatically
- Configurable worker staleness threshold (default: 24 hours)
- Dry-run mode for safe testing
- Generates diagnostic JSON reports

**Usage:**
```bash
# Run with default settings (24-hour staleness threshold)
./scripts/bead-assignee-stuck-sweeper.sh

# Dry run to see what would be repaired
./scripts/bead-assignee-stuck-sweeper.sh --dry-run

# Custom staleness threshold (e.g., 12 hours)
./scripts/bead-assignee-stuck-sweeper.sh --worker-stale-hours 12
```

**Environment Variables:**
- `DB_PATH` - Path to beads.db (default: `/.beads/beads.db`)
- `OUTPUT_DIR` - Diagnostic output directory (default: `/.beads/diagnostics`)
- `WORKER_STALE_HOURS` - Hours before worker considered stale (default: `24`)
- `DRY_RUN` - Set to `true` for dry-run mode (default: `false`)

**Output:**
- Console: Colored progress messages and summary
- File: `/.beads/diagnostics/assignee-stuck-sweeper-<timestamp>.json`

**Example Report:**
```json
{
  "timestamp": "2026-08-26T12:52:14Z",
  "workspace": "irreversible-command-gate",
  "configuration": {
    "worker_stale_hours": 24,
    "dry_run": false
  },
  "summary": {
    "total_assigned_open": 5,
    "active_workers": 3,
    "stale_workers": 2,
    "repaired": 2,
    "failed": 0
  },
  "stale_beads": [
    {"id": "abc123", "assignee": "worker-old", "title": "Fix bug", "reason": "stale_48h"},
    {"id": "def456", "assignee": "worker-gone", "title": "Add feature", "reason": "no_activity"}
  ]
}
```

### `bead-starvation-diagnostic.sh`

**Purpose:** Direct database scan to categorize starvation causes

**Features:**
- Queries SQLite database directly for complete visibility
- Categorizes beads: truly_blocked, stuck_state, unknown
- Auto-repairs stuck-state beads (assigned-but-open)
- Reports blocking dependency chains
- Generates diagnostic JSON reports

**Usage:**
```bash
./scripts/bead-starvation-diagnostic.sh
```

**Limitations:**
- Does NOT verify worker liveness (clears all assigned-but-open beads)
- Use `bead-assignee-stuck-sweeper.sh` instead for safer automated repair

### `bead-starvation-auto-repair.sh`

**Purpose:** Process starvation alert beads with embedded diagnostics

**Features:**
- Extracts diagnostic context from alert bead bodies
- Executes automated repairs based on diagnostic data
- Supports dry-run mode
- Integrates with bead-dependency-validator

**Usage:**
```bash
./scripts/bead-starvation-auto-repair.sh <bead_id> [--dry-run]
```

### `bead-starvation-alert-generator.sh`

**Purpose:** Generate starvation alert beads with embedded diagnostics

**Usage:**
```bash
./scripts/bead-starvation-alert-generator.sh
```

## Deployment as Periodic Job

### Option 1: Cron Job (Local)

Add to crontab for daily execution:

```bash
# Daily at 2 AM UTC
0 2 * * * /home/coding/irreversible-command-gate/scripts/bead-assignee-stuck-sweeper.sh >> /var/log/bead-sweeper.log 2>&1
```

### Option 2: Argo Workflow (Cluster)

**IMPORTANT:** This requires a container image with the bead CLI installed. DO NOT use `:latest` tags.

**Prerequisites:**
1. Create a utility container with bead-rs CLI installed
2. Pin to specific semver tag from `containers/<name>/VERSION`
3. Add WorkflowTemplate to `declarative-config/k8s/iad-ci/argo-workflows/`

**Example WorkflowTemplate structure:**
```yaml
apiVersion: argoproj.io/v1alpha1
kind: WorkflowTemplate
metadata:
  name: bead-starvation-sweeper
  namespace: argo-workflows
  annotations:
    description: "Automated sweeper to detect and repair assigned-but-open beads"
spec:
  entrypoint: sweep-workspaces
  templates:
  - name: sweep-workspaces
    steps:
    - - name: sweep-workspace
        template: sweep-single-workspace
        arguments:
          parameters:
          - name: workspace
            value: "{{item}}"
        withItems:
        - irreversible-command-gate
        - NEEDLE
        - SEAM

  - name: sweep-single-workspace
    inputs:
      parameters:
      - name: workspace
    container:
      # TODO: Create and version this image properly
      # Check containers/bead-sweeper/VERSION for actual tag
      image: ronaldraygun/bead-sweeper:v1.0.0
      command: ["/bin/bash"]
      args:
      - "-c"
      - |
        WORKSPACE="{{inputs.parameters.workspace}}"
        WORKSPACE_PATH="/home/coding/${WORKSPACE}"
        SCRIPT_PATH="${WORKSPACE_PATH}/scripts/bead-assignee-stuck-sweeper.sh"

        if [ ! -f "$SCRIPT_PATH" ]; then
          echo "Sweeper script not found, skipping ${WORKSPACE}"
          exit 0
        fi

        cd "$WORKSPACE_PATH"
        export DB_PATH="${WORKSPACE_PATH}/.beads/beads.db"
        export OUTPUT_DIR="${WORKSPACE_PATH}/.beads/diagnostics"
        export WORKER_STALE_HOURS="24"

        "$SCRIPT_PATH"
      volumeMounts:
      - name: home-coding
        mountPath: /home/coding
        readOnly: true
    volumes:
    - name: home-coding
      hostPath:
        path: /home/coding
        type: Directory
```

**Note:** For immediate deployment, use Option 1 (cron job) or Option 3 (NEEDLE integration) instead.

### Option 3: NEEDLE Fleet Integration

Create a NEEDLE bead that runs the sweeper periodically:

```bash
bead create \
  --title "Daily bead starvation sweep" \
  --priority 3 \
  --issue-type task \
  --label automation \
  --label recurring
```

Then create a cron job that claims and closes this bead daily, running the sweeper as part of the work.

## Monitoring and Alerting

### Log Aggregation

Scripts output structured JSON to `.beads/diagnostics/`. Monitor these files:

```bash
# Recent sweeper reports
ls -lt ~/.beads/diagnostics/assignee-stuck-sweeper-*.json | head -5

# Check for increasing stale counts
jq -r '.summary.stale_workers' ~/.beads/diagnostics/assignee-stuck-sweeper-*.json | \
  awk '{sum+=$1} END {print "Total stale beads:", sum}'
```

### Prometheus Metrics

Add metrics export to the sweeper script for monitoring:

```bash
# Export to node exporter text file collector
cat <<EOF > /var/lib/node_exporter/textfile_collector/bead_sweeper.prom
# HELP bead_stale_workers Current number of stale workers
# TYPE bead_stale_workers gauge
bead_stale_workers{workspace="irreversible-command-gate"} $STALE_COUNT
# HELP bead_sweeper_last_success Unix timestamp of last successful sweep
# TYPE bead_sweeper_last_success gauge
bead_sweeper_last_success{workspace="irreversible-command-gate"} $(date +%s)
EOF
```

## Troubleshooting

### Script Reports No Stale Beads But `bead ready` Returns Empty

1. Check for truly blocked beads (dependencies):
   ```bash
   sqlite3 .beads/beads.db "
   SELECT i.id, i.title 
   FROM issues i
   JOIN dependencies d ON i.id = d.blocked_issue_id
   WHERE i.base_status = 'open' 
     AND d.blocker_issue_id IN (SELECT id FROM issues WHERE base_status != 'closed')
   "
   ```

2. Check for manually blocked beads:
   ```bash
   bead list --status open --json | jq '.[] | select(.manual_blocked == true)'
   ```

### Script Fails to Clear Assignee

1. Check bead revision conflicts:
   ```bash
   bead show <id> --json | jq '.[0].revision'
   ```

2. Verify database permissions:
   ```bash
   ls -la .beads/beads.db
   ```

3. Check bead CLI version:
   ```bash
   bead --version
   ```

## References

- Original incident: 2026-08-16 fleet-wide sweep (583 stuck beads across 47 workspaces)
- NEEDLE documentation: `NEEDLE/docs/adr/015-concurrent-same-repo-worker-isolation.md`
- Bead-rs CLI: `~/.local/bin/bead`
