# Argo Workflow PATH-Wrapper Integration

This guide explains how to integrate the irreversible-command-gate (icg) PATH-wrapper into Argo Workflow pods to guard CI/CD pipelines against destructive operations.

## Overview

The PATH-wrapper integration extends icg coverage from interactive Claude Code/Codex sessions to Argo Workflow pods, protecting CI/CD pipelines including the icg project's own release pipeline.

## Architecture

### Components

1. **Guarded Builder Image** (`ronaldraygun/argo-guarded-builder:0.1.0`)
   - Extends `ronaldraygun/needle-ci-builder:with-deps`
   - Pre-installs icg binary and PATH-wrapper symlinks
   - Includes default rule pack at `/etc/icg/rule-pack.json`

2. **PATH-Wrapper Mechanism**
   - Symlinks common tools (`git`, `kubectl`, `vault`, `docker`, etc.) to icg binary
   - Intercepts commands before execution
   - Checks against rule packs for allow/deny decisions
   - Passes allowed commands to real binaries

3. **Rule Packs**
   - Define guarded patterns and safe patterns
   - Can be baked into image or mounted at runtime
   - Support repository-specific overrides

## Deployment

### Step 1: Build the Guarded Builder Image

```bash
cd /home/coding/irreversible-command-gate/containers/argo-guarded-builder

# Build with pinned version
VERSION=$(cat VERSION)
docker build -t ronaldraygun/argo-guarded-builder:${VERSION} .

# Push to registry
docker push ronaldraygun/argo-guarded-builder:${VERSION}
```

### Step 2: Deploy Workflow Template

```bash
# Apply the workflow template to iad-ci
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  apply -f containers/argo-guarded-builder/icg-guarded-ci-workflowtemplate.yml \
  -n argo-workflows
```

### Step 3: Update Existing Workflows

Modify existing workflow templates to use the guarded image:

```yaml
# Before
image: debian:bookworm

# After
image: ronaldraygun/argo-guarded-builder:0.1.0
```

## Usage

### Running Guarded Workflows

```bash
# Submit a guarded workflow
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  create -f - <<EOF
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: icg-guarded-
  namespace: argo-workflows
spec:
  workflowTemplateRef:
    name: icg-guarded-ci
  arguments:
    parameters:
      - name: repo
        value: https://github.com/jedarden/myproject.git
      - name: revision
        value: main
EOF
```

### Testing Denials

```bash
# In a running workflow pod, test a denied operation
kubectl exec -it <pod-name> -- git push --force
# Expected: command denied: force-push not allowed without prior pull [pack=git, pattern=force-push-without-pull]
```

### Monitoring Coverage

```bash
# Check icg status in a running pod
kubectl exec -it <pod-name> -- icg status

# View denial log
kubectl exec -it <pod-name> -- cat /var/cache/icg/denials.jsonl
```

## Rule Pack Management

### Default Rule Pack

The builder image includes a default rule pack at build time. To update it:

```bash
# Download latest rule pack
curl -fsSL "https://github.com/jedarden/irreversible-command-gate/releases/latest/download/rule-pack.json" \
  -o /tmp/rule-pack.json

# Rebuild image with updated pack
docker build --build-arg RULE_PACK_PATH=/tmp/rule-pack.json \
  -t ronaldraygun/argo-guarded-builder:0.1.0 .
```

### Custom Rule Packs

For project-specific rules, mount a custom ConfigMap:

```yaml
spec:
  templates:
    - name: build-with-custom-rules
      container:
        image: ronaldraygun/argo-guarded-builder:0.1.0
        volumeMounts:
          - name: custom-rules
            mountPath: /etc/icg/rule-pack.json
            subPath: rule-pack.json
        env:
          - name: ICG_RULE_PACK
            value: /etc/icg/rule-pack.json
  volumes:
    - name: custom-rules
      configMap:
        name: project-icg-rules
```

### Repository Overrides

For repository-specific exceptions:

