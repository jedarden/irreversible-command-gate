# Automated Bead Starvation Repair System

## Overview

This system automatically detects and repairs beads stuck in the **assigned-but-open** state - a known silent failure mode where beads have assignees but remain in open status, making them invisible to the pluck/ready frontier and causing fleet starvation.

## The Problem

The assigned-but-open state occurs when:
- A bead is claimed and assigned to a worker
- The worker processes it but fails to release/close it properly
- The bead remains assigned but stuck in open status
- Subsequent `bead pluck --ready` queries skip these beads
- Workers spin with nothing to claim while hundreds of beads are stuck

**Historical impact:** 583 beads were found stuck across 47 workspaces, with 10 workspaces fully starved.

## Solution Architecture

The system consists of three components:

### 1. Direct Repair Script (`bead-starvation-direct-repair.sh`)

**Core functionality:**
- Queries `beads.db` directly for `open AND assignee IS NOT NULL` beads
- Executes `bead update <id> --clear-assignee` for each stuck bead
- Logs all repairs to `.beads/diagnostics/repair-log-<timestamp>.json`
- Supports dry-run mode for validation
- Provides verification after repair

**Key features:**
- **No dependencies on alert beads** - works directly on the database
- **Idempotent** - safe to run multiple times
- **Auditable** - detailed JSON logs for every repair
- **Safe** - dry-run mode by default for testing

### 2. Systemd Service (`bead-starvation-direct-repair.service`)

**Configuration:**
- Runs as `oneshot` service (completes, then exits)
- Runs as the `coding` user with proper permissions
- **Checkpoint auto-flush disabled** during repair runs (`CHECKPOINT_AUTO_FLUSH=false`)
- Security hardening with `ProtectSystem=strict`, `NoNewPrivileges=true`
- Output logged to systemd journal

### 3. Systemd Timer (`bead-starvation-direct-repair.timer`)

**Schedule:**
- Runs every **15 minutes** (`OnCalendar=*:0/15`)
- Persistent scheduling (catches up after reboots)
- Randomized start time (±60s) to avoid drift

## Installation

```bash
# Install the service and timer
sudo ./scripts/install-direct-repair-service.sh
```

**Manual installation (if preferred):**
```bash
sudo cp systemd/bead-starvation-direct-repair.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now bead-starvation-direct-repair.timer
```

## Usage

### Check Service Status
```bash
# Timer status (next run time)
systemctl status bead-starvation-direct-repair.timer

# Recent runs
journalctl -u bead-starvation-direct-repair.service -n 50

# Follow logs in real-time
journalctl -u bead-starvation-direct-repair.service -f
```

### Manual Testing
```bash
# Dry-run (safe, shows what would be done)
./scripts/bead-starvation-direct-repair.sh --dry-run --verbose

# Verbose mode with detailed output
./scripts/bead-starvation-direct-repair.sh --verbose

# Manual trigger of repair
./scripts/bead-starvation-direct-repair.sh
```

### Manual Service Trigger
```bash
# Run immediately (bypassing timer)
sudo systemctl start bead-starvation-direct-repair.service
```

## Audit Trail

All repairs are logged to `.beads/diagnostics/repair-log-<timestamp>.json`:

```json
{
  "timestamp": "2026-08-26T12:00:00Z",
  "event": "repair_run_completed",
  "dry_run": false,
  "total_found": 3,
  "total_attempted": 3,
  "successful": 3,
  "failed": 0,
  "repairs": [
    {
      "bead_id": "irrevers-abc123",
      "assignee": "worker-alpha",
      "status": "success"
    }
  ]
}
```

## Monitoring and Validation

### Verify Service is Running
```bash
# Check timer is active and enabled
systemctl is-active bead-starvation-direct-repair.timer
systemctl is-enabled bead-starvation-direct-repair.timer

# Check next scheduled run
systemctl list-timers bead-starvation-direct-repair.timer
```

### Check Repair Logs
```bash
# List recent repair logs
ls -lt .beads/diagnostics/repair-log-*.json | head -10

# View latest repair results
jq . .beads/diagnostics/repair-log-$(ls -t .beads/diagnostics/repair-log-*.json | head -1 | xargs basename)
```

