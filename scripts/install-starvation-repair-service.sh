#!/usr/bin/env bash
# Installation script for bead-starvation-repair systemd service
# Run with sudo to install the service and timer

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SERVICE_DIR="$PROJECT_DIR/systemd"

echo "Installing bead-starvation-repair systemd service..."
echo "Service files location: $SERVICE_DIR"
echo ""

# Verify files exist
if [ ! -f "$SERVICE_DIR/bead-starvation-repair.service" ]; then
    echo "ERROR: Service file not found at $SERVICE_DIR/bead-starvation-repair.service"
    exit 1
fi

if [ ! -f "$SERVICE_DIR/bead-starvation-repair.timer" ]; then
    echo "ERROR: Timer file not found at $SERVICE_DIR/bead-starvation-repair.timer"
    exit 1
fi

# Copy service files to systemd directory
echo "Copying service files to /etc/systemd/system/..."
cp "$SERVICE_DIR/bead-starvation-repair.service" /etc/systemd/system/
cp "$SERVICE_DIR/bead-starvation-repair.timer" /etc/systemd/system/

# Reload systemd daemon
echo "Reloading systemd daemon..."
systemctl daemon-reload

# Enable and start the timer
echo "Enabling bead-starvation-repair timer..."
systemctl enable bead-starvation-repair.timer

echo "Starting bead-starvation-repair timer..."
systemctl start bead-starvation-repair.timer

# Display status
echo ""
echo "✓ Installation complete!"
echo ""
echo "Service status:"
systemctl status bead-starvation-repair.timer --no-pager
echo ""
echo "Timer schedule: Every 15 minutes"
echo "Log viewing: journalctl -u bead-starvation-repair.service -f"
echo "Manual trigger: systemctl start bead-starvation-repair.service"
echo "To disable: systemctl disable --now bead-starvation-repair.timer"
