# Quick Reference: Argo Workflow Guard Integration

## Where the Workflow Templates Live

The WorkflowTemplates that run this repo's CI (`icg-ci`, the push-triggered
CI gate; `icg-guarded-builder`, the image bootstrap) have **one source of
truth**: `jedarden/declarative-config`, at `k8s/iad-ci/argo-workflows/`
(`icg-ci-workflowtemplate.yml`, `icg-guarded-builder-workflowtemplate.yml`).
ArgoCD (application `argo-workflows-ns-iad-ci`, cluster `iad-ci`) syncs them
into the cluster; editing the live objects with `kubectl` fights `selfHeal`
and is reverted. To change a template, edit the manifest in
declarative-config, commit, and push.

This repository carries **no** template copy. Earlier copies under
`containers/argo-guarded-builder/` — two WorkflowTemplate files that were
deployed nowhere — were deleted: they read as authoritative, and editing
them changed nothing (irrevers-fc96ecad).

## For Operators: Deploying icg Guarded Workflows

### 5-Minute Setup

```bash
# 1. Build the guarded builder image (run from irreversible-command-gate repo)
cd /home/coding/irreversible-command-gate
VERSION=$(cat containers/argo-guarded-builder/VERSION)
docker build \
  --build-arg ICG_VERSION=${VERSION} \
  -f containers/argo-guarded-builder/Dockerfile \
  -t ronaldraygun/argo-guarded-builder:${VERSION} \
  .
docker push ronaldraygun/argo-guarded-builder:${VERSION}

# 2. The workflow template: no deploy step here. It lives in
#    jedarden/declarative-config (k8s/iad-ci/argo-workflows/) and ArgoCD
#    syncs it to iad-ci. To change it, edit the manifest there, commit,
#    and push -- never apply template YAML from this repo; it ships none.

# 3. Test with a sample workflow (icg-guarded-builder builds this repo's
#    guarded image from its default parameters; icg-ci is the
#    push-triggered CI template)
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  create -f - <<EOF
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: test-icg-guarded-builder-
  namespace: argo-workflows
spec:
  workflowTemplateRef:
    name: icg-guarded-builder
EOF
```

### Updating Existing Workflows

Find and replace base images:

```bash
# List workflows using old builder images
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  get workflowtemplates -n argo-workflows \
  -o json | jq '.items[] | select(.spec.templates[].container.image | contains("debian:bookworm")) | .metadata.name'

# Update each one in its declarative-config manifest
# (jedarden/declarative-config, k8s/iad-ci/argo-workflows/), commit, push,
# and let ArgoCD sync application argo-workflows-ns-iad-ci. Do not edit or
# patch the live objects -- selfHeal reverts them.
```

Replace:
```yaml
image: debian:bookworm
```

With:
```yaml
image: ronaldraygun/argo-guarded-builder:0.1.1
imagePullPolicy: IfNotPresent
```

### Checking icg Status in Running Pods

```bash
# Get pod name
POD=$(kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  get pods -n argo-workflows -l workflows.argoproj.io/workflow=<workflow-name> \
  -o jsonpath='{.items[0].metadata.name}')

# Check icg status
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  exec -it $POD -n argo-workflows -- icg status

# View denial log
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  exec -it $POD -n argo-workflows -- cat /var/cache/icg/denials.jsonl
```

### Troubleshooting

**Problem**: Commands not being intercepted

```bash
# Verify symlinks exist
kubectl exec -it $POD -- ls -la /usr/local/bin/ | grep icg

# Check PATH order
kubectl exec -it $POD -- echo $PATH

# Test wrapper directly
kubectl exec -it $POD -- icg check --command "git push --force"
```

**Problem**: All commands allowed (rule pack missing)

```bash
# Check if rule pack exists
kubectl exec -it $POD -- ls -la /etc/icg/

# Test rule pack load
kubectl exec -it $POD -- icg coverage --pack /etc/icg/packs
```

## For Developers: Adding icg to Your Workflow

### Minimal Integration

```yaml
apiVersion: argoproj.io/v1alpha1
kind: WorkflowTemplate
metadata:
  name: my-project-ci
  namespace: argo-workflows
spec:
  templates:
    - name: build
      container:
        # Use guarded builder instead of plain Debian
        image: ronaldraygun/argo-guarded-builder:0.1.1
        command: [bash, -c]
        args:
          - |
            # Normal operations work as before
            git clone https://github.com/user/repo.git
            cd repo
            cargo build --release

            # Dangerous operations are now blocked
            # git push --force  # DENIED!
```

### With Custom Rule Pack

```yaml
spec:
  templates:
    - name: build
      container:
        image: ronaldraygun/argo-guarded-builder:0.1.1
        volumeMounts:
          - name: custom-rules
            mountPath: /etc/icg/packs/runtime.json
            subPath: rule-pack.json
        env:
          - name: ICG_RULE_PACK
            value: /etc/icg/packs/runtime.json
  volumes:
    - name: custom-rules
      configMap:
        name: my-project-icg-rules
```

### Testing Your Integration

```bash
# 1. Submit workflow
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  create -f my-workflow.yml

# 2. Watch logs for icg denials
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  logs -f <pod-name> -n argo-workflows | grep "icg"

# 3. If denied, check the denial log
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  exec -it <pod-name> -n argo-workflows -- cat /var/cache/icg/denials.jsonl
```

## Emergency Recovery

If icg is blocking legitimate operations:

1. **Immediate**: Use the real binary directly (bypasses wrapper)
   ```bash
   /usr/bin/git instead of git
   ```

2. **Short-term**: Remove symlinks in the pod
   ```bash
   kubectl exec -it <pod-name> -- rm /usr/local/bin/git
   ```

3. **Long-term**: Fix rule pack and redeploy
   ```bash
   # Update rule pack
   # Rebuild image
   # Update workflow template
   ```

## Maintenance

### Updating the Guarded Builder

```bash
# 1. Increment VERSION
echo "0.2.0" > containers/argo-guarded-builder/VERSION

# 2. Rebuild image
cd /home/coding/irreversible-command-gate
VERSION=$(cat containers/argo-guarded-builder/VERSION)
docker build \
  --build-arg ICG_VERSION=${VERSION} \
  -f containers/argo-guarded-builder/Dockerfile \
  -t ronaldraygun/argo-guarded-builder:${VERSION} \
  .
docker push ronaldraygun/argo-guarded-builder:${VERSION}

# 3. Update workflow templates to use the new version
#    (edit the image tag in jedarden/declarative-config,
#    k8s/iad-ci/argo-workflows/, and let ArgoCD sync)

# 4. Test with low-risk workflow first
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  create -f test-workflow.yml

# 5. Monitor for issues
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  logs -f <pod-name> -n argo-workflows
```

## File Locations

- **Builder Image**: `/home/coding/irreversible-command-gate/containers/argo-guarded-builder/`
- **Workflow Templates (single source of truth)**: `k8s/iad-ci/argo-workflows/` in `jedarden/declarative-config` — this repo ships none
- **Deployment Guide**: `/home/coding/irreversible-command-gate/docs/monitoring-deployment-guide.md`
- **Training Manual**: `/home/coding/irreversible-command-gate/docs/operators/training-manual.md`

## Getting Help

1. Check this guide first
2. Review `/home/coding/irreversible-command-gate/docs/monitoring-deployment-guide.md`
3. Consult `/home/coding/irreversible-command-gate/docs/operators/training-manual.md`
4. Check icg status: `icg status`
5. Review denial logs: `/var/cache/icg/denials.jsonl`
