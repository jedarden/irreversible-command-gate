# Shipped releases inventory: git tags and GitHub Releases

Plain enumeration only. This doc is the evidence base for correcting the
stale claim in `docs/plan/plan.md` (~line 457) that "no release has ever
been cut" and that `gh release list` returns empty — both are false as of
the collection dates below. **No edits to `docs/plan/plan.md` are made
here**; downstream plan reconciliation should cite this doc.

Collected: 2026-09-12 (initial pass, through v0.1.57); re-verified and
extended on 2026-09-13 through v0.1.61 from local git tags (after
`git fetch --tags origin`) and `gh release list` /
`gh api repos/jedarden/irreversible-command-gate/releases`. All rows
below were regenerated from the 2026-09-13 sources; the v0.1.0–v0.1.57
rows matched the initial pass exactly.

## The two facts, stated separately

These are distinct claims and downstream edits must cite the right one:

1. **Git tags exist.** The repo carries **62 tags, `v0.1.0` through
   `v0.1.61`**, contiguous with no gaps. All are lightweight tags
   (`git for-each-ref` reports `objecttype: commit` for every one), so
   each tag's "date" is the commit's author/commit date, not a separate
   tag-creation date.
2. **GitHub Releases exist.** `gh release list` on
   `jedarden/irreversible-command-gate` returns **61 published Releases,
   `v0.1.1` through `v0.1.61`**, none draft, none prerelease, all
   targeting `main`. `v0.1.61` is marked **Latest** (published
   2026-09-13T06:56:38Z).

### Set differences

- **Tags without a matching GitHub Release: `v0.1.0` only.** It is the
  sole tag that was never released.
- **Releases without a matching git tag: none.** Every Release points at
  a real tag.

## Enumeration method

- Tags: `git fetch --tags origin`, then
  `git for-each-ref refs/tags --sort=version:refname
  --format='%(refname:short)|%(objecttype)|%(creatordate:iso-strict)|%(objectname:short)'`.
- Releases: `gh release list --repo jedarden/irreversible-command-gate
  --limit 100` (tab-separated; `--json` was not used), cross-referenced
  by exact tag-name set difference. Draft/prerelease status, target
  branch, and assets verified in a single
  `gh api 'repos/.../releases?per_page=100'` sweep over all 61
  (0 drafts, 0 prereleases, `target_commitish` = `main` on all).

## Inventory

Combined table — since the two sets differ only by `v0.1.0`, one row per
version enumerates both the tag and (where present) the Release. Tag
dates are normalized to UTC for sortability; `v0.1.0`–`v0.1.3` were
captured by git with a `-04:00` offset and are converted here (e.g.
`v0.1.1` tagged `2026-09-05T23:36:55-04:00` = `2026-09-06T03:36:55Z`).
Every row from `v0.1.1` down has a published Release.

