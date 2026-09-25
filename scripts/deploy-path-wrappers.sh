#!/usr/bin/env bash
#
# deploy-path-wrappers.sh -- agent-scoped PATH-wrapper fallback for hookless
# harnesses.
#
#   sudo scripts/deploy-path-wrappers.sh install   # deploy (root-owned dir)
#   sudo scripts/deploy-path-wrappers.sh canary    # fake-target canaries
#   scripts/deploy-path-wrappers.sh verify         # structural + resolution audit
#   scripts/deploy-path-wrappers.sh status         # read-only report
#   sudo scripts/deploy-path-wrappers.sh remove    # idempotent removal
#
# WHAT THIS IS: the fallback enforcement channel for a harness that has no
# blocking native PreToolUse hook. Pack-derived `tool_keywords` become
# symlinks to the icg binary under a root-owned directory
# (/usr/local/libexec/icg-wrappers); a launch wrapper (`icg-harness-env`)
# prepends that directory to PATH for the harness process tree only, so the
# agent's `git`/`bao`/... invocations are evaluated before the real binary
# execs. The operator's login shell is never touched.
#
# WHAT THIS IS NOT: it is not equivalent to a native pre-tool hook. A wrapper
# sees only a PATH-resolved subprocess exec, and is blind to absolute-path
# invocations, structured Write/Edit tool calls, MCP and library calls, and
# anything disabled with ICG_DISABLED=1. It also shadows tools for every
# process launched through the same PATH, not just agent tool calls. Deploy
# the native hook where one exists; treat this as the additional, weaker
# layer. docs/operators/path-wrapper-fallback.md documents each blind spot
# and how these are tested.
#
# Every default path can be overridden (--wrapper-dir/--icg/--pack-dir/
# --launcher), which is how the test suite exercises the whole script
# unprivileged against a temporary staging tree.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAUNCHER_SRC="$SCRIPT_DIR/icg-harness-env.sh"

WRAPPER_DIR="/usr/local/libexec/icg-wrappers"
ICG_BIN="/usr/local/bin/icg"
PACK_DIR="/etc/icg/packs"
LAUNCHER="/usr/local/bin/icg-harness-env"
COMMAND=""
DRY_RUN="${DRY_RUN:-0}"

# A tool name that is not a real binary on any sane host and appears in no
# shipped pack. The canary adds a temporary symlink for it to the wrapper
# directory and a harmless stub "real binary" to a throwaway directory, so
# the full chain (PATH resolve -> guard -> exec real binary) is proven
# without touching any real tool.
CANARY_TOOL="icgwrapcanary"

info() { printf '==> %s\n' "$*"; }
ok()   { printf '  ok %s\n' "$*"; }
warn() { printf '  !! %s\n' "$*" >&2; }
die()  { printf ' error: %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
  cat <<'EOF'

Commands:
  install   Create the root-owned wrapper directory, install pack-derived
            symlinks and the launch wrapper, then verify. Refuses an empty
            pack set and refuses to clobber foreign entries.
  canary    Run harmless fake-target canaries through the wrapper directory:
            reach-through exec, enforced denial, practice pass-through,
            missing-target refusal, recursion refusal. Self-cleaning.
  verify    Audit the deployment: directory ownership/mode, symlink
            integrity, kubectl absence, and that every wrapper resolves a
            later real binary (absences are warnings, not failures).
  status    Read-only deployment report, including PATH visibility.
  remove    Remove icg's wrapper symlinks and the launch wrapper. Leaves
            unrelated files alone; idempotent.

Options:
  --wrapper-dir <dir>   Wrapper symlink directory (default /usr/local/libexec/icg-wrappers)
  --icg <path>          icg binary the symlinks point at (default /usr/local/bin/icg)
  --pack-dir <dir>      Rule-pack directory symlinks are derived from (default /etc/icg/packs)
  --launcher <path>     Where icg-harness-env is installed (default /usr/local/bin/icg-harness-env)
  --dry-run             install/remove: print the actions, change nothing
  --help                This text
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    install|canary|verify|status|remove)
      [ -z "$COMMAND" ] || die "one command per run (got both $COMMAND and $1)"
      COMMAND="$1"; shift ;;
    --wrapper-dir) WRAPPER_DIR="${2:?--wrapper-dir needs a directory}"; shift 2 ;;
    --icg)         ICG_BIN="${2:?--icg needs a path}"; shift 2 ;;
    --pack-dir)    PACK_DIR="${2:?--pack-dir needs a directory}"; shift 2 ;;
    --launcher)    LAUNCHER="${2:?--launcher needs a path}"; shift 2 ;;
    --dry-run)     DRY_RUN=1; shift ;;
    --help|-h)     usage; exit 0 ;;
    *)             die "unknown argument: $1 (try --help)" ;;
  esac
