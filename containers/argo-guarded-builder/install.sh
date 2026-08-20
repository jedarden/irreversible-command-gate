#!/bin/bash
set -euo pipefail

# Runtime PATH-wrapper installation script for Argo Workflow containers
#
# This script installs icg and the PATH-wrapper in an already-running container,
# useful for containers that can't be rebuilt or for testing without rebuilding images.

if [ -z "${ICG_VERSION:-}" ] || [ "${ICG_VERSION}" = "latest" ]; then
    echo "ICG_VERSION must be set to a pinned release version (for example 0.1.0)" >&2
    exit 2
fi
ICG_INSTALL_DIR="${ICG_INSTALL_DIR:-/usr/local/bin}"
ICG_PACK_DIR="${ICG_PACK_DIR:-/etc/icg/packs}"
ICG_RULE_PACK_PATH="${ICG_RULE_PACK_PATH:-${ICG_PACK_DIR}/runtime.json}"
ICG_RELEASE_VERSION="${ICG_VERSION#v}"

echo "🔒 Installing irreversible-command-gate PATH-wrapper..."
echo "   Version: ${ICG_VERSION}"
echo "   Install dir: ${ICG_INSTALL_DIR}"
echo

# Create directories
sudo mkdir -p "${ICG_INSTALL_DIR}" "${ICG_PACK_DIR}" /etc/icg/overrides /var/cache/icg

# Download icg binary
echo "📦 Downloading icg binary..."
ICG_DOWNLOAD_URL="https://github.com/jedarden/irreversible-command-gate/releases/download/v${ICG_RELEASE_VERSION}/icg"

curl -fsSL "${ICG_DOWNLOAD_URL}" -o /tmp/icg
sudo mv /tmp/icg "${ICG_INSTALL_DIR}/icg"
sudo chmod +x "${ICG_INSTALL_DIR}/icg"
echo "✓ icg installed to ${ICG_INSTALL_DIR}/icg"

# Verify installation
echo "🔍 Verifying installation..."
"${ICG_INSTALL_DIR}/icg" --version
echo

# Discover real binaries to shadow
echo "🔗 Setting up PATH-wrapper symlinks..."
declare -A TOOLS_TO_CHECK=(
    [bao]=bao
    [vault]=vault
    [git]=git
    [bead]=bead
    [bf]=bf
    [docker]=docker
    [kubectl]=kubectl
    [helm]=helm
    [cargo]=cargo
    [rustc]=rustc
    [npm]=npm
    [node]=node
    [python3]=python3
    [pip]=pip
)

INSTALLED_COUNT=0
for tool_key in "${!TOOLS_TO_CHECK[@]}"; do
    tool_name="${TOOLS_TO_CHECK[$tool_key]}"
    if command -v "${tool_name}" >/dev/null 2>&1; then
        # Skip if the tool is already a symlink to icg (prevent infinite loops)
        REAL_PATH=$(readlink -f "$(command -v "${tool_name}")")
        if [[ "${REAL_PATH}" == *"icg" ]]; then
            echo "  ⏭️  Skipping ${tool_name} (already icg wrapper)"
            continue
        fi

        # Create symlink
        sudo ln -sfn "${ICG_INSTALL_DIR}/icg" "${ICG_INSTALL_DIR}/${tool_name}"
        echo "  ✓ ${tool_name}"
        INSTALLED_COUNT=$((INSTALLED_COUNT + 1))
    else
        echo "  ⏭️  Skipping ${tool_name} (not found in PATH)"
    fi
done

echo
echo "✅ Installation complete: ${INSTALLED_COUNT} tools guarded"

# Download rule pack if specified
if [ -n "${ICG_RULE_PACK_URL:-}" ]; then
    echo "📦 Downloading rule pack from ${ICG_RULE_PACK_URL}..."
    sudo mkdir -p "${ICG_PACK_DIR}"
    curl -fsSL "${ICG_RULE_PACK_URL}" -o /tmp/icg-rule-pack.json
    sudo mv /tmp/icg-rule-pack.json "${ICG_RULE_PACK_PATH}"
    echo "✓ Rule pack installed to ${ICG_RULE_PACK_PATH}"
else
    echo "⚠️  No rule pack URL specified (ICG_RULE_PACK_URL)"
    echo "   Rule pack will need to be installed manually"
fi

echo
echo "🎯 PATH-wrapper is now active!"
echo
echo "Test it with:"
echo "  ${ICG_INSTALL_DIR}/icg status"
echo "  ${ICG_INSTALL_DIR}/icg coverage"
echo
