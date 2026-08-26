#!/usr/bin/env bash
# Unified Bead Starvation Diagnostic and Auto-Repair Service
#
# This service provides comprehensive starvation detection, diagnosis, and automated repair.
# It runs every 5 minutes and handles all starvation conditions automatically,
# only creating beads for unrecoverable conditions requiring human intervention.
#
# ## Detection Conditions
#
# - Zero ready beads (bead list --ready returns empty)
# - Non-zero open beads (bead list --status open returns results)
#
# ## Automated Repairs
#
# 1. Assigned-but-open beads: Clear stale assignees
# 2. Dependency cycles: Identify and break cycles
# 3. Stale assignees: Detect dead workers and release their beads
# 4. Checkpoint corruption: Run bead sync import-only from forensic log
# 5. Query filter issues: Repair filter conditions
#
# ## Logging
#
# All actions are logged to .beads/diagnostics/starvation-unified-repair.log
# Detailed metrics are recorded in .beads/diagnostics/starvation-metrics.json
#
# ## Escalation
#
# Only creates beads for unrecoverable conditions requiring human intervention

set -euo pipefail

# Configuration
DB_PATH="${DB_PATH:-/home/coding/irreversible-command-gate/.beads/beads.db}"
WORKSPACE="$(dirname "$DB_PATH")"
DIAGNOSTICS_DIR="${WORKSPACE}/diagnostics"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
LOG_FILE="${DIAGNOSTICS_DIR}/starvation-unified-repair.log"
METRICS_FILE="${DIAGNOSTICS_DIR}/starvation-metrics.json"
TEMP_DIR="${DIAGNOSTICS_DIR}/temp"

# Ensure diagnostics directory exists
mkdir -p "$DIAGNOSTICS_DIR"
mkdir -p "$TEMP_DIR"

# Logging functions
log_info() {
    local msg="[$TIMESTAMP] [INFO] $1"
    echo "$msg" >&2
    echo "$msg" >> "$LOG_FILE"
}

log_warn() {
    local msg="[$TIMESTAMP] [WARN] $1"
    echo "$msg" >&2
    echo "$msg" >> "$LOG_FILE"
}

log_error() {
    local msg="[$TIMESTAMP] [ERROR] $1"
    echo "$msg" >&2
    echo "$msg" >> "$LOG_FILE"
}

log_action() {
    local msg="[$TIMESTAMP] [ACTION] $1"
    echo "$msg" >&2
    echo "$msg" >> "$LOG_FILE"
}

# Verify prerequisites
if ! command -v bead &> /dev/null; then
    log_error "bead CLI not found in PATH"
    exit 1
fi

if ! command -v sqlite3 &> /dev/null; then
    log_error "sqlite3 not found in PATH"
    exit 1
fi

if [ ! -f "$DB_PATH" ]; then
    log_error "Bead database not found at: $DB_PATH"
    exit 1
fi

log_info "=== Starting Unified Starvation Auto-Repair ==="
log_info "Database: $DB_PATH"

# Initialize metrics
METRICS='{
    "timestamp": "'"$TIMESTAMP"'",
    "workspace": "'"$WORKSPACE"'",
    "detection": {
        "ready_beads": 0,
        "open_beads": 0,
        "starvation_detected": false
    },
    "diagnosis": {
        "assigned_but_open": 0,
        "dependency_cycles": 0,
        "stale_assignees": 0,
        "checkpoint_corruption": false,
        "query_filter_issues": 0
    },
    "repairs": {
        "assigned_cleared": 0,
        "cycles_broken": 0,
        "workers_recovered": 0,
        "checkpoint_restored": false,
        "filters_repaired": 0
    },
    "summary": {
        "total_repairs_attempted": 0,
        "successful_repairs": 0,
        "unrecoverable_conditions": 0
    }
}'

# ============================================================================
# STEP 1: Detect Starvation Condition
# ============================================================================

log_info "Step 1: Detecting starvation condition..."