done
[ -n "$COMMAND" ] || { usage; exit 2; }

# Canonicalize the four deployment paths before anything else runs. The
# canary probes and the launcher execute from throwaway directories; a
# relative --pack-dir (or a relative default) re-resolved from a probe's cwd
# points at nothing, ICG_RULE_PACK then loads an EMPTY engine, and an empty
# engine allows everything -- the canaries exist to catch that class of
# silent failure, but the paths must not be ambiguous in the first place.
# -m because the wrapper dir and launcher legitimately do not exist yet.
WRAPPER_DIR="$(realpath -m -- "$WRAPPER_DIR")"
ICG_BIN="$(realpath -m -- "$ICG_BIN")"
PACK_DIR="$(realpath -m -- "$PACK_DIR")"
LAUNCHER="$(realpath -m -- "$LAUNCHER")"

if [ "$DRY_RUN" = 1 ]; then
  info "dry run -- nothing will be changed"
fi

# --------------------------------------------------------------------------
# shared helpers
# --------------------------------------------------------------------------

resolved() { realpath "$1" 2>/dev/null || readlink -f "$1" 2>/dev/null || echo "$1"; }

writable() { [ -w "$1" ]; }

# Writes under /usr and /usr/local are a root question; a staged install via
# explicit --wrapper-dir/--launcher paths only needs plain write access.
require_write_access() {
  if [ "$(id -u)" -eq 0 ]; then return 0; fi
  local missing=0 dir
  for dir in "$WRAPPER_DIR" "$WRAPPER_DIR/.." "$(dirname "$LAUNCHER")"; do
    if [ -e "$dir" ] && ! writable "$dir"; then missing=1; fi
  done
  if [ "$missing" -eq 1 ]; then
    die "not root and cannot write the target directories. Re-run with sudo, \
or stage the deployment with --wrapper-dir/--launcher."
  fi
}

create_root_owned_dir() { # create_root_owned_dir <dir>
  if [ "$(id -u)" -eq 0 ]; then
    install -d -o root -g root -m 0755 "$1"
  else
    install -d -m 0755 "$1"
  fi
}

# --------------------------------------------------------------------------
# install
# --------------------------------------------------------------------------

