#!/bin/bash
# Frontier Consistency Service Background Wrapper
# This script runs the frontier consistency service as a background daemon

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY_PATH="$WORKSPACE_ROOT/target/debug/frontier-consistency-check"
PID_FILE="$WORKSPACE_ROOT/.beads/frontier-consistency-service.pid"
LOG_FILE="$WORKSPACE_ROOT/.beads/frontier-consistency-service.log"

# Ensure we're in the workspace
cd "$WORKSPACE_ROOT"

# Check if service is already running
if [ -f "$PID_FILE" ]; then
    PID=$(cat "$PID_FILE")
    if ps -p "$PID" > /dev/null 2>&1; then
        echo "Service is already running (PID: $PID)"
        exit 1
    else
        echo "Removing stale PID file"
        rm -f "$PID_FILE"
    fi
fi

# Ensure binary exists
if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at $BINARY_PATH"
    echo "Please run: cargo build --bin frontier-consistency-check"
    exit 1
fi

echo "Starting Frontier Consistency Service..."
echo "Workspace: $WORKSPACE_ROOT"
echo "Binary: $BINARY_PATH"
echo "Log file: $LOG_FILE"

# Start the service in background
nohup "$BINARY_PATH" \
    --workspace "$WORKSPACE_ROOT" \
    >> "$LOG_FILE" 2>&1 &

PID=$!
echo $PID > "$PID_FILE"

echo "Service started successfully (PID: $PID)"
echo "Monitor logs: tail -f $LOG_FILE"
echo "Stop service: kill $PID"
echo "Or use: scripts/stop-frontier-consistency-service.sh"

exit 0
