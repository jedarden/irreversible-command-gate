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

# Explicit modes: the artifact directory's entire purpose is being
# un-writable by the guarded process, so its permissions must not depend on
# the ambient umask of the build environment -- and here that umask is 0000,
# because this image is built by kaniko. Confirmed against 0.1.0: every
# directory a RUN created without an explicit mode shipped at 0777
# (/etc/icg, /etc/icg/overrides, /var/cache/icg, /root/.cargo), which made
# verify_artifact_directory_security (src/trust_pointer.rs) report a security
# violation on every hook invocation in every guarded CI pod
# (irrevers-beee1069). /etc/icg/packs only escaped because the COPY below
# imposed the source directory's mode on it.
#
# The chmod deliberately lands BEFORE `COPY packs /etc/icg/packs/`, and the
# chmod is what wins: a COPY into a destination directory that already exists
# only adds entries inside it and leaves the directory's own metadata alone.
# Applying the source's mode is what the COPY does when it has to create the
# destination itself -- which is precisely the 0.1.0 escape described above.
# The assertion RUN near the bottom of this file fails the build if a change
# in builder behavior ever relaxes this anyway.
RUN mkdir -p /etc/icg/packs /etc/icg/overrides /var/cache/icg \
    && chown -R root:root /etc/icg /var/cache/icg \
    && chmod 0755 /etc/icg /etc/icg/packs /etc/icg/overrides /var/cache/icg
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
    && icg_semver="$(/usr/local/bin/icg --version | awk '{print $2}')" \
    && case "$icg_semver" in \
         [0-9]*.[0-9]*.[0-9]*) : ;; \
         *) echo "icg --version is not semver: '$icg_semver'" >&2; exit 1 ;; \
       esac \
    && /usr/local/bin/icg coverage

# Assert the trust directory the guard depends on cannot regress to being
# writable by anything other than root. A build-time failure here beats a
# runtime security violation the guard only warns about and continues past.
RUN for dir in /etc/icg /etc/icg/packs /etc/icg/overrides /var/cache/icg; do \
      owner="$(stat -c '%U' "$dir")"; \
      mode="$(stat -c '%a' "$dir")"; \
      if [ "$owner" != "root" ]; then \
        echo "SECURITY: $dir is owned by $owner, not root" >&2; exit 1; \
      fi; \
      case "$mode" in \
        *2|*3|*6|*7) echo "SECURITY: $dir is world-writable (mode $mode)" >&2; exit 1 ;; \
      esac; \
    done

HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/icg", "--version"]

WORKDIR /workspace
CMD ["icg", "status"]
