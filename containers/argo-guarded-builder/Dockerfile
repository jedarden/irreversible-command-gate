# syntax=docker/dockerfile:1

# Argo Workflow Guarded Builder Image
#
# This image extends the shared iad-ci Rust executor image with
# irreversible-command-gate (icg) PATH-wrapper protection. The binary is built
# from the repository in this image build rather than downloaded from a
# release: the release workflow uses this image, so downloading the binary it
# is supposed to protect would create a bootstrap cycle.

ARG BASE_IMAGE=ronaldraygun/needle-ci-builder:0.1.5-with-deps

FROM ${BASE_IMAGE} AS icg-builder

WORKDIR /tmp/icg-source
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked --bin icg

FROM ${BASE_IMAGE}

LABEL maintainer="jedarden"
LABEL description="Rust builder with irreversible-command-gate PATH-wrapper protection"

ARG ICG_VERSION=0.1.0
ENV ICG_VERSION="${ICG_VERSION}"
ENV ICG_INSTALL_DIR="/usr/local/bin"
ENV ICG_RULE_PACK="/etc/icg/packs"
ENV ICG_RULE_PACK_PATH="/etc/icg/packs"
ENV ICG_PACK_DIR="/etc/icg/packs"

# The compatibility matrix installs the pinned Codex CLI with npm. The base
# executor intentionally does not need Node, so add it only to this image.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends nodejs npm \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /etc/icg/packs /etc/icg/overrides /var/cache/icg
COPY --from=icg-builder /tmp/icg-source/target/release/icg /usr/local/bin/icg
COPY packs /etc/icg/packs/

# Put the wrapper directory first while retaining the base image's Rust and
# system-tool paths. The wrapper skips its own symlink when resolving the real
# binary, so the original tools remain available later in PATH.
ENV PATH="/usr/local/bin:${PATH}"

# These are the command front-ends currently used by the image and the
# commands covered by the installed rule packs. Missing tools are skipped so
# the image remains usable if the shared executor changes its tool set.
RUN chmod 0755 /usr/local/bin/icg \
    && for tool in git bead bf bao vault cargo rustc rustup npm node python3 pip docker kubectl helm; do \
         tool_path="$(command -v "$tool" || true)"; \
         if [ -n "$tool_path" ] && [ "$tool_path" != "/usr/local/bin/icg" ]; then \
           ln -sfn /usr/local/bin/icg "/usr/local/bin/$tool"; \
         fi; \
       done \
    && /usr/local/bin/icg --version \
    && test "$(/usr/local/bin/icg --version | awk '{print $2}')" = "${ICG_VERSION}" \
    && /usr/local/bin/icg coverage

HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/icg", "--version"]

WORKDIR /workspace
CMD ["icg", "status"]
