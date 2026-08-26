#!/usr/bin/env bash
set -euo pipefail

# Independent Checkpoint Verification Script
#
# This script verifies checkpoint integrity without relying on the bead CLI,
# which is useful when the bead system itself may be malfunctioning.
#
# Usage:
#   ./scripts/checkpoint-verify.sh [--workspace PATH] [--verbose]

set -a
# Configuration
WORKSPACE="${WORKSPACE:-.}"
VERBOSE="${VERBOSE:-false}"
BEAD_CLI="${BEAD_CLI:-bead}"
TEMP_DIR="${TEMP_DIR:-/tmp/checkpoint-verify-$$}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Cleanup trap
cleanup() {
    rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

log_info() {
    echo -e "${BLUE}ℹ${NC} $*"
}

log_success() {
    echo -e "${GREEN}✓${NC} $*"
}

log_warning() {
    echo -e "${YELLOW}⚠${NC} $*"
}

log_error() {
    echo -e "${RED}✗${NC} $*"
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --workspace)
            WORKSPACE="$2"
            shift 2
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --help)
            echo "Usage: $0 [--workspace PATH] [--verbose]"
            echo ""
            echo "Verify bead checkpoint integrity independently of the bead CLI."
            echo ""
            echo "Options:"
            echo "  --workspace PATH   Path to workspace root (default: current directory)"
            echo "  --verbose           Show detailed output"
            echo "  --help              Show this help message"
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Verify workspace
if [[ ! -d "$WORKSPACE" ]]; then
    log_error "Workspace directory not found: $WORKSPACE"
    exit 1
fi

# Paths
BEADS_DIR="$WORKSPACE/.beads"
CHECKPOINT_DIR="$BEADS_DIR/checkpoint"
CURRENT_CHECKPOINT="$CHECKPOINT_DIR/current.json"
FORENSIC_JSONL="$CHECKPOINT_DIR/forensic.jsonl"
OBJECTS_DIR="$CHECKPOINT_DIR/objects"
BEADS_DB="$BEADS_DIR/beads.db"

log_info "Checkpoint Verification"
log_info "Workspace: $WORKSPACE"
echo

# Step 1: Verify checkpoint directory structure
log_info "Step 1: Verifying checkpoint directory structure..."
structure_ok=true

if [[ ! -d "$CHECKPOINT_DIR" ]]; then
    log_error "Checkpoint directory missing: $CHECKPOINT_DIR"
    structure_ok=false
else
    log_success "Checkpoint directory exists"
fi

if [[ ! -f "$CURRENT_CHECKPOINT" ]]; then
    log_warning "current.json missing"
    structure_ok=false
else
    log_success "current.json exists"
fi

if [[ ! -f "$FORENSIC_JSONL" ]]; then
    log_error "forensic.jsonl missing"
    structure_ok=false
else
    log_success "forensic.jsonl exists"
fi

if [[ ! -d "$OBJECTS_DIR" ]]; then
    log_warning "objects directory missing"
else
    log_success "objects directory exists"
fi

echo

# Step 2: Parse checkpoint metadata
log_info "Step 2: Parsing checkpoint metadata..."

if [[ -f "$CURRENT_CHECKPOINT" ]]; then
    # Extract metadata using jq
    if command -v jq &> /dev/null; then
        CHECKPOINT_TIMESTAMP=$(jq -r '.created_at // empty' "$CURRENT_CHECKPOINT" 2>/dev/null || echo "")
        CHECKPOINT_ISSUE_COUNT=$(jq -r '.issue_count // empty' "$CURRENT_CHECKPOINT" 2>/dev/null || echo "")
        CHECKPOINT_UUID=$(jq -r '.store_uuid // empty' "$CURRENT_CHECKPOINT" 2>/dev/null || echo "")
        ACTIVE_ROOT=$(jq -r '.active_root.path // empty' "$CURRENT_CHECKPOINT" 2>/dev/null || echo "")

        if [[ -n "$CHECKPOINT_TIMESTAMP" ]]; then
            log_success "Checkpoint timestamp: $CHECKPOINT_TIMESTAMP"
        else
            log_error "Could not parse checkpoint timestamp"
        fi

        if [[ -n "$CHECKPOINT_ISSUE_COUNT" ]]; then
            log_success "Checkpoint issue count: $CHECKPOINT_ISSUE_COUNT"
        fi

        if [[ -n "$CHECKPOINT_UUID" ]]; then
            log_success "Store UUID: $CHECKPOINT_UUID"
        fi

        if [[ -n "$ACTIVE_ROOT" ]]; then
            log_success "Active root: $ACTIVE_ROOT"
        fi
    else
        log_warning "jq not available - skipping detailed metadata parsing"
    fi
