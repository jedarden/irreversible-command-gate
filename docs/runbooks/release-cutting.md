# Release-cutting runbook

This is the human release gate for `irreversible-command-gate`. A push to
`main` may run CI and build candidate artifacts, but it must not publish a
trusted release automatically. The operator runs `gh release create` only
after the checks below are complete.

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

## Procedure

Set these values from the verified `icg-ci` run and review record:

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
