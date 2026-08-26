#!/usr/bin/env bash
# Automated Bead Starvation Repair Processor
# Reads starvation alert beads with embedded diagnostics and executes automated repairs
# Designed to process alerts created by bead-starvation-alert-generator.sh

set -euo pipefail

DB_PATH="${DB_PATH:-/home/coding/irreversible-command-gate/.beads/beads.db}"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

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

# Parse command line arguments
BEAD_ID="${1:-}"
DRY_RUN="${DRY_RUN:-false}"

if [ -z "$BEAD_ID" ]; then
    log_error "Usage: $0 <bead_id> [--dry-run]"
    exit 1
fi

if [ "${2:-}" = "--dry-run" ] || [ "$DRY_RUN" = "true" ]; then
    DRY_RUN=true
    log_warn "DRY RUN MODE - No changes will be made"
fi

log_info "Processing starvation alert bead: $BEAD_ID"

# Step 1: Extract the bead body/description
log_info "Reading bead content..."

# Get the bead body - use bead show with JSON output for easier parsing
BEAD_JSON=$(bead show "$BEAD_ID" --json 2>/dev/null)

if [ -z "$BEAD_JSON" ]; then
    log_error "Could not retrieve bead data for $BEAD_ID"
    exit 1
fi

# Extract the body/description from JSON (bead show returns an array)
BEAD_BODY=$(echo "$BEAD_JSON" | jq -r '.[0].body // .[0].description // ""')

if [ -z "$BEAD_BODY" ]; then
    log_error "Could not extract bead body from JSON output"
    exit 1
fi

if [ -z "$BEAD_BODY" ]; then
    log_error "Could not extract bead body from $BEAD_ID"
    exit 1
fi

# Step 2: Extract diagnostic context from the bead body
log_info "Extracting diagnostic context..."

# Try to extract JSON from the bead body (handles escaped markdown)
# Extract everything between ```json and the closing ``` or \`\`\`
DIAGNOSTIC_JSON=$(echo "$BEAD_BODY" | sed -n '/```json/,/\\`\\`\\`/p' | sed '1d; /\\`\\`\\`/d' | sed '/^$/d' | sed '/^---$/d' | sed '/^This alert was auto-generated/d')

if [ -z "$DIAGNOSTIC_JSON" ] || [ "$DIAGNOSTIC_JSON" = "null" ]; then
    log_error "Could not extract diagnostic JSON from bead body"
    log_error "Body preview: $(echo "$BEAD_BODY" | head -c 200)..."
    exit 1
fi

# Validate JSON
if ! echo "$DIAGNOSTIC_JSON" | jq empty 2>/dev/null; then
    log_error "Invalid diagnostic JSON in bead body"
    log_error "JSON preview: $(echo "$DIAGNOSTIC_JSON" | head -c 200)..."
    exit 1
fi

# Step 3: Parse diagnostic data
log_info "Parsing diagnostic data..."

TOTAL_OPEN=$(echo "$DIAGNOSTIC_JSON" | jq -r '.summary.total_open_beads // 0')
READY_BEADS=$(echo "$DIAGNOSTIC_JSON" | jq -r '.summary.ready_beads // 0')
ASSIGNED_BUT_OPEN=$(echo "$DIAGNOSTIC_JSON" | jq -r '.summary.assigned_but_open // 0')
BLOCKED_BEADS=$(echo "$DIAGNOSTIC_JSON" | jq -r '.summary.blocked_beads // 0')
ORPHANED_DEPS=$(echo "$DIAGNOSTIC_JSON" | jq -r '.summary.orphaned_dependencies // 0')

log_info "Diagnostic Summary:"
log_info "  Total open beads: $TOTAL_OPEN"
log_info "  Ready beads: $READY_BEADS"
log_info "  Assigned-but-open: $ASSIGNED_BUT_OPEN"
log_info "  Blocked beads: $BLOCKED_BEADS"
log_info "  Orphaned dependencies: $ORPHANED_DEPS"

# Step 4: Execute automated repairs
TOTAL_REPAIRS=0
SUCCESSFUL_REPAIRS=0

