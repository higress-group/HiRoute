use hiroute_domain::{BillingClass, ModelEndpointCapabilityV1, OfferV1};

use super::*;

fn current_catalog() -> TrustedReleaseCatalog {
    const MANIFEST: &[u8] =
        include_bytes!("../../../../../assets/release-facts/current/bundle/manifest.json");
    const REGISTRY: &[u8] = include_bytes!(
        "../../../../../assets/release-facts/current/bundle/connector-registry.json"
    );
    const MODEL_DATA: &[u8] =
        include_bytes!("../../../../../assets/release-facts/current/bundle/model-data.json");
    TrustedReleaseCatalog::load_bundled_release_facts(MANIFEST, MANIFEST, REGISTRY, MODEL_DATA)
        .unwrap()
}

fn discovery() -> RegisteredComputeDiscoveryFactV1 {
    RegisteredComputeDiscoveryFactV1 {
        agent_id: "agent.claude-code".into(),
        scanner_id: "scanner.claude.v1".into(),
        scanner_version: "1".into(),
        discovered_source_ref: "claude/settings/v1".into(),
        configuration_revision: 1,
        connection_option_id: "zhipu.coding-plan.cn.v1".into(),
        endpoint_profile_id: "endpoint.zhipu.coding-plan.cn.v1".into(),
        endpoint_profile_revision: 1,
        registered_base_url: "https://open.bigmodel.cn/api/anthropic".into(),
        observed_model_id: "glm-5.3".into(),
        model_configuration_id: "model.zhipu.glm-5.3".into(),
        protected_credential_available: true,
    }
}

#[test]
fn discovered_messages_source_stays_bound_to_messages_when_responses_is_also_available() {
    let catalog = current_catalog();
    let candidate = catalog.authorize_compute_discovery(discovery()).unwrap();
    let ids = catalog.compute_projection_ids(&candidate).unwrap();
    let prepared = catalog
        .prepare_compute_projection(
            candidate,
            hiroute_domain::ComputeProjectionExpectationV1 {
                source_revision: 0,
                source_digest: None,
                binding_revision: 0,
                binding_digest: None,
                inventory_revision: 0,
                inventory_digest: None,
            },
            true,
        )
        .unwrap();
    assert_eq!(prepared.desired.binding.binding_id, ids.binding_id);
    assert_eq!(
        prepared.desired.binding.capability_id,
        "cap.zhipu.glm-5.3.coding-plan.messages"
    );
}

#[test]
fn projection_ids_keep_source_and_binding_stable_across_scanner_refresh() {
    let catalog = current_catalog();
    let original = catalog
        .authorize_compute_discovery(discovery())
        .and_then(|candidate| catalog.compute_projection_ids(&candidate))
        .unwrap();

    let mut refreshed = discovery();
    refreshed.scanner_id = "scanner.claude.v2".into();
    refreshed.scanner_version = "2".into();
    refreshed.discovered_source_ref = "claude/settings/v2".into();
    refreshed.configuration_revision = 2;
    let refreshed = catalog
        .authorize_compute_discovery(refreshed)
        .and_then(|candidate| catalog.compute_projection_ids(&candidate))
        .unwrap();

    assert_eq!(original.source_id, refreshed.source_id);
    assert_eq!(original.binding_id, refreshed.binding_id);
}

#[test]
fn exact_model_offer_capability_change_keeps_source_and_changes_binding() {
    let source_identity_digest = CanonicalDigest::of_bytes(b"stable-source");
    let original_fact = discovery();
    let original_capability = ModelEndpointCapabilityV1 {
        capability_id: "capability.fixture.messages".into(),
        revision: 1,
        model_configuration_id: original_fact.model_configuration_id.clone(),
        connector_id: "connector.messages.fixture".into(),
        connector_revision: 1,
        endpoint_profile_id: original_fact.endpoint_profile_id.clone(),
        endpoint_profile_revision: 1,
        protocol_endpoint_id: "endpoint.messages.fixture.messages".into(),
        upstream_protocol: UpstreamProtocol::Messages,
        upstream_model_id: original_fact.observed_model_id.clone(),
        required_adapter_ref: "adapter.messages.fixture".into(),
        required_adapter_revision: 1,
        evidence_digest: CanonicalDigest::of_bytes(b"capability-v1"),
    };
    let original_offer = OfferV1 {
        offer_id: "offer.fixture.messages".into(),
        revision: 1,
        endpoint_profile_id: original_fact.endpoint_profile_id.clone(),
        endpoint_profile_revision: 1,
        service_offering_id: "coding-plan".into(),
        entitlement_id: "api-key".into(),
        usage_scope: "account".into(),
        region_id: "test".into(),
        model_configuration_ids: vec![original_fact.model_configuration_id.clone()],
        billing_class: BillingClass::Subscription,
        evidence_digest: CanonicalDigest::of_bytes(b"offer-v1"),
    };
    let original = projection_ids(
        &source_identity_digest,
        &original_fact,
        &original_capability,
        &original_offer,
    )
    .unwrap()
    .0;

    let mut changed_fact = original_fact;
    changed_fact.model_configuration_id = "model.fixture.next".into();
    changed_fact.observed_model_id = "fixture-next-model".into();
    let mut changed_capability = original_capability;
    changed_capability.capability_id = "capability.fixture.next".into();
    changed_capability.model_configuration_id = changed_fact.model_configuration_id.clone();
    changed_capability.upstream_model_id = changed_fact.observed_model_id.clone();
    let mut changed_offer = original_offer;
    changed_offer.offer_id = "offer.fixture.next".into();
    changed_offer.model_configuration_ids = vec![changed_fact.model_configuration_id.clone()];
    let changed = projection_ids(
        &source_identity_digest,
        &changed_fact,
        &changed_capability,
        &changed_offer,
    )
    .unwrap()
    .0;

    assert_eq!(original.source_id, changed.source_id);
    assert_ne!(original.binding_id, changed.binding_id);
}