else
    log_error "Cannot parse metadata - current.json missing"
fi

echo

# Step 3: Parse forensic.jsonl independently
log_info "Step 3: Parsing forensic.jsonl independently..."

if [[ -f "$FORENSIC_JSONL" ]]; then
    # Count records and extract issue IDs
    FORENSIC_COUNT=$(wc -l < "$FORENSIC_JSONL" 2>/dev/null || echo "0")
    log_success "Forensic record count: $FORENSIC_COUNT"

    # Extract unique issue IDs (independent of bead CLI)
    mkdir -p "$TEMP_DIR"
    if command -v jq &> /dev/null; then
        jq -r '.issue.id // empty' "$FORENSIC_JSONL" 2>/dev/null | sort -u > "$TEMP_DIR/checkpoint_issues.txt"
        CHECKPOINT_ISSUE_LIST=$(cat "$TEMP_DIR/checkpoint_issues.txt")

        if [[ "$VERBOSE" == "true" ]]; then
            log_info "Checkpoint issue IDs:"
            echo "$CHECKPOINT_ISSUE_LIST" | head -20
            if [[ $(echo "$CHECKPOINT_ISSUE_LIST" | wc -l) -gt 20 ]]; then
                log_info "... and more ($(echo "$CHECKPOINT_ISSUE_LIST" | wc -l) total)"
            fi
        fi
    else
        log_warning "jq not available - skipping issue ID extraction"
    fi
else
    log_error "Cannot parse forensic.jsonl - file missing"
fi

echo

# Step 4: Query database independently
log_info "Step 4: Querying database independently..."

DATABASE_OK=true
if [[ -f "$BEADS_DB" ]]; then
    log_success "Database file exists"

    # Try to query database using sqlite3
    if command -v sqlite3 &> /dev/null; then
        DB_QUERY_RESULT=$(sqlite3 "$BEADS_DB" "SELECT id FROM issues ORDER BY id;" 2>/dev/null || echo "")

        if [[ -n "$DB_QUERY_RESULT" ]]; then
            DATABASE_COUNT=$(echo "$DB_QUERY_RESULT" | wc -l)
            log_success "Database issue count: $DATABASE_COUNT"

            # Save database issue list for comparison
            echo "$DB_QUERY_RESULT" > "$TEMP_DIR/database_issues.txt"
        else
            log_error "Database query failed or returned no results"
            DATABASE_OK=false
        fi
    else
        log_warning "sqlite3 not available - skipping independent database query"

        # Fall back to bead CLI
        if "$BEAD_CLI" list --json &> /dev/null; then
            DATABASE_COUNT=$("$BEAD_CLI" list --json 2>/dev/null | jq -r '.id // empty' | wc -l)
            log_success "Database issue count (via bead CLI): $DATABASE_COUNT"
        else
            log_error "Neither sqlite3 nor bead CLI available for database query"
            DATABASE_OK=false
        fi
    fi
else
    log_error "Database file missing: $BEADS_DB"
    DATABASE_OK=false
fi

echo

# Step 5: Compare checkpoint and database
log_info "Step 5: Comparing checkpoint and database..."

