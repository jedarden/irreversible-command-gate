#!/usr/bin/env bash
# Installation script for bead-starvation-direct-repair systemd service
# Automatically detects and fixes beads in 'assigned-but-open' state

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SERVICE_DIR="$PROJECT_DIR/systemd"
SERVICE_NAME="bead-starvation-direct-repair"

echo "Installing $SERVICE_NAME systemd service..."
echo "Service files location: $SERVICE_DIR"
echo ""

# Check if running as root
if [ "$EUID" -ne 0 ]; then
    echo "ERROR: This script must be run as root (use sudo)"
    echo ""
    echo "Usage: sudo $0"
    exit 1
fi

# Verify files exist
if [ ! -f "$SERVICE_DIR/$SERVICE_NAME.service" ]; then
    echo "ERROR: Service file not found at $SERVICE_DIR/$SERVICE_NAME.service"
    exit 1
fi

if [ ! -f "$SERVICE_DIR/$SERVICE_NAME.timer" ]; then
    echo "ERROR: Timer file not found at $SERVICE_DIR/$SERVICE_NAME.timer"
    exit 1
fi

if [ ! -f "$SCRIPT_DIR/bead-starvation-direct-repair.sh" ]; then
    echo "ERROR: Repair script not found at $SCRIPT_DIR/bead-starvation-direct-repair.sh"
    exit 1
fi

# Copy service files to systemd directory
echo "Copying service files to /etc/systemd/system/..."
cp "$SERVICE_DIR/$SERVICE_NAME.service" /etc/systemd/system/
cp "$SERVICE_DIR/$SERVICE_NAME.timer" /etc/systemd/system/

# Reload systemd daemon
echo "Reloading systemd daemon..."
systemctl daemon-reload

# Enable and start the timer
echo "Enabling $SERVICE_NAME timer..."
systemctl enable "$SERVICE_NAME.timer"

echo "Starting $SERVICE_NAME timer..."
systemctl start "$SERVICE_NAME.timer"

# Display status
echo ""
echo "✓ Installation complete!"
echo ""
echo "Service status:"
systemctl status "$SERVICE_NAME.timer" --no-pager
echo ""
echo "Next scheduled runs:"
systemctl list-timers "$SERVICE_NAME.timer" --no-pager
echo ""
echo "Timer schedule: Every 15 minutes"
echo "Log viewing: journalctl -u $SERVICE_NAME.service -f"
echo "Manual trigger: systemctl start $SERVICE_NAME.service"
echo "Manual dry-run: $SCRIPT_DIR/bead-starvation-direct-repair.sh --dry-run --verbose"
echo ""
echo "To disable: systemctl disable --now $SERVICE_NAME.timer"
echo "To stop: systemctl stop $SERVICE_NAME.timer"