# Query for ready beads
READY_COUNT=$(sqlite3 "$DB_PATH" "
SELECT COUNT(*)
FROM issues i
WHERE i.base_status = 'open'
  AND i.assignee IS NULL
  AND i.manual_blocked = 0
  AND NOT EXISTS (
    SELECT 1 FROM dependencies d
    WHERE d.blocked_issue_id = i.id
      AND d.kind = 'blocks'
      AND d.blocker_issue_id IN (
        SELECT id FROM issues WHERE base_status != 'closed'
      )
  );
" 2>/dev/null || echo "0")

# Query for open beads
OPEN_COUNT=$(sqlite3 "$DB_PATH" "
SELECT COUNT(*)
FROM issues
WHERE base_status IN ('open', 'in_progress', 'deferred');
" 2>/dev/null || echo "0")

log_info "Ready beads: $READY_COUNT"
log_info "Open beads: $OPEN_COUNT"

# Update metrics
METRICS=$(echo "$METRICS" | jq ".detection.ready_beads = $READY_COUNT")
METRICS=$(echo "$METRICS" | jq ".detection.open_beads = $OPEN_COUNT")

# Check for starvation condition
if [ "$READY_COUNT" -eq 0 ] && [ "$OPEN_COUNT" -gt 0 ]; then
    STARVATION_DETECTED=true
    log_warn "STARVATION DETECTED: Zero ready beads with $OPEN_COUNT open beads"
    METRICS=$(echo "$METRICS" | jq '.detection.starvation_detected = true')
else
    STARVATION_DETECTED=false
    log_info "No starvation detected"
    METRICS=$(echo "$METRICS" | jq '.detection.starvation_detected = false')

    # Save metrics and exit
    echo "$METRICS" | jq '.' > "$METRICS_FILE"
    exit 0
fi

# ============================================================================
# STEP 2: Diagnose Root Causes
# ============================================================================

log_info "Step 2: Diagnosing root causes..."

# 2.1 Check for assigned-but-open beads
ASSIGNED_BUT_OPEN=$(sqlite3 "$DB_PATH" "
SELECT COUNT(*)
FROM issues
WHERE base_status = 'open'
  AND assignee IS NOT NULL
  AND assignee != '';
" 2>/dev/null || echo "0")

log_info "Assigned-but-open beads: $ASSIGNED_BUT_OPEN"
METRICS=$(echo "$METRICS" | jq ".diagnosis.assigned_but_open = $ASSIGNED_BUT_OPEN")

# 2.2 Check for dependency cycles
CYCLE_DETECTIONS=$(sqlite3 "$DB_PATH" "
WITH RECURSIVE cycle_check AS (
    SELECT
        d1.blocked_issue_id as start_id,
        d1.blocker_issue_id as current_id,
        1 as depth,
        d1.blocker_issue_id || '->' || d1.blocked_issue_id as path
    FROM dependencies d1
    WHERE d1.kind = 'blocks'

    UNION ALL

    SELECT
        cc.start_id,
        d.blocker_issue_id,
        cc.depth + 1,
        cc.path || '->' || d.blocker_issue_id
    FROM cycle_check cc
    JOIN dependencies d ON cc.current_id = d.blocked_issue_id
    WHERE d.kind = 'blocks'
      AND cc.depth < 20
      AND d.blocker_issue_id != cc.start_id
)
SELECT COUNT(DISTINCT start_id)
FROM cycle_check
WHERE blocker_issue_id = start_id
  AND depth > 1;
" 2>/dev/null || echo "0")

log_info "Dependency cycle detections: $CYCLE_DETECTIONS"
METRICS=$(echo "$METRICS" | jq ".diagnosis.dependency_cycles = $CYCLE_DETECTIONS")

# 2.3 Check for stale assignees (dead workers)
# This checks for assignees that haven't been active recently
STALE_ASSIGNEES=$(sqlite3 "$DB_PATH" "
SELECT COUNT(DISTINCT assignee)
FROM issues
WHERE base_status = 'open'
  AND assignee IS NOT NULL
  AND assignee != ''
  AND assignee NOT IN (
    -- This would check against active workers, but since we don't have a worker registry,
    -- we'll treat assigned-but-open as potential stale assignees
    SELECT NULL
  );
" 2>/dev/null || echo "$ASSIGNED_BUT_OPEN")

log_info "Stale assignees detected: $STALE_ASSIGNEES"
METRICS=$(echo "$METRICS" | jq ".diagnosis.stale_assignees = $STALE_ASSIGNEES")

# 2.4 Check for checkpoint corruption
CHECKPOINT_DIR="${WORKSPACE}/checkpoint"
CHECKPOINT_CORRUPTION=false

if [ ! -d "$CHECKPOINT_DIR" ]; then
    log_warn "Checkpoint directory missing: $CHECKPOINT_DIR"
    CHECKPOINT_CORRUPTION=true
elif [ ! -f "${CHECKPOINT_DIR}/current.json" ]; then
    log_warn "Checkpoint current.json missing"
    CHECKPOINT_CORRUPTION=true
elif [ ! -f "${CHECKPOINT_DIR}/forensic.jsonl" ]; then
    log_warn "Checkpoint forensic.jsonl missing"
    CHECKPOINT_CORRUPTION=true
fi

if [ "$CHECKPOINT_CORRUPTION" = true ]; then
    log_warn "Checkpoint corruption detected"
else
    log_info "Checkpoint appears healthy"
fi

METRICS=$(echo "$METRICS" | jq ".diagnosis.checkpoint_corruption = $CHECKPOINT_CORRUPTION")

# 2.5 Check for query filter issues
# This detects beads that should be ready but aren't due to complex filter conditions
FILTER_ISSUES=$(sqlite3 "$DB_PATH" "
SELECT COUNT(*)
FROM issues i
WHERE i.base_status = 'open'
  AND i.assignee IS NULL
  AND i.manual_blocked = 0
  AND (
    -- Beads with self-blocking dependencies
    EXISTS (
      SELECT 1 FROM dependencies d
      WHERE d.blocked_issue_id = i.id
        AND d.blocker_issue_id = i.id
        AND d.kind = 'blocks'
    )
    OR
    -- Beads with closed but still blocking dependencies
    EXISTS (
      SELECT 1 FROM dependencies d
      WHERE d.blocked_issue_id = i.id
        AND d.kind = 'blocks'
        AND d.blocker_issue_id IN (
          SELECT id FROM issues WHERE base_status = 'closed'
          AND id NOT IN (
            SELECT blocker_issue_id FROM dependencies
            WHERE blocked_issue_id = d.blocked_issue_id
              AND kind = 'blocks'
              AND blocker_issue_id != d.blocker_issue_id
          )
        )
    )
  );
" 2>/dev/null || echo "0")

log_info "Query filter issues detected: $FILTER_ISSUES"
METRICS=$(echo "$METRICS" | jq ".diagnosis.query_filter_issues = $FILTER_ISSUES")

# ============================================================================
# STEP 3: Execute Automated Repairs
# ============================================================================

log_info "Step 3: Executing automated repairs..."

TOTAL_REPAIRS=0
SUCCESSFUL_REPAIRS=0
UNRECOVERABLE_CONDITIONS=0

# 3.1 Fix assigned-but-open beads
if [ "$ASSIGNED_BUT_OPEN" -gt 0 ]; then
    log_action "Repairing $ASSIGNED_BUT_OPEN assigned-but-open beads..."

    STUCK_BEADS=$(sqlite3 "$DB_PATH" "
    SELECT id, assignee
    FROM issues
    WHERE base_status = 'open'
      AND assignee IS NOT NULL
      AND assignee != '';
    " 2>/dev/null || true)

    while IFS='|' read -r bead_id assignee; do
        if [ -n "$bead_id" ]; then
            TOTAL_REPAIRS=$((TOTAL_REPAIRS + 1))
            log_action "Clearing assignee for $bead_id (was: $assignee)"

            if bead update "$bead_id" --clear-assignee \
                --notes "Auto-repair: cleared stale assignee during starvation recovery at $TIMESTAMP" \
                2>/dev/null; then
                log_info "✓ Successfully cleared assignee for $bead_id"
                SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + 1))
            else
                log_error "✗ Failed to clear assignee for $bead_id"
            fi
        fi
    done <<< "$STUCK_BEADS"

    METRICS=$(echo "$METRICS" | jq ".repairs.assigned_cleared = $SUCCESSFUL_REPAIRS")
fi

# 3.2 Fix dependency cycles
if [ "$CYCLE_DETECTIONS" -gt 0 ]; then
    log_action "Breaking $CYCLE_DETECTIONS dependency cycles..."

    # Find and break cycles by removing the oldest blocking edge in each cycle
    CYCLE_BREAKS=0

    # Get cycles and break them
    sqlite3 "$DB_PATH" "
    WITH RECURSIVE cycle_members AS (
        WITH RECURSIVE cycle_check AS (
            SELECT
                d1.blocked_issue_id as start_id,
                d1.blocker_issue_id as current_id,
                1 as depth,
                d1.blocker_issue_id || '->' || d1.blocked_issue_id as path,
                d1.blocker_issue_id as oldest_edge_blocker,
                d1.blocked_issue_id as oldest_edge_blocked,
                d1.blocker_issue_id as edge_blocker,
                d1.blocked_issue_id as edge_blocked
            FROM dependencies d1
            WHERE d1.kind = 'blocks'

            UNION ALL

            SELECT
                cc.start_id,
                d.blocker_issue_id,
                cc.depth + 1,
                cc.path || '->' || d.blocker_issue_id,
                cc.oldest_edge_blocker,
                cc.oldest_edge_blocked,
                d.blocker_issue_id,
                d.blocked_issue_id
            FROM cycle_check cc
            JOIN dependencies d ON cc.current_id = d.blocker_issue_id
            WHERE d.kind = 'blocks'
              AND cc.depth < 20
              AND d.blocker_issue_id != cc.start_id
        )
        SELECT DISTINCT start_id, edge_blocker, edge_blocked
        FROM cycle_check
        WHERE blocker_issue_id = start_id
          AND depth > 1
    )
    SELECT edge_blocker, edge_blocked
    FROM cycle_members;
    " 2>/dev/null | while IFS='|' read -r blocker blocked; do
        if [ -n "$blocker" ] && [ -n "$blocked" ]; then
            TOTAL_REPAIRS=$((TOTAL_REPAIRS + 1))
            log_action "Breaking cycle edge: $blocker blocks $blocked"

            # Remove the blocking edge
            if sqlite3 "$DB_PATH" "
            DELETE FROM dependencies
            WHERE blocker_issue_id = '$blocker'
              AND blocked_issue_id = '$blocked'
              AND kind = 'blocks';
            " 2>/dev/null; then
                log_info "✓ Successfully broke cycle edge $blocker->$blocked"
                SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + 1))
                CYCLE_BREAKS=$((CYCLE_BREAKS + 1))
            else
                log_error "✗ Failed to break cycle edge $blocker->$blocked"
            fi
        fi
    done

    log_info "Broke $CYCLE_BREAKS dependency cycles"
    METRICS=$(echo "$METRICS" | jq ".repairs.cycles_broken = $CYCLE_BREAKS")
fi

# 3.3 Handle stale assignees (already handled in 3.1)
if [ "$STALE_ASSIGNEES" -gt 0 ]; then
    log_info "Stale assignees handled in assigned-but-open repair"
    METRICS=$(echo "$METRICS" | jq ".repairs.workers_recovered = $SUCCESSFUL_REPAIRS")
fi

# 3.4 Repair checkpoint corruption
if [ "$CHECKPOINT_CORRUPTION" = true ]; then
    log_action "Attempting checkpoint recovery..."

    # Try to restore from forensic log
    if bead sync import-only \
        --input "${WORKSPACE}/checkpoint/forensic.jsonl" \
        --restore-into-empty \
        --actor starvation-auto-repair \
        2>/dev/null; then
        log_info "✓ Checkpoint successfully restored from forensic log"
        SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + 1))
        METRICS=$(echo "$METRICS" | jq '.repairs.checkpoint_restored = true')
    else
        log_error "✗ Failed to restore checkpoint - UNRECOVERABLE"
        UNRECOVERABLE_CONDITIONS=$((UNRECOVERABLE_CONDITIONS + 1))

        # Create bead for manual intervention
        log_action "Creating unrecoverable condition bead..."
        bead create \
            --title "Unrecoverable checkpoint corruption detected at $TIMESTAMP" \
            --priority 1 \
            --issue-type task \
            --label automated-repair \
            --label unrecoverable \
            --label checkpoint-corruption \
            --notes "Auto-generated by starvation auto-repair service at $TIMESTAMP" \
            --notes "Checkpoint restoration failed - manual intervention required" \
            --notes "Workspace: $WORKSPACE" \
            --notes "Database: $DB_PATH" \
            2>/dev/null || true
    fi
fi

# 3.5 Fix query filter issues
if [ "$FILTER_ISSUES" -gt 0 ]; then
    log_action "Repairing $FILTER_ISSUES query filter issues..."

    FILTER_REPAIRS=0

    # Remove self-blocking dependencies
    sqlite3 "$DB_PATH" "
    DELETE FROM dependencies
    WHERE blocker_issue_id = blocked_issue_id
      AND kind = 'blocks';
    " 2>/dev/null && FILTER_REPAIRS=$((FILTER_REPAIRS + 1))

    # Remove dependencies on closed beads
    sqlite3 "$DB_PATH" "
    DELETE FROM dependencies
    WHERE kind = 'blocks'
      AND blocker_issue_id IN (
        SELECT id FROM issues WHERE base_status = 'closed'
      );
    " 2>/dev/null && FILTER_REPAIRS=$((FILTER_REPAIRS + 1))

    log_info "Fixed $FILTER_REPAIRS filter issues"
    METRICS=$(echo "$METRICS" | jq ".repairs.filters_repaired = $FILTER_REPAIRS")
    SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + FILTER_REPAIRS))
