use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};

fn load_kubectl_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/kubectl.json").expect("kubectl pack should load"))
        .expect("kubectl pack should validate");
    engine
}

fn assert_denied(engine: &Engine, command: &str, pattern_id: &str) {
    let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
    assert!(
        matches!(
            result,
            CheckResult::Denied {
                ref pack_id,
                pattern_id: ref actual_pattern_id,
                ..
            } if pack_id == "kubectl" && actual_pattern_id == pattern_id
        ),
        "expected {command:?} to be denied by {pattern_id}, got {result:?}"
    );
}

fn assert_allowed(engine: &Engine, command: &str) {
    assert_eq!(
        engine.evaluate_command(&CommandSource::Hook(command.to_string())),
        CheckResult::Allowed,
        "kubectl command should remain allowed: {command}"
    );
}

#[test]
fn manifest_declares_three_tier1_deny_rules_and_is_never_wrapper_shadowed() {
    let pack = load_pack("packs/kubectl.json").expect("kubectl pack should load");

    assert_eq!(pack.id, "kubectl");
    // The keyword dispatches the hook front-end; `icg install` deliberately
    // skips kubectl, so this never becomes a PATH wrapper.
    assert_eq!(pack.tool_keywords, ["kubectl"]);
    assert!(pack.applies_to.is_empty());
    assert_eq!(pack.safe_patterns.len(), 3);
    assert_eq!(pack.guarded_patterns.len(), 3);

    for pattern in &pack.guarded_patterns {
        assert!(pattern.enabled);
        assert_eq!(pattern.tier, Tier::Tier1);
        assert_eq!(pattern.redirect.channel, Channel::Deny);
        assert!(pattern.redirect.rewrite_template.is_none());
        assert!(matches!(pattern.check, Check::CommandRegex { .. }));
        assert!(
            pattern
                .redirect
                .reason_template
                .contains("declarative-config"),
            "{} must name the sanctioned GitOps alternative",
            pattern.id
        );
    }

    let severity = |id: &str| {
        pack.guarded_patterns
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("missing pattern {id}"))
            .severity
    };
    assert_eq!(severity("kubectl-delete"), Severity::Critical);
    assert_eq!(severity("kubectl-mutating-verb"), Severity::High);
    assert_eq!(severity("kubectl-create-outside-argo"), Severity::High);
}

#[test]
fn denies_delete_regardless_of_flags_before_the_verb() {
    let engine = load_kubectl_engine();

    for command in [
        "kubectl delete pvc data-postgres-0 -n commitgraph-production",
        "kubectl -n prod delete pod foo",
        "kubectl --server=http://traefik-iad-ci:8001 delete deploy api",
        "kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig delete wf x -n argo-workflows",
        "sudo kubectl delete namespace scratch",
    ] {
        assert_denied(&engine, command, "kubectl-delete");
    }
}

#[test]
fn denies_every_mutating_verb_org_rule_guard_denied() {
    let engine = load_kubectl_engine();

    // Parity with org-rule-guard.py's MUTATING set (apply, patch, edit,
    // replace, set, annotate, label, scale, autoscale, cordon, uncordon,
    // drain, taint, evict, mutating rollout), plus expose/run.
    for command in [
        "kubectl apply -f deployment.yaml -n commitgraph-production",
        "kubectl patch deploy api -n prod -p '{}'",
        "kubectl edit configmap app -n prod",
        "kubectl replace -f svc.yaml",
        "kubectl set image deploy/api api=example/api:1.2.3",
        "kubectl annotate ns prod owner=me",
        "kubectl label node n1 role=db",
        "kubectl scale deploy/api --replicas=0 -n prod",
        "kubectl autoscale deploy/api --max=5",
        "kubectl cordon node-1",
        "kubectl uncordon node-1",
        "kubectl drain node-1 --ignore-daemonsets",
        "kubectl taint nodes node-1 key=value:NoSchedule",
        "kubectl rollout restart deploy/api -n prod",
        "kubectl rollout undo deploy/api -n prod",
        "kubectl expose deploy api --port=80",
        "kubectl run shell --image=busybox -it",
    ] {
        assert_denied(&engine, command, "kubectl-mutating-verb");
    }
}

#[test]
fn create_is_denied_except_argo_workflow_submission() {
    let engine = load_kubectl_engine();

    assert_denied(
        &engine,
        "kubectl create deployment web --image=nginx -n prod",
        "kubectl-create-outside-argo",
    );
    assert_denied(
        &engine,
        "kubectl create secret generic app --from-literal=k=v",
        "kubectl-create-outside-argo",
    );

    for command in [
        "kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig create -f wf.yaml -n argo-workflows",
        "kubectl --kubeconfig=/home/coding/.kube/iad-ci.kubeconfig create -f - -n argo-workflows",
    ] {
        assert_allowed(&engine, command);
    }
}

#[test]
fn read_only_verbs_and_the_credential_free_proxy_stay_allowed() {
    let engine = load_kubectl_engine();

    for command in [
        "kubectl get pods -n prod",
        "kubectl --server=http://traefik-ardenone-cluster:8001 get pods -n devpod-observer",
        "kubectl describe pod x -n prod",
        "kubectl logs -n argo-workflows pod-x -c main -f",
        "kubectl top nodes",
        "kubectl exec -n prod pod-x -- ls /",
        "kubectl auth can-i create pods --server=http://traefik-iad-ci:8001",
        "kubectl rollout status deploy/api -n prod",
        "kubectl -n prod rollout history deploy/api",
        "kubectl diff -f deployment.yaml",
    ] {
        assert_allowed(&engine, command);
    }
}

#[test]
fn mutating_verbs_fire_through_timeout_xargs_and_nice_wrappers() {
    let engine = load_kubectl_engine();

    // The same shapes org-rule-guard.py misses: its wrapper skip list is
    // sudo/command/exec/time/nohup, so `timeout 30 kubectl delete ...`
    // sails through there. icg must unwrap these like sudo.
    for (command, pattern_id) in [
        (
            "timeout 30 kubectl delete pvc data-postgres-0 -n commitgraph-production",
            "kubectl-delete",
        ),
        (
            "timeout -k 10s 5m kubectl delete namespace scratch",
            "kubectl-delete",
        ),
        ("xargs kubectl delete pod x", "kubectl-delete"),
        ("xargs -0 -n 1 kubectl delete pod x", "kubectl-delete"),
        (
            "nice -n 5 kubectl scale deploy/api --replicas=0 -n prod",
            "kubectl-mutating-verb",
        ),
        (
            "timeout --preserve-status 30 kubectl patch configmap app -n prod -p '{}'",
            "kubectl-mutating-verb",
        ),
    ] {
        assert_denied(&engine, command, pattern_id);
    }

    // Read-only verbs keep their safe-pattern coverage through a wrapper --
    // unwrapping must not widen guarded matching.
    for command in [
        "timeout 30 kubectl get pods -n prod",
        "xargs -0 kubectl get pods -n prod",
        "nice -n 10 kubectl top nodes",
    ] {
        assert_allowed(&engine, command);
    }
}

#[test]
fn mutating_words_as_values_or_downstream_text_do_not_trip_the_rules() {
    let engine = load_kubectl_engine();

    // A verb only counts in verb position: after whitespace, before any
    // pipe or command separator.
    for command in [
        "kubectl get pods -n prod -l app=delete-me",
        "kubectl get events -n prod --field-selector reason=ScalingReplicaSet",
        "kubectl get pods -n prod | grep delete",
        "kubectl get deploy -o name; echo apply",
    ] {
        assert_allowed(&engine, command);
    }
}
