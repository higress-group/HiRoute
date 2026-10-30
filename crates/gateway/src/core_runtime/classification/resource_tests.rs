use super::*;
use crate::replay::{ReplayConfig, ReplayManager};
use crate::server::composition::ProductionPorts;
use crate::server::publication::GatewayPublicationInstaller;
use crate::server::request_plan::IngressProtocol;
use hiroute_gateway_core::runtime::body::BudgetTree;
use serde_json::json;

#[tokio::test]
async fn optional_projection_capacity_falls_back_without_changing_input_or_source_liveness() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-classifier-resource-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let publications =
        Arc::new(GatewayPublicationInstaller::open(root.join("publication.json")).unwrap());
    let runtime = ProductionGatewayRuntime::compose(ProductionPorts::fail_closed(publications));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.join("replay"),
        memory_threshold_bytes: 64,
        record_bytes: 64,
        orphan_ttl: Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(4096, 4096).unwrap().stream(4096).unwrap();
    let replay = manager.begin_request(budget).unwrap();
    let user = "native auxiliary input".repeat(512);
    let request = super::super::adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"smart-route", "input":user, "stream":false}),
    )
    .unwrap();
    let original = serde_json::to_value(&request).unwrap();
    let digest = hiroute_domain::CanonicalDigest::of(&"resource-fixture").unwrap();
    let strategy = ComplexityV1::compile_with_classifier(
        Vec::new(),
        CompiledClassifierKindV1::Rest,
        Some(digest.as_str().to_owned()),
    )
    .unwrap();
    let history = AgentTurnHistorySnapshot {
        previous_decision: None,
        visible_conversation: Vec::new(),
        history_partial: true,
        assessment_from: None,
        assessment_target: None,
        _pin: None,
    };
    for mode in ["active", "cancelled", "expired"] {
        let cancellation = CancellationToken::new();
        if mode == "cancelled" {
            cancellation.cancel();
        }
        let now = Instant::now();
        let result = runtime
            .classify_for_planner(
                &strategy,
                ClassificationRequest {
                    native_network: None,
                    admitted_network: None,
                    request: &request,
                    current_input: None,
                    native_user_unavailable: false,
                    current_input_origin_unverified: false,
                    replay: &replay,
                    classifier: None,
                    history: &history,
                    correlated: None,
                    publication_revision: 1,
                    overall_deadline: if mode == "expired" {
                        now
                    } else {
                        now + Duration::from_secs(5)
                    },
                    cancellation,
                    started_at: now,
                    admitted_credential_state: None,
                },
            )
            .await;
        match mode {
            "cancelled" => assert!(matches!(result, Err(ClassificationError::Cancelled))),
            "expired" => assert!(matches!(result, Err(ClassificationError::Deadline))),
            _ => {
                let outcome = result.expect("optional projection must not reject the request");
                assert!(outcome.decision.fallback_used);
                assert_eq!(
                    outcome.decision.fallback_reason,
                    Some(ClassifierFallbackReasonV1::Unavailable)
                );
                assert_eq!(
                    outcome.decision.execution_group,
                    hiroute_domain::ExecutionGroupV1::Primary
                );
                assert!(outcome.assessment.is_none());
            }
        }
        assert_eq!(serde_json::to_value(&request).unwrap(), original);
    }
    drop(replay);
    drop(manager);
    drop(runtime);
    std::fs::remove_dir_all(root).unwrap();
}
