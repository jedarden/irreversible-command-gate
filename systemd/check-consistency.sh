#!/usr/bin/env bash
#
# systemd/check-consistency.sh — fail when this repo's units and a host drift apart.
#
# Direction (a), repo -> disk: every unit tracked in this directory must
#   reference only Exec* / WorkingDirectory paths that exist in the repo
#   working tree. A tracked unit pointing at a deleted script is the bug this
#   check exists to catch BEFORE the commit deleting the script lands.
#
# Direction (b), host -> repo: every unit installed in the host unit dir
#   (default ~/.config/systemd/user) whose Exec* lines execute a path inside
#   this repository must reference a path that still exists. This is the exact
#   condition that produced bead irrevers-46f2b741: commit 092e82c deleted
#   scripts/bead-starvation-unified-repair.sh, the installed copy of its unit
#   stayed behind in ~/.config/systemd/user/, and the timer kept waking every
#   five minutes to fail 203/EXEC — for a week.
#
# "Exists in the repo tree" means exists in the working checkout on this
# host, because that is what systemd resolves against. A binary under
# target/ is never in git but is real to a unit once built; a script that
# exists only in git history is dead to a unit.
#
# Exit 0 = consistent, 1 = at least one violation (each printed with its
# remediation), 2 = usage error.
#
# Flags and environment (exercised by tests/systemd_consistency_tests.rs):
#   --repo-only         skip the host scan; CI has no ~/.config/systemd/user
#   ICG_SYSTEMD_DIR     override the tracked-unit directory (default: this one)
#   ICG_HOST_UNIT_DIR   override the host unit directory
#   ICG_REPO_ROOT       override the repo root that defines "inside this repo"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${ICG_REPO_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
UNIT_DIR="${ICG_SYSTEMD_DIR:-$SCRIPT_DIR}"
HOST_DIR="${ICG_HOST_UNIT_DIR:-$HOME/.config/systemd/user}"

REPO_ONLY=0
[ "${1:-}" = "--repo-only" ] && REPO_ONLY=1
[ $# -gt 0 ] && [ "${1:-}" != "--repo-only" ] && { echo "usage: $0 [--repo-only]" >&2; exit 2; }

# repo_paths_in_unit <file> <exec|all>
#   Print every Exec* path token (plus WorkingDirectory when <all>) that sits
#   under $REPO_ROOT. Tokens are split on whitespace; systemd exec modifiers
#   (- + ! !!) are stripped from argv[0]. %h-style specifiers and paths
#   outside the repo never match and are ignored.
repo_paths_in_unit() {
  local file=$1 mode=$2
  awk -v root="$REPO_ROOT/" -v mode="$mode" '
    $0 ~ /^[[:space:]]*(Exec[A-Za-z]*)([[:space:]]*=[[:space:]]*)/ ||
    (mode == "all" && $0 ~ /^[[:space:]]*WorkingDirectory([[:space:]]*=[[:space:]]*)/) {
      line = $0
      sub(/^[^=]*=[[:space:]]*/, "", line)
      n = split(line, tok, /[[:space:]]+/)
      for (i = 1; i <= n; i++) {
        t = tok[i]
        if (t == "") continue
        # Modifiers are only valid on argv[0], but stripping them everywhere
        # is harmless: a repo-rooted path never starts with - + or !.
        sub(/^[-+!]+/, "", t)
        if (index(t, root) == 1) print t
      }
    }
  ' "$file"
}

violations=0
say() { printf '%s\n' "$*"; }

# --------------------------------------------------------------------------
# Direction (a): tracked units must reference paths that exist
# --------------------------------------------------------------------------
shopt -s nullglob
tracked=()
for unit in "$UNIT_DIR"/*.service "$UNIT_DIR"/*.timer; do
  tracked+=("$unit")
  while IFS= read -r p; do
    if [ ! -e "$p" ]; then
      say "VIOLATION (repo -> disk): tracked unit ${unit#$REPO_ROOT/} references a path that does not exist in the working tree:"
      say "  $p"
      say "  Restore the path, or delete the unit in the SAME commit and say in the commit"
      say "  message that hosts need systemd/uninstall.sh run."
      violations=$((violations + 1))
    fi
  done < <(repo_paths_in_unit "$unit" all)
done

# --------------------------------------------------------------------------
# Direction (b): host units executing repo paths that no longer exist
# --------------------------------------------------------------------------
if [ "$REPO_ONLY" -eq 1 ]; then
  :
elif [ ! -d "$HOST_DIR" ]; then
  echo "note: host unit dir $HOST_DIR does not exist — skipping host scan" >&2
else
  for unit in "$HOST_DIR"/*.service "$HOST_DIR"/*.timer; do
    name="${unit##*/}"
    # A symlink into this repo whose target is gone is the shape a repo-side
    # deletion leaves behind when uninstall.sh was never run.
    if [ -L "$unit" ] && [ ! -e "$unit" ]; then
      target="$(readlink "$unit")"
      case "$target" in
        "$REPO_ROOT"/*)
          say "VIOLATION (host -> repo): $name is a dangling symlink into this repo: $target"
          say "  The tracked unit was deleted; the host copy stayed. Remove it:"
          say "    rm '$unit' && systemctl --user daemon-reload"
          violations=$((violations + 1))
          ;;
      esac
      continue
    fi
    [ -r "$unit" ] || continue
    while IFS= read -r p; do
      if [ ! -e "$p" ]; then
        say "VIOLATION (host -> repo): installed unit $name executes a repo path that does not exist:"
        say "  $p"
        say "  One enable away from 203/EXEC. Remove the unit from the host:"
        say "    systemctl --user disable --now '$name'"
        say "    rm '$unit' && systemctl --user daemon-reload && systemctl --user reset-failed '$name'"
        violations=$((violations + 1))
      fi
    done < <(repo_paths_in_unit "$unit" exec)
  done
fi

# --------------------------------------------------------------------------
if [ "$violations" -gt 0 ]; then
  say "check-consistency: $violations violation(s) — repo units and host have drifted apart"
  exit 1
fi
host_state="done"
[ "$REPO_ONLY" -eq 1 ] && host_state="skipped"
say "check-consistency: consistent (${#tracked[@]} tracked unit(s), host scan $host_state)"