cmd_install() {
  [ -x "$ICG_BIN" ] || die "icg binary not found or not executable: $ICG_BIN"
  [ -f "$LAUNCHER_SRC" ] || die "launcher source not found beside this script: $LAUNCHER_SRC"
  local pack_count
  pack_count="$(find "$PACK_DIR" -maxdepth 1 -name '*.json' 2>/dev/null | wc -l)"
  [ "${pack_count:-0}" -gt 0 ] \
    || die "no rule packs in $PACK_DIR. Wrapper symlinks are derived from pack \
tool_keywords; installing without packs yields an empty directory that guards nothing."

  # Everything in the wrapper directory is an icg symlink by invariant. If a
  # foreign entry is already there, refuse: install must not clobber, and
  # leaving it would shadow a real binary for every process on this PATH.
  if [ -d "$WRAPPER_DIR" ]; then
    local foreign
    foreign="$(find "$WRAPPER_DIR" -mindepth 1 -maxdepth 1 ! -type l -print -quit 2>/dev/null)"
    [ -z "$foreign" ] || die "refusing to install: $WRAPPER_DIR contains a non-symlink \
entry ($foreign). A wrapper directory is icg-symlinks only; remove the entry by hand."
  fi

  require_write_access
  info "Creating wrapper directory $WRAPPER_DIR (root-owned 0755)"
  if [ "$DRY_RUN" = 1 ]; then
    printf '   would run: install -d -o root -g root -m 0755 %s\n' "$WRAPPER_DIR"
  else
    create_root_owned_dir "$WRAPPER_DIR"
  fi

  info "Installing pack-derived wrapper symlinks"
  if [ "$DRY_RUN" = 1 ]; then
    printf '   would run: %s install --dir %s --pack %s --force\n' "$ICG_BIN" "$WRAPPER_DIR" "$PACK_DIR"
  else
    "$ICG_BIN" install --dir "$WRAPPER_DIR" --pack "$PACK_DIR" --force
  fi

  info "Installing launch wrapper $LAUNCHER"
  if [ "$DRY_RUN" = 1 ]; then
    printf '   would run: install -m 0755 %s %s\n' "$LAUNCHER_SRC" "$LAUNCHER"
  else
    install -m 0755 "$LAUNCHER_SRC" "$LAUNCHER"
    if [ "$(id -u)" -eq 0 ]; then chown root:root "$LAUNCHER"; fi
  fi

  if [ "$DRY_RUN" = 1 ]; then
    ok "dry run complete; nothing changed"
    return 0
  fi

  # An install that has not been verified is indistinguishable from a broken
  # one, so verify is part of install, not a separate courtesy step.
  cmd_verify

  cat <<EOF

  Deployment summary
    wrappers  $WRAPPER_DIR
    icg       $ICG_BIN
    packs     $PACK_DIR
    launcher  $LAUNCHER

  Trial first, then enforcement:

    $LAUNCHER --practice <harness command ...>     # reports, blocks nothing
    icg status --denials --pattern-summary --since 7d
    $LAUNCHER <harness command ...>                # enforcing

  A wrapper is a fallback for harnesses without a blocking native hook. It
  does not see absolute paths, structured edits, MCP/library calls, or a
  harness with its own hook -- it is not equivalent to that hook.
EOF
}

# --------------------------------------------------------------------------
# canary
# --------------------------------------------------------------------------

