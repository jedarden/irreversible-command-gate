#!/usr/bin/env bash
#
# systemd/uninstall.sh — stop, disable and remove every unit this repo
# installed on the host.
#
# Run this on a host after a commit deletes tracked units, or when retiring
# the repo from a machine. It is deliberately conservative: a host file that
# is not one of our symlinks is reported and left alone, never deleted.
#
# "Our symlinks" includes the dangling kind. install.sh is the only thing
# that creates a symlink pointing into this directory, so a host symlink
# whose tracked unit a commit has since deleted is still ours by
# construction — removing it is exactly the cleanup the deleting commit's
# message asks hosts to run. A regular file (a copied unit from before this
# scaffolding) proves nothing about its origin, so it is still only
# reported: check-consistency.sh prints the commands for it.
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
  echo "uninstall: no units tracked in $SCRIPT_DIR."
  echo "  Symlinks that a deleted unit left behind are still removed by the"
  echo "  orphan scan below. Residue that is not our symlink (a copied unit"
  echo "  from before this scaffolding) is found by check-consistency.sh,"
  echo "  which prints the exact removal commands."
fi

remove_link() { # remove_link <dest> <name>
  # disable --now is idempotent: a unit already gone from systemd reports an
  # error we do not care about. It also removes the wants symlink.
  run systemctl --user disable --now "$2" >/dev/null 2>&1 || true
  run rm "$1"
  run systemctl --user reset-failed "$2" >/dev/null 2>&1 || true
}

removed=0
skipped=0

for unit in "${units[@]}"; do
  name="${unit##*/}"
  dest="$HOST_DIR/$name"

  if [ -L "$dest" ]; then
    target="$(readlink "$dest")"
    case "$target" in
      "$SCRIPT_DIR"/*) remove_link "$dest" "$name"; removed=$((removed + 1)) ;;
      *)
        printf 'uninstall: skipping %s — it symlinks to %s, not into %s\n' "$dest" "$target" "$SCRIPT_DIR" >&2
        skipped=$((skipped + 1))
        ;;
    esac
  elif [ -e "$dest" ]; then
    printf 'uninstall: skipping %s — regular file, not our symlink; remove it by hand if intended\n' "$dest" >&2
    skipped=$((skipped + 1))
  fi
done

# Orphan scan: symlinks into this directory whose tracked unit no longer
# exists — the shape a same-commit unit deletion leaves on every host that
# has not uninstalled yet (the exact failure mode of 092e82c, where the
# deleted units' host copies stayed and one fired 203/EXEC for a week).
# Only a symlink pointing into $SCRIPT_DIR is removed here, because only
# install.sh ever creates one; anything else is counted as skipped above or
# left for check-consistency.sh to name.
if [ -d "$HOST_DIR" ]; then
  for dest in "$HOST_DIR"/*; do
    [ -L "$dest" ] || continue
    name="${dest##*/}"
    tracked=0
    for unit in "${units[@]}"; do
      [ "${unit##*/}" = "$name" ] && tracked=1
    done
    [ "$tracked" -eq 0 ] || continue
    target="$(readlink "$dest")"
    case "$target" in
      "$SCRIPT_DIR"/*)
        remove_link "$dest" "$name"
        removed=$((removed + 1))
        if [ "$DRY_RUN" = 1 ]; then
          printf 'uninstall: would remove orphaned symlink %s — tracked unit %s was deleted from the repo\n' "$dest" "$target" >&2
        else
          printf 'uninstall: removed orphaned symlink %s — tracked unit %s was deleted from the repo\n' "$dest" "$target" >&2
        fi
        ;;
    esac
  done
fi

run systemctl --user daemon-reload
if [ "$DRY_RUN" = 1 ]; then
  echo "uninstall: dry run complete — nothing changed."
else
  echo "uninstall: done. Removed $removed symlink(s) that pointed into $SCRIPT_DIR${skipped:+, skipped $skipped}."
fi
