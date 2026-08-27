#!/bin/bash
# Pluck Query Debugger Background Service
# This script runs the pluck query debugger as a periodic background daemon

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY_PATH="$WORKSPACE_ROOT/target/debug/pluck-query-debugger"
PID_FILE="$WORKSPACE_ROOT/.beads/pluck-debugger-service.pid"
LOG_FILE="$WORKSPACE_ROOT/.beads/pluck-debugger-service.log"

# Default check interval: 10 minutes (600 seconds)
CHECK_INTERVAL=${PLUCK_DEBUGGER_INTERVAL:-600}

# Ensure we're in the workspace
cd "$WORKSPACE_ROOT"

# Check if service is already running
if [ -f "$PID_FILE" ]; then
    PID=$(cat "$PID_FILE")
    if ps -p "$PID" > /dev/null 2>&1; then
        echo "Pluck Query Debugger service is already running (PID: $PID)"
        exit 1
    else
        echo "Removing stale PID file"
        rm -f "$PID_FILE"
    fi
fi

# Ensure binary exists
if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at $BINARY_PATH"
    echo "Building binary..."
    cargo build --bin pluck-query-debugger --release
fi

echo "Starting Pluck Query Debugger Service..."
echo "Workspace: $WORKSPACE_ROOT"
echo "Binary: $BINARY_PATH"
echo "Check interval: ${CHECK_INTERVAL}s"
echo "Log file: $LOG_FILE"
echo ""

# Create a simple loop script that runs the debugger periodically
cat > /tmp/pluck-debugger-loop-$$.sh << 'LOOPEOF'
#!/bin/bash
WORKSPACE="$1"
INTERVAL="$2"
LOG_FILE="$3"

while true; do
    echo "=== Pluck Query Debugger run at $(date -u +"%Y-%m-%d %H:%M:%S UTC") ===" >> "$LOG_FILE"

    cd "$WORKSPACE"
    cargo run --bin pluck-query-debugger -- --summary >> "$LOG_FILE" 2>&1

    EXIT_CODE=$?
    if [ $EXIT_CODE -eq 1 ]; then
        echo "🚨 Starvation detected and diagnostic bead filed" >> "$LOG_FILE"
    elif [ $EXIT_CODE -ne 0 ]; then
        echo "⚠️  Unexpected error (exit code: $EXIT_CODE)" >> "$LOG_FILE"
    fi

    echo "" >> "$LOG_FILE"
    echo "Next check in ${INTERVAL}s..." >> "$LOG_FILE"
    sleep "$INTERVAL"
done
LOOPEOF

chmod +x /tmp/pluck-debugger-loop-$$.sh

# Start the service in background
nohup /tmp/pluck-debugger-loop-$$.sh "$WORKSPACE_ROOT" "$CHECK_INTERVAL" "$LOG_FILE" >> "$LOG_FILE" 2>&1 &

PID=$!
echo $PID > "$PID_FILE"

echo "Service started successfully (PID: $PID)"
echo "Monitor logs: tail -f $LOG_FILE"
echo "Stop service: kill $PID"
echo "Or use: scripts/stop-pluck-debugger-service.sh"

exit 0