| Tag | Commit | Tag date (UTC) | Release published (UTC) | Notes |
|---|---|---|---|---|
| v0.1.0 | `f0fe556` | 2026-08-27T21:48:52Z | — | **No GitHub Release** |
| v0.1.1 | `a4f6e0c` | 2026-09-06T03:36:55Z | 2026-09-06T13:06:02Z | |
| v0.1.2 | `1b6f6a6` | 2026-09-06T15:09:19Z | 2026-09-06T15:40:34Z | |
| v0.1.3 | `bb1beb4` | 2026-09-06T20:07:54Z | 2026-09-06T20:24:14Z | |
| v0.1.4 | `e120f73` | 2026-09-08T03:38:29Z | 2026-09-08T03:45:33Z | |
| v0.1.5 | `c70033d` | 2026-09-08T04:27:34Z | 2026-09-08T04:34:15Z | |
| v0.1.6 | `aab687d` | 2026-09-08T04:45:03Z | 2026-09-08T04:51:34Z | |
| v0.1.7 | `d12afdb` | 2026-09-10T22:28:10Z | 2026-09-10T22:35:19Z | |
| v0.1.8 | `65ff86e` | 2026-09-10T23:10:45Z | 2026-09-10T23:17:09Z | |
| v0.1.9 | `586131d` | 2026-09-10T23:28:05Z | 2026-09-10T23:34:36Z | |
| v0.1.10 | `ccb6941` | 2026-09-10T23:45:37Z | 2026-09-10T23:52:02Z | |
| v0.1.11 | `739bbf8` | 2026-09-11T00:14:20Z | 2026-09-11T00:20:47Z | |
| v0.1.12 | `714358d` | 2026-09-11T00:32:01Z | 2026-09-11T00:38:26Z | |
| v0.1.13 | `351ff24` | 2026-09-11T01:13:36Z | 2026-09-11T01:20:00Z | |
| v0.1.14 | `019c765` | 2026-09-11T01:30:42Z | 2026-09-11T01:37:17Z | |
| v0.1.15 | `01109eb` | 2026-09-11T02:05:56Z | 2026-09-11T02:12:38Z | |
| v0.1.16 | `3c9ca30` | 2026-09-11T02:23:45Z | 2026-09-11T02:30:20Z | |
| v0.1.17 | `250e796` | 2026-09-11T02:41:25Z | 2026-09-11T02:47:50Z | |
| v0.1.18 | `3c503b1` | 2026-09-11T03:09:37Z | 2026-09-11T03:16:01Z | |
| v0.1.19 | `bad737a` | 2026-09-11T04:26:46Z | 2026-09-11T04:33:23Z | |
| v0.1.20 | `e32df6e` | 2026-09-11T05:19:37Z | 2026-09-11T05:26:13Z | |
| v0.1.21 | `524fa38` | 2026-09-11T06:30:01Z | 2026-09-11T06:36:37Z | |
| v0.1.22 | `eb0dbc0` | 2026-09-11T06:47:30Z | 2026-09-11T06:54:03Z | |
| v0.1.23 | `4bba29a` | 2026-09-11T07:41:49Z | 2026-09-11T07:48:12Z | |
| v0.1.24 | `ab52d73` | 2026-09-11T07:59:09Z | 2026-09-11T08:05:45Z | |
| v0.1.25 | `aa6b20f` | 2026-09-11T08:16:41Z | 2026-09-11T08:23:17Z | |
| v0.1.26 | `08b2f59` | 2026-09-11T08:34:15Z | 2026-09-11T08:40:52Z | |
| v0.1.27 | `8882a45` | 2026-09-11T09:59:16Z | 2026-09-11T10:05:49Z | |
| v0.1.28 | `1a2fd1c` | 2026-09-11T10:27:47Z | 2026-09-11T10:34:12Z | |
| v0.1.29 | `6fdeb3a` | 2026-09-11T11:56:26Z | 2026-09-11T12:03:04Z | |
| v0.1.30 | `7fe4ab2` | 2026-09-11T12:53:25Z | 2026-09-11T13:00:07Z | |
| v0.1.31 | `dd214f8` | 2026-09-11T13:11:32Z | 2026-09-11T13:18:00Z | |
| v0.1.32 | `f64f9ff` | 2026-09-11T14:17:21Z | 2026-09-11T14:24:13Z | |
| v0.1.33 | `de4091b` | 2026-09-11T14:54:27Z | 2026-09-11T15:01:04Z | |
| v0.1.34 | `7cc53e2` | 2026-09-11T15:57:56Z | 2026-09-11T16:04:45Z | |
| v0.1.35 | `a59047c` | 2026-09-11T16:37:16Z | 2026-09-11T16:43:57Z | |
| v0.1.36 | `59a6866` | 2026-09-11T17:06:42Z | 2026-09-11T17:13:53Z | |
| v0.1.37 | `68cce95` | 2026-09-11T19:42:49Z | 2026-09-11T19:49:12Z | |
| v0.1.38 | `feb24d2` | 2026-09-11T20:13:23Z | 2026-09-11T20:19:54Z | |
| v0.1.39 | `694e14c` | 2026-09-11T20:42:50Z | 2026-09-11T20:49:25Z | |
| v0.1.40 | `a36a4de` | 2026-09-11T21:00:35Z | 2026-09-11T21:08:06Z | |
| v0.1.41 | `e9b5a45` | 2026-09-11T21:31:58Z | 2026-09-11T21:40:04Z | |
| v0.1.42 | `9db1323` | 2026-09-11T22:17:27Z | 2026-09-11T22:24:08Z | |
| v0.1.43 | `827345d` | 2026-09-11T22:52:23Z | 2026-09-11T22:59:05Z | |
| v0.1.44 | `5ea9e00` | 2026-09-11T23:21:45Z | 2026-09-11T23:28:43Z | |
| v0.1.45 | `3031bcb` | 2026-09-11T23:56:34Z | 2026-09-12T00:03:17Z | |
| v0.1.46 | `840cc32` | 2026-09-12T00:49:07Z | 2026-09-12T00:56:04Z | |
| v0.1.47 | `1842c5f` | 2026-09-12T01:26:10Z | 2026-09-12T01:33:35Z | |
| v0.1.48 | `b374117` | 2026-09-12T02:10:43Z | 2026-09-12T02:17:10Z | |
| v0.1.49 | `e6757a1` | 2026-09-12T03:48:24Z | 2026-09-12T03:55:07Z | |
| v0.1.50 | `f090254` | 2026-09-12T04:44:18Z | 2026-09-12T04:50:45Z | |
| v0.1.51 | `8d39972` | 2026-09-12T05:46:14Z | 2026-09-12T05:52:58Z | |
| v0.1.52 | `c2ac9dd` | 2026-09-12T06:43:24Z | 2026-09-12T06:50:06Z | |
| v0.1.53 | `4095c6e` | 2026-09-12T08:56:17Z | 2026-09-12T09:03:01Z | |
| v0.1.54 | `cc38a75` | 2026-09-12T09:26:04Z | 2026-09-12T09:32:32Z | |
| v0.1.55 | `19e8b4e` | 2026-09-12T10:14:49Z | 2026-09-12T10:21:16Z | |
| v0.1.56 | `ee6a8f9` | 2026-09-12T13:03:30Z | 2026-09-12T13:11:41Z | |
| v0.1.57 | `09084d6` | 2026-09-12T14:38:07Z | 2026-09-12T14:46:28Z | |
| v0.1.58 | `9d15d22` | 2026-09-12T16:28:45Z | 2026-09-12T16:36:00Z | |
| v0.1.59 | `213d384` | 2026-09-12T16:51:40Z | 2026-09-12T16:58:45Z | |
| v0.1.60 | `5d88357` | 2026-09-13T05:27:42Z | 2026-09-13T05:36:08Z | |
| v0.1.61 | `566256b` | 2026-09-13T06:49:08Z | 2026-09-13T06:56:38Z | **Latest** |

## Artifact evidence

Releases carry real assets, not just a tag echo. Spot-check on
`v0.1.61`: `icg` (the binary), `icg-packs.tar.gz`, `pack-manifest.json`,
`rule-pack.json` — the same four assets verified on `v0.1.57` in the
initial pass. Release title format is `irreversible-command-gate
vN.N.N`; `target_commitish` is `main` on all 61.

## Observations for the plan reconciliation (factual, not analysis)

- Every Release was published **minutes after** its tag (typically
  5–8 min), consistent with an automated tag→release pipeline; the
  dense ~15–60 min cadence from `v0.1.7` (2026-09-10) onward matches
  `icg-ci` running `build-and-release` per merge, and continued
  uninterrupted through v0.1.61 on 2026-09-13 (the release commits
  themselves — `5d88357`, `566256b` — are the tagged commits).
- The correct replacement for plan.md's "no release has ever been cut"
  (line ~457) is: **61 GitHub Releases exist (v0.1.1–v0.1.61); only the
  very first tag, v0.1.0, lacks a Release.**
