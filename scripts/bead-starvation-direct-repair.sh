#!/usr/bin/env bash
# Direct Bead Starvation Repair Script
# Automatically detects and fixes beads in 'assigned-but-open' state
# This is a silent failure mode where beads have assignees but are stuck in open status

set -euo pipefail

# Configuration
DB_PATH="${DB_PATH:-$(pwd)/.beads/beads.db}"
DIAGNOSTICS_DIR="$(dirname "$DB_PATH")/diagnostics"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
REPAIR_LOG="$DIAGNOSTICS_DIR/repair-log-$TIMESTAMP.json"

# Parse arguments
DRY_RUN="${DRY_RUN:-false}"
VERBOSE="${VERBOSE:-false}"

for arg in "$@"; do
    case "$arg" in
        --dry-run)
            DRY_RUN=true
            ;;
        --verbose)
            VERBOSE=true
            ;;
        --help)
            echo "Usage: $0 [--dry-run] [--verbose]"
            echo ""
            echo "Options:"
            echo "  --dry-run    Show what would be done without making changes"
            echo "  --verbose    Show detailed output"
            echo ""
            echo "Environment variables:"
            echo "  DB_PATH      Path to beads.db (default: ./beads.db)"
            echo "  DRY_RUN      Set to 'true' for dry-run mode"
            echo "  VERBOSE      Set to 'true' for verbose output"
            exit 0
            ;;
        *)
            echo "Unknown option: $arg"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Logging functions
log_info() {
    echo "[INFO] $*" >&2
    if [ "$VERBOSE" = true ]; then
        echo "{\"timestamp\":\"$TIMESTAMP\",\"level\":\"info\",\"message\":\"$*\"}" >&3
    fi
}

log_warn() {
    echo "[WARN] $*" >&2
    echo "{\"timestamp\":\"$TIMESTAMP\",\"level\":\"warn\",\"message\":\"$*\"}" >&3
}

log_error() {
    echo "[ERROR] $*" >&2
    echo "{\"timestamp\":\"$TIMESTAMP\",\"level\":\"error\",\"message\":\"$*\"}" >&3
}

log_action() {
    echo "[ACTION] $*" >&2
    echo "{\"timestamp\":\"$TIMESTAMP\",\"level\":\"action\",\"message\":\"$*\"}" >&3
}

# Ensure diagnostics directory exists
mkdir -p "$DIAGNOSTICS_DIR"

# Open file descriptor for logging
exec 3>"$REPAIR_LOG"
echo "{\"timestamp\":\"$TIMESTAMP\",\"event\":\"repair_run_started\",\"dry_run\":$DRY_RUN}" >&3

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

log_info "Starting direct bead starvation repair"
log_info "Database: $DB_PATH"
log_info "Diagnostics: $DIAGNOSTICS_DIR"

if [ "$DRY_RUN" = true ]; then
    log_warn "DRY RUN MODE - No changes will be made"
fi

# Query for assigned-but-open beads
log_info "Querying for assigned-but-open beads..."

QUERY="
SELECT i.id, i.title, i.assignee, i.created_at
FROM issues i
WHERE i.base_status = 'open'
  AND i.assignee IS NOT NULL
  AND i.assignee != ''
ORDER BY i.created_at ASC;
"

STUCK_BEADS=$(sqlite3 "$DB_PATH" "$QUERY" 2>/dev/null || true)

if [ -z "$STUCK_BEADS" ]; then
    log_info "No assigned-but-open beads found - database is healthy"
    echo "{\"timestamp\":\"$TIMESTAMP\",\"event\":\"scan_complete\",\"stuck_beads\":0}" >&3
    exec 3>&-
    exit 0
fi

# Parse and count stuck beads
BEAD_COUNT=$(echo "$STUCK_BEADS" | wc -l)
log_info "Found $BEAD_COUNT assigned-but-open beads"

# Track repair statistics
TOTAL_REPAIRS=0
SUCCESSFUL_REPAIRS=0
FAILED_REPAIRS=0
REPAIR_DETAILS=()

# Process each stuck bead
while IFS='|' read -r bead_id title assignee created_at; do
    TOTAL_REPAIRS=$((TOTAL_REPAIRS + 1))

    if [ -z "$bead_id" ]; then
        continue
    fi

    log_action "Processing bead: $bead_id (assignee: $assignee)"

    repair_detail="{\"bead_id\":\"$bead_id\",\"assignee\":\"$assignee\"}"

    if [ "$DRY_RUN" = true ]; then
        log_warn "[DRY-RUN] Would clear assignee for $bead_id"
        SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + 1))
        repair_detail="$repair_detail,\"status\":\"dry_run_success\""
    else
        # Attempt to clear the assignee
        if bead update "$bead_id" --clear-assignee \
            --notes "Auto-repair: cleared stale assignee (bead was stuck in assigned-but-open state at $TIMESTAMP)" \
            2>&1 >/dev/null; then
            log_info "✓ Successfully cleared assignee for $bead_id"
            SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + 1))
            repair_detail="$repair_detail,\"status\":\"success\""
        else
            log_error "✗ Failed to clear assignee for $bead_id"
            FAILED_REPAIRS=$((FAILED_REPAIRS + 1))
            repair_detail="$repair_detail,\"status\":\"failed\""
        fi
    fi

    REPAIR_DETAILS+=("$repair_detail")
done <<< "$STUCK_BEADS"

# Generate final report
log_info "Repair Summary:"
log_info "  Total stuck beads found: $BEAD_COUNT"
log_info "  Total repairs attempted: $TOTAL_REPAIRS"
log_info "  Successful repairs: $SUCCESSFUL_REPAIRS"
log_info "  Failed repairs: $FAILED_REPAIRS"

# Write detailed repair log
cat >&3 <<EOF
{"timestamp":"$TIMESTAMP","event":"repair_run_completed","dry_run":$DRY_RUN,"total_found":$BEAD_COUNT,"total_attempted":$TOTAL_REPAIRS,"successful":$SUCCESSFUL_REPAIRS,"failed":$FAILED_REPAIRS,"repairs":[${REPAIR_DETAILS[*]}]}
EOF

exec 3>&-

# Verify the fix worked
if [ "$DRY_RUN" = false ] && [ "$SUCCESSFUL_REPAIRS" -gt 0 ]; then
    log_info "Verifying repairs..."

    REMAINING_STUCK=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM issues WHERE base_status = 'open' AND assignee IS NOT NULL AND assignee != '';" 2>/dev/null || echo "?")

    log_info "Remaining stuck beads: $REMAINING_STUCK (was: $BEAD_COUNT)"

    if [ "$REMAINING_STUCK" -lt "$BEAD_COUNT" ]; then
        FIXED_COUNT=$((BEAD_COUNT - REMAINING_STUCK))
        log_info "✓ Successfully fixed $FIXED_COUNT beads"
    fi
fi

# Exit with appropriate code
if [ "$FAILED_REPAIRS" -gt 0 ]; then
    log_warn "Some repairs failed - check log: $REPAIR_LOG"
    exit 1
elif [ "$SUCCESSFUL_REPAIRS" -gt 0 ]; then
    log_info "✓ All repairs completed successfully"
    exit 0
else
    log_info "No repairs were needed"
    exit 0
fi
