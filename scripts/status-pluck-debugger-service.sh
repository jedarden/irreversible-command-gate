#!/bin/bash
# Check Pluck Query Debugger Service Status

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PID_FILE="$WORKSPACE_ROOT/.beads/pluck-debugger-service.pid"
LOG_FILE="$WORKSPACE_ROOT/.beads/pluck-debugger-service.log"

# Ensure we're in the workspace
cd "$WORKSPACE_ROOT"

echo "Pluck Query Debugger Service Status"
echo "=================================="
echo ""

# Check if PID file exists
if [ ! -f "$PID_FILE" ]; then
    echo "Status: STOPPED (no PID file)"
    exit 0
fi

# Read PID
PID=$(cat "$PID_FILE")

# Check if process is running
if ! ps -p "$PID" > /dev/null 2>&1; then
    echo "Status: STOPPED (stale PID file)"
    echo "PID file exists but process $PID is not running"
    echo "Clean up with: rm $PID_FILE"
    exit 1
fi

# Process is running
echo "Status: RUNNING"
echo "PID: $PID"
echo "Started: $(ps -p $PID -o lstart=)"
echo "CPU time: $(ps -p $PID -o cputime=)"
echo "Memory: $(ps -p $PID -o rss= | awk '{printf "%.1f MB", $1/1024}')"
echo ""

# Show recent log entries if available
if [ -f "$LOG_FILE" ]; then
    echo "Recent log entries:"
    echo "-------------------"
    tail -10 "$LOG_FILE"
else
    echo "No log file found at: $LOG_FILE"
fi

exit 0
