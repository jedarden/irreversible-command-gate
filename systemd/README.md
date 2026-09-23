# `systemd/` — the single source of truth for units this repo installs

## Why this directory exists

Commit `092e82c` deleted `scripts/bead-starvation-unified-repair.sh` and the
twelve units under `systemd/` from the repository — but the *installed* copies
on codinghome lived in `~/.config/systemd/user/`, outside the repo, and stayed
behind. `bead-starvation-unified-repair.timer` kept waking every five minutes
to fail `203/EXEC` for a week (bead `irrevers-46f2b741`). A deleted script
leaving a live unit was only possible because the script and its unit lived in
different places with no link between them. This directory is that link.

**The invariant, in one sentence: any unit this repo installs on a host is
tracked here, installed as a symlink — never a copy — and any commit that
deletes a script must, in the same commit, delete its tracked unit and note in
the commit message that hosts need `systemd/uninstall.sh` run.**

The symlink is what makes deletion loud: removing the tracked unit breaks the
host link immediately and visibly (`not-found` after `daemon-reload`), instead
of leaving a self-contained copy that only fails when it fires.

Nothing is tracked here today. The repository ships no units, and that is the
correct state after the `092e82c` removal — this scaffolding exists so the
*next* unit anyone wants to install has an obvious home and a check guarding
it. Same pattern as `aide-de-camp/deploy/`, `SEAM/scripts/systemd/` and
`tradegraph/scripts/` on this host.

## Usage

```bash
systemd/install.sh            # symlink every tracked unit into ~/.config/systemd/user/,
                               # daemon-reload, enable units with an [Install] section,
                               # then run the consistency check
systemd/install.sh --now      # ... and start them immediately
systemd/uninstall.sh          # stop, disable and remove exactly our symlinks — including
                               # orphaned ones whose tracked unit a commit already
                               # deleted; never touches a host file that is not one of
                               # our symlinks
systemd/uninstall.sh --dry-run  # show what would be removed (and why), change nothing
systemd/check-consistency.sh  # exit 1 on drift, printing each violation and its fix
systemd/check-consistency.sh --repo-only   # CI form: no ~/.config/systemd/user needed
```

Both install and uninstall refuse to clobber or delete a host file that is not
one of our own symlinks — pre-scaffolding residue is surfaced, not silently
replaced or removed.

## What the check catches

Direction (a), **repo → disk**: a tracked unit whose `Exec*` or
`WorkingDirectory` references a path that does not exist in the working tree.
This fires at the moment someone deletes a script and forgets its unit —
before the commit lands, not a week later in the journal.

Direction (b), **host → repo**: a unit installed in `~/.config/systemd/user/`
whose `Exec*` lines execute a path under this repository that no longer
exists — including dangling symlinks into the repo. This is the exact
condition that produced the original bead, detectable mechanically. A
disabled unit counts: it is one `enable` away from the same `203/EXEC`, which
is why `icg-frontier-consistency.service` (found by this check on its first
run, 2026-09-14) had gone unnoticed — it never fired.

Direction (c), **tracked → host**: a unit this repo tracks, when present in
the host unit dir at all, must be `install.sh`'s symlink to the tracked
file. A regular file at that destination is a copy — and a copy is invisible
to direction (b) for as long as every path it references still exists, which
is exactly the window in which it starts drifting from its tracked unit. A
symlink to anywhere else (another checkout, a renamed file) is the same drift
one checkout-deletion away. A tracked unit this host never installed is not
a violation: not every host installs every unit.

"Exists in the repo tree" means exists in the working checkout on the host:
that is what systemd resolves against. A `target/` binary is never in git but
is real to a unit once built; a script that exists only in git history is
dead to a unit.

## Deleting a script, or a unit

The required cleanup, in the order it should happen:

1. **Deleting a script** — delete its tracked unit **in the same commit** and
   say in the commit message that hosts need `systemd/uninstall.sh` run.
   `check-consistency.sh` direction (a) blocks the push if the unit is left
   behind pointing at the vanished script.
2. **Deleting a unit only** (retiring it while its script stays) — same rule:
   delete it in the same commit that stops using it, and mention
   `systemd/uninstall.sh` in the commit message.
3. **On each host after pulling such a commit** — run `systemd/uninstall.sh`.
   It removes the still-tracked symlinks *and* the orphaned ones: a host
   symlink pointing into this directory is ours by construction (only
   `install.sh` creates one), so uninstall removes it even when its tracked
   unit has already been deleted and the link dangles. `systemctl --user
   daemon-reload` then makes systemd drop the unit for good.
4. **A plain-file copy** (pre-scaffolding residue like the retired
   `icg-frontier-consistency.service`) proves nothing about its origin, so
   nothing deletes it automatically. `check-consistency.sh` names it and
   prints the exact `systemctl`/`rm` commands until a human removes it —
   direction (c) flags every copy sitting at a tracked unit's destination,
   whether or not its paths still resolve, so it cannot wait quietly for the
   day its script dies.

## Limits

- `Exec*` argument tokens are split on whitespace; quoted paths containing
  spaces would be mis-split. No unit here has ever needed one.
- `%h`-style specifiers are ignored (they never match the repo root).
- Drop-in overrides (`*.service.d/`) are not parsed; the main unit file is.
- Directions (b) and (c) need the host, so CI runs only direction (a) — via
  `cargo test` (the release gate in the `icg-ci` workflow) and as an explicit
  step of `scripts/definition-of-done.sh` — while
  `tests/systemd_consistency_tests.rs` exercises all three directions against
  fixtures, and `tests/systemd_lifecycle_tests.rs` proves the round trip:
  the full check passes over what `install.sh` leaves behind and over what
  `uninstall.sh` leaves behind.