```yaml
spec:
  templates:
    - name: build-with-override
      container:
        image: ronaldraygun/argo-guarded-builder:0.1.0
        volumeMounts:
          - name: repo-override
            mountPath: /etc/icg/overrides/repo-name.toml
            subPath: override.toml
  volumes:
    - name: repo-override
      configMap:
        name: repo-icg-override
```

## Self-Referential Protection

The icg project's own release pipeline (`icg-ci`) should use this guarded image:

1. **Update `icg-ci-workflowtemplate.yml`** to use `ronaldraygun/argo-guarded-builder:0.1.0`
2. **Verify the workflow can still build and release icg**
3. **Test that destructive operations in the release workflow are properly denied**

This ensures the guard protecting everything else is itself protected.

## Troubleshooting

### Commands Not Being Intercepted

**Symptoms**: Dangerous commands execute without being checked

**Checks**:
1. Verify symlinks exist: `ls -la /usr/local/bin/ | grep icg`
2. Check PATH order: `echo $PATH` (should have `/usr/local/bin` first)
3. Test wrapper directly: `icg check --command "git push --force"`

**Fix**: Reinstall symlinks or adjust PATH in workflow template

### Rule Pack Not Loading

**Symptoms**: All commands are allowed (fail-open)

**Checks**:
1. Verify rule pack exists: `ls -la /etc/icg/`
2. Test load: `icg coverage --pack /etc/icg/rule-pack.json`
3. Check permissions: `stat /etc/icg/rule-pack.json`

**Fix**: Ensure rule pack is mounted or copied into the container

### Release Workflow Updates

**Symptoms**: New icg version not available in workflows

**Steps**:
1. Increment `VERSION` file in `containers/argo-guarded-builder/`
2. Rebuild and push new image: `docker build -t ronaldraygun/argo-guarded-builder:0.2.0 .`
3. Update workflow templates to reference new tag
4. Test with non-critical workflow first
5. Roll out updates gradually

## Security Considerations

### Rule Pack Integrity

1. **Versioning**: Rule packs should be versioned and tagged in releases
2. **Signing**: Production rule packs should be cryptographically signed
3. **Validation**: Workflow should validate rule pack checksums before use

### Fail-Open Behavior

icg fails-open if the rule pack is missing or corrupted. This is intentional:

- A missing rule pack shouldn't block all CI/CD
- Errors are logged to `/var/cache/icg/denials.jsonl`
- Monitoring should alert on missing rule packs

### Audit Trail

Denied commands are logged with full context:

```json
{
  "timestamp": "2026-08-20T12:34:56Z",
  "id": "denial-abc123",
  "pack_id": "git",
  "pattern_id": "force-push-without-pull",
  "severity": "critical",
  "reason": "force-push not allowed without prior pull",
  "context": {
    "tool": "git",
    "command": "git push --force",
    "user": "argo-workflow",
    "pod": "icg-guarded-abc123"
  }
}
```

Logs are persisted to `/var/cache/icg/denials.jsonl` and can be exported for incident review.

## Rollout Strategy

1. **Phase 1**: Deploy to low-risk workflows (testing, feature branches)
2. **Phase 2**: Deploy to medium-risk workflows (integration, staging)
3. **Phase 3**: Deploy to high-risk workflows (production releases)
4. **Phase 4**: Enable for icg's own release pipeline (self-referential)

## Monitoring

### Key Metrics

1. **Denial Rate**: Percentage of commands denied vs. allowed
2. **Rule Pack Version**: Which rule pack is active
3. **Coverage**: Which tools are being guarded
4. **Alerts**: Critical severity denials should trigger alerts

### Health Checks

The builder image includes a health check:

```yaml
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD icg --version || exit 1
```

This ensures icg is functional before the workflow starts.

## Related Documentation

- [Main README](../../../README.md)
- [Operator Training Manual](../../../docs/operators/training-manual.md)
- [Rule Pack Best Practices](../../../docs/developers/rule-pack-best-practices.md)
- [Container README](containers/argo-guarded-builder/README.md)
