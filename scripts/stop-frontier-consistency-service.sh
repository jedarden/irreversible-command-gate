#!/bin/bash
# Frontier Consistency Service Stop Script
# This script stops the frontier consistency service background daemon

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PID_FILE="$WORKSPACE_ROOT/.beads/frontier-consistency-service.pid"

# Ensure we're in the workspace
cd "$WORKSPACE_ROOT"

# Check if PID file exists
if [ ! -f "$PID_FILE" ]; then
    echo "Service is not running (no PID file found)"
    exit 0
fi

PID=$(cat "$PID_FILE")

# Check if process is running
if ! ps -p "$PID" > /dev/null 2>&1; then
    echo "Service is not running (stale PID file)"
    rm -f "$PID_FILE"
    exit 0
fi

echo "Stopping Frontier Consistency Service (PID: $PID)..."
kill "$PID"

# Wait for process to terminate
TIMEOUT=10
while ps -p "$PID" > /dev/null 2>&1 && [ $TIMEOUT -gt 0 ]; do
    sleep 1
    TIMEOUT=$((TIMEOUT - 1))
done

# Force kill if still running
if ps -p "$PID" > /dev/null 2>&1; then
    echo "Service did not stop gracefully, forcing..."
    kill -9 "$PID"
    sleep 1
fi

# Remove PID file
rm -f "$PID_FILE"

echo "Service stopped successfully"
exit 0