# 4.1 Fix assigned-but-open beads
if [ "$ASSIGNED_BUT_OPEN" -gt 0 ]; then
    log_action "Fixing $ASSIGNED_BUT_OPEN assigned-but-open beads..."

    STUCK_BEADS=$(sqlite3 "$DB_PATH" "SELECT id FROM issues WHERE base_status = 'open' AND assignee IS NOT NULL;")

    for bead_id in $STUCK_BEADS; do
        TOTAL_REPAIRS=$((TOTAL_REPAIRS + 1))
        log_action "Attempting repair: $bead_id"

        if [ "$DRY_RUN" = true ]; then
            log_warn "[DRY-RUN] Would clear assignee for $bead_id"
            SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + 1))
        else
            if bead update "$bead_id" --clear-assignee --notes "Auto-repair: cleared stale assignee (processed from starvation alert $BEAD_ID at $TIMESTAMP)" 2>/dev/null; then
                log_info "✓ Successfully cleared assignee for $bead_id"
                SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + 1))
            else
                log_error "✗ Failed to clear assignee for $bead_id"
            fi
        fi
    done
else
    log_info "No assigned-but-open beads to repair"
fi

# 4.2 Fix orphaned dependencies
if [ "$ORPHANED_DEPS" -gt 0 ]; then
    log_action "Fixing $ORPHANED_DEPS orphaned dependencies..."

    if [ "$DRY_RUN" = true ]; then
        log_warn "[DRY-RUN] Would run bead-dependency-validator"
    else
        # Try to run the dependency validator if it exists
        if command -v bead-dependency-validator &> /dev/null; then
            log_info "Running bead-dependency-validator..."
            if bead-dependency-validator --db-path "$DB_PATH" 2>&1 | tee /tmp/validator_output.txt; then
                log_info "✓ Dependency validator completed successfully"
                SUCCESSFUL_REPAIRS=$((SUCCESSFUL_REPAIRS + 1))
            else
                log_error "✗ Dependency validator encountered errors"
            fi
            TOTAL_REPAIRS=$((TOTAL_REPAIRS + 1))
        else
            log_warn "bead-dependency-validator not found, skipping orphaned dependency repair"
        fi
    fi
else
    log_info "No orphaned dependencies to repair"
fi

# 4.3 Run dependency validator for circular dependencies
if [ "$BLOCKED_BEADS" -gt 0 ]; then
    log_action "Checking for circular dependencies among $BLOCKED_BEADS blocked beads..."

    if [ "$DRY_RUN" = true ]; then
        log_warn "[DRY-RUN] Would check for circular dependencies"
    else
        # Check if there are obvious circular patterns
        CIRCULAR_COUNT=$(echo "$DIAGNOSTIC_JSON" | jq -r '.circular_dependency_candidates | length // 0')

        if [ "$CIRCULAR_COUNT" -gt 0 ]; then
            log_action "Found $CIRCULAR_COUNT circular dependency candidates"

            # Extract and display circular dependencies
            echo "$DIAGNOSTIC_JSON" | jq -r '.circular_dependency_candidates[]?' | while read -r circular_pair; do
                log_warn "Circular pair: $circular_pair"
            done

            # Manual circular dependency resolution would go here
            log_warn "Circular dependencies require manual resolution"
        fi
    fi
else
    log_info "No circular dependency patterns detected"
fi

# Step 5: Verify repairs
if [ "$DRY_RUN" = false ]; then
    log_info "Verifying repair results..."

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
    ")

    log_info "Ready bead count after repairs: $NEW_READY_COUNT (was: $READY_BEADS)"

    if [ "$NEW_READY_COUNT" -gt "$READY_BEADS" ]; then
        log_info "✓ Repair successful! Ready beads increased by $((NEW_READY_COUNT - READY_BEADS))"
    elif [ "$NEW_READY_COUNT" -eq "$READY_BEADS" ]; then
        log_warn "No change in ready bead count"
    else
        log_error "Ready bead count decreased unexpectedly"
    fi
fi

# Step 6: Generate repair summary
log_info "Repair Summary:"
log_info "  Total repairs attempted: $TOTAL_REPAIRS"
log_info "  Successful repairs: $SUCCESSFUL_REPAIRS"

if [ "$SUCCESSFUL_REPAIRS" -eq "$TOTAL_REPAIRS" ] && [ "$TOTAL_REPAIRS" -gt 0 ]; then
    log_info "✓ All repairs completed successfully"
    exit 0
elif [ "$TOTAL_REPAIRS" -eq 0 ]; then
    log_info "No repairs were needed"
    exit 0
else
    log_warn "Some repairs failed or were incomplete"
    exit 1
fi
