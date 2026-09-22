use hiroute_domain::{
    AgentKindV1, AgentProfilesArtifactV1, AuthenticationKind, BillingClass, CanonicalDigest,
    ComputeContractError, ConnectionOrigin, ConnectorRegistryBundleV1, ConnectorRuntimeKind,
    NativeReasoningCapabilityV1, ReleaseFactsManifestV2, ReleaseModelDataBundleV2,
    UpstreamProtocol,
};

const MANIFEST: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/manifest.json");
const REGISTRY: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json");
const MODEL_DATA: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/model-data.json");
const AGENT_PROFILES: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/agent-profiles.json");

fn current() -> (
    ReleaseFactsManifestV2,
    ConnectorRegistryBundleV1,
    ReleaseModelDataBundleV2,
) {
    (
        serde_json::from_slice(MANIFEST).unwrap(),
        serde_json::from_slice(REGISTRY).unwrap(),
        serde_json::from_slice(MODEL_DATA).unwrap(),
    )
}

#[test]
fn bundled_current_catalog_binds_the_exact_client_resources() {
    let (manifest, registry, model_data) = current();
    manifest.validate_shape().unwrap();
    model_data.validate_against(&registry).unwrap();
    assert_eq!(manifest.catalog_id, "client-bundled/current");
    assert_eq!(manifest.product_release, "mvp-current");
    assert_eq!(manifest.product_release, registry.product_release);
    assert_eq!(manifest.product_release, model_data.data.product_release);
    assert_eq!(
        manifest.connector_registry_digest,
        CanonicalDigest::of_bytes(REGISTRY)
    );
    assert_eq!(
        manifest.model_data_digest,
        CanonicalDigest::of_bytes(MODEL_DATA)
    );
    assert_eq!(
        manifest.cross_reference_digest,
        model_data.cross_reference_digest(&registry).unwrap()
    );
    assert!(!registry.connectors.is_empty());
    assert!(!model_data.data.models.is_empty());
    assert!(!model_data.metadata_catalog.model_records.is_empty());
}

#[test]
fn bundled_current_agent_profiles_are_exact_typed_artifacts() {
    let artifact: AgentProfilesArtifactV1 = serde_json::from_slice(AGENT_PROFILES).unwrap();
    artifact.validate().unwrap();
    assert_eq!(
        artifact.tool_version,
        hiroute_domain::RELEASE_FACTS_TOOL_VERSION_V2
    );
    let codex = artifact
        .profiles
        .iter()
        .find(|profile| profile.kind == AgentKindV1::Codex)
        .unwrap();
    assert!(
        codex.legacy_exact_versions.is_empty(),
        "Codex discovery must not reintroduce a binary-version admission gate"
    );
    let claude = artifact
        .profiles
        .iter()
        .find(|profile| profile.kind == AgentKindV1::ClaudeCode)
        .unwrap();
    assert!(claude.legacy_exact_versions.is_empty());
}

#[test]
fn bundled_current_catalog_admits_codex_terra_only_on_its_exact_provider_endpoint() {
    let (_, registry, model_data) = current();
    let option = registry
        .connection_option("codex.subscription.global.v1")
        .unwrap();
    assert_eq!(option.origin, ConnectionOrigin::AgentSubscription);
    assert_eq!(option.connector_id, "connector.cpa.codex");
    assert_eq!(option.endpoint_profile_id, "endpoint.cpa.codex");
    assert_eq!(option.billing_class, BillingClass::Subscription);

    let connector = registry.connector(&option.connector_id).unwrap();
    assert_eq!(connector.runtime_kind, ConnectorRuntimeKind::CpaBridge);
    assert_eq!(
        connector.authentication,
        AuthenticationKind::ConnectorOwnedOpaque
    );
    let endpoint_profile = registry
        .endpoint_profile(&option.endpoint_profile_id)
        .unwrap();
    let endpoint = endpoint_profile.protocol_endpoints.first().unwrap();
    assert_eq!(endpoint.protocol, UpstreamProtocol::Responses);
    assert!(
        endpoint.authorize_exact_destination("https://chatgpt.com/backend-api/codex/responses")
    );
    assert!(!endpoint.authorize_exact_destination("http://localhost:4317/v1/responses"));

    let model = model_data
        .data
        .models
        .iter()
        .find(|model| model.model_configuration_id == "model.openai.gpt-5.6-terra")
        .unwrap();
    let capability_index = model_data
        .data
        .model_endpoint_capabilities
        .iter()
        .position(|capability| {
            capability.capability_id == "cap.openai.gpt-5.6-terra.codex.responses"
        })
        .unwrap();
    let capability = &model_data.data.model_endpoint_capabilities[capability_index];
    assert_eq!(
        capability.model_configuration_id,
        model.model_configuration_id
    );
    assert_eq!(capability.connector_id, connector.connector_id);
    assert_eq!(
        capability.endpoint_profile_id,
        endpoint_profile.endpoint_profile_id
    );
    assert_eq!(capability.upstream_protocol, UpstreamProtocol::Responses);
    assert_eq!(capability.upstream_model_id, "gpt-5.6-terra");

    let offer = model_data.data.offer("offer.codex.subscription").unwrap();
    assert_eq!(offer.billing_class, BillingClass::Subscription);
    assert!(
        offer
            .model_configuration_ids
            .contains(&model.model_configuration_id)
    );
    assert!(
        model_data
            .data
            .price_rates
            .iter()
            .all(|rate| rate.model_configuration_id != model.model_configuration_id)
    );

    let reasoning = model_data
        .rating_snapshot
        .models
        .iter()
        .find(|reasoning| reasoning.model_configuration_id == model.model_configuration_id)
        .unwrap();
    let NativeReasoningCapabilityV1::Discrete {
        parameter,
        profiles,
    } = &reasoning.capability
    else {
        panic!("Codex Terra must retain discrete native reasoning");
    };
    assert_eq!(parameter, "reasoning_effort");
    assert!(profiles.iter().any(|profile| profile == "medium"));

    let mut wrong_provider = model_data;
    wrong_provider.data.model_endpoint_capabilities[capability_index].connector_id =
        "connector.zhipu.p0".into();
    assert_eq!(
        wrong_provider.validate_against(&registry),
        Err(ComputeContractError::CrossReference)
    );
}

#[test]
fn bundled_current_catalog_keeps_zhipu_glm_scoped_to_its_messages_endpoint() {
    let (_, registry, model_data) = current();
    let model = model_data
        .data
        .models
        .iter()
        .find(|model| model.model_configuration_id == "model.zhipu.glm-5.3")
        .unwrap();
    let capability = model_data
        .data
        .model_endpoint_capabilities
        .iter()
        .find(|capability| capability.capability_id == "cap.zhipu.glm-5.3.coding-plan.messages")
        .unwrap();
    assert_eq!(
        capability.model_configuration_id,
        model.model_configuration_id
    );
    assert_eq!(capability.connector_id, "connector.zhipu.p0");
    assert_eq!(
        capability.endpoint_profile_id,
        "endpoint.zhipu.coding-plan.cn.v1"
    );
    assert_eq!(capability.upstream_protocol, UpstreamProtocol::Messages);
    assert_eq!(capability.upstream_model_id, "glm-5.3");
    let offer = model_data.data.offer("offer.zhipu.coding-plan").unwrap();
    assert_eq!(offer.billing_class, BillingClass::Subscription);
    assert!(
        offer
            .model_configuration_ids
            .contains(&model.model_configuration_id)
    );
    model_data.validate_against(&registry).unwrap();
}
