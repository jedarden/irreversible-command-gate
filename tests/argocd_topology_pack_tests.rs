use icg::engine::{CheckResult, ContentSource, Engine};
use icg::rule_pack::load_pack;

fn engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/argocd-topology.json").expect("ArgoCD topology pack loads"))
        .expect("ArgoCD topology pack validates");
    engine
}

#[test]
fn denies_duplicate_ardenone_root() {
    let result = engine().evaluate_content(&ContentSource::Write {
        file_path: "k8s/ardenone-cluster/ardenone-cluster-application.yml".to_string(),
        content: r#"
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: applications-ardenone-cluster
spec:
  source:
    path: ./k8s/ardenone-cluster
  destination:
    server: https://kubernetes.default.svc
"#
        .into(),
    });

    assert!(matches!(
        result,
        CheckResult::Denied { pack_id, pattern_id, .. }
            if pack_id == "argocd-topology"
                && pattern_id == "duplicate-ardenone-cluster-root"
    ));
}

#[test]
fn allows_generated_child_application() {
    for (file_path, content) in [(
        "k8s/ardenone-cluster/investment-research-mcp/investment-research-mcp-application.yml",
        "kind: Application\nmetadata:\n  name: investment-research-mcp\nspec:\n  source:\n    path: ./k8s/ardenone-cluster/investment-research-mcp\n  destination:\n    server: https://k3s-server-a.ardenone.com:6443\n",
    )] {
        assert!(matches!(
            engine().evaluate_content(&ContentSource::Write {
                file_path: file_path.to_string(),
                content: content.into(),
            }),
            CheckResult::Allowed
        ));
    }
}
