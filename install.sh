#!/usr/bin/env bash
#
# irreversible-command-gate installer
# https://github.com/jedarden/irreversible-command-gate
#
#   curl -fsSL https://raw.githubusercontent.com/jedarden/irreversible-command-gate/main/install.sh | sudo bash
#   sudo ./install.sh --from-checkout          # from a clone
#
# Installs the guard binary and its rule packs root-owned, then PROVES the
# result actually denies before reporting success.
#
# That last step is the reason this script exists. `icg hook` fails open by
# design: with no readable pack directory it answers
# {"permissionDecision":"allow"} and exits 0, silently. A partial install
# therefore looks exactly like a working one -- you wire the hook, see no
# errors, and are guarded by nothing. Copying a binary into place is the easy
# half; this script refuses to finish unless a known-destructive command is
# actually blocked end to end.

set -euo pipefail

REPO="jedarden/irreversible-command-gate"
VERSION="latest"
PREFIX="/usr/local/bin"
CONFIG_DIR="/etc/icg"
CACHE_DIR="/var/cache/icg"
WRAPPER_DIR=""
PACK_SOURCE=""
FROM_CHECKOUT=0
WRITE_HOOK=0
SETTINGS=""
UNINSTALL=0
DRY_RUN=0
KEEP_TMP=0

if [ -t 1 ]; then
  R=$'\033[31m'; G=$'\033[32m'; Y=$'\033[33m'; B=$'\033[34m'; D=$'\033[2m'; N=$'\033[0m'
else
  R=''; G=''; Y=''; B=''; D=''; N=''
fi
info()  { printf '%s==>%s %s\n' "$B" "$N" "$*"; }
ok()    { printf '%s  ok%s %s\n' "$G" "$N" "$*"; }
warn()  { printf '%s  !!%s %s\n' "$Y" "$N" "$*" >&2; }
die()   { printf '%s error:%s %s\n' "$R" "$N" "$*" >&2; exit 1; }
run()   { if [ "$DRY_RUN" = 1 ]; then printf '%s   would run:%s %s\n' "$D" "$N" "$*"; else "$@"; fi; }

usage() {
  sed -n '3,20p' "$0" | sed 's/^# \{0,1\}//'
  cat <<'EOF'

Options:
  --version <tag>       Release to install (default: latest)
  --from-checkout       Install from this checkout: build with cargo and use packs/
  --prefix <dir>        Directory for the binary        (default: /usr/local/bin)
  --config-dir <dir>    Directory for the packs         (default: /etc/icg)
  --cache-dir <dir>     Directory for telemetry         (default: /var/cache/icg)
  --pack-source <dir>   Install packs from here instead of the release tarball
                        or the checkout. For an offline or hand-carried pack
                        set; the manifest check is skipped, the self-test is
                        not.
  --wrapper-dir <dir>   Also install PATH-wrapper symlinks here. Scope this to
                        the agent's PATH; see the deployment guide's "Scoping
                        the wrapper to the agent". Omit to skip the wrapper.
  --hook                Register the PreToolUse hook in a Claude Code settings file
  --settings <path>     Which settings file  (default: the invoking user's
                        ~/.claude/settings.json)
  --uninstall           Remove the binary, packs, wrappers and hook entry
  --dry-run             Print what would happen, change nothing
  --help                This text
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version)      VERSION="${2:?--version needs a tag}"; shift 2 ;;
    --from-checkout) FROM_CHECKOUT=1; shift ;;
    --prefix)       PREFIX="${2:?--prefix needs a directory}"; shift 2 ;;
    --config-dir)   CONFIG_DIR="${2:?--config-dir needs a directory}"; shift 2 ;;
    --cache-dir)    CACHE_DIR="${2:?--cache-dir needs a directory}"; shift 2 ;;
    --pack-source)  PACK_SOURCE="${2:?--pack-source needs a directory}"; shift 2 ;;
    --wrapper-dir)  WRAPPER_DIR="${2:?--wrapper-dir needs a directory}"; shift 2 ;;
    --hook)         WRITE_HOOK=1; shift ;;
    --settings)     SETTINGS="${2:?--settings needs a path}"; WRITE_HOOK=1; shift 2 ;;
    --uninstall)    UNINSTALL=1; shift ;;
    --dry-run)      DRY_RUN=1; shift ;;
    --keep-tmp)     KEEP_TMP=1; shift ;;
    --help|-h)      usage; exit 0 ;;
    *)              die "unknown option: $1 (try --help)" ;;
  esac