cmd_canary() {
  [ -d "$WRAPPER_DIR" ] || die "wrapper directory does not exist: $WRAPPER_DIR (run install first)"
  [ -x "$ICG_BIN" ] || die "icg binary not found or not executable: $ICG_BIN"
  require_write_access
  writable "$WRAPPER_DIR" || die "cannot write $WRAPPER_DIR (canaries add and remove one temporary symlink)"

  local failures=0

  # Harmless stub "real binary": prints a marker and its argv, changes
  # nothing. It lives in a throwaway directory, never in the wrapper
  # directory itself, and is removed with the canary symlink below.
  local stub_dir stub
  stub_dir="$(mktemp -d)"
  stub="$stub_dir/$CANARY_TOOL"
  printf '#!/bin/sh\nprintf %%s "icg-canary-reached"\nprintf " argv0=[%%s] args=[%%s]\\n" "$0" "$*"\n' > "$stub"
  chmod 0755 "$stub"

  local canary_link="$WRAPPER_DIR/$CANARY_TOOL"
  # Defensive on purpose: the EXIT trap outlives this function's locals
  # (they are gone by the time a successful run reaches script exit), and
  # `set -u` would turn the cleanup itself into a nonzero exit.
  cleanup() {
    [ -n "${canary_link:-}" ] && rm -f "$canary_link"
    [ -n "${stub_dir:-}" ] && rm -rf "$stub_dir"
  }
  trap cleanup EXIT

  ln -sfn "$ICG_BIN" "$canary_link"

  # A benign recursion-guard environment: explicit ICG vars so a
  # stale ICG_DISABLED/ICG_PRACTICE in the invoking shell cannot skew the
  # probes.
  probe_env=(
    "PATH=$WRAPPER_DIR:$stub_dir:/usr/bin:/bin"
    "ICG_RULE_PACK=$PACK_DIR"
    "ICG_DISABLED="
  )

  printf '\n Canary 1: fake target is reached through the wrapper (practice mode)\n'
  if env "${probe_env[@]}" ICG_PRACTICE=1 "$canary_link" probe-one \
      | grep -q '^icg-canary-reached argv0=.*args=\[probe-one\]$'; then
    ok "guard ran and execed the fake target later in PATH, argv preserved"
  else
    warn "the wrapper did not reach the fake target -- the exec chain is broken"
    failures=$((failures + 1))
  fi

  printf '\n Canary 2: enforced denial blocks before the real binary runs\n'
  # `git commit -m "..."` with no pathspec is denied by the shipped git pack
  # (git-commit-without-pathspec) and is harmless to attempt: the denial
  # happens before any exec. Do NOT canary with `git push --force` -- that
  # pattern is a rewrite (updated_input), not a deny, so the wrapper would
  # strip the flag and exec git instead of refusing.
  # Probes 2 and 3 run from the neutral stub directory so that even a
  # mis-enforcing wrapper execs git into "not a git repository" instead of
  # against whatever repository the operator happened to run this from.
  local deny_out deny_rc
  deny_out="$(cd "$stub_dir" \
    && env "PATH=$WRAPPER_DIR:$PATH" "ICG_RULE_PACK=$PACK_DIR" "ICG_DISABLED=" \
         "$WRAPPER_DIR/git" commit -m "icg wrapper canary" 2>&1)" && deny_rc=0 || deny_rc=$?
  if [ "$deny_rc" -eq 0 ]; then
    warn "a pathspec-less git commit was NOT denied through the wrapper -- enforcement is not real"
    failures=$((failures + 1))
  elif printf '%s' "$deny_out" | grep -q 'command denied'; then
    ok "pathspec-less git commit denied before exec"
  else
    warn "the git commit failed, but not by icg's denial -- check the pack dir"
    failures=$((failures + 1))
  fi

  printf '\n Canary 3: practice mode reports the same denial without blocking\n'
  # Practice execs the real git after reporting, and git then fails its own
  # way in the stub directory -- so the exit code proves nothing. What proves
  # practice is the PAIR: icg's practice report AND git's own error, which
  # can only exist if the real binary actually ran.
  local practice_out practice_rc
  practice_out="$(cd "$stub_dir" \
    && env "PATH=$WRAPPER_DIR:$PATH" "ICG_RULE_PACK=$PACK_DIR" "ICG_DISABLED=" ICG_PRACTICE=1 \
         "$WRAPPER_DIR/git" commit -m "icg wrapper canary" 2>&1)" && practice_rc=0 || practice_rc=$?
  if ! printf '%s' "$practice_out" | grep -q 'icg practice:'; then
    warn "practice run printed no practice report"
    failures=$((failures + 1))
  elif ! printf '%s' "$practice_out" | grep -q 'not a git repository'; then
    warn "practice never execed the real git -- a practice report that still blocks is not practice"
    failures=$((failures + 1))
  else
    ok "practice reported the denial and still execed the real binary (git rc=$practice_rc)"
  fi

  printf '\n Canary 4: a wrapper with no real binary later in PATH refuses\n'
  local refusal_out refusal_rc
  refusal_out="$(env "PATH=$WRAPPER_DIR:/usr/bin:/bin" "ICG_RULE_PACK=$PACK_DIR" "ICG_DISABLED=" \
    "$canary_link" probe-one 2>&1)" && refusal_rc=0 || refusal_rc=$?
  if [ "$refusal_rc" -ne 0 ] && printf '%s' "$refusal_out" | grep -q 'could not find the real'; then
    ok "missing target refused without recursing or executing anything"
  else
    warn "missing-target case did not refuse cleanly (rc=$refusal_rc)"
    failures=$((failures + 1))
  fi

  printf '\n Canary 5: a second icg symlink later in PATH is skipped, not execed\n'
  local link_dir link_rc
  link_dir="$(mktemp -d)"
  ln -s "$ICG_BIN" "$link_dir/$CANARY_TOOL"
  link_out="$(env "PATH=$WRAPPER_DIR:$link_dir:/usr/bin:/bin" "ICG_RULE_PACK=$PACK_DIR" "ICG_DISABLED=" \
    "$canary_link" probe-one 2>&1)" && link_rc=0 || link_rc=$?
  rm -rf "$link_dir"
  if [ "$link_rc" -ne 0 ] && printf '%s' "$link_out" | grep -q 'could not find the real'; then
    ok "wrapper-skipping is canonical: the second icg link was not execed"
  else
    warn "a second icg symlink was not skipped -- wrapper recursion is possible (rc=$link_rc)"
    failures=$((failures + 1))
  fi

  # The trap restores the pack-derived-only invariant of the wrapper
  # directory even when a probe failed.
  if [ "$failures" -eq 0 ]; then
    printf '\n All canaries passed.\n'
    cleanup
    trap - EXIT
  else
    printf '\n %s canary(ies) FAILED -- do not enforce until they pass.\n' "$failures" >&2
    exit 1
  fi
}

