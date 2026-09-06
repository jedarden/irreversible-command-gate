#!/usr/bin/env bash
# Reproducible source for docs/assets/icg-demo.gif.
#
# Runs the real `icg` binary against the repo's own packs/ directory and prints
# the real verdicts -- nothing here is staged. The only cosmetic liberties are
# hard-wrapping long lines to the terminal width and trimming each decision to
# its first few lines; run the commands yourself to see them in full.
#
# Regenerate the GIF with:
#   cargo build --release
#   PATH="$PWD/target/release:$PATH" vhs docs/assets/demo.tape
#
# Requires vhs (github.com/charmbracelet/vhs) and ttyd.
set -u

W=94
BOLD=$'\033[1m'; DIM=$'\033[2m'; RESET=$'\033[0m'
RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; CYAN=$'\033[36m'

paint() { # paint <colour> <text...>
  local colour=$1; shift
  printf '%s\n' "$*" | fold -s -w "$W" | while IFS= read -r l; do
    printf '%s%s%s\n' "$colour" "$l" "$RESET"
  done
}

demo() { # demo <command> [lines]
  printf '%s$%s %s%s%s\n' "$DIM" "$RESET" "$BOLD" "$1" "$RESET"
  sleep 0.4
  icg check --command "$1" 2>/dev/null | head -"${2:-3}" | while IFS= read -r line; do
    case "$line" in
      DENIED*)  paint "$RED$BOLD"    "$line" ;;
      REWRITE*) paint "$CYAN$BOLD"   "$line" ;;
      WARNING*) paint "$YELLOW$BOLD" "$line" ;;
      ALLOW*)   paint "$GREEN$BOLD"  "$line" ;;
      *)        paint "$DIM"         "$line" ;;
    esac
  done
  echo
  sleep 1.3
}

printf '%s# every verdict below is real `icg check` output, not a mock-up%s\n\n' "$DIM" "$RESET"
sleep 1.0

demo 'git status' 1
demo 'git push --force origin main' 2
demo 'bao kv get -field=token secret/app/db' 1
demo 'docker system prune --all --volumes' 2
demo 'git credential fill' 2

printf '%s# ~10 ms per check. Nothing ran. Each verdict names the rule and the way forward.%s\n' "$DIM" "$RESET"
sleep 3