done

BIN="$PREFIX/icg"
PACK_DIR="$CONFIG_DIR/packs"

# --------------------------------------------------------------------------
# preflight
# --------------------------------------------------------------------------
[ "$(uname -s)" = "Linux" ] || die "Linux is the supported target; found $(uname -s)"
if [ "$DRY_RUN" = 0 ] && [ "$(id -u)" -ne 0 ]; then
  die "root is required to write $PREFIX and $CONFIG_DIR. Re-run with sudo, or pass --dry-run."
fi
for tool in install tar; do
  command -v "$tool" >/dev/null || die "missing required tool: $tool"
done

# --------------------------------------------------------------------------
# uninstall
# --------------------------------------------------------------------------
if [ "$UNINSTALL" = 1 ]; then
  info "Removing icg"
  if [ -n "$WRAPPER_DIR" ] && [ -x "$BIN" ]; then
    # --force is required: without it `icg install --uninstall` prompts on
    # stdin, reads EOF in a script, prints "Uninstall cancelled" and exits 0.
    # The symlinks survive and nothing reports a failure.
    run "$BIN" install --dir "$WRAPPER_DIR" --uninstall --force >/dev/null \
      || warn "wrapper removal reported an error"
    if [ "$DRY_RUN" = 0 ]; then
      LEFT="$(find "$WRAPPER_DIR" -maxdepth 1 -type l -lname "$BIN" 2>/dev/null | wc -l)"
      [ "$LEFT" -eq 0 ] \
        || warn "$LEFT wrapper symlink(s) still point at $BIN in $WRAPPER_DIR -- remove them by hand"
    fi
  fi
  run rm -f "$BIN"
  run rm -rf "$PACK_DIR" "$CONFIG_DIR/packs.previous"
  warn "Left in place, in case they are still wanted:"
  warn "  $CONFIG_DIR/trust-pointer.json, $CONFIG_DIR/*.json, $CACHE_DIR (telemetry and denial history)"
  warn "  the PreToolUse hook entry in your settings file -- remove it by hand"
  ok "removed"
  exit 0
fi

TMP=""
cleanup() { [ -n "$TMP" ] && [ "$KEEP_TMP" = 0 ] && rm -rf "$TMP"; }
trap cleanup EXIT

# --------------------------------------------------------------------------
# obtain the artifacts
# --------------------------------------------------------------------------
if [ "$FROM_CHECKOUT" = 1 ]; then
  HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  [ -d "$HERE/packs" ] || die "--from-checkout needs a packs/ directory beside this script"
  command -v cargo >/dev/null || die "--from-checkout needs a Rust toolchain (cargo)"
  info "Building from $HERE"
  run env -C "$HERE" cargo build --release --bin icg
  # Do not assume ./target: CARGO_TARGET_DIR and .cargo/config.toml both move
  # it, and guessing produced a "cannot stat target/release/icg" failure
  # immediately after a successful build.
  SRC_BIN="${CARGO_TARGET_DIR:-$HERE/target}/release/icg"
  if [ "$DRY_RUN" = 0 ] && [ ! -x "$SRC_BIN" ]; then
    SRC_BIN="$(env -C "$HERE" cargo metadata --format-version 1 --no-deps 2>/dev/null \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null)/release/icg"
  fi
  [ "$DRY_RUN" = 1 ] || [ -x "$SRC_BIN" ] \
    || die "built binary not found. Looked for $SRC_BIN -- set CARGO_TARGET_DIR if your build tree is elsewhere."
  SRC_PACKS="$HERE/packs"
  SRC_MANIFEST=""
  RELEASE_DESC="local checkout"
else
  command -v curl >/dev/null || die "missing required tool: curl"
  TMP="$(mktemp -d)"
  if [ "$VERSION" = "latest" ]; then
    BASE="https://github.com/$REPO/releases/latest/download"
  else
    BASE="https://github.com/$REPO/releases/download/$VERSION"
  fi
  info "Downloading $VERSION from $REPO"
  for asset in icg icg-packs.tar.gz pack-manifest.json; do
    run curl -fsSL --retry 3 -o "$TMP/$asset" "$BASE/$asset" \
      || die "could not download $asset from $BASE -- check the release tag"
  done
  if [ "$DRY_RUN" = 0 ]; then
    tar -xzf "$TMP/icg-packs.tar.gz" -C "$TMP"
    [ -d "$TMP/packs" ] || die "icg-packs.tar.gz did not contain a packs/ directory"
  fi
  SRC_BIN="$TMP/icg"
  SRC_PACKS="$TMP/packs"
  SRC_MANIFEST="$TMP/pack-manifest.json"
  RELEASE_DESC="$VERSION"
