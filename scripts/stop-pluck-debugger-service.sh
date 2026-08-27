#!/bin/bash
# Stop Pluck Query Debugger Service

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PID_FILE="$WORKSPACE_ROOT/.beads/pluck-debugger-service.pid"

# Ensure we're in the workspace
cd "$WORKSPACE_ROOT"

# Check if PID file exists
if [ ! -f "$PID_FILE" ]; then
    echo "Pluck Query Debugger service is not running (no PID file)"
    exit 0
fi

# Read PID
PID=$(cat "$PID_FILE")

# Check if process is running
if ! ps -p "$PID" > /dev/null 2>&1; then
    echo "Removing stale PID file (process not running)"
    rm -f "$PID_FILE"
    exit 0
fi

echo "Stopping Pluck Query Debugger service (PID: $PID)..."
kill "$PID"

# Wait for process to terminate
for i in {1..10}; do
    if ! ps -p "$PID" > /dev/null 2>&1; then
        echo "Service stopped successfully"
        rm -f "$PID_FILE"
        exit 0
    fi
    sleep 1
done

# Force kill if still running
echo "Force killing service..."
kill -9 "$PID" 2>/dev/null || true
rm -f "$PID_FILE"

echo "Service stopped (force killed)"
exit 0
