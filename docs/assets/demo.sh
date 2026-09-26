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
#   BIN="$(cargo metadata --format-version 1 --no-deps | python3 -c \
#     'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/icg"
#   PATH="$(dirname "$BIN"):$PATH" vhs docs/assets/demo.tape
#
# Requires vhs (github.com/charmbracelet/vhs) and ttyd.
#
# The verdicts printed here are pinned by tests/demo_verdict_regression_tests.rs:
# it feeds every command and file body below to the real binary against the
# repo's own packs/ and asserts the ALLOW / REWRITE / WARNING / DENIED prefix
# README.md promises for it. If a pack or engine change moves a pinned
# verdict, update that test and regenerate the GIF with the recipe below in
# the same change, so the README keeps showing real output.
#
# Size of the committed asset: the GIF captured in 0ae6afb is 614658 bytes,
# and every capture so far has landed between 450 KB and 620 KB. A recapture
# legitimately moves the exact number -- update this note with it in the same
# commit -- but treat a committed GIF far below that band, or one whose final
# byte is not the GIF trailer ';', as a truncated capture:
#   tail -c 1 docs/assets/icg-demo.gif | od -An -c   # expect ';'
# scripts/check-doc-assets (in the DoD) already fails a missing, emptied or
# wrong-magic one mechanically; truncation is what this note catches by eye.
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

# Same, for a file-content check -- the Write/Edit and apply_patch path.
demo_file() { # demo_file <content> <label> [lines]
  printf '%s$%s %sprintf %s | icg check --file -%s\n' "$DIM" "$RESET" "$BOLD" "$2" "$RESET"
  sleep 0.4
  printf '%s' "$1" | icg check --file - 2>/dev/null | head -"${3:-3}" | while IFS= read -r line; do
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

# All four verdicts, and both input modes.
demo 'git status' 1
demo 'git push --force origin main' 2
demo 'bao kv get -field=token secret/app/db' 1
demo 'bao kv destroy secret/app/db' 2
demo 'git credential fill' 2
demo_file 'image: ronaldraygun/armor:latest
' "'image: ronaldraygun/armor:latest'" 2

printf '%s# ~15–20 ms per check. Nothing ran. Each verdict names the rule and the way forward.%s\n' "$DIM" "$RESET"
sleep 3
