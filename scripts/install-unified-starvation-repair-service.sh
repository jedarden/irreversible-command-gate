#!/usr/bin/env bash
# Installation script for Unified Bead Starvation Auto-Repair Service
#
# This script installs the user-level systemd service and timer that run every 5 minutes
# to detect, diagnose, and automatically repair bead starvation conditions.
#
# Usage:
#   ./install-unified-starvation-repair-service.sh [--uninstall]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
SERVICE_NAME="bead-starvation-unified-repair"
SYSTEMD_DIR="${HOME}/.config/systemd/user"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Ensure user systemd directory exists
mkdir -p "$SYSTEMD_DIR"

# Parse arguments
UNINSTALL=false
if [ "${1:-}" = "--uninstall" ]; then
    UNINSTALL=true
fi

# Uninstall
if [ "$UNINSTALL" = true ]; then
    log_info "Uninstalling ${SERVICE_NAME} service..."

    # Stop and disable timer
    systemctl --user stop "${SERVICE_NAME}.timer" 2>/dev/null || true
    systemctl --user disable "${SERVICE_NAME}.timer" 2>/dev/null || true

    # Stop and disable service
    systemctl --user stop "${SERVICE_NAME}.service" 2>/dev/null || true
    systemctl --user disable "${SERVICE_NAME}.service" 2>/dev/null || true

    # Remove files
    rm -f "${SYSTEMD_DIR}/${SERVICE_NAME}.service"
    rm -f "${SYSTEMD_DIR}/${SERVICE_NAME}.timer"

    # Reload systemd
    systemctl --user daemon-reload

    log_info "✓ ${SERVICE_NAME} service uninstalled"
    exit 0
fi

# Install
log_info "Installing ${SERVICE_NAME} service..."

# Verify files exist
SERVICE_FILE="${PROJECT_ROOT}/systemd/${SERVICE_NAME}.service"
TIMER_FILE="${PROJECT_ROOT}/systemd/${SERVICE_NAME}.timer"
SCRIPT_FILE="${PROJECT_ROOT}/scripts/bead-starvation-unified-repair.sh"

if [ ! -f "$SERVICE_FILE" ]; then
    log_error "Service file not found: $SERVICE_FILE"
    exit 1
fi

if [ ! -f "$TIMER_FILE" ]; then
    log_error "Timer file not found: $TIMER_FILE"
    exit 1
fi

if [ ! -f "$SCRIPT_FILE" ]; then
    log_error "Script file not found: $SCRIPT_FILE"
    exit 1
fi

# Make script executable
chmod +x "$SCRIPT_FILE"

# Copy systemd files
cp "$SERVICE_FILE" "${SYSTEMD_DIR}/"
cp "$TIMER_FILE" "${SYSTEMD_DIR}/"

# Reload systemd
systemctl --user daemon-reload

# Enable and start timer
systemctl --user enable "${SERVICE_NAME}.timer"
systemctl --user start "${SERVICE_NAME}.timer"

# Show status
log_info "✓ ${SERVICE_NAME} service installed"
echo ""
log_info "Service status:"
systemctl --user status "${SERVICE_NAME}.timer" --no-pager
echo ""
log_info "Next run:"
systemctl --user list-timers "${SERVICE_NAME}.timer" --no-pager

log_info "Installation complete!"
log_info "The service will run every 5 minutes."
log_info "View logs with: journalctl --user -u ${SERVICE_NAME}.service -f"
