#!/bin/sh
# Definition of done for irreversible-command-gate.
#
# Why this script exists: a `git archive` extraction (the shape NEEDLE's
# close gate and this repo's own verification checklist both use) lands at a
# fresh path with archive mtimes, and cargo's global config directs every
# CARGO_TARGET_DIR-less invocation at the fleet-shared /build/target-workers.
# When a recent build of identical content is cached there, the fingerprint
# matches, `cargo build` no-ops, and `cargo test` runs test binaries whose
# baked CARGO_MANIFEST_DIR still names the PREVIOUS -- since deleted --
# extraction. Manifest-relative fixture reads then ENOENT and the suite fails
# with no code change at all: irrevers-f231c112 was closed at 2026-09-18
# 01:39Z and reopened a minute later on exactly this (beads_scope_tests
# failing against a deleted extraction path), reproduced by deleting a
# verification extraction and re-running the untouched tree.
#
# This script points the build at a repo-dedicated target dir and forces the
# workspace crate's targets to rebuild from the current tree, so the binaries
# that run always carry this tree's paths. Registry dependencies are
# path-independent, so caching them across runs is safe and keeps the forced
# rebuild at the ~20s the workspace crate costs, not a cold dep tree.
#
# Usage: scripts/definition-of-done.sh [--fast|--slow]
#   --fast (default)  cargo build --all-targets, cargo test, the OpenCode
#                     plugin's node suite (opencode-plugin/, skipped loudly
#                     when node/npm are absent), and the repo-side systemd
#                     consistency gate (systemd/check-consistency.sh
#                     --repo-only; the host-side half runs on hosts — see
#                     systemd/README.md)
#   --slow            additionally cargo clippy --all-targets -- -D warnings
#                     (fmt is deliberately not gated here: HEAD carries
#                     unrelated in-flight formatting in tests owned by other
#                     beads, and fmt does not affect the built contract)
# Prints one "command<TAB>exit" line per step and exits non-zero if any step
# failed.
set -u

SLOW=0
case "${1:-}" in
  "" | --fast) ;;
  --slow) SLOW=1 ;;
  *)
    echo "usage: $0 [--fast|--slow]" >&2
    exit 2
    ;;
esac

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT" || exit 41

# Repo-dedicated cache, overridable for one-off isolation. Falls back inside
# the tree when /build is not mounted, so the script degrades to a plain
# (slower, per-run-cold) verification rather than failing.
if [ -d /build ]; then
  CARGO_TARGET_DIR="${ICG_VERIFY_TARGET_DIR:-/build/target-icg-verify}"
else
  CARGO_TARGET_DIR="${ICG_VERIFY_TARGET_DIR:-$REPO_ROOT/target}"
fi
export CARGO_TARGET_DIR

# The stale-path guard: rebuild every workspace target from THIS tree.
# Fingerprint matches against another path's identical content are exactly
# the failure above, and mtimes are the lever that breaks the match.
find src tests examples -name '*.rs' -exec touch {} + 2>/dev/null
touch Cargo.toml 2>/dev/null

status=0
run() {
  "$@"
  rc=$?
  printf '%s\t%d\n' "$*" "$rc"
  if [ "$rc" -ne 0 ]; then
    status=1
  fi
  return 0
}

run cargo build --all-targets
run cargo test
# The OpenCode plugin's runtime suite (node --test, no dependencies): the
# gate semantics live there, so done includes it. Loud skip, never a silent
# one, on a box without the runtime.
if command -v node >/dev/null 2>&1 && command -v npm >/dev/null 2>&1; then
  run npm test --prefix opencode-plugin
else
  printf 'npm test --prefix opencode-plugin\tSKIP (node/npm not on PATH)\n'
fi
# Type-level check on the deployed plugin artifact (skips loudly without
# tsc — see scripts/opencode-plugin-typecheck).
run scripts/opencode-plugin-typecheck
# The systemd consistency gate, repo-side half: a tree whose tracked units
# reference paths missing from the working tree cannot be "done" — that is
# the drift that once fired 203/EXEC for a week (irrevers-46f2b741). CI
# covers this via cargo test (tests/systemd_consistency_tests.rs); running
# the script directly also gates the executable bit and the script itself,
# not just what the tests exercise. The host-side half (symlink pairing,
# installed-unit scan) runs on hosts via systemd/install.sh's self-check.
run systemd/check-consistency.sh --repo-only
if [ "$SLOW" -eq 1 ]; then
  run cargo clippy --all-targets -- -D warnings
fi

exit $status