fi

if [ -n "$PACK_SOURCE" ]; then
  [ -d "$PACK_SOURCE" ] || die "--pack-source directory does not exist: $PACK_SOURCE"
  SRC_PACKS="$PACK_SOURCE"
  SRC_MANIFEST=""
  info "Using packs from $PACK_SOURCE (release manifest check skipped)"
fi

# --------------------------------------------------------------------------
# install, root-owned
# --------------------------------------------------------------------------
info "Installing the binary to $BIN"
run install -d -o root -g root -m 0755 "$PREFIX"
run install -o root -g root -m 0755 "$SRC_BIN" "$BIN"

info "Installing rule packs to $PACK_DIR"
run install -d -o root -g root -m 0755 "$CONFIG_DIR" "$PACK_DIR"
if [ "$DRY_RUN" = 0 ]; then
  # Replace the directory's contents wholesale. A partial pack set is a
  # coverage hole that nothing downstream reports.
  pack_files=("$SRC_PACKS"/*.json)
  [ -e "${pack_files[0]}" ] \
    || die "no rule packs found in $SRC_PACKS. Installing the binary without packs \
produces a guard that silently allows everything -- refusing."
  find "$PACK_DIR" -maxdepth 1 -name '*.json' -delete
  install -o root -g root -m 0644 "${pack_files[@]}" "$PACK_DIR/"
else
  printf '%s   would install:%s %s/*.json -> %s\n' "$D" "$N" "$SRC_PACKS" "$PACK_DIR"
fi

# The hook initializes its telemetry store before evaluating, so the agent
# identity must be able to write here. Root-only would make every check emit
# warnings; world-writable would be worse. 0750 plus a group is the middle.
info "Creating $CACHE_DIR for telemetry and denial history"
run install -d -o root -g root -m 0750 "$CACHE_DIR"

if [ -n "$WRAPPER_DIR" ]; then
  info "Installing PATH-wrapper symlinks to $WRAPPER_DIR"
  run install -d -o root -g root -m 0755 "$WRAPPER_DIR"
  run "$BIN" install --dir "$WRAPPER_DIR" --pack "$PACK_DIR" --force
fi

if [ "$DRY_RUN" = 1 ]; then
  info "Dry run complete. Nothing was changed."
  exit 0
fi

# --------------------------------------------------------------------------
# verify -- the part that makes this script worth running
# --------------------------------------------------------------------------
info "Verifying the installation"

"$BIN" --version >/dev/null 2>&1 || die "$BIN did not run"
ok "binary runs: $("$BIN" --version)"

PACK_COUNT="$("$BIN" coverage --list --pack "$PACK_DIR" 2>/dev/null | grep -c '^✓' || true)"
[ "${PACK_COUNT:-0}" -gt 0 ] \
  || die "no rule packs loaded from $PACK_DIR. The hook would fail open and allow everything."
ok "$PACK_COUNT rule packs load"

if [ -n "$SRC_MANIFEST" ] && [ -f "$SRC_MANIFEST" ]; then
  run install -o root -g root -m 0644 "$SRC_MANIFEST" "$CONFIG_DIR/pack-manifest.json"
  "$BIN" pack-manifest --pack-dir "$PACK_DIR" --verify "$CONFIG_DIR/pack-manifest.json" >/dev/null \
    || die "installed packs do not match the release manifest -- refusing to report success"
  ok "packs are byte-identical to the release manifest"
else
  warn "no pack manifest to verify against (installing from a checkout)"
fi

# Live self-test through the hook, which is the path the harness uses. A
# static inventory cannot tell you the guard will actually block; this can.
probe_hook() { # probe_hook <command-json> -> decision
  printf '{"tool_name":"Bash","tool_input":{"command":%s}}' "$1" \
    | ICG_RULE_PACK="$PACK_DIR" "$BIN" hook 2>/dev/null
}

DENY_PROBE="$(probe_hook '"bao kv destroy secret/icg-install-selftest"')"
case "$DENY_PROBE" in
  *'"permissionDecision":"deny"'*) ok "a destructive command is denied through the hook" ;;
  *) die "SELF-TEST FAILED: the hook did not deny a known-destructive command.
       response: ${DENY_PROBE:-<empty>}
       The guard is installed but not enforcing. Do not rely on it." ;;
esac

ALLOW_PROBE="$(probe_hook '"git status"')"
case "$ALLOW_PROBE" in
  *'"permissionDecision":"allow"'*) ok "an ordinary command is allowed through the hook" ;;
  *) die "SELF-TEST FAILED: the hook did not allow a harmless command.
       response: ${ALLOW_PROBE:-<empty>}
       A guard that blocks ordinary work will be turned off." ;;
esac

# --------------------------------------------------------------------------
# hook registration
# --------------------------------------------------------------------------
HOOK_JSON='{"matcher":"Bash|Write|Edit","hooks":[{"type":"command","command":"'"$BIN"' hook","timeout":10}]}'

if [ "$WRITE_HOOK" = 1 ]; then
  if [ -z "$SETTINGS" ]; then
    TARGET_USER="${SUDO_USER:-$(id -un)}"
    TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
    SETTINGS="$TARGET_HOME/.claude/settings.json"
  fi
  command -v python3 >/dev/null || die "--hook needs python3 to edit $SETTINGS safely"
  info "Registering the PreToolUse hook in $SETTINGS"
  python3 - "$SETTINGS" "$BIN" <<'PY'
import json, os, sys, pathlib
settings_path, binary = pathlib.Path(sys.argv[1]), sys.argv[2]
settings_path.parent.mkdir(parents=True, exist_ok=True)
data = {}
if settings_path.exists() and settings_path.stat().st_size:
    try:
        data = json.loads(settings_path.read_text())
    except json.JSONDecodeError:
        sys.exit(f"{settings_path} is not valid JSON; refusing to overwrite it")
entry = {"type": "command", "command": f"{binary} hook", "timeout": 10}
hooks = data.setdefault("hooks", {})
pre = hooks.setdefault("PreToolUse", [])
for block in pre:
    if block.get("matcher") == "Bash|Write|Edit":
        inner = block.setdefault("hooks", [])
        if any(h.get("command") == entry["command"] for h in inner):
            print("  already registered; left unchanged")
            break
        inner.append(entry)
        break
else:
    pre.append({"matcher": "Bash|Write|Edit", "hooks": [entry]})
backup = settings_path.with_suffix(settings_path.suffix + ".icg-backup")
if settings_path.exists():
    backup.write_text(settings_path.read_text())
settings_path.write_text(json.dumps(data, indent=2) + "\n")
print(f"  wrote {settings_path}" + (f" (previous copy at {backup})" if backup.exists() else ""))
PY
  if [ -n "${SUDO_USER:-}" ]; then
    chown "$SUDO_USER" "$SETTINGS" "$SETTINGS.icg-backup" 2>/dev/null || true
  fi
  ok "hook registered -- restart the harness for it to take effect"
else
  info "Hook not registered (pass --hook to do it, or add this yourself)"
  cat <<EOF

  Merge into ~/.claude/settings.json, under "hooks":

    "PreToolUse": [ $HOOK_JSON ]

  For a local Codex CLI, the same shape in ~/.codex/hooks.json with
  matcher "Bash|apply_patch".
EOF
fi

# --------------------------------------------------------------------------
# done
# --------------------------------------------------------------------------
printf '\n'
ok "icg $RELEASE_DESC installed and verified"
cat <<EOF

  binary   $BIN
  packs    $PACK_DIR  ($PACK_COUNT loaded)
  cache    $CACHE_DIR
$( [ -n "$WRAPPER_DIR" ] && echo "  wrappers $WRAPPER_DIR" )
  Confirm the harness sees it:   icg health --check-hooks
  See what is enforced:          icg coverage --list
  Try a denial yourself:         icg check --command "git push --force origin main"

  The guard fails open on purpose: if the pack directory ever becomes
  unreadable it allows everything rather than wedging your agent. Re-run
  'icg coverage --list' after any change to $CONFIG_DIR.
EOF
