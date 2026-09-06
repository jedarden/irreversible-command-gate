# Release-cutting runbook

This is the release gate for `irreversible-command-gate`.

**Read this first: the release is cut by CI, not by hand.** The `icg-ci`
WorkflowTemplate's `build-and-release` step reads the version out of
`Cargo.toml`, and if no *published* release carries that tag it runs the
Layer 1 gates, tags Forgejo, builds the artifacts, and calls
`gh release create` itself. Both v0.1.1 and v0.1.2 were published that way —
their notes carry the template's "Built from commit:" preamble.

So the act that cuts a release is **merging a version bump to `main`**, and
the review below has to happen *before* that push, not after it. An earlier
revision of this runbook described an operator running `gh release create`
manually after review; that has not been how it works since the template
gained its release step, and following it would have you either duplicating
a release CI already made or waiting for an approval step that does not
exist.

Two consequences worth stating plainly:

- **The version bump is the release trigger.** Do not bump `Cargo.toml`
  "to be ready" and push it — that publishes.
- **CI clones the tip of Forgejo `main`, not the commit that triggered the
  run.** A run that starts before your push and reaches `build-and-release`
  after it will build your commit. Do not push a bump while a run is in
  flight if you need the released commit to be the reviewed one.

The steps below remain the review that must precede that push, plus the
manual fallback for the case where CI produced artifacts but the release was
not created.

## Release gate

Before publishing, record the candidate's full commit SHA and confirm all of
the following:

1. The `icg-ci` run passed on that exact commit. The run must include all
   required Layer 1 gates:
   - the fixed deny-regression suite passed; and
   - the `coverage-diff/v1` report was produced by that run. Any removed or
     newly disabled guarded pattern, widened safe pattern, or narrowed destructive guarded
     pattern has either been eliminated or has an explicit, reviewed
     justification. If the release contains `overrides/<repo>.toml`, the
     report must also include the override diff: every newly exempted rule ID
     has an explicit, reviewed justification, and each override passes its
     expiry and 90-day re-justification checks.
2. Layer 2 review is complete against that same `coverage-diff/v1` report.
   The review record includes the candidate SHA, CI run or report artifact,
   reviewer identity and time, decision, and a disposition for every finding.
   A second, adversarial pass is recorded when the report contains findings;
   unexplained findings are a release blocker.
3. The release tag is new and the candidate is still the commit that was
   reviewed. Do not release a newer or different commit merely because it is
   currently at the branch tip.

Layers 1, 2, and the minimal Layer 4 trust pointer are the complete Phase 0
gate. There is no additional approval-workflow layer. If any required
evidence is missing or is bound to a different commit, stop without creating
the release.

## Procedure — normal path

1. Complete the release gate above against the commit currently on `main`.
2. Bump `version` in `Cargo.toml`. Update every doc that cites the release
   tag or the `icg --version` banner; `install_docs_cite_the_current_release_version`
   fails the build if you miss one.
3. Run the gates locally before pushing — these are the same commands
   `build-and-release` runs, and a failure here is a failure there:

   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   cargo run --quiet -- build-pack --pack-dir packs --output /tmp/current-merged.json
   cargo run --quiet -- pack-manifest --pack-dir packs --output /tmp/current-manifest.json
   gh release download "$PREVIOUS_TAG" --repo jedarden/irreversible-command-gate \
     --pattern rule-pack.json --dir /tmp --clobber
   cargo run --quiet -- regression-suite packs --release-gate --output /tmp/suite.json
   cargo run --quiet -- coverage-diff /tmp/rule-pack.json /tmp/current-merged.json
   cargo run --quiet -- redos-check /tmp/current-merged.json --timeout-ms 100
   ```

4. Commit and push to Forgejo `main`. CI does the rest.
5. Verify the published release (step 4 of the manual procedure below), then
   replace the template's generic notes with real ones:

   ```bash
   gh release edit "$TAG" --repo jedarden/irreversible-command-gate \
     --notes-file /path/to/release-notes.md
   ```

   The template cannot know what changed, so its notes are a placeholder.
   Editing them is part of cutting the release, not an optional extra.
6. Advance the Layer 4 trust pointer, as in step 5 below.

## Procedure — manual fallback

Use this only when CI produced verified artifacts but did not create the
release (for example, the workflow failed after the gates and before
`gh release create`). Set these values from the verified `icg-ci` run and
review record:

```bash
REPO=jedarden/irreversible-command-gate
TAG=vX.Y.Z
CANDIDATE="<full-reviewed-commit-sha>"
RELEASE_NOTES_FILE="/path/to/release-notes.md"
```

1. Confirm that the local commit object exists and that GitHub CLI is
   authenticated for the intended repository:

   ```bash
   git cat-file -e "${CANDIDATE}^{commit}"
   gh auth status
   gh repo view "$REPO"
   test "$(gh api "repos/$REPO/commits/$CANDIDATE" --jq '.sha')" = "$CANDIDATE"
   ```

   Re-check the `icg-ci` result and Layer 2 record after setting `CANDIDATE`.
   The SHA in both records must equal `$CANDIDATE` exactly.

2. Confirm that the tag or release does not already exist. An existing tag is
   not to be moved or reused for a different commit:

   ```bash
   if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
     echo "release already exists: $TAG" >&2
     exit 1
   fi
   if gh api "repos/$REPO/git/ref/tags/$TAG" >/dev/null 2>&1; then
     echo "tag already exists: $TAG" >&2
     exit 1
   fi
   ```

3. Publish the release, targeting the reviewed commit explicitly. Attach only
   the artifacts produced by the passing `icg-ci` run:

   ```bash
   gh release create "$TAG" \
     --repo "$REPO" \
     --target "$CANDIDATE" \
     --title "$TAG" \
     --notes-file "$RELEASE_NOTES_FILE" \
     <artifacts-from-the-verified-icg-ci-run>
   ```

   Replace the final placeholder with the actual verified artifact paths
   before running the command. Do not run this command if the CI result,
   review record, or target SHA is uncertain.

4. Verify the published release before advancing trust:

   ```bash
   gh release view "$TAG" --repo "$REPO" \
     --json tagName,targetCommitish,isDraft,isPrerelease,url,assets
   ```

   Confirm that the tag resolves to `$CANDIDATE`, the release is published
   with the intended prerelease status, and every attached asset came from
   the verified CI run. If any value is wrong, do not move the tag. Follow
   the rollback procedure for the trust pointer instead.

5. Advance the Layer 4 trust pointer through its configured mechanism to this
   release only after the verification above succeeds. The pointer must name
   this release or its exact commit; it must not be replaced with a bare
   `latest` reference. Record the pointer update with the same release
   evidence.

The release URL, tag, target SHA, CI run or artifact URL, Layer 2 review
record, and trust-pointer update are the release record. Keep them together
so a later operator can establish exactly what was reviewed and published.
