# Override approval runbook

Use this runbook for a legitimate, repository-specific exception to a guarded
deny rule. An override is a release-bound artifact, not a local disable switch.
It is active only when its exact `release_ref` matches the host's trusted
reference, the exempted IDs exist as deny rules in that release, and its
expiration and freshness checks pass.

For an immediate service emergency, use the [emergency bypass
runbook](emergency-bypass.md). Do not turn an incident bypass into an override
after the fact without completing this review.

## Eligibility and required evidence

Approve an override only when:

- the exception is limited to one named repository and one or more specific
  guarded rule IDs;
- the requester has documented why the operation is legitimate and why a safe
  redirect, workflow change, or rule-pack fix cannot address it;
- the risk, compensating controls, owner, migration/removal plan, and expiry
  are recorded;
- `expires_at` is a future ISO date and the exception will be re-justified at
  least every 90 days; and
- the requester and reviewer have enough repository and release context to
  reproduce the decision.

Wildcards, blanket disablement, mutable release references, and exemptions for
unknown or non-deny rules are not eligible.

## Step 1: Create a request

The request command records intent but does not change enforcement:

```bash
icg override create \
  --repo jedarden/example \
  --pattern-id image-tag-bare-sha \
  --justification "The migration job consumes immutable SHA references from the audited build system; it cannot yet use the enforced form. Owner: team@example. Removal target: <date>." \
  --output /tmp/override-request-example.json
```

Capture the request file, repository path/identifier, requested rule, owner,
compensating control, and proposed expiry in the review record. The request is
not an active bypass; do not install it under `/etc/icg/overrides`.

## Step 2: Layer 1 validation

The release author or CI validates the request against the candidate pack:

1. Confirm the repository scope is exact and the rule ID is a unique guarded
   `deny` rule in the candidate release.
2. Confirm the normal fixed deny-regression suite still covers the rule. The
   exempted case may be allowed only for the exact repository; the same command
   outside that repository must remain denied.
3. Check the expiry and `last_justified_at` dates, non-blank justification, and
   migration plan.
4. Add or update `overrides/<repo>.toml` in the candidate release. Its schema
   must be:

   ```toml
   schema = "icg-override/v1"
   repository = "jedarden/example"
   release_ref = "vX.Y.Z"
   exempted_rule_ids = ["image-tag-bare-sha"]
   expires_at = "2026-12-31"
   last_justified_at = "2026-08-17"
   justification = "<reviewed justification>"
   ```

5. Run the override-aware coverage and regression checks against the previous
   and current releases:

   ```bash
   icg coverage-diff previous-release.json current-release.json \
     --previous-override overrides/example.previous.toml \
     --current-override overrides/example.toml \
     --justification "<why this new exemption is necessary>"
   icg regression-suite current-release.json \
     --override-file overrides/example.toml \
     --repository jedarden/example \
     --trusted-ref vX.Y.Z
   ```

Every newly exempted rule ID appears as a coverage regression. Layer 1 must
show that all other guarded cases still deny and that the exemption is scoped
to the named repository. A newly exempted rule without a reviewed
justification blocks the release.

## Step 3: Layer 2 approval

An independent reviewer examines the request and the exact Layer 1 report.
Record the candidate SHA/release reference, CI run and report artifact,
requester, reviewer, UTC time, decision, and a disposition for every finding.
The reviewer should challenge the necessity, scope, compensating controls,
expiry, and removal plan. When the report has findings, record a second
adversarial pass. The requester cannot self-approve a high-risk exception.

Approve only when the reviewer can answer yes to all of these:

- Is the exception narrower than changing the rule globally?
- Does the repository actually need it, with evidence?
- Does the exact release still protect the operation everywhere else?
- Is the expiry and 90-day re-justification owner clear?
- Is the trusted release reference immutable and the evidence SHA-matched?

An approval record without an exact release reference is non-activatable. It
documents review but must not create an active bypass artifact.

## Step 4: Produce and ship the artifact

After Layer 2 approval and once the exact trusted reference is known, produce
the release-bound artifact. The CLI can validate the requested rule when given
the candidate pack:

```bash
icg override approve \
  --request /tmp/override-request-example.json \
  --approver security-team-lead \
  --expiration 2026-12-31 \
  --release-ref vX.Y.Z \
  --pack current-release.json \
  --output overrides/example.toml
```

Review the generated TOML, then commit it with the same release candidate. Do
not copy an approved file into a host by hand. The normal release process
publishes the pack and override together, and the host accepts the override
only after its trust pointer and loaded pack prove the same release binding.

After deployment, verify both the active exception and its boundary:

```bash
icg override list
cd /path/to/example
icg check --file deployment.yaml
cd /tmp
icg check --file /path/to/example/deployment.yaml
```

The repository-scoped check may be allowed by the approved rule; the same
content outside the repository must remain denied. Preserve the deployment
reference and verification output with the approval record.

## Renewal, revocation, and expiry

Review active and aging exceptions regularly:

```bash
icg override list --include-expired
```

Before 90 days, re-justify the exception in a new reviewed release, even if
`expires_at` is later. If the migration is complete, remove the exemption in a
new release. If the exception expires or becomes stale, the normal deny rule
must resume; do not extend it by editing the installed file. A changed rule,
repository scope, trusted reference, or expiry requires the Layer 1/Layer 2
process again.