### Manual Database Query
```bash
# Check for stuck beads manually
sqlite3 .beads/beads.db "SELECT id, title, assignee FROM issues WHERE base_status = 'open' AND assignee IS NOT NULL;"
```

## Troubleshooting

### Service Not Running
```bash
# Check if service files are installed
ls -la /etc/systemd/system/bead-starvation-direct-repair.*

# Check service status
systemctl status bead-starvation-direct-repair.service

# Check for errors in logs
journalctl -u bead-starvation-direct-repair.service -p err
```

### Bead CLI Not Found
```bash
# Verify bead is in PATH
which bead

# Check PATH in service file
systemctl cat bead-starvation-direct-repair.service | grep PATH
```

### Database Locked
```bash
# Check if another process has the database open
lsof .beads/beads.db

# Wait for the lock to clear or restart the service
sudo systemctl restart bead-starvation-direct-repair.service
```

### Repairs Not Persisting
- Verify checkpoint auto-flush is disabled during repairs
- Check bead CLI version supports `--clear-assignee`
- Review repair logs for error messages

## Service Management

### Stop the Service
```bash
# Stop the timer (prevents future runs)
sudo systemctl stop bead-starvation-direct-repair.timer

# Disable from starting on boot
sudo systemctl disable bead-starvation-direct-repair.timer
```

### Uninstall
```bash
# Stop and disable
sudo systemctl stop bead-starvation-direct-repair.timer
sudo systemctl disable bead-starvation-direct-repair.timer

# Remove service files
sudo rm /etc/systemd/system/bead-starvation-direct-repair.{service,timer}

# Reload systemd
sudo systemctl daemon-reload
```

## Architecture Notes

### Why Direct Database Query?

The earlier approach relied on **starvation alert beads** being created first, then processed. This added complexity and delay:
- Alert generation required scheduled runs
- Alert beads could fail to be created
- Processing required parsing JSON from bead bodies

The **direct repair approach**:
- Eliminates the alert generation step
- Works directly on the database state
- Faster detection and repair (15-minute intervals)
- Simpler, more reliable architecture

### Why Disable Checkpoint Auto-Flush?

During repair runs, we're making multiple rapid `bead update` calls to clear assignees. Auto-flushing the checkpoint after each update:
- Slows down the repair process significantly
- Creates unnecessary git commits for intermediate states
- Increases database write load

With auto-flush disabled:
- Repairs complete faster
- Single checkpoint flush at the end captures all changes
- Reduced database and git load

### Safety Considerations

1. **Dry-run first** - Always test with `--dry-run` before enabling the service
2. **Audit logs** - Every repair is logged with timestamp and bead ID
3. **Non-destructive** - Only clears assignees, never deletes or modifies bead content
4. **Reversible** - Beads can be reassigned if needed (though unlikely)
5. **Idempotent** - Safe to run multiple times without side effects

## Related Files

- `scripts/bead-starvation-direct-repair.sh` - Main repair script
- `scripts/install-direct-repair-service.sh` - Installation helper
- `systemd/bead-starvation-direct-repair.service` - Systemd service unit
- `systemd/bead-starvation-direct-repair.timer` - Systemd timer unit
- `.beads/diagnostics/repair-log-*.json` - Audit logs

## Historical Context

- **2026-08-14**: Fleet-wide sweep found 583 stuck beads across 47 workspaces
- **2026-08-16**: 10 workspaces fully starved (zero ready beads despite live workers)
- **2026-08-24**: `bead reopen` behavior fixed to clear assignees
- **2026-08-26**: Direct automated repair system deployed

## Future Enhancements

Potential improvements for future versions:
1. **Configurable intervals** - Allow customization of repair frequency
2. **Workspace-wide scans** - Detect and repair across multiple workspaces
3. **Metrics export** - Export repair statistics to monitoring systems
4. **Smart thresholds** - Only trigger if stuck count exceeds threshold
5. **Dependency validation** - Check for circular dependencies automatically

---

**Maintained by:** NEEDLE Fleet Operations Team  
**Last updated:** 2026-08-26  
**Related beads:** irrevers-60754e7c, irrevers-df444996