if [[ "$structure_ok" == "true" && "$DATABASE_OK" == "true" ]]; then
    if [[ -f "$TEMP_DIR/checkpoint_issues.txt" && -f "$TEMP_DIR/database_issues.txt" ]]; then
        # Find issues in checkpoint but not in database
        MISSING_IN_DB=$(comm -13 <(sort "$TEMP_DIR/database_issues.txt") <(sort "$TEMP_DIR/checkpoint_issues.txt") || true)

        # Find issues in database but not in checkpoint
        MISSING_IN_CHECKPOINT=$(comm -23 <(sort "$TEMP_DIR/database_issues.txt") <(sort "$TEMP_DIR/checkpoint_issues.txt") || true)

        MISSING_IN_DB_COUNT=$(echo "$MISSING_IN_DB" | grep -v '^$' | wc -l)
        MISSING_IN_CHECKPOINT_COUNT=$(echo "$MISSING_IN_CHECKPOINT" | grep -v '^$' | wc -l)

        if [[ "$MISSING_IN_DB_COUNT" -eq 0 && "$MISSING_IN_CHECKPOINT_COUNT" -eq 0 ]]; then
            log_success "Checkpoint and database are synchronized"
        else
            log_warning "Checkpoint drift detected:"

            if [[ "$MISSING_IN_DB_COUNT" -gt 0 ]]; then
                log_warning "  Issues in checkpoint but missing in database: $MISSING_IN_DB_COUNT"
                if [[ "$VERBOSE" == "true" ]]; then
                    echo "$MISSING_IN_DB" | head -10 | while read -r issue_id; do
                        log_warning "    - $issue_id"
                    done
                fi
            fi

            if [[ "$MISSING_IN_CHECKPOINT_COUNT" -gt 0 ]]; then
                log_warning "  Issues in database but missing in checkpoint: $MISSING_IN_CHECKPOINT_COUNT"
                if [[ "$VERBOSE" == "true" ]]; then
                    echo "$MISSING_IN_CHECKPOINT" | head -10 | while read -r issue_id; do
                        log_warning "    - $issue_id"
                    done
                fi
            fi
        fi
    fi
else
    log_warning "Cannot compare - checkpoint or database verification failed"
fi

echo

# Step 6: Check staleness
log_info "Step 6: Checking checkpoint staleness..."

if [[ -n "$CHECKPOINT_TIMESTAMP" ]]; then
    # Convert timestamp to seconds since epoch
    if command -v date &> /dev/null; then
        # Try GNU date first
        CHECKPOINT_EPOCH=$(date -d "$CHECKPOINT_TIMESTAMP" +%s 2>/dev/null || echo "")

        # If GNU date failed, try BSD date
        if [[ -z "$CHECKPOINT_EPOCH" ]]; then
            CHECKPOINT_EPOCH=$(date -j -f "%Y-%m-%dT%H:%M:%SZ" "$CHECKPOINT_TIMESTAMP" +%s 2>/dev/null || echo "")
        fi

        if [[ -n "$CHECKPOINT_EPOCH" ]]; then
            CURRENT_EPOCH=$(date +%s)
            AGE_SECONDS=$((CURRENT_EPOCH - CHECKPOINT_EPOCH))
            AGE_MINUTES=$((AGE_SECONDS / 60))

            log_success "Checkpoint age: ${AGE_MINUTES} minutes"

            if [[ "$AGE_MINUTES" -gt 10 ]]; then
                log_warning "Checkpoint is stale (> 10 minutes old)"
            elif [[ "$AGE_MINUTES" -gt 5 ]]; then
                log_warning "Checkpoint is moderately stale (> 5 minutes old)"
            else
                log_success "Checkpoint is fresh"
            fi
        fi
    fi
fi

echo

# Summary
log_info "Summary:"

ISSUES_FOUND=0

if [[ "$structure_ok" != "true" ]]; then
    log_error "Checkpoint structure issues detected"
    ((ISSUES_FOUND++))
fi

if [[ "$DATABASE_OK" != "true" ]]; then
    log_error "Database access issues detected"
    ((ISSUES_FOUND++))
fi

if [[ "$MISSING_IN_DB_COUNT" -gt 0 ]]; then
    log_error "Issues in checkpoint but not in database"
    ((ISSUES_FOUND++))
fi

if [[ "$MISSING_IN_CHECKPOINT_COUNT" -gt 0 ]]; then
    log_error "Issues in database but not in checkpoint"
    ((ISSUES_FOUND++))
fi

if [[ "$ISSUES_FOUND" -eq 0 ]]; then
    log_success "No issues detected - checkpoint is healthy"
    exit 0
else
    log_error "Verification completed with $ISSUES_FOUND issue(s) found"
    exit 1
fi
