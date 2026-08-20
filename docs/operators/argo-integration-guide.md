# Quick Reference: Argo Workflow Guard Integration

## For Operators: Deploying icg Guarded Workflows

### 5-Minute Setup

```bash
# 1. Build the guarded builder image (run from irreversible-command-gate repo)
cd containers/argo-guarded-builder
VERSION=$(cat VERSION)
docker build -t ronaldraygun/argo-guarded-builder:${VERSION} .
docker push ronaldraygun/argo-guarded-builder:${VERSION}

# 2. Deploy the workflow template to iad-ci
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  apply -f icg-guarded-ci-workflowtemplate.yml -n argo-workflows

# 3. Test with a sample workflow
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  create -f - <<EOF
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: test-icg-guarded-
  namespace: argo-workflows
spec:
  workflowTemplateRef:
    name: icg-guarded-ci
  arguments:
    parameters:
      - name: repo
        value: https://github.com/jedarden/irreversible-command-gate.git
      - name: revision
        value: main
EOF
```

### Updating Existing Workflows

Find and replace base images:

```bash
# List workflows using old builder images
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  get workflowtemplates -n argo-workflows \
  -o json | jq '.items[] | select(.spec.templates[].container.image | contains("debian:bookworm")) | .metadata.name'

# Update each workflow (manual edit or use kubectl patch)
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  edit workflowtemplate <template-name> -n argo-workflows
```

Replace:
```yaml
image: debian:bookworm
```

With:
```yaml
image: ronaldraygun/argo-guarded-builder:0.1.0
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
kubectl exec -it $POD -- icg coverage --pack /etc/icg/rule-pack.json
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
        image: ronaldraygun/argo-guarded-builder:0.1.0
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
cd containers/argo-guarded-builder
VERSION=$(cat VERSION)
docker build -t ronaldraygun/argo-guarded-builder:${VERSION} .
docker push ronaldraygun/argo-guarded-builder:${VERSION}

# 3. Update workflow templates to use new version
# (find and replace image tags)

# 4. Test with low-risk workflow first
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  create -f test-workflow.yml

# 5. Monitor for issues
kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig \
  logs -f <pod-name> -n argo-workflows
```

## File Locations

- **Builder Image**: `/home/coding/irreversible-command-gate/containers/argo-guarded-builder/`
- **Workflow Template**: `containers/argo-guarded-builder/icg-guarded-ci-workflowtemplate.yml`
- **Deployment Guide**: `/home/coding/irreversible-command-gate/docs/monitoring-deployment-guide.md`
- **Training Manual**: `/home/coding/irreversible-command-gate/docs/operators/training-manual.md`

## Getting Help

1. Check this guide first
2. Review `/home/coding/irreversible-command-gate/docs/monitoring-deployment-guide.md`
3. Consult `/home/coding/irreversible-command-gate/docs/operators/training-manual.md`
4. Check icg status: `icg status`
5. Review denial logs: `/var/cache/icg/denials.jsonl`