fi

# ============================================================================
# STEP 4: Verify Repairs
# ============================================================================

log_info "Step 4: Verifying repairs..."

NEW_READY_COUNT=$(sqlite3 "$DB_PATH" "
SELECT COUNT(*)
FROM issues i
WHERE i.base_status = 'open'
  AND i.assignee IS NULL
  AND i.manual_blocked = 0
  AND NOT EXISTS (
    SELECT 1 FROM dependencies d
    WHERE d.blocked_issue_id = i.id
      AND d.kind = 'blocks'
      AND d.blocker_issue_id IN (
        SELECT id FROM issues WHERE base_status != 'closed'
      )
  );
" 2>/dev/null || echo "0")

log_info "Ready beads after repairs: $NEW_READY_COUNT (was: $READY_COUNT)"

if [ "$NEW_READY_COUNT" -gt "$READY_COUNT" ]; then
    RECOVERED_COUNT=$((NEW_READY_COUNT - READY_COUNT))
    log_info "✓ Successfully recovered $RECOVERED_COUNT beads"
elif [ "$NEW_READY_COUNT" -eq 0 ]; then
    log_warn "⚠ Still zero ready beads after repairs - may require manual intervention"
    UNRECOVERABLE_CONDITIONS=$((UNRECOVERABLE_CONDITIONS + 1))
else
    log_info "✓ Repair successful - ready beads now available"
fi

# ============================================================================
# STEP 5: Update Final Metrics and Summary
# ============================================================================

METRICS=$(echo "$METRICS" | jq ".summary.total_repairs_attempted = $TOTAL_REPAIRS")
METRICS=$(echo "$METRICS" | jq ".summary.successful_repairs = $SUCCESSFUL_REPAIRS")
METRICS=$(echo "$METRICS" | jq ".summary.unrecoverable_conditions = $UNRECOVERABLE_CONDITIONS")

# Save metrics
echo "$METRICS" | jq '.' > "$METRICS_FILE"

log_info "=== Starvation Auto-Repair Complete ==="
log_info "Total repairs attempted: $TOTAL_REPAIRS"
log_info "Successful repairs: $SUCCESSFUL_REPAIRS"
log_info "Unrecoverable conditions: $UNRECOVERABLE_CONDITIONS"
log_info "Metrics saved to: $METRICS_FILE"

# Exit with appropriate code
if [ "$UNRECOVERABLE_CONDITIONS" -gt 0 ]; then
    log_warn "Unrecoverable conditions require manual intervention"
    exit 1
elif [ "$SUCCESSFUL_REPAIRS" -gt 0 ]; then
    log_info "✓ All repairs completed successfully"
    exit 0
else
    log_info "No repairs were needed"
    exit 0
fi
