#!/usr/bin/env bash
# Automated Assignee-Stuck-Bead Sweeper
# Scans all open beads and identifies/recoveries beads in the 'assigned-but-open' state
# Verifies assignee worker liveness before clearing stale assignments
#
# Usage: bead-assignee-stuck-sweeper.sh [--dry-run] [--worker-stale-hours HOURS]
#
# This automates the fix discovered on 2026-08-16 when 583 beads were found stuck
# across 47 workspaces.

set -euo pipefail

DB_PATH="${DB_PATH:-/home/coding/irreversible-command-gate/.beads/beads.db}"
OUTPUT_DIR="${OUTPUT_DIR:-/home/coding/irreversible-command-gate/.beads/diagnostics}"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
OUTPUT_FILE="${OUTPUT_DIR}/assignee-stuck-sweeper-${TIMESTAMP}.json"

# Configuration
WORKER_STALE_HOURS="${WORKER_STALE_HOURS:-24}"  # Consider worker stale if no activity for 24 hours
DRY_RUN="${DRY_RUN:-false}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1" >&2
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1" >&2
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1" >&2
}

log_action() {
    echo -e "${BLUE}[ACTION]${NC} $1" >&2
}

log_detail() {
    echo -e "${CYAN}[DETAIL]${NC} $1" >&2
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --worker-stale-hours)
            WORKER_STALE_HOURS="$2"
            shift 2
            ;;
        *)
            log_error "Unknown option: $1"
            echo "Usage: $0 [--dry-run] [--worker-stale-hours HOURS]" >&2
            exit 1
            ;;
    esac
done

if [ "$DRY_RUN" = true ]; then
    log_warn "DRY RUN MODE - No changes will be made"
fi

log_info "Assignee-Stuck-Bead Sweeper starting at $TIMESTAMP"
log_detail "Worker stale threshold: ${WORKER_STALE_HOURS} hours"

# Ensure we have the bead CLI
if ! command -v bead &> /dev/null; then
    log_error "bead CLI not found"
    exit 1
fi

# Ensure database exists
if [ ! -f "$DB_PATH" ]; then
    log_error "Bead database not found at $DB_PATH"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"

# Calculate stale timestamp cutoff
STALE_CUTOFF=$(date -u -d "${WORKER_STALE_HOURS} hours ago" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v-${WORKER_STALE_HOURS}H +"%Y-%m-%dT%H:%M:%SZ")

log_detail "Stale cutoff: ${STALE_CUTOFF}"

# Step 1: Find all assigned-but-open beads
log_info "Step 1: Finding assigned-but-open beads..."

ASSIGNED_OPEN_BEADS=$(
    sqlite3 "$DB_PATH" <<'EOF'
SELECT
    i.id,
    i.title,
    i.assignee,
    i.priority,
    i.updated_at,
    i.created_at
FROM issues i
WHERE i.base_status = 'open'
  AND i.assignee IS NOT NULL
ORDER BY i.updated_at DESC;
EOF
)

ASSIGNED_OPEN_COUNT=$(echo "$ASSIGNED_OPEN_BEADS" | wc -l)

if [ "$ASSIGNED_OPEN_COUNT" -eq 0 ] || [ -z "$ASSIGNED_OPEN_BEADS" ]; then
    log_info "✓ No assigned-but-open beads found"
    echo "{\"timestamp\":\"$TIMESTAMP\",\"workspace\":\"irreversible-command-gate\",\"summary\":{\"total_assigned_open\":0,\"stale_workers\":0,\"repaired\":0}}"
    exit 0
fi

log_info "Found $ASSIGNED_OPEN_COUNT assigned-but-open beads"

# Step 2: For each bead, check if the assignee worker is still alive
log_info "Step 2: Checking worker liveness..."

STALE_BEADS=()
ACTIVE_BEADS=()
WORKER_CHECKSUMS=()

echo "$ASSIGNED_OPEN_BEADS" | while IFS='|' read -r bead_id title assignee priority updated_at created_at; do
    [ -z "$bead_id" ] && continue

    log_detail "Checking bead: $bead_id (assignee: $assignee)"

    # Check for recent events from this worker
    LAST_ACTIVITY=$(sqlite3 "$DB_PATH" "SELECT MAX(time) FROM events WHERE actor = '$assignee';")

    if [ -z "$LAST_ACTIVITY" ] || [ "$LAST_ACTIVITY" = "NULL" ]; then
        log_warn "Worker $assignee has no activity history"
        STALE_BEADS+=("$bead_id|$assignee|$title|no_activity")
        continue
    fi

    log_detail "Last activity by $assignee: $LAST_ACTIVITY"

    # Compare timestamps
    LAST_ACTIVITY_SEC=$(date -u -d "$LAST_ACTIVITY" +%s 2>/dev/null || date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "$LAST_ACTIVITY" +%s)
    STALE_CUTOFF_SEC=$(date -u -d "$STALE_CUTOFF" +%s 2>/dev/null || date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "$STALE_CUTOFF" +%s)

    if [ "$LAST_ACTIVITY_SEC" -lt "$STALE_CUTOFF_SEC" ]; then
        STALE_HOURS=$(( (STALE_CUTOFF_SEC - LAST_ACTIVITY_SEC) / 3600 ))
        log_warn "Worker $assignee is stale (last activity ${STALE_HOURS}h ago)"
        STALE_BEADS+=("$bead_id|$assignee|$title|stale_${STALE_HOURS}h")
    else
        log_info "✓ Worker $assignee is active"
        ACTIVE_BEADS+=("$bead_id|$assignee|$title")
    fi
