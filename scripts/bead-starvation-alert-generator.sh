#!/usr/bin/env bash
# Enhanced Starvation Alert Generator
# Creates a starvation alert bead with embedded diagnostic context
# The alert bead contains all information needed for automated repair

set -euo pipefail

DB_PATH="${DB_PATH:-/home/coding/irreversible-command-gate/.beads/beads.db}"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
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

log_info "Collecting diagnostic context..."

# Step 1: Get total open bead count
TOTAL_OPEN=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM issues WHERE base_status IN ('open', 'in_progress', 'deferred');")
log_info "Total open beads: $TOTAL_OPEN"

# Step 2: Get ready bead count (what `bead list --ready` would return)
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
")
log_info "Ready beads (unblocked, unassigned): $READY_COUNT"

# Step 3: Detect assigned-but-open beads
ASSIGNED_BUT_OPEN=$(sqlite3 "$DB_PATH" "
SELECT COUNT(*)
FROM issues
WHERE base_status = 'open' AND assignee IS NOT NULL;
")
log_info "Assigned-but-open beads: $ASSIGNED_BUT_OPEN"

# Step 4: Get sample of 5 open bead IDs with their states
BEAD_SAMPLE=$(sqlite3 -json "$DB_PATH" "
SELECT
    id,
    title,
    base_status,
    COALESCE(assignee, 'unassigned') as assignee,
    manual_blocked,
    priority,
    issue_type
FROM issues
WHERE base_status IN ('open', 'in_progress', 'deferred')
ORDER BY
    CASE priority WHEN 4 THEN 1 WHEN 3 THEN 2 WHEN 2 THEN 3 WHEN 1 THEN 4 WHEN 0 THEN 5 END,
    created_at ASC
LIMIT 5;
")

# Step 5: Get label distribution of open beads
LABEL_DISTRIBUTION=$(sqlite3 -json "$DB_PATH" "
SELECT
    label,
    COUNT(*) as count
FROM labels l
JOIN issues i ON l.issue_id = i.id
WHERE i.base_status IN ('open', 'in_progress', 'deferred')
GROUP BY label
ORDER BY count DESC
LIMIT 20;
")

# Step 6: Get blocking chain information
BLOCKED_BEADS_COUNT=$(sqlite3 "$DB_PATH" "
SELECT COUNT(DISTINCT d.blocked_issue_id)
FROM dependencies d
WHERE d.kind = 'blocks'
  AND d.blocked_issue_id IN (
    SELECT id FROM issues WHERE base_status != 'closed'
  )
  AND d.blocker_issue_id IN (
    SELECT id FROM issues WHERE base_status != 'closed'
  );
")

# Step 7: Get circular dependency candidates
CIRCULAR_CANDIDATES=$(sqlite3 -json "$DB_PATH" "
WITH cycle_candidates AS (
    SELECT
        d1.blocked_issue_id as bead1,
        d2.blocked_issue_id as bead2
    FROM dependencies d1
    JOIN dependencies d2 ON d1.blocker_issue_id = d2.blocked_issue_id
                       AND d1.blocked_issue_id = d2.blocker_issue_id
    WHERE d1.kind = 'blocks' AND d2.kind = 'blocks'
      AND d1.blocked_issue_id != d2.blocked_issue_id
    LIMIT 10
)
SELECT
    bead1,
    bead2
FROM cycle_candidates;
")

# Step 8: Get orphaned dependency count
ORPHANED_DEPS=$(sqlite3 "$DB_PATH" "
SELECT COUNT(*)
FROM dependencies d
LEFT JOIN issues i ON d.blocker_issue_id = i.id
WHERE d.kind = 'blocks'
  AND (i.id IS NULL OR i.base_status = 'closed');
")

# Build the diagnostic context JSON
DIAGNOSTIC_CONTEXT=$(jq -n \
    --arg timestamp "$TIMESTAMP" \
    --arg total_open "$TOTAL_OPEN" \
    --arg ready_count "$READY_COUNT" \
    --arg assigned_but_open "$ASSIGNED_BUT_OPEN" \
    --arg blocked_beads_count "$BLOCKED_BEADS_COUNT" \
    --arg orphaned_deps "$ORPHANED_DEPS" \
    --arg bead_sample "$BEAD_SAMPLE" \
    --arg label_distribution "$LABEL_DISTRIBUTION" \
    --arg circular_candidates "$CIRCULAR_CANDIDATES" \
    '{
        timestamp: $timestamp,
        summary: {
            total_open_beads: ($total_open | tonumber),
            ready_beads: ($ready_count | tonumber),
            assigned_but_open: ($assigned_but_open | tonumber),
            blocked_beads: ($blocked_beads_count | tonumber),
            orphaned_dependencies: ($orphaned_deps | tonumber)
        },
        bead_sample: ($bead_sample | if . == "" then [] else fromjson end),
        label_distribution: ($label_distribution | if . == "" then [] else fromjson end),
        circular_dependency_candidates: ($circular_candidates | if . == "" then [] else fromjson end),
        pluck_query_filters: {
            ready_frontier: {
                filter: "--ready",
                description: "Open, unassigned, not blocked by open beads",
                value: ($ready_count | tonumber),
                sql: "base_status = \"open\" AND assignee IS NULL AND manual_blocked = 0 AND NOT EXISTS (SELECT 1 FROM dependencies WHERE blocked_issue_id = issues.id AND kind = \"blocks\" AND blocker_issue_id IN (SELECT id FROM issues WHERE base_status != \"closed\"))"
            },
            status_filter: {
                filter: "--status",
                possible_values: ["open", "in_progress", "deferred", "closed"],
                active_filter: "IN (open, in_progress, deferred)",
                total_matching: ($total_open | tonumber)
            },
            assignee_filter: {
                filter: "--assignee",
                description: "Filter by exact assignee name",
                stuck_state_detected: (($assigned_but_open | tonumber) > 0),
                stuck_state_count: ($assigned_but_open | tonumber)
            }
        }
    }')

# ==============================================================================
# VALIDATION: Check for contradictory data before creating alert
# ==============================================================================

# Validate: If description claims "open beads exist", count must be > 0
# This prevents false-positive alerts with contradictory data
if [ "$TOTAL_OPEN" -eq 0 ]; then
    log_error "VALIDATION FAILURE: Alert would claim 'open beads exist' but TOTAL_OPEN is 0"
    log_error "This is a contradictory condition - suppressing alert creation"
    log_error "Ready count: $READY_COUNT, Total open: $TOTAL_OPEN, Assigned but open: $ASSIGNED_BUT_OPEN"

    # Log validation failure to diagnostic log if available
    VALIDATION_LOG="/tmp/starvation-alert-validation-failures.log"
    echo "[$TIMESTAMP] VALIDATION_FAILURE: Contradictory data - TOTAL_OPEN=0 but READY_COUNT=$READY_COUNT" >> "$VALIDATION_LOG"
    echo "  Reason: Cannot create alert claiming 'open beads exist' when count is 0" >> "$VALIDATION_LOG"
    echo "  Workspace: $(pwd)" >> "$VALIDATION_LOG"

    log_error "Validation failure logged to $VALIDATION_LOG"
    log_error "Alert creation suppressed - no bead created"

    # Exit with error to indicate validation failure
    exit 1
fi

# Determine if starvation is detected
IS_STARVATION=false
STARVATION_REASON=""

if [ "$READY_COUNT" -eq 0 ] && [ "$TOTAL_OPEN" -gt 0 ]; then
    IS_STARVATION=true
    STARVATION_REASON="No ready beads found despite $TOTAL_OPEN open beads"

    if [ "$ASSIGNED_BUT_OPEN" -gt 0 ]; then
        STARVATION_REASON="$STARVATION_REASON ($ASSIGNED_BUT_OPEN beads in assigned-but-open stuck state)"
    fi

    if [ "$BLOCKED_BEADS_COUNT" -gt 0 ]; then
        STARVATION_REASON="$STARVATION_REASON ($BLOCKED_BEADS_COUNT beads blocked by open dependencies)"
    fi
fi

# Generate the alert bead title
if [ "$IS_STARVATION" = true ]; then
    ALERT_TITLE="🚨 Starvation Alert: Beads invisible in ready frontier ($TIMESTAMP)"
    ALERT_PRIORITY="1"
    ALERT_TYPE="incident"
else
    ALERT_TITLE="⚠️ Starvation Warning: Low ready bead count ($READY_COUNT/$TOTAL_OPEN reachable at $TIMESTAMP)"
    ALERT_PRIORITY="2"
    ALERT_TYPE="warning"
fi

# Build the bead body with embedded diagnostic context
# Using a heredoc to avoid variable expansion issues
cat > /tmp/starvation_alert_body.txt <<'EOF'
# Starvation Alert: Embedded Diagnostic Context

## Alert Timestamp
EOF

echo "$TIMESTAMP" >> /tmp/starvation_alert_body.txt
echo "" >> /tmp/starvation_alert_body.txt

cat >> /tmp/starvation_alert_body.txt <<'EOF'
## Starvation Status
EOF

if [ "$IS_STARVATION" = true ]; then
    echo "**STARVATION DETECTED**" >> /tmp/starvation_alert_body.txt
    echo "" >> /tmp/starvation_alert_body.txt
    echo "**Reason:** $STARVATION_REASON" >> /tmp/starvation_alert_body.txt
else
    echo "**WARNING**: Low ready bead count but not full starvation" >> /tmp/starvation_alert_body.txt
fi

cat >> /tmp/starvation_alert_body.txt <<EOF

## Summary Statistics
- **Total Open Beads:** $TOTAL_OPEN
- **Ready Beads (accessible):** $READY_COUNT
- **Assigned-but-Open (stuck state):** $ASSIGNED_BUT_OPEN
- **Blocked Beads:** $BLOCKED_BEADS_COUNT
- **Orphaned Dependencies:** $ORPHANED_DEPS

## Pluck Query Filters Analysis
### \`--ready\` Filter
The \`bead list --ready\` command uses these filters:
- **Status:** \`open\` (excludes in_progress, deferred, closed)
- **Assignee:** NULL (unassigned only)
- **Manual Blocked:** false
- **Blocking Dependencies:** None from open beads

**Current Results:**
- Ready beads matching filter: **$READY_COUNT**
- Total open beads: **$TOTAL_OPEN**
- Starvation gap: **$((TOTAL_OPEN - READY_COUNT))** beads excluded

### \`--status\` Filter
- **Current active filter:** \`IN (open, in_progress, deferred)\`
- **Beads matching:** $TOTAL_OPEN

### \`--assignee\` Filter
- **Possible values:** Any assignee name or NULL for unassigned
EOF

if [ "$ASSIGNED_BUT_OPEN" -gt 0 ]; then
    echo "- **Stuck state detected:** **YES** - $ASSIGNED_BUT_OPEN beads have assignees but are open" >> /tmp/starvation_alert_body.txt
else
    echo "- **Stuck state detected:** No" >> /tmp/starvation_alert_body.txt
fi

cat >> /tmp/starvation_alert_body.txt <<'EOF'

## Sample of Open Beads (First 5 by priority/age)
EOF

if [ -n "$BEAD_SAMPLE" ] && [ "$BEAD_SAMPLE" != "[]" ]; then
    echo "$BEAD_SAMPLE" | jq -r '
        .[] |
        "
### \(.id) - \(.title)
- **Status:** \(.base_status)
- **Assignee:** \(.assignee)
- **Priority:** P\(.priority)
- **Type:** \(.issue_type)
- **Manual Blocked:** \(.manual_blocked)
"' >> /tmp/starvation_alert_body.txt
else
    echo "No bead sample available" >> /tmp/starvation_alert_body.txt
fi

cat >> /tmp/starvation_alert_body.txt <<'EOF'

## Label Distribution (Open Beads)
EOF

if [ -n "$LABEL_DISTRIBUTION" ] && [ "$LABEL_DISTRIBUTION" != "[]" ]; then
    echo "$LABEL_DISTRIBUTION" | jq -r '
        if length > 0 then
            .[] |
            "- **\(.label):** \(.count) beads"
        else
            "No labels found on open beads"
        end' >> /tmp/starvation_alert_body.txt
else
    echo "No label distribution data available" >> /tmp/starvation_alert_body.txt
fi

cat >> /tmp/starvation_alert_body.txt <<'EOF'

## Dependency Graph Issues
### Circular Dependency Candidates
EOF

if [ -n "$CIRCULAR_CANDIDATES" ] && [ "$CIRCULAR_CANDIDATES" != "[]" ]; then
    CIRCULAR_COUNT=$(echo "$CIRCULAR_CANDIDATES" | jq 'length')
    if [ "$CIRCULAR_COUNT" -gt 0 ]; then
        echo "Potential circular dependencies detected:" >> /tmp/starvation_alert_body.txt
        echo "$CIRCULAR_CANDIDATES" | jq -r '.[] | "- \(.bead1) ↔ \(.bead2)"' >> /tmp/starvation_alert_body.txt
    else
        echo "No obvious circular dependency patterns detected" >> /tmp/starvation_alert_body.txt
    fi
else
    echo "No circular dependency data available" >> /tmp/starvation_alert_body.txt
fi

cat >> /tmp/starvation_alert_body.txt <<EOF

### Orphaned Dependencies
- **Count:** $ORPHANED_DEPS dependencies reference closed/non-existent beads

## Automated Repair Recommendations

### 1. Fix Assigned-but-Open Beads
\`\`\`bash
# Get stuck-state beads
sqlite3 .beads/beads.db "SELECT id FROM issues WHERE base_status = 'open' AND assignee IS NOT NULL;" | while read bead_id; do
  bead update "\$bead_id" --clear-assignee --notes "Auto-repair: cleared stale assignee (starvation alert $TIMESTAMP)"
done
\`\`\`

### 2. Fix Orphaned Dependencies
\`\`\`bash
# Run dependency validator
cargo run --bin bead-dependency-validator
\`\`\`

### 3. Verify Ready Frontier
\`\`\`bash
# Check ready bead count after repairs
bead list --ready --limit 100 | wc -l
\`\`\`

## Machine-Readable Diagnostic Context
Below is the complete diagnostic context in JSON format for automated processing:

\`\`\`json
EOF

echo "$DIAGNOSTIC_CONTEXT" | jq '.' >> /tmp/starvation_alert_body.txt

cat >> /tmp/starvation_alert_body.txt <<'EOF'
\`\`\`

---

This alert was auto-generated by the enhanced starvation diagnostic system.
All diagnostic context is embedded in this bead for automated repair processing.
EOF

# Read the complete body into a variable
BEAD_BODY=$(cat /tmp/starvation_alert_body.txt)

# Create the bead
log_info "Creating starvation alert bead..."

if [ "$IS_STARVATION" = true ]; then
    log_warn "Starvation detected - creating incident bead"
else
    log_info "Warning level alert - creating diagnostic bead"
fi

# Create the bead with embedded diagnostics using --description
BEAD_ID=$(bead create \
    --title "$ALERT_TITLE" \
    --description "$BEAD_BODY" \
    --priority "$ALERT_PRIORITY" \
    --issue-type "$ALERT_TYPE" \
    --label "starvation-alert" \
    --label "monitoring")

if [ -n "$BEAD_ID" ]; then
    log_info "Alert bead created: $BEAD_ID"

    # Also output the diagnostic context to stdout for logging
    echo "$DIAGNOSTIC_CONTEXT" | jq '.'

    log_info "Starvation alert generation complete"
    exit 0
else
    log_error "Failed to create alert bead"
    exit 1
fi
