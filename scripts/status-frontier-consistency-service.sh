#!/bin/bash
# Frontier Consistency Service Status Script
# This script checks the status of the frontier consistency service

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PID_FILE="$WORKSPACE_ROOT/.beads/frontier-consistency-service.pid"
LOG_FILE="$WORKSPACE_ROOT/.beads/frontier-consistency-service.log"

# Ensure we're in the workspace
cd "$WORKSPACE_ROOT"

echo "=== Frontier Consistency Service Status ==="
echo ""

# Check if PID file exists
if [ ! -f "$PID_FILE" ]; then
    echo "Status: STOPPED (no PID file)"
    echo ""
    echo "To start: ./scripts/start-frontier-consistency-service.sh"
    exit 0
fi

PID=$(cat "$PID_FILE")

# Check if process is running
if ! ps -p "$PID" > /dev/null 2>&1; then
    echo "Status: STOPPED (stale PID file)"
    echo "PID: $PID (not running)"
    rm -f "$PID_FILE"
    echo ""
    echo "To start: ./scripts/start-frontier-consistency-service.sh"
    exit 0
fi

echo "Status: RUNNING"
echo "PID: $PID"
echo ""

# Show process info
echo "Process Details:"
ps -p "$PID" -o pid,ppid,%cpu,%mem,etime,cmd | tail -1
echo ""

# Show recent log entries
echo "Recent Log Entries (last 10 lines):"
echo "--------------------------------------"
tail -10 "$LOG_FILE"
echo ""

# Show latest diagnostic report
REPAIR_LOG="$WORKSPACE_ROOT/.beads/diagnostics/frontier-repair.jsonl"
if [ -f "$REPAIR_LOG" ]; then
    echo "Latest Consistency Check Report:"
    echo "--------------------------------"
    tail -1 "$REPAIR_LOG" | jq -r '
        "Cycle: \(.cycle_start // "unknown")",
        "Duration: \(.duration_seconds // "N/A")s",
        "Database beads: \(.total_database_beads // 0)",
        "Ready beads: \(.total_ready_beads // 0)",
        "Discrepancies: \(.discrepancies | length)",
        "Diagnoses: \(.diagnoses | length)",
        "Repairs: \(.repairs | length)",
        "Persistent issues: \(.persistent_reports | length)",
        "Alert triggered: \(.alert_triggered // false)"
    ' 2>/dev/null || echo "Unable to parse report"
    echo ""
fi

echo "Commands:"
echo "  Monitor logs: tail -f $LOG_FILE"
echo "  Stop service: ./scripts/stop-frontier-consistency-service.sh"
echo "  Restart: ./scripts/stop-frontier-consistency-service.sh && ./scripts/start-frontier-consistency-service.sh"

exit 0