done

# Step 3: Clear stale assignees
log_info "Step 3: Clearing stale assignees..."

REPAIRED_COUNT=0
FAILED_COUNT=0

if [ ${#STALE_BEADS[@]} -eq 0 ]; then
    log_info "✓ No stale workers detected - all assignees are active"
else
    log_action "Found ${#STALE_BEADS[@]} beads with stale workers, attempting repair..."

    for bead_info in "${STALE_BEADS[@]}"; do
        IFS='|' read -r bead_id assignee title reason <<< "$bead_info"

        log_action "Repairing: $bead_id ($title) - assignee $assignee ($reason)"

        if [ "$DRY_RUN" = true ]; then
            log_warn "[DRY-RUN] Would clear assignee for $bead_id"
            REPAIRED_COUNT=$((REPAIRED_COUNT + 1))
        else
            if bead update "$bead_id" --clear-assignee --notes "Auto-repair: cleared stale assignee '${assignee}' (${reason}, sweeper $TIMESTAMP)" 2>/dev/null; then
                log_info "✓ Successfully cleared assignee for $bead_id"
                REPAIRED_COUNT=$((REPAIRED_COUNT + 1))
            else
                log_error "✗ Failed to clear assignee for $bead_id"
                FAILED_COUNT=$((FAILED_COUNT + 1))
            fi
        fi
    done
fi

# Step 4: Verify repairs
if [ "$DRY_RUN" = false ] && [ "$REPAIRED_COUNT" -gt 0 ]; then
    log_info "Step 4: Verifying repairs..."

    NEW_ASSIGNED_OPEN_COUNT=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM issues WHERE base_status = 'open' AND assignee IS NOT NULL;")

    log_info "Assigned-but-open count after repair: $NEW_ASSIGNED_OPEN_COUNT (was: $ASSIGNED_OPEN_COUNT)"

    EXPECTED_COUNT=$((ASSIGNED_OPEN_COUNT - REPAIRED_COUNT))

    if [ "$NEW_ASSIGNED_OPEN_COUNT" -eq "$EXPECTED_COUNT" ]; then
        log_info "✓ Verification successful - all stale beads recovered"
    else
        log_warn "Verification shows unexpected count (expected: $EXPECTED_COUNT, actual: $NEW_ASSIGNED_OPEN_COUNT)"
    fi
fi

# Step 5: Generate report
log_info "Step 5: Generating report..."

REPORT=$(cat <<EOF
{
  "timestamp": "$TIMESTAMP",
  "workspace": "irreversible-command-gate",
  "configuration": {
    "worker_stale_hours": $WORKER_STALE_HOURS,
    "dry_run": $DRY_RUN
  },
  "summary": {
    "total_assigned_open": $ASSIGNED_OPEN_COUNT,
    "active_workers": ${#ACTIVE_BEADS[@]},
    "stale_workers": ${#STALE_BEADS[@]},
    "repaired": $REPAIRED_COUNT,
    "failed": $FAILED_COUNT
  },
  "stale_beads": [
$(for bead_info in "${STALE_BEADS[@]}"; do
    IFS='|' read -r bead_id assignee title reason <<< "$bead_info"
    echo "    {\"id\": \"$bead_id\", \"assignee\": \"$assignee\", \"title\": \"$title\", \"reason\": \"$reason\"},"
done | sed '$ s/,$//')
  ]
}
EOF
)

# Save report
echo "$REPORT" | jq '.' > "$OUTPUT_FILE"

log_info "Report saved to: $OUTPUT_FILE"

# Step 6: Print summary
echo "" >&2
echo "=== Sweeper Summary ===" >&2
echo "Total assigned-but-open beads: $ASSIGNED_OPEN_COUNT" >&2
echo "Beads with active workers: ${#ACTIVE_BEADS[@]}" >&2
echo "Beads with stale workers: ${#STALE_BEADS[@]}" >&2
echo "Repairs attempted: $REPAIRED_COUNT" >&2
echo "Repairs failed: $FAILED_COUNT" >&2
echo "======================" >&2

if [ "$FAILED_COUNT" -gt 0 ]; then
    exit 1
else
    exit 0
fi
