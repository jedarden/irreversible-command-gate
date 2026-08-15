# Per-repository overrides

An override is a release artifact, not a local disable switch. A repository
may carry one file at `overrides/<repo>.toml`, but a host honors it only after
all of these checks pass:

- the file declares `schema = "icg-override/v1"` and the exact repository;
- every exempted ID is a unique guarded `deny` rule in the loaded release;
- `release_ref` exactly matches the host's trusted release reference (a mutable
  `latest` reference is rejected);
- `expires_at` is a future ISO date; and
- `last_justified_at` is no more than 90 days old and `justification` is
  non-blank.

The 90-day freshness check is independent of expiry. A long expiry cannot
turn an exception into permanent Swiss-cheese coverage: the override must be
re-justified in a reviewed release before the cadence elapses. Expired or
stale overrides are rejected and the normal rule remains enforced.

## File format

```toml
schema = "icg-override/v1"
repository = "jedarden/example"
release_ref = "v1.2.3"
exempted_rule_ids = ["git-force-push"]
expires_at = "2026-11-15"
last_justified_at = "2026-08-15"
justification = "The repository's migration job uses the guarded operation under its own lock."
```

The release reference is the release's existing trust binding; there is no
override-specific private key. This matches the project's release-integrity
decision: the commit, Layer 1 checks, Layer 2 review, human release, and
Layer 4 trust pointer together are the signature. A file copied into a local
checkout or edited after release is not a valid bypass because it cannot prove
the trusted release reference and rule set that authorized it.

## Release gates

The existing fixed deny suite still contains a case for every guarded rule,
including an exempted rule. The override-aware gate verifies that all
non-exempted cases still deny and that an exempted case is no longer denied by
that exact rule. This preserves Layer 1's behavioral coverage.

The existing `coverage-diff/v1` report also includes a `Newly exempted rule
IDs` section when invoked with the previous and current override files. Every
new exemption is a coverage regression and therefore requires the same
non-blank release justification and Layer 2 review as a removed rule or
narrowed destructive pattern. Removing an exemption is reported separately as
coverage strengthening.

Example commands:

```bash
icg coverage-diff previous-release.json current-release.json \
  --previous-override overrides/example.previous.toml \
  --current-override overrides/example.toml \
  --justification "The migration is isolated and scheduled for removal."

icg regression-suite current-release.json \
  --override-file overrides/example.toml \
  --repository jedarden/example \
  --trusted-ref v1.2.3
```

The hook adapter accepts the same three override inputs (`--override-file`,
`--repository`, and `--trusted-ref`) only as a complete set. Supplying a
partial set is an error; omitting the override inputs leaves normal rule-pack
enforcement unchanged.
