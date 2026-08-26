# Bead Starvation Detection and Auto-Repair Monitor
#
# Automated service for detecting bead starvation and repairing stuck states.
# Runs as a continuous deployment with internal scheduling loop (no Job/CronJob).
#
# ## Functionality
#
# This service integrates three core capabilities:
# 1. Starvation Detection: Detects when open beads exist but bead list --ready returns empty
# 2. Assignment Repair: Clears stuck assigned-but-open beads (2026-08-16 failure mode: 583 beads)
# 3. Integrity Verification: Compares database, checkpoint, and git views for consistency
#
# ## Automated Repairs
#
# - Queries for open beads with no assignee but not appearing in ready queries
# - Checks for stuck assigned-but-open beads
# - Automatically applies 'bead release' or 'bead update --clear-assignee' for stuck beads
# - Verifies worker liveness before clearing assignments (prevents disrupting active workers)
# - Publishes diagnostic events to .beads/diagnostics/
#
# ## Architecture
#
# - Uses existing bead-integrity-check-bin for integrity verification
# - Uses assignment-repair-monitor for stuck assignment detection and repair
# - Internal scheduling loop (default: 5 minutes)
# - HTTP health endpoints for Kubernetes probes
# - Prometheus metrics export for monitoring

FROM debian:bookworm-slim AS base

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    sqlite3 \
    curl \
    ca-certificates \
    git \
    procps \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r beadmonitor && useradd -r -g beadmonitor beadmonitor

# Build stage
FROM rust:1.81-slim-bookworm AS builder

WORKDIR /build

# Copy Cargo files for main project
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/

# Build assignment-repair-monitor binary
RUN cargo build --release --bin assignment-repair-monitor

# Build bead-integrity-check binary from scripts directory
WORKDIR /build/scripts
COPY scripts/bead-integrity-check/Cargo.toml scripts/bead-integrity-check/Cargo.lock ./scripts/bead-integrity-check/
COPY scripts/bead-integrity-check/src ./scripts/bead-integrity-check/src
RUN cargo build --release --bin bead-integrity-check --manifest-path scripts/bead-integrity-check/Cargo.toml

# Final stage
FROM base

# Copy binaries from builder
COPY --from=builder /build/target/release/assignment-repair-monitor /usr/local/bin/
COPY --from=builder /build/target/release/bead-integrity-check /usr/local/bin/

# Copy unified repair script
COPY scripts/bead-starvation-unified-repair.sh /usr/local/bin/starvation-repair.sh
RUN chmod +x /usr/local/bin/starvation-repair.sh

# Create workspace directory
RUN mkdir -p /workspace/.beads/diagnostics

# Set ownership
RUN chown -R beadmonitor:beadmonitor /workspace

# Switch to non-root user
USER beadmonitor

WORKDIR /workspace

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:9097/health/live || exit 1

# Default configuration
ENV ICG_WORKSPACE_PATH=/workspace
ENV ICG_CHECK_INTERVAL_SECONDS=300
ENV ICG_AUTO_REPAIR_ENABLED=true
ENV ICG_MONITOR_HOST=0.0.0.0
ENV ICG_MONITOR_PORT=9097
ENV DB_PATH=/workspace/.beads/beads.db
ENV ASSIGNMENT_TIMEOUT_HOURS=4

EXPOSE 9097

# Run the assignment repair monitor with integrated starvation detection
CMD ["assignment-repair-monitor"]
