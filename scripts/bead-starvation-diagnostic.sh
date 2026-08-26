#!/usr/bin/env bash
# Bead Starvation Diagnostic Tool
# Queries bead-rs SQLite database to identify and repair starvation causes

set -euo pipefail

DB_PATH="/home/coding/irreversible-command-gate/.beads/beads.db"
OUTPUT_DIR="/home/coding/irreversible-command-gate/.beads/diagnostics"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
OUTPUT_FILE="${OUTPUT_DIR}/starvation-report-${TIMESTAMP}.json"

mkdir -p "$OUTPUT_DIR"

# Ensure we have the bead CLI
if ! command -v bead &> /dev/null; then
    echo "Error: bead CLI not found" >&2
    exit 1
fi

# Step 1: Query all open beads with status, assignee, dependencies, and labels
echo "Querying bead database..." >&2

QUERY=$(cat <<'EOF'
WITH open_beads AS (
    SELECT
        i.id,
        i.title,
        i.base_status,
        i.assignee,
        i.priority,
        i.issue_type,
        i.manual_blocked,
        i.ready_since,
        i.updated_at,
        i.created_at
    FROM issues i
    WHERE i.base_status IN ('open', 'in_progress', 'deferred')
),
bead_labels AS (
    SELECT
        issue_id,
        GROUP_CONCAT(label, ',') AS labels
    FROM labels
    GROUP BY issue_id
),
bead_dependencies AS (
    SELECT
        d.blocked_issue_id,
        d.blocker_issue_id,
        i.title AS blocker_title,
        i.base_status AS blocker_status,
        d.kind
    FROM dependencies d
    JOIN issues i ON d.blocker_issue_id = i.id
    WHERE d.kind = 'blocks'
),
dependency_summary AS (
    SELECT
        blocked_issue_id,
        COUNT(CASE WHEN blocker_status != 'closed' THEN 1 END) AS open_blocking_count,
        GROUP_CONCAT(
            CASE WHEN blocker_status != 'closed'
                THEN blocker_id || ':' || blocker_title || '(' || blocker_status || ')'
            END,
            '; '
        ) AS open_blockers
    FROM (
        SELECT
            d.blocked_issue_id,
            d.blocker_issue_id AS blocker_id,
            i.title AS blocker_title,
            i.base_status AS blocker_status
        FROM dependencies d
        JOIN issues i ON d.blocker_issue_id = i.id
        WHERE d.kind = 'blocks'
    )
    GROUP BY blocked_issue_id
)
SELECT
    json_object(
        'id', ob.id,
        'title', ob.title,
        'status', ob.base_status,
        'assignee', COALESCE(ob.assignee, 'unassigned'),
        'priority', ob.priority,
        'issue_type', ob.issue_type,
        'manual_blocked', ob.manual_blocked,
        'ready_since', ob.ready_since,
        'updated_at', ob.updated_at,
        'labels', COALESCE(bl.labels, ''),
        'open_blocking_count', COALESCE(ds.open_blocking_count, 0),
        'open_blockers', COALESCE(ds.open_blockers, '')
    ) AS bead_json
FROM open_beads ob
LEFT JOIN bead_labels bl ON ob.id = bl.issue_id
LEFT JOIN dependency_summary ds ON ob.id = ds.blocked_issue_id
ORDER BY
    CASE ob.priority
        WHEN 4 THEN 1
        WHEN 3 THEN 2
        WHEN 2 THEN 3
        WHEN 1 THEN 4
        WHEN 0 THEN 5
    END,
    ob.created_at;
EOF
)

# Execute query and build JSON array
BEADS_JSON=$(
    sqlite3 "$DB_PATH" <<< "$QUERY" | jq -s '{beads: .}'
)

# Step 2: Categorize beads
echo "Categorizing beads..." >&2

CATEGORIZED=$(
    echo "$BEADS_JSON" | jq '
        .beads |= map(
            .category = (
                if .open_blocking_count > 0 then
                    "truly_blocked"
                elif (.status == "open" and .assignee != "unassigned") then
                    "stuck_state"
                else
                    "unknown"
                end
            )
        )
    '
)

# Step 3: Create diagnostic report
echo "Creating diagnostic report..." >&2

REPORT=$(
    echo "$CATEGORIZED" | jq "
        {
            timestamp: \"$TIMESTAMP\",
            workspace: \"irreversible-command-gate\",
            summary: {
                total_open: (.beads | length),
                truly_blocked: (.beads | map(select(.category == \"truly_blocked\")) | length),
                stuck_state: (.beads | map(select(.category == \"stuck_state\")) | length),
                unknown: (.beads | map(select(.category == \"unknown\")) | length)
            },
            stuck_state_beads: [.beads[] | select(.category == \"stuck_state\")],
            truly_blocked_beads: [.beads[] | select(.category == \"truly_blocked\")],
            unknown_beads: [.beads[] | select(.category == \"unknown\")]
        }
    "
)

# Save report
echo "$REPORT" | jq '.' > "$OUTPUT_FILE"

echo "Report saved to: $OUTPUT_FILE" >&2

# Step 4: Print summary to stdout
echo "$REPORT" | jq '.'

# Step 5: Repair stuck-state beads
STUCK_COUNT=$(echo "$REPORT" | jq '.summary.stuck_state')
if [ "$STUCK_COUNT" -gt 0 ]; then
    echo "" >&2
    echo "Repairing $STUCK_COUNT stuck-state beads..." >&2

    echo "$REPORT" | jq -r '.stuck_state_beads[].id' | while read -r bead_id; do
        echo "Attempting repair: $bead_id" >&2
        if bead update "$bead_id" --clear-assignee --notes "Auto-repair: cleared stale assignee from stuck state (diagnostic $TIMESTAMP)" 2>/dev/null; then
            echo "  ✓ Successfully cleared assignee for $bead_id" >&2
        else
            echo "  ✗ Failed to clear assignee for $bead_id" >&2
        fi
    done
else
    echo "No stuck-state beads found." >&2
fi

# Step 6: Print blocking chains for truly blocked beads
TRULY_BLOCKED_COUNT=$(echo "$REPORT" | jq '.summary.truly_blocked')
if [ "$TRULY_BLOCKED_COUNT" -gt 0 ]; then
    echo "" >&2
    echo "Blocking dependency chains for $TRULY_BLOCKED_COUNT truly blocked beads:" >&2
    echo "$REPORT" | jq -r '.truly_blocked_beads[] | "\(.id): \(.title) blocked by \(.open_blocking_count) dependencies: \(.open_blockers)"' >&2
fi

echo "" >&2
echo "Diagnostic complete. Full report: $OUTPUT_FILE" >&2
