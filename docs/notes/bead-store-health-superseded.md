# `bead doctor` supersedes the removed bead-store health subsystem

Records the end state of the subsystem deleted in `092e82c` and the host-side
cleanup that followed it, so the next reader of the refactor commit knows the
removal is complete rather than half-done. Cross-references the tracking bead
`irrevers-46f2b741` and its host-retirement child.

## What the subsystem was

A unified starvation diagnostic/auto-repair layer for the bead store, added
across three commits on 2026-08-26:

- `481ce89` — unified automated starvation diagnostic and auto-repair service
- `255faed` — enable auto-repair in the starvation diagnostic system
- `79bc790` — automated bead assignment state validation and repair

It grew into ten `src/` modules, seven of the eight binaries, eighteen scripts,
twelve `systemd/` units, container images, Kubernetes manifests, and a bundled
SQLite build — all living inside what is supposed to be a PreToolUse command
guard. It shared no types and no imports with the guard in either direction.

## Why it was removed

`092e82c` ("refactor: remove the bead-store health subsystem; bead doctor
already does it") deleted it because **`bead doctor` already covers every one
of its seven tools** — `--starvation-check`, `--starvation-recovery [--force]`,
`--visibility-check`, `--rehearse`, `--repair --scope dependencies`, plus
`bead list --ready --verbose` for per-bead exclusion reasons. Worse, the
duplicate was incorrect: its hand-written SQL had no notion of
`resource_locks`, `leases`, or `claim_epoch`, so it would clear a live
worker's claim on a two-machine fleet. The removal is not a regression; the
replacement is both pre-existing and safer.

## The host side had to be cleaned up separately

The repo deleted `systemd/bead-starvation-unified-repair.{service,timer}` in
`092e82c`, but the *installed* copies at
`~/.config/systemd/user/` on codinghome were user-level state outside the
repo and stayed behind with their `timers.target.wants` symlink. The service
pointed at `scripts/bead-starvation-unified-repair.sh`, which no longer
existed — so the timer kept waking every 5 minutes to fail with
`203/EXEC` ("No such file or directory"), indefinitely. A deleted script
leaving a live unit was only possible because the script and its unit live in
different places with no link between them.

**Retired 2026-09-13 23:17:23 EDT** (= 2026-09-14T03:17:23Z) by unravel child
`irrevers-4d3b6f16`, verified before and after acting: timer disabled (`--now`,
removing the wants symlink), both unit files deleted, `daemon-reload`,
`reset-failed`; afterwards zero matching timers, `not-found` on both units,
nothing in `~/.config/systemd/user/` referencing the name, and no journal
entries past the retirement timestamp.

The other failed user units on this host (`starvation-watchdog`,
`bead-self-heal`, etc.) belong to other subsystems and repos and were left
untouched.

## A second orphan the first sweep missed

`systemd/check-consistency.sh` (landed 2026-09-14, `irrevers-ccb71837`) found
another leftover of the same class on its first run:
`icg-frontier-consistency.service` is still installed in
`~/.config/systemd/user/` as a plain file, its repo-side unit was one of the
twelve deleted in `092e82c`, and its `ExecStart` points at
`target/debug/frontier-consistency-check`, which no longer builds from this
tree. Unlike unified-repair it is **disabled and inactive** — one `enable`
away from the same `203/EXEC`, which is exactly why nothing was failing
loudly and the retirement sweep (which searched for the unified-repair name)
did not see it. Removing it is a host write, outside the scaffolding bead's
scope; the check prints the remediation on every run until it is done:

```console
systemctl --user disable --now icg-frontier-consistency.service
rm ~/.config/systemd/user/icg-frontier-consistency.service
systemctl --user daemon-reload
systemctl --user reset-failed icg-frontier-consistency.service 2>/dev/null || true
```

## The successor repair path

`bead doctor` is the one starvation-repair tool, and on this host it runs as
the user-level `bead-doctor.service` / `bead-doctor.timer`. That unit had its
own outage — exit 127 because the unit's PATH lacked `python3`, which the
`~/.local/bin/bead` wrapper needs — fixed and verified 2026-09-13 via
`irrevers-2cdc887a` (now closed). The timer is enabled and firing again.

Anything that looks like "automated starvation repair" should be assumed to
mean `bead doctor` / `bead-doctor.timer`. Nothing else exists.

## What remains open

- `irrevers-ccb71837` — **landed 2026-09-14**: `systemd/` now carries
  `install.sh` / `uninstall.sh` / `check-consistency.sh` plus the invariant
  in `systemd/README.md`, AGENTS.md documents the rule, and
  `tests/systemd_consistency_tests.rs` runs the repo-side half in CI.
- Removing `icg-frontier-consistency.service` from the host (above) — a
  host-side action for the operator or the retirement strand; the check will
  keep naming it until then.
- `irrevers-46f2b741` — the parent bead; closes once the remaining WANTED
  items above land.