# --------------------------------------------------------------------------
# verify
# --------------------------------------------------------------------------

cmd_verify() {
  [ -d "$WRAPPER_DIR" ] || die "wrapper directory does not exist: $WRAPPER_DIR (run install first)"
  [ -x "$ICG_BIN" ] || die "icg binary not found or not executable: $ICG_BIN"

  local failures=0

  printf '\n Verifying %s\n' "$WRAPPER_DIR"

  # Ownership and mode. On a deployed (system) path the directory must be
  # root-owned; a user-writable wrapper directory would let the guarded
  # agent replace what its PATH resolves to, which is not a guard.
  local mode owner
  mode="$(stat -c '%a' "$WRAPPER_DIR")"
  owner="$(stat -c '%U' "$WRAPPER_DIR")"
  if [ "$((8#$mode & 8#022))" -ne 0 ]; then
    warn "directory is group/world-writable (mode $mode) -- a guarded agent must not be able to rewrite it"
    failures=$((failures + 1))
  else
    ok "directory mode $mode (not group/world-writable)"
  fi
  if [ "$(id -u)" -eq 0 ] || [ "$owner" = "root" ]; then
    if [ "$owner" = "root" ]; then
      ok "directory is owned by root"
    else
      warn "directory is owned by $owner, not root -- re-run install as root"
      failures=$((failures + 1))
    fi
  else
    warn "skipping ownership check (staged install run as $(id -un))"
  fi

  # Symlink integrity: every entry is a symlink to the icg binary.
  local icg_real entry target name count=0 foreign=0
  icg_real="$(resolved "$ICG_BIN")"
  for entry in "$WRAPPER_DIR"/*; do
    [ -e "$entry" ] || continue
    name="$(basename "$entry")"
    if [ ! -L "$entry" ]; then
      warn "$name is not a symlink -- a wrapper directory is icg-symlinks only"
      foreign=$((foreign + 1))
      continue
    fi
    target="$(resolved "$entry")"
    if [ "$target" != "$icg_real" ]; then
      warn "$name is a symlink but does not point at $ICG_BIN (points at $target)"
      foreign=$((foreign + 1))
      continue
    fi
    count=$((count + 1))
  done
  if [ "$foreign" -eq 0 ]; then
    ok "all $count entr(y/ies) are icg wrapper symlinks"
  else
    failures=$((failures + 1))
  fi
  [ "$count" -gt 0 ] || { warn "no icg wrapper symlinks found"; failures=$((failures + 1)); }

  # kubectl is never shadowed: the shipped kubectl pack lists the keyword,
  # but icg install hard-skips it (never shadowed per policy), so that pack
  # guards the hook front-end only and cluster triage stays un-intercepted.
  if [ -e "$WRAPPER_DIR/kubectl" ]; then
    warn "kubectl is present in the wrapper directory -- shadowing kubectl is against policy"
    failures=$((failures + 1))
  else
    ok "kubectl is not shadowed (pack guards the hook front-end only)"
  fi

  # Resolution: every wrapper must find a REAL binary later in PATH. This
  # mirrors real_binary_in_path in the wrapper itself, including the
  # canonical skip of icg links, without executing any tool.
  local missing=0
  local orig_path="$PATH"
  for entry in "$WRAPPER_DIR"/*; do
    [ -L "$entry" ] || continue
    name="$(basename "$entry")"
    local found="" dir candidate candidate_real
    IFS=':' read -ra dirs <<< "$(printf '%s' "$orig_path" | sed "s|$WRAPPER_DIR||g")"
    for dir in "${dirs[@]}"; do
      [ -n "$dir" ] || continue
      candidate="$dir/$name"
      [ -e "$candidate" ] || continue
      [ -f "$candidate" ] || continue
      [ -x "$candidate" ] || continue
      candidate_real="$(resolved "$candidate")"
      [ "$candidate_real" = "$icg_real" ] && continue
      found="$candidate"
      break
    done
    if [ -n "$found" ]; then
      ok "$name -> $found"
    else
      warn "$name has no real binary later in PATH; the wrapper will refuse it (command not found becomes an explicit icg refusal)"
      missing=$((missing + 1))
    fi
  done
  if [ "$missing" -gt 0 ]; then
    printf '  !! %s wrapped tool(s) have no real binary in PATH (listed above)\n' "$missing" >&2
  fi

  # The launch wrapper: installed, executable, and the checked-in source --
  # a modified launcher is a modified guard boundary.
  if [ -e "$LAUNCHER" ]; then
    if [ ! -x "$LAUNCHER" ]; then
      warn "launch wrapper $LAUNCHER is not executable"
      failures=$((failures + 1))
    elif [ -f "$LAUNCHER_SRC" ] && ! cmp -s "$LAUNCHER" "$LAUNCHER_SRC"; then
      warn "launch wrapper $LAUNCHER differs from $LAUNCHER_SRC -- reinstall to resync"
      failures=$((failures + 1))
    else
      ok "launch wrapper $LAUNCHER matches the checked-in source"
    fi
  else
    warn "launch wrapper $LAUNCHER is not installed (run install)"
    failures=$((failures + 1))
  fi

  if [ "$failures" -eq 0 ]; then
    printf '\n Verification passed (%s wrapper(s)%s).\n' "$count" \
      "$([ "$missing" -gt 0 ] && printf ", %s without a real binary" "$missing")"
  else
    printf '\n Verification FAILED with %s problem(s).\n' "$failures" >&2
    exit 1
  fi
}

# --------------------------------------------------------------------------
# status
# --------------------------------------------------------------------------

cmd_status() {
  if [ -d "$WRAPPER_DIR" ]; then
    ok "wrapper directory: $WRAPPER_DIR ($(stat -c '%U:%a' "$WRAPPER_DIR"))"
    local n=0 entry
    for entry in "$WRAPPER_DIR"/*; do
      [ -L "$entry" ] || continue
      printf '    %s -> %s\n' "$(basename "$entry")" "$(readlink "$entry")"
      n=$((n + 1))
    done
    [ "$n" -gt 0 ] || warn "no wrapper symlinks installed"
    [ -e "$WRAPPER_DIR/kubectl" ] && warn "kubectl IS shadowed (against policy)" \
      || ok "kubectl not shadowed"
  else
    warn "wrapper directory not present: $WRAPPER_DIR"
  fi
  if [ -x "$LAUNCHER" ]; then
    ok "launch wrapper: $LAUNCHER"
  else
    warn "launch wrapper not installed: $LAUNCHER"
  fi
  case ":$PATH:" in
    *":$WRAPPER_DIR:"*) warn "the wrapper directory IS in this shell's PATH -- it must only appear in a harness launch environment" ;;
    *)                  ok "this shell's PATH does not contain the wrapper directory (correct)" ;;
  esac
  printf '\n  launch a harness under the wrappers:\n    %s [--practice] <harness command ...>\n' "$LAUNCHER"
}

# --------------------------------------------------------------------------
# remove
# --------------------------------------------------------------------------

cmd_remove() {
  require_write_access

  if [ -d "$WRAPPER_DIR" ]; then
    info "Removing icg wrapper symlinks from $WRAPPER_DIR"
    if [ "$DRY_RUN" = 1 ]; then
      printf '   would run: %s install --dir %s --uninstall --force\n' "$ICG_BIN" "$WRAPPER_DIR"
    else
      # Count before removing: `icg install --uninstall` deletes the links
      # itself on its own (discarded) stdout, so a report built only from
      # what the sweep below deletes would say "removed 0" over a removal
      # of every wrapper.
      local icg_real ours=0 entry target
      icg_real="$(resolved "$ICG_BIN")"
      for entry in "$WRAPPER_DIR"/*; do
        [ -L "$entry" ] || continue
        target="$(resolved "$entry")"
        if [ "$target" = "$icg_real" ]; then
          ours=$((ours + 1))
        fi
      done
      # icg's own uninstall matches links pointing at its current path; the
      # sweep below also catches links left by an earlier binary location.
      "$ICG_BIN" install --dir "$WRAPPER_DIR" --uninstall --force >/dev/null 2>&1 || true
      for entry in "$WRAPPER_DIR"/*; do
        [ -L "$entry" ] || continue
        target="$(resolved "$entry")"
        if [ "$target" = "$icg_real" ]; then
          rm -f "$entry"
        fi
      done
      printf '  ok removed %s wrapper symlink(s)\n' "$ours"
      # rmdir is the truth here, not a directory scan: a dangling unrelated
      # symlink fails `[ -e ]` yet still occupies the directory, and an
      # unguarded rmdir failure would abort the whole script (set -e).
      if rmdir "$WRAPPER_DIR" 2>/dev/null; then
        ok "removed empty directory $WRAPPER_DIR"
      else
        warn "$WRAPPER_DIR left in place: it holds entries that are not ours to remove"
      fi
    fi
  else
    ok "wrapper directory already absent: $WRAPPER_DIR"
  fi

  if [ -e "$LAUNCHER" ]; then
    if [ "$DRY_RUN" = 1 ]; then
      printf '   would remove: %s\n' "$LAUNCHER"
    elif [ -f "$LAUNCHER_SRC" ] && cmp -s "$LAUNCHER" "$LAUNCHER_SRC"; then
      rm -f "$LAUNCHER" && ok "removed launch wrapper $LAUNCHER"
    else
      warn "$LAUNCHER differs from the checked-in launcher -- not ours to remove; left in place"
    fi
  else
    ok "launch wrapper already absent: $LAUNCHER"
  fi

  [ "$DRY_RUN" = 1 ] && { ok "dry run complete; nothing changed"; return 0; }
  ok "removal complete (binary and packs untouched)"
}

case "$COMMAND" in
  install) cmd_install ;;
  canary)  cmd_canary ;;
  verify)  cmd_verify ;;
  status)  cmd_status ;;
  remove)  cmd_remove ;;
esac
