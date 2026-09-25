# systemd unit lifecycle runbook

This runbook governs every unit this repository installs on a host. Units live
in `systemd/` in the repo and reach the host as **symlinks** into
`~/.config/systemd/user/` — never copies. The design rationale and the
statement of the invariant live in [`systemd/README.md`](../../systemd/README.md);
this page is the procedure: how to install, how to verify, and what to do when
the check says the repo and the host have drifted.

The incident behind all of it: a commit deleted a unit's script from the repo,
the installed *copy* of the unit stayed behind on the host, and its timer woke
every five minutes to fail `203/EXEC` — for a week (bead `irrevers-46f2b741`).
A symlink breaks loudly the day its target is deleted; a copy breaks silently,
when it next fires. That asymmetry is the whole rule.

## The symlink requirement

Non-negotiable, for every unit this repo tracks:

- The host destination `~/.config/systemd/user/<name>` must be a symlink to
  the tracked file in the checkout that installed it. `systemd/install.sh` is
  the only thing that may create one.
- Never `cp` a unit file into the host unit directory. A regular file there is
  a copy: invisible to every path check for as long as the paths it references
  exist, and the silent half of the incident above.
- Never hand-edit an installed unit. Edit the tracked file in `systemd/` — the
  symlink makes the edit live on the host after
  `systemctl --user daemon-reload`.
- The checkout that ran `install.sh` owns its links. A symlink into a checkout
  that has since been deleted is an orphan; `uninstall.sh` removes it by
  construction (it is the only link-maker), and the consistency check names it.

## Installing units on a host

```bash
cd /path/to/irreversible-command-gate
systemd/install.sh --dry-run   # preview: what would be linked, enabled, run
systemd/install.sh             # link, daemon-reload, enable [Install] units, self-check
systemd/install.sh --now       # ... and start the enabled units immediately
```

What install does, in order:

1. Symlinks every `*.service` / `*.timer` tracked in `systemd/` into
   `$ICG_HOST_UNIT_DIR` (default `~/.config/systemd/user`).
2. Runs `systemctl --user daemon-reload`.
3. Enables every unit that carries an `[Install]` section (units without one
   are linked but not enabled — enable would be a hard error otherwise).
4. Runs `systemd/check-consistency.sh` as a self-verify. **An install that
   leaves drift behind is not an install** — including drift it did not
   create. If a pre-existing violation is on the host (a copied unit, say),
   the install links its own units and then fails on the check. Clear the
   violation first; see the drift response below.

Refusals, both by design — resolve by hand, then re-run:

- a **regular file** at a destination is never overwritten;
- a symlink pointing somewhere other than this checkout is never replaced.

The repository currently tracks zero units, so a fresh install prints
"no units tracked — nothing to install" and exits 0. That is the expected
state: the scaffolding is preventive. The next unit anyone installs gets this
lifecycle for free by being committed under `systemd/`.

## Verifying a host

```bash
systemd/check-consistency.sh              # full check: repo side + host scan
systemd/check-consistency.sh --repo-only  # CI / extraction form: no host scan
```

Exit codes: `0` consistent, `1` drift (every violation printed with its exact
remediation), `2` usage error. Overrides for tests and unusual layouts:
`ICG_SYSTEMD_DIR`, `ICG_HOST_UNIT_DIR`, `ICG_REPO_ROOT`.

Run the full check from the checkout the host's symlinks point into — usually
the only checkout on the machine. The three drift directions it reports:

| Direction | Meaning | Remedy |
| --- | --- | --- |
| repo → disk | a tracked unit references an `Exec*`/`WorkingDirectory` path missing from the working tree | restore the path, or delete the unit **in the same commit** as the script it ran (and say in the commit message that hosts need `systemd/uninstall.sh`) |
| host → repo | an installed unit executes — or dangles as a symlink into — a repo path that no longer exists; a disabled unit counts, it is one `enable` away from the same `203/EXEC` | the exact `systemctl --user disable --now` / `rm` / `daemon-reload` / `reset-failed` commands are printed; run them |
| tracked → host | something other than install.sh's symlink sits at a tracked unit's destination: a copy, or a link into another checkout | remove it by hand (the check prints how), then re-run `systemd/install.sh` |

CI cannot run the host half (no `~/.config/systemd/user` on a runner), so CI
exercises the check against fixtures (`tests/systemd_consistency_tests.rs`,
`tests/systemd_lifecycle_tests.rs`) and runs `--repo-only`. The
[definition of done](../../scripts/definition-of-done.sh) runs the **full**
check on a real host — that is where actual drift gets caught. If the tree
being verified cannot be the one the host linked (a `git archive` extraction,
such as NEEDLE's close gate), the DoD runs `--repo-only` and prints a note
saying why.

## Drift response

Something flagged — the gate is red, or a unit misbehaved in the journal:

1. Run the full check from the linked checkout and read the direction of every
   violation. The remediation lines are the procedure; run them as printed.
2. If the flagged unit is a **regular file you did not create** (a copy from
   before this scaffolding — `icg-frontier-consistency.service` on codinghome
   is the known instance), the repo will not delete what it did not link.
   Confirm it is not wanted (`systemctl --user is-enabled <name>` /
   `is-active <name>`), then remove it with the commands the check prints.
3. Re-run `systemd/install.sh` if the tracked unit should be installed here,
   then `systemctl --user daemon-reload`.
4. Re-run the full check until it exits 0.

Stop conditions:

- A flagged unit turns out to be load-bearing (active, doing real work): do
  **not** delete it. Restore the repo path it executes, or bring the unit
  under management — copy it into `systemd/`, delete the host copy, run
  `install.sh` — and re-check.
- A violation's remediation would touch a file that is not this repo's:
  stop; the check is repo-scoped, so that means the environment is not what
  the check assumed. Resolve the environment, not the message.

## Retiring units, decommissioning a host

Retiring one unit (or the script it ran):

1. Delete the tracked unit — and its script, if that is being retired — **in
   the same commit**, and say in the commit message that hosts need
   `systemd/uninstall.sh` run. The repo-side check blocks a commit that
   deletes the script and leaves the unit pointing at it.
2. After pulling that commit on each host: `systemd/uninstall.sh --dry-run`
   to preview, then `systemd/uninstall.sh`. It stops, disables and removes
   exactly this repo's symlinks — including orphaned ones whose tracked unit
   the commit already deleted — and never touches a host file that is not one
   of them.
3. Run the full check on the host. Expect exit 0.

Decommissioning the repo on a host entirely is the same procedure with every
unit: dry-run, uninstall, daemon-reload (uninstall does it), full check.

## Where this is enforced

- **CI** (`icg-ci` release gate, `cargo test`): the check's semantics against
  fixtures — all three directions, plus the install/uninstall round trip.
- **Definition of done** (`scripts/definition-of-done.sh`): the script runs
  directly — repo side always, host-side scan whenever the tree is the
  checkout the host linked, `--repo-only` with a note in an extraction.
- A red gate is a finding, not a flake: fix what it names. Never `--repo-only`
  your way past a host violation on a real host.
