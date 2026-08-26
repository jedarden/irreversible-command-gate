# Assignment Repair Monitor Container
#
# Automated service for detecting and repairing stuck bead assignment states.
# Runs as a continuous deployment with internal scheduling loop.
#
# ## Functionality
#
# - Queries for open beads with no assignee but not appearing in ready queries
# - Checks for stuck assigned-but-open beads (known failure mode from 2026-08-16)
# - Automatically applies 'bead release' or 'bead update --clear-assignee' for stuck beads
# - Publishes diagnostic events to .beads/diagnostics/
# - Runs on configurable periodic schedule (default: 5 minutes)
#
# ## Usage
#
# This container is deployed as a Kubernetes Deployment with persistent volume
# mount to the workspace .beads directory.

FROM debian:bookworm-slim AS base

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    sqlite3 \
    curl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r beadmonitor && useradd -r -g beadmonitor beadmonitor

# Build stage
FROM rust:1.81-slim-bookworm AS builder

WORKDIR /build

# Copy Cargo files
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/

# Build the assignment-repair-monitor binary
RUN cargo build --release --bin assignment-repair-monitor

# Final stage
FROM base

# Copy the binary from builder
COPY --from=builder /build/target/release/assignment-repair-monitor /usr/local/bin/

# Create workspace directory
RUN mkdir -p /workspace/.beads/diagnostics

# Set ownership
RUN chown -R beadmonitor:beadmonitor /workspace

# Switch to non-root user
USER beadmonitor

WORKDIR /workspace

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:9096/health/live || exit 1

# Default configuration
ENV ICG_WORKSPACE_PATH=/workspace
ENV ICG_CHECK_INTERVAL_SECONDS=300
ENV ICG_AUTO_REPAIR_ENABLED=true
ENV ICG_MONITOR_HOST=0.0.0.0
ENV ICG_MONITOR_PORT=9096

EXPOSE 9096

CMD ["assignment-repair-monitor"]
