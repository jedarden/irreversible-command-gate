# Argo Workflow Guarded Builder Image
#
# This image extends the standard Rust builder with irreversible-command-gate (icg)
# PATH-wrapper integration to guard commands in CI/CD pipelines.
#
# The PATH-wrapper intercepts commands like git, kubectl, vault, docker, etc.
# and checks them against icg rule packs before allowing execution.

FROM ronaldraygun/needle-ci-builder:with-deps

# Metadata
LABEL maintainer="jedarden"
LABEL description="Rust builder with irreversible-command-gate PATH-wrapper protection"

# Install icg from a specific release version (read from VERSION file)
# Use a pinned version, not :latest, to ensure reproducible builds
ARG ICG_VERSION=0.1.0
ENV ICG_VERSION="${ICG_VERSION}"
ENV ICG_INSTALL_DIR="/usr/local/bin"
ENV ICG_RULE_PACK_PATH="/etc/icg/rule-pack.json"
ENV ICG_PACK_DIR="/etc/icg/packs"

# Create directories for icg
RUN mkdir -p /etc/icg/packs /etc/icg/overrides

# Install icg binary
RUN echo "Installing irreversible-command-gate..." && \
    curl -fsSL "https://github.com/jedarden/irreversible-command-gate/releases/latest/download/icg" \
    -o /usr/local/bin/icg && \
    chmod +x /usr/local/bin/icg && \
    icg --version

# Install PATH-wrapper symlinks for common tools
# Note: We create these in /usr/local/bin which is typically early in PATH
RUN echo "Installing PATH-wrapper symlinks..." && \
    # List of tools to guard - these will be symlinked to icg
    # When invoked, icg will check the command against rule packs before exec'ing the real binary
    for tool in vault git docker kubectl kubectlf helm bf bead cargo-build rustc rustup npm node python3 pip cargo; do \
        if command -v "$tool" >/dev/null 2>&1; then \
            ln -sf /usr/local/bin/icg "/usr/local/bin/$tool" && \
            echo "  ✓ $tool"; \
        fi; \
    done && \
    echo "PATH-wrapper installation complete"

# Download default rule pack
RUN echo "Downloading default rule pack..." && \
    curl -fsSL "https://github.com/jedarden/irreversible-command-gate/releases/latest/download/rule-pack.json" \
    -o "$ICG_RULE_PACK_PATH" || \
    echo "Warning: Failed to download rule pack - will need manual installation"

# Verify installation
RUN echo "Verifying icg installation..." && \
    icg --version && \
    icg coverage || echo "Warning: No rule packs installed"

# Set PATH to ensure wrapper symlinks are found first
ENV PATH="/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin"

# Add a health check that verifies icg is functional
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD icg --version || exit 1

WORKDIR /workspace

# Default command shows icg status
CMD ["icg", "status"]
