#!/usr/bin/env bash
#
# systemd/install.sh — install every unit tracked in this directory onto the
# host as a SYMLINK, never a copy.
#
# The symlink is the whole point. Commit 092e82c deleted a unit's script from
# the repo while a copied unit file lived on in ~/.config/systemd/user/, and
# the timer kept firing into 203/EXEC for a week (bead irrevers-46f2b741).
# A symlink breaks loudly the moment its target is deleted; a copy breaks
# silently. This is the same pattern aide-de-camp, SEAM and tradegraph
# already use on this host.
#
# Usage: systemd/install.sh [--now] [--dry-run]
#   --now       start enabled services/timers immediately after enabling
#   --dry-run   print what would happen, change nothing
#
# Environment: ICG_HOST_UNIT_DIR overrides the destination directory
# (default ~/.config/systemd/user). The tracked-unit directory is always this
# script's own directory — that pairing is the invariant.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOST_DIR="${ICG_HOST_UNIT_DIR:-$HOME/.config/systemd/user}"

NOW=0
DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --now)     NOW=1 ;;
    --dry-run) DRY_RUN=1 ;;
    *) printf 'usage: %s [--now] [--dry-run]\n' "$0" >&2; exit 2 ;;
  esac
done

run() { if [ "$DRY_RUN" = 1 ]; then printf '  would run: %s\n' "$*"; else "$@"; fi; }

shopt -s nullglob
units=()
for unit in "$SCRIPT_DIR"/*.service "$SCRIPT_DIR"/*.timer; do
  units+=("$unit")
done
if [ "${#units[@]}" -eq 0 ]; then
  echo "install: no units tracked in $SCRIPT_DIR — nothing to install."
  echo "  (This scaffolding is preventive; see systemd/README.md.)"
  exit 0
fi

[ "$DRY_RUN" = 1 ] || mkdir -p "$HOST_DIR"

installed=0
for unit in "${units[@]}"; do
  name="${unit##*/}"
  dest="$HOST_DIR/$name"

  # Refuse to clobber anything that is not already our symlink. A plain file
  # at $dest is exactly the pre-scaffolding residue this repo used to leave
  # behind; overwriting it silently would hide it rather than surface it.
  if [ -e "$dest" ] || [ -L "$dest" ]; then
    if [ -L "$dest" ]; then
      current="$(readlink "$dest")"
      case "$current" in
        "$SCRIPT_DIR"/*) echo "  ok (already linked): $name"; installed=$((installed + 1)); continue ;;
        *) printf 'install: refusing to replace %s — it symlinks to %s, not into %s\n  Resolve by hand, then re-run.\n' "$dest" "$current" "$SCRIPT_DIR" >&2; exit 1 ;;
      esac
    else
      printf 'install: refusing to overwrite %s — it is a regular file, not our symlink.\n  If it is leftover from before this scaffolding, remove it by hand, then re-run.\n' "$dest" >&2
      exit 1
    fi
  fi

  run ln -s "$unit" "$dest"
  [ "$DRY_RUN" = 1 ] || echo "  linked: $name"
  installed=$((installed + 1))
done

run systemctl --user daemon-reload

# Only units that ask for it ([Install]) can be enabled; enable is a hard
# error otherwise, so gate it on the section's presence.
for unit in "${units[@]}"; do
  name="${unit##*/}"
  if grep -q '^\[Install\]' "$unit"; then
    if [ "$NOW" = 1 ]; then
      run systemctl --user enable --now "$name"
    else
      run systemctl --user enable "$name"
    fi
  fi
done

if [ "$DRY_RUN" = 1 ]; then
  echo "install: dry run complete — nothing changed ($installed unit(s) would be linked)."
  exit 0
fi

# Self-verify: an install that leaves drift behind is not an install.
"$SCRIPT_DIR/check-consistency.sh"
echo "install: $installed unit(s) linked into $HOST_DIR and consistent."
