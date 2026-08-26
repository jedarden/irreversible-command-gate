#!/usr/bin/env bash
# Scheduled Bead Starvation Repair Service
# Queries for open starvation-alert beads and processes them automatically
# Designed to be run via systemd timer every 15 minutes

set -euo pipefail

DB_PATH="${DB_PATH:-/home/coding/irreversible-command-gate/.beads/beads.db}"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPAIR_SCRIPT="$SCRIPT_DIR/bead-starvation-auto-repair.sh"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
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

# Ensure we have required tools
if ! command -v bead &> /dev/null; then
    log_error "bead CLI not found"
    exit 1
fi

if ! command -v sqlite3 &> /dev/null; then
    log_error "sqlite3 not found"
    exit 1
fi

if [ ! -f "$DB_PATH" ]; then
    log_error "Bead database not found at $DB_PATH"
    exit 1
fi

if [ ! -f "$REPAIR_SCRIPT" ]; then
    log_error "Repair script not found at $REPAIR_SCRIPT"
    exit 1
fi

log_info "Starting scheduled starvation repair run at $TIMESTAMP"

# Query for open starvation-alert beads
log_info "Querying for open starvation-alert beads..."

# Get beads with issue-type 'starvation-alert' or label 'starvation-alert'
STARVATION_ALERTS=$(sqlite3 "$DB_PATH" "
SELECT DISTINCT i.id
FROM issues i
LEFT JOIN labels l ON i.id = l.issue_id
WHERE i.base_status = 'open'
  AND (i.issue_type = 'starvation-alert' OR l.label = 'starvation-alert');
")

if [ -z "$STARVATION_ALERTS" ]; then
    log_info "No open starvation-alert beads found"
    exit 0
fi

# Count alerts
ALERT_COUNT=$(echo "$STARVATION_ALERTS" | wc -l)
log_info "Found $ALERT_COUNT open starvation-alert bead(s)"

# Track statistics
TOTAL_PROCESSED=0
SUCCESSFUL_REPAIRS=0
FAILED_REPAIRS=0
ESCALATED_ALERTS=0

# Process each starvation alert
for alert_id in $STARVATION_ALERTS; do
    TOTAL_PROCESSED=$((TOTAL_PROCESSED + 1))
    log_action "Processing starvation alert: $alert_id"

    # Run the repair script for this alert
    if bash "$REPAIR_SCRIPT" "$alert_id" 2>&1; then
        log_info "✓ Repair completed successfully for $alert_id"
        SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + 1))

        # Close the alert bead since repair was successful
        log_action "Closing starvation alert: $alert_id"
        if bead close "$alert_id" --reason "Auto-repair completed successfully at $TIMESTAMP - repaired $TOTAL_PROCESSED beads" 2>/dev/null; then
            log_info "✓ Closed starvation alert $alert_id"
        else
            log_warn "Failed to close starvation alert $alert_id (but repair succeeded)"
        fi
    else
        log_warn "✗ Repair failed or incomplete for $alert_id"
        FAILED_REPAIRS=$((FAILED_REPAIRS + 1))

        # Check if this is due to circular dependencies (requires manual intervention)
        # by looking at the bead body for circular dependency markers
        BEAD_JSON=$(bead show "$alert_id" --json 2>/dev/null || echo '')
        if [ -n "$BEAD_JSON" ]; then
            BEAD_BODY=$(echo "$BEAD_JSON" | jq -r '.[0].body // .[0].description // ""')
            if echo "$BEAD_BODY" | grep -q "circular_dependency"; then
                log_warn "Circular dependencies detected - this alert requires manual resolution"
                ESCALATED_ALERTS=$((ESCALATED_ALERTS + 1))
            fi
        fi
    fi
done

# Generate summary
log_info "Scheduled Repair Run Summary:"
log_info "  Total alerts processed: $TOTAL_PROCESSED"
log_info "  Successful repairs: $SUCCESSFUL_REPAIRS"
log_info "  Failed repairs: $FAILED_REPAIRS"
log_info "  Alerts requiring manual intervention: $ESCALATED_ALERTS"

if [ "$SUCCESSFUL_REPAIRS" -gt 0 ]; then
    log_info "✓ Automated repairs completed and alerts closed"
fi

if [ "$ESCALATED_ALERTS" -gt 0 ]; then
    log_warn "⚠ $ESCALATED_ALERTS alert(s) require manual circular-dependency resolution"
fi

exit 0
