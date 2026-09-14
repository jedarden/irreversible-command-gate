#!/usr/bin/env bash
#
# systemd/uninstall.sh — stop, disable and remove every unit this repo
# installed on the host.
#
# Run this on a host after a commit deletes tracked units, or when retiring
# the repo from a machine. It is deliberately conservative: a host file that
# is not one of our symlinks is reported and left alone, never deleted.
#
# Usage: systemd/uninstall.sh [--dry-run]
# Environment: ICG_HOST_UNIT_DIR overrides the installed-unit directory
# (default ~/.config/systemd/user).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOST_DIR="${ICG_HOST_UNIT_DIR:-$HOME/.config/systemd/user}"

DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    *) printf 'usage: %s [--dry-run]\n' "$0" >&2; exit 2 ;;
  esac
done

run() { if [ "$DRY_RUN" = 1 ]; then printf '  would run: %s\n' "$*"; else "$@"; fi; }

shopt -s nullglob
units=()
for unit in "$SCRIPT_DIR"/*.service "$SCRIPT_DIR"/*.timer; do
  units+=("$unit")
done
if [ "${#units[@]}" -eq 0 ]; then
  echo "uninstall: no units tracked in $SCRIPT_DIR — nothing to uninstall."
  echo "  (Residue from units deleted before this scaffolding is found by"
  echo "   check-consistency.sh, which prints the exact removal commands.)"
  exit 0
fi

for unit in "${units[@]}"; do
  name="${unit##*/}"
  dest="$HOST_DIR/$name"

  # disable --now is idempotent: a unit already gone from systemd reports an
  # error we do not care about. It also removes the wants symlink.
  run systemctl --user disable --now "$name" >/dev/null 2>&1 || true

  if [ -L "$dest" ]; then
    target="$(readlink "$dest")"
    case "$target" in
      "$SCRIPT_DIR"/*) run rm "$dest" ;;
      *)
        printf 'uninstall: skipping %s — it symlinks to %s, not into %s\n' "$dest" "$target" "$SCRIPT_DIR" >&2
        ;;
    esac
  elif [ -e "$dest" ]; then
    printf 'uninstall: skipping %s — regular file, not our symlink; remove it by hand if intended\n' "$dest" >&2
  fi

  run systemctl --user reset-failed "$name" >/dev/null 2>&1 || true
done

run systemctl --user daemon-reload
if [ "$DRY_RUN" = 1 ]; then
  echo "uninstall: dry run complete — nothing changed."
else
  echo "uninstall: done. Removed only symlinks that pointed into $SCRIPT_DIR."
fi
