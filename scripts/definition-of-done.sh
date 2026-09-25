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
#   --fast (default)  cargo fmt --all -- --check (icg-ci's first gate in
#                     build-and-release; the local DoD skipped it while HEAD
#                     carried other beads' in-flight formatting, and fmt debt
#                     sailed through three times — irrevers-59887aa4,
#                     irrevers-9bb1c696, and the 2026-09-23 five-file
#                     failure — before surfacing in CI), cargo build
#                     --all-targets, cargo test, the OpenCode plugin's node
#                     suite (opencode-plugin/, skipped loudly when node/npm
#                     are absent), the systemd consistency gate
#                     (systemd/check-consistency.sh — the repo side always,
#                     plus the host-side orphan/copy scan whenever this tree
#                     is the checkout the host's unit symlinks point into; a
#                     git-archive extraction gets --repo-only, loudly — see
#                     systemd/README.md and
#                     docs/runbooks/systemd-unit-lifecycle.md),
#                     and the README asset-reference gate
#                     (scripts/check-doc-assets), and the README latency
#                     claim's regression gate (a release build plus
#                     scripts/bench-check-latency --assert-under 50 on the
#                     shipped pack set — docs/notes/check-latency-benchmark.md)
#   --slow            additionally cargo clippy --all-targets -- -D warnings
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
last_rc=0
run() {
  "$@"
  rc=$?
  last_rc=$rc
  printf '%s\t%d\n' "$*" "$rc"
  if [ "$rc" -ne 0 ]; then
    status=1
  fi
  return 0
}

# First, to match CI: build-and-release gates fmt before anything else, and
# a fmt failure there costs a full CI cycle to discover. Content-based, so
# the mtime touches above are irrelevant to it; runs in seconds.
run cargo fmt --all -- --check
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
# The systemd consistency gate: a tree whose tracked units reference paths
# missing from the working tree cannot be "done" — that is the drift that
# once fired 203/EXEC for a week (irrevers-46f2b741). CI covers the check's
# semantics via cargo test (tests/systemd_consistency_tests.rs,
# tests/systemd_lifecycle_tests.rs); running the script directly also gates
# the executable bit and the script itself, not just what the tests exercise.
#
# The host-side half — orphaned installed units executing dead repo paths,
# copies sitting at tracked-unit destinations — runs here too, as the full
# scan, not only via install.sh's self-check. One guard: a tree that tracks
# units yet has no host symlink pointing into it (a git-archive extraction,
# such as NEEDLE's close gate produces, or a second checkout) cannot be the
# checkout the host linked, so there direction (c)'s symlink comparison would
# false-positive against the installed checkout's links and direction (b) has
# nothing to match. Those trees run --repo-only, with a note saying so.
host_dir="${ICG_HOST_UNIT_DIR:-$HOME/.config/systemd/user}"
tracked_units=0
for unit in "$REPO_ROOT"/systemd/*.service "$REPO_ROOT"/systemd/*.timer; do
  [ -e "$unit" ] && tracked_units=1
done
linked_here=1
if [ "$tracked_units" -eq 1 ] && [ -d "$host_dir" ]; then
  linked_here=0
  for unit in "$host_dir"/*; do
    [ -L "$unit" ] || continue
    case "$(readlink "$unit")" in "$REPO_ROOT"/*) linked_here=1 ;; esac
  done
fi
if [ "$tracked_units" -eq 1 ] && [ "$linked_here" -eq 0 ]; then
  echo "note: units are tracked but no host symlink in $host_dir points into this tree" \
       "(extraction or non-installed checkout) — running --repo-only"
  run systemd/check-consistency.sh --repo-only
else
  run systemd/check-consistency.sh
fi
# README's local asset references must resolve: the demo GIF went stale once
# (irrevers-8c3bab0e) and a missing, emptied, or wrong-typed one would render
# broken on the mirror with every code gate green. CI covers the gate
# semantics via tests/doc_asset_check_tests.rs (rust-verify never runs repo
# scripts); this direct run also gates the script's executable bit, the way
# the systemd gate does above.
run scripts/check-doc-assets
# The README's warm-cache latency claim, held up by a gate (the claim: p50
# ~15–20 ms on the shipped 11-pack set; the record:
# docs/notes/check-latency-benchmark.md). The gate builds the RELEASE
# profile first — the profile the claim is about; a dev-profile binary
# measures something else — then asserts every case's p50 under 50 ms:
# ~3x the measured median under reference load, loose enough that
# background load moving the p50 between 15 and 20 ms cannot flake a
# median, tight enough to catch the rot that would hollow the claim out
# (pack-count growth, hot-path I/O, a per-pattern compile regression). The
# shipped pack set is pinned (--pack + --cwd /tmp, the note's canonical
# run) so the gate measures what README claims, not whatever /etc/icg/packs
# happens to hold on the box running the gate. Deliberately absent from
# cargo test and the shared-runner CI — the same absolute-time-assertion
# reasoning, recorded in the note. Skipped, loudly, when the release build
# itself fails: benchmarking the previous tree's stale binary would gate
# nothing.
run cargo build --release
if [ "$last_rc" -eq 0 ]; then
  run scripts/bench-check-latency --pack "$REPO_ROOT/packs" --cwd /tmp \
    --assert-under 50 --iterations 40 --warmup 5
else
  printf 'scripts/bench-check-latency --assert-under 50\tSKIP (release build failed)\n'
fi
if [ "$SLOW" -eq 1 ]; then
  run cargo clippy --all-targets -- -D warnings
fi

exit $status
