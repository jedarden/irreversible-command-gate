#!/bin/sh
# Launch a harness command with ICG's PATH wrappers scoped to its process tree.
#
#   icg-harness-env [--practice] <command> [args...]
#
# This is the sanctioned way to put a hookless harness under the PATH-wrapper
# fallback (see docs/operators/path-wrapper-fallback.md). It prepends the
# wrapper directory to PATH for the launched command ONLY:
#
#   - the calling shell's PATH is never modified (this script execs; it does
#     not export anything into its parent),
#   - nothing here belongs in a login shell rc file -- the wrapper directory
#     must stay out of the operator's own environment,
#   - the wrapper directory itself is root-owned, so the guarded agent cannot
#     replace what its PATH resolves to.
#
# Mode:
#   --practice   launches with ICG_PRACTICE=1: the guard reports what it would
#                deny and blocks nothing. This is the correct first launch of a
#                new harness; read `icg status --denials` before enforcing.
#   (default)    enforcing: denials block the tool call. Switch only after the
#                practice trial and the canaries have passed.
#
# Failure behavior: if the wrapper directory is missing or holds no wrapper
# symlinks, the launcher refuses to start the harness rather than silently
# launching something that only looks guarded. Bypassing that check is always
# possible -- just launch the harness without this launcher -- because the
# wrapper is a convenience boundary, not a security one, and never a
# substitute for the harness's native PreToolUse hook.

set -u

WRAPPER_DIR="${ICG_WRAPPER_DIR:-/usr/local/libexec/icg-wrappers}"
PRACTICE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --practice) PRACTICE=1; shift ;;
    --help|-h)
      sed -n '2,4p' "$0" | sed 's/^# \{0,1\}//'
      printf '\n  --practice  report would-be denials, block nothing (trial mode)\n'
      exit 0
      ;;
    --) shift; break ;;
    -*) printf 'icg-harness-env: unknown option: %s\n' "$1" >&2; exit 2 ;;
    *)  break ;;
  esac
done

if [ $# -eq 0 ]; then
  printf 'usage: icg-harness-env [--practice] <command> [args...]\n' >&2
  exit 2
fi

if [ ! -d "$WRAPPER_DIR" ]; then
  printf 'icg-harness-env: wrapper directory %s does not exist.\n' "$WRAPPER_DIR" >&2
  printf '  Deploy it first: sudo scripts/deploy-path-wrappers.sh install\n' >&2
  printf '  Refusing to launch a harness that only looks guarded.\n' >&2
  exit 1
fi

# At least one icg wrapper symlink must be present, or PATH prepending would
# be a no-op the operator believes is coverage.
WRAPPER_COUNT=0
for entry in "$WRAPPER_DIR"/*; do
  [ -L "$entry" ] && WRAPPER_COUNT=$((WRAPPER_COUNT + 1))
done
if [ "$WRAPPER_COUNT" -eq 0 ]; then
  printf 'icg-harness-env: no wrapper symlinks in %s -- run deploy-path-wrappers.sh install\n' "$WRAPPER_DIR" >&2
  exit 1
fi

if [ -n "${ICG_DISABLED:-}" ]; then
  printf 'icg-harness-env: WARNING: ICG_DISABLED=%s is set -- the guard will stand down for this launch.\n' "$ICG_DISABLED" >&2
fi

if [ "$PRACTICE" = 1 ]; then
  # Exported, not just set: the wrapper process that evaluates each tool call
  # is a grandchild of this launcher, and an unexported variable would leave
  # every wrapper enforcing while the operator believes they are trialing.
  ICG_PRACTICE=1
  export ICG_PRACTICE
  printf 'icg-harness-env: PRACTICE mode -- denials are reported, not blocked. This is not enforcement.\n' >&2
fi

PATH="$WRAPPER_DIR:$PATH"
export PATH
exec "$@"
