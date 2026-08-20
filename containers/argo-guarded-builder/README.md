# Argo Workflow Guarded Builder

This directory contains the Dockerfile and configuration for building an Argo Workflow executor image with irreversible-command-gate (icg) PATH-wrapper protection.

## Purpose

Extends the standard `ronaldraygun/needle-ci-builder:with-deps` image with icg PATH-wrapper integration to guard commands in CI/CD pipelines. This prevents destructive operations in Argo Workflow pods while allowing normal development workflows.

## What This Does

The PATH-wrapper intercepts commands before they execute by shadowing them with symlinks to the icg binary:

1. **Installation**: Symlinks common tools (`git`, `kubectl`, `vault`, `docker`, etc.) to `/usr/local/bin/icg`
2. **Interception**: When these commands are invoked, icg checks them against rule packs
3. **Decision**: Commands are either allowed, denied, or rewritten with safe alternatives
4. **Execution**: Allowed commands are passed through to the real binary

## Build

```bash
# Build the image
cd containers/argo-guarded-builder
docker build -t ronaldraygun/argo-guarded-builder:latest .

# Push to registry
docker push ronaldraygun/argo-guarded-builder:latest
```

## Usage in Argo Workflows

Replace the base image in workflow templates:

```yaml
spec:
  templates:
    - name: build-with-guard
      container:
        image: ronaldraygun/argo-guarded-builder:latest
        command: [bash, -c]
        args:
          - |
            # Normal commands are now guarded
            git clone https://github.com/user/repo.git
            cd repo
            cargo build --release
            
            # Dangerous operations are blocked
            # git push --force  # DENIED by icg
            # vault kv delete secret/...  # DENIED by icg
```

## Rule Pack Management

Rule packs define which commands are guarded and the denial messages. They can be:

1. **Baked into the image**: Included at build time in `/etc/icg/rule-pack.json`
2. **Mounted at runtime**: Via ConfigMaps or Secrets in Argo Workflows
3. **Downloaded dynamically**: Via `icg update` during workflow execution

### Example: Mount Custom Rule Pack

```yaml
spec:
  templates:
    - name: build-with-custom-rules
      container:
        image: ronaldraygun/argo-guarded-builder:latest
        volumeMounts:
          - name: rule-pack
            mountPath: /etc/icg/rule-pack.json
            subPath: rule-pack.json
        env:
          - name: ICG_RULE_PACK
            value: /etc/icg/rule-pack.json
  volumes:
    - name: rule-pack
      configMap:
        name: icg-rule-pack
```

## Workflow Template Example

See `icg-guarded-ci-workflowtemplate.yml` for a complete example of using this image in Argo Workflows.

## Monitoring

Check icg status in running pods:

```bash
# In a running workflow pod
kubectl exec -it <pod-name> -- icg status
kubectl exec -it <pod-name> -- icg coverage
```

## Troubleshooting

### Commands Not Being Intercepted

1. Verify symlinks exist: `ls -la /usr/local/bin/ | grep icg`
2. Check PATH order: `echo $PATH` (should have `/usr/local/bin` first)
3. Test wrapper: `which git` should point to symlink

### Rule Pack Not Loading

1. Check rule pack exists: `ls -la /etc/icg/`
2. Test load: `icg coverage --pack /etc/icg/rule-pack.json`
3. Check logs: Pod logs will show icg warnings

### Release Workflow Updates

When updating this builder image:

1. Update the `FROM` base image version
2. Rebuild and push with new tag
3. Update workflow templates to use new image tag
4. Test with a non-critical workflow first

## Security Considerations

1. **Rule Pack Integrity**: Rule packs should be versioned and signed in production
2. **Fallback Behavior**: icg fails-open if rule pack is missing (allows all commands)
3. **Audit Trail**: Denied commands are logged to `/var/cache/icg/denials.jsonl`
4. **Runtime Overrides**: Repository-specific overrides can be mounted for exceptions

## Related Documentation

- [Main README](../../../README.md)
- [Operator Training Manual](../../../docs/operators/training-manual.md)
- [Rule Pack Best Practices](../../../docs/developers/rule-pack-best-practices.md)
