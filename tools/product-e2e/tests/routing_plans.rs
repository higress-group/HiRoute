use std::collections::{BTreeMap, BTreeSet};

use hiroute_application_api::{CommandLifecycle, command_by_id};
use hiroute_domain::{
    AgentIngressProtocolV1, CanonicalDigest, ConnectorRuntimeKind,
    GatewayCandidatePricingIdentityV1, GatewayCandidateProtocolProfileV1,
    GatewayExecutableCandidateV2, GatewayExecutableRoutingV2, GatewayModelRouteV2,
    GatewayOperationalTargetV1,
};
use serde::{Deserialize, Serialize};

#[path = "../src/routing/mod.rs"]
mod routing;

use routing::{
    PRODUCTION_EVIDENCE, PRODUCTION_EVIDENCE_OWNER, RoutingPlanContractV1, ScenarioState,
};

const SCENARIO: &str =
    include_str!("../../../e2e/product/scenarios/routing/routing-contract.v1.json");
const FIXTURE: &str = include_str!("../../../e2e/product/fixtures/routing/compiler-input.v1.json");
const PUBLICATION_GOLDEN: &str =
    include_str!("../../../e2e/product/golden/routing/compiled-publication.v2.json");
const BOUNDARY_GOLDEN: &str =
    include_str!("../../../e2e/product/golden/routing/routing-boundary.v1.json");

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RoutingFixture {
    schema: String,
    desired_modes: BTreeSet<String>,
    compiler_inputs: BTreeSet<String>,
    runtime_forbidden_inputs: BTreeSet<String>,
    contains_secret_literal: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundaryGolden {
    scenario_id: String,
    compiler_contract: String,
    publication_closure_repair: String,
    production_cli_daemon_routing: String,
    production_evidence: String,
    composition_owner: String,
    publication_schema: String,
    publication_digest: CanonicalDigest,
    gateway_snapshot_schema: String,
    gateway_snapshot_digest: CanonicalDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenG0Snapshot {
    schema_version: String,
    admission: hiroute_domain::GatewayAdmissionStateV1,
    workspace_id: String,
    authority_id: String,
    authority_epoch: u64,
    publication_revision: u64,
    payload_digest: String,
    catalog_renderer_revision: String,
    aliases: Vec<FrozenG0Alias>,
    grants: Vec<FrozenG0Grant>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenG0Alias {
    served_model_id: String,
    purpose: String,
    agent_plan_revision: u64,
    protocols: Vec<AgentIngressProtocolV1>,
    overall_timeout_ms: u64,
    max_attempts: u32,
    routing: Option<GatewayExecutableRoutingV2>,
    candidates: Vec<FrozenG0Candidate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenG0Candidate {
    local_id: u32,
    stable_target_key: String,
    adapter_id: String,
    credential_refs: Vec<String>,
    credential_destination_ref: String,
    upstream_model_id: String,
    native_transport_model: String,
    endpoint: String,
    connector_runtime: ConnectorRuntimeKind,
    operational_target: GatewayOperationalTargetV1,
    operational_target_digest: CanonicalDigest,
    protocol_profiles: Vec<GatewayCandidateProtocolProfileV1>,
    protocol_profile_digest: CanonicalDigest,
    pricing_identity: Option<GatewayCandidatePricingIdentityV1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenG0Grant {
    grant_id: String,
    generation: u64,
    bearer_token_sha256: String,
    protocol: AgentIngressProtocolV1,
    routes: BTreeMap<String, GatewayModelRouteV2>,
}

#[derive(Deserialize)]
struct LegacySnapshotSource {
    workspace_id: String,
    authority_id: String,
    authority_epoch: u64,
    publication_revision: u64,
    catalog_renderer_revision: String,
    aliases: Vec<LegacyFrozenAlias>,
    grants: Vec<LegacyFrozenG0Grant>,
}

#[derive(Serialize)]
struct LegacyFrozenG0Snapshot {
    schema_version: String,
    admission: hiroute_domain::GatewayAdmissionStateV1,
    workspace_id: String,
    authority_id: String,
    authority_epoch: u64,
    publication_revision: u64,
    payload_digest: String,
    catalog_renderer_revision: String,
    aliases: Vec<LegacyFrozenAlias>,
    grants: Vec<LegacyFrozenG0Grant>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyFrozenG0Grant {
    grant_id: String,
    generation: u64,
    bearer_token_sha256: String,
    allowed_protocols: Vec<AgentIngressProtocolV1>,
    allowed_aliases: Vec<String>,
}

// The historical snapshot hashed typed serialization, rather than the publication's
// JSON member order. Freeze only the layers that set that order; opaque candidates
// retain the original capability contract instead of using today's execution DTO.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyFrozenAlias {
    served_model_id: String,
    purpose: String,
    agent_plan_revision: u64,
    protocols: Vec<AgentIngressProtocolV1>,
    overall_timeout_ms: u64,
    max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    routing: Option<LegacyFrozenRouting>,
    candidates: Box<serde_json::value::RawValue>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyFrozenRouting {
    agent_plan_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan_display_name: Option<String>,
    request_owned: LegacyFrozenRequestOwned,
    groups: Box<serde_json::value::RawValue>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyFrozenRequestOwned {
    strategy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    classifier: Option<Box<serde_json::value::RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reselect_on_user_message: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    simple_groups: Option<Box<serde_json::value::RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    complex_groups: Option<Box<serde_json::value::RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_policy: Option<Box<serde_json::value::RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ordered_groups: Option<Box<serde_json::value::RawValue>>,
}

#[test]
fn routing_plans_reports_compiler_and_production_journey_green() {
    let scenario: RoutingPlanContractV1 = serde_json::from_str(SCENARIO).unwrap();
    scenario.validate().unwrap();
    let compiler = scenario
        .scenario_states
        .iter()
        .find(|value| value.scenario_id == "compiler-contract")
        .unwrap();
    let production = scenario
        .scenario_states
        .iter()
        .find(|value| value.scenario_id == "production-cli-daemon-routing")
        .unwrap();
    let repair = scenario
        .scenario_states
        .iter()
        .find(|value| value.scenario_id == "publication-closure-repair")
        .unwrap();
    assert_eq!(compiler.state, ScenarioState::Green);
    assert_eq!(repair.state, ScenarioState::Green);
    assert_eq!(repair.evidence_owner, "PROCESS-25012");
    assert_eq!(production.state, ScenarioState::Green);
    assert_eq!(production.evidence_owner, PRODUCTION_EVIDENCE_OWNER);
    assert!(production.blocker.is_none());
    for command_id in [
        "routing.options",
        "routing.list",
        "routing.show",
        "routing.preview",
        "routing.apply",
    ] {
        assert_eq!(
            command_by_id(command_id).unwrap().lifecycle,
            CommandLifecycle::Released
        );
    }
}

#[test]
fn routing_plans_golden_is_a_closed_deterministic_publication() {
    let frozen: serde_json::Value = serde_json::from_str(PUBLICATION_GOLDEN).unwrap();
    let bytes = compact_json_preserving_member_order(PUBLICATION_GOLDEN);
    let publication = serde_json::from_str::<hiroute_domain::GatewayPublicationV1>(include_str!(
        "../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    let boundary: BoundaryGolden = serde_json::from_str(BOUNDARY_GOLDEN).unwrap();
    assert_eq!(boundary.scenario_id, "process-25004-routing-plans");
    assert_eq!(boundary.compiler_contract, "green");
    assert_eq!(boundary.publication_closure_repair, "green");
    assert_eq!(boundary.production_cli_daemon_routing, "green");
    assert_eq!(boundary.production_evidence, PRODUCTION_EVIDENCE);
    assert_eq!(boundary.composition_owner, PRODUCTION_EVIDENCE_OWNER);
    assert_eq!(
        boundary.publication_schema,
        "hiroute.gateway-publication/v2"
    );
    assert_eq!(
        boundary.publication_digest,
        CanonicalDigest::of_bytes(&bytes)
    );
    let legacy_snapshot_source: LegacySnapshotSource = serde_json::from_slice(&bytes).unwrap();
    let legacy_snapshot = LegacyFrozenG0Snapshot {
        schema_version: boundary.gateway_snapshot_schema.clone(),
        admission: hiroute_domain::GatewayAdmissionStateV1::NewCallsAllowed,
        workspace_id: legacy_snapshot_source.workspace_id,
        authority_id: legacy_snapshot_source.authority_id,
        authority_epoch: legacy_snapshot_source.authority_epoch,
        publication_revision: legacy_snapshot_source.publication_revision,
        payload_digest: String::new(),
        catalog_renderer_revision: legacy_snapshot_source.catalog_renderer_revision,
        aliases: legacy_snapshot_source.aliases,
        grants: legacy_snapshot_source.grants,
    };
    assert_eq!(
        serde_json::to_value(&legacy_snapshot.aliases).unwrap(),
        frozen["aliases"]
    );
    assert_eq!(
        CanonicalDigest::of_bytes(&serde_json::to_vec(&legacy_snapshot).unwrap()),
        boundary.gateway_snapshot_digest
    );
    publication.validate_current_contract().unwrap();
    assert_eq!(
        publication.schema,
        hiroute_domain::GATEWAY_PUBLICATION_SCHEMA_V3
    );
    assert!(publication.plans.iter().all(|plan| {
        plan.body.schema == hiroute_domain::AGENT_PLAN_COMPILED_SCHEMA_V3
            && plan.body.compiler_revision == hiroute_domain::AGENT_PLAN_COMPILER_REVISION_V3
    }));
    let gateway_snapshot = publication.gateway_snapshot().unwrap();
    assert_eq!(
        boundary.gateway_snapshot_schema,
        hiroute_domain::GATEWAY_SNAPSHOT_SCHEMA_V3
    );
    assert_eq!(
        gateway_snapshot.payload_digest,
        gateway_snapshot.canonical_digest().unwrap().as_str()
    );
    let g0: FrozenG0Snapshot =
        serde_json::from_slice(&serde_json::to_vec(&gateway_snapshot).unwrap()).unwrap();
    assert_eq!(g0.schema_version, boundary.gateway_snapshot_schema);
    assert_eq!(
        g0.admission,
        hiroute_domain::GatewayAdmissionStateV1::NewCallsAllowed
    );
    assert_eq!(g0.workspace_id, publication.workspace_id.as_str());
    assert_eq!(g0.authority_id, publication.authority_id);
    assert_eq!(g0.authority_epoch, publication.authority_epoch);
    assert_eq!(
        g0.publication_revision,
        publication.publication_revision.get()
    );
    assert_eq!(g0.payload_digest, gateway_snapshot.payload_digest);
    assert_eq!(
        g0.catalog_renderer_revision,
        publication.catalog_renderer_revision
    );
    assert_eq!(g0.aliases.len(), publication.aliases.len());
    assert_eq!(g0.grants.len(), publication.grants.len());
    for (frozen_alias, exact_alias) in g0.aliases.iter().zip(&gateway_snapshot.aliases) {
        assert_eq!(
            frozen_alias.served_model_id,
            exact_alias.served_model_id.as_str()
        );
        assert_eq!(frozen_alias.purpose, exact_alias.purpose);
        assert_eq!(
            frozen_alias.agent_plan_revision,
            exact_alias.agent_plan_revision
        );
        assert_eq!(frozen_alias.protocols, exact_alias.protocols);
        assert_eq!(
            frozen_alias.overall_timeout_ms,
            exact_alias.overall_timeout_ms
        );
        assert_eq!(frozen_alias.max_attempts, exact_alias.max_attempts);
        assert_eq!(frozen_alias.routing, exact_alias.routing);
        assert_eq!(frozen_alias.candidates.len(), exact_alias.candidates.len());
        for (frozen, exact) in frozen_alias.candidates.iter().zip(&exact_alias.candidates) {
            assert_frozen_candidate_matches(frozen, exact);
        }
    }
    assert!(g0.grants.iter().all(|grant| {
        !grant.grant_id.is_empty()
            && grant.generation > 0
            && grant.bearer_token_sha256.starts_with("sha256:")
            && matches!(
                grant.protocol,
                AgentIngressProtocolV1::Responses | AgentIngressProtocolV1::Messages
            )
            && !grant.routes.is_empty()
    }));
}

fn compact_json_preserving_member_order(source: &str) -> Vec<u8> {
    let mut compact = Vec::with_capacity(source.len());
    let mut in_string = false;
    let mut escaped = false;
    for byte in source.bytes() {
        if in_string {
            compact.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
            compact.push(byte);
        } else if !byte.is_ascii_whitespace() {
            compact.push(byte);
        }
    }
    debug_assert!(!in_string);
    compact
}

fn assert_frozen_candidate_matches(
    frozen: &FrozenG0Candidate,
    exact: &GatewayExecutableCandidateV2,
) {
    assert!(frozen.local_id > 0);
    assert_eq!(frozen.local_id, exact.local_id);
    assert_eq!(frozen.stable_target_key, exact.stable_target_key);
    assert_eq!(frozen.adapter_id, exact.adapter_id);
    assert!(!frozen.credential_refs.is_empty());
    assert_eq!(
        frozen.credential_refs.iter().collect::<BTreeSet<_>>().len(),
        frozen.credential_refs.len(),
        "credential refs must be unique without changing their frozen order"
    );
    assert_eq!(frozen.credential_refs, exact.credential_refs);
    assert_eq!(
        frozen.credential_destination_ref,
        exact.credential_destination_ref
    );
    assert_eq!(frozen.upstream_model_id, exact.upstream_model_id);
    assert_eq!(frozen.native_transport_model, exact.native_transport_model);
    assert_eq!(frozen.endpoint, exact.endpoint);
    assert_eq!(frozen.connector_runtime, exact.connector_runtime);
    assert_eq!(frozen.operational_target, exact.operational_target);
    assert_eq!(
        frozen.operational_target_digest,
        exact.operational_target_digest
    );
    assert_eq!(frozen.protocol_profiles, exact.protocol_profiles);
    assert_eq!(
        frozen.protocol_profile_digest,
        exact.protocol_profile_digest
    );
    assert_eq!(frozen.pricing_identity, exact.pricing_identity);
    assert!(
        frozen
            .operational_target
            .validate_for(frozen.connector_runtime, &frozen.endpoint)
    );
    assert_eq!(
        frozen.operational_target_digest,
        CanonicalDigest::of(&frozen.operational_target).unwrap()
    );
    assert_eq!(
        frozen.protocol_profile_digest,
        CanonicalDigest::of(&frozen.protocol_profiles).unwrap()
    );
}

#[test]
fn routing_plans_fixture_separates_compile_inputs_from_runtime_inputs() {
    let fixture: RoutingFixture = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture.schema, "hiroute.routing-compiler-fixture/v1");
    assert_eq!(
        fixture.desired_modes,
        BTreeSet::from([
            "custom".to_owned(),
            "free_first".to_owned(),
            "smart_saving".to_owned(),
        ])
    );
    assert_eq!(
        fixture.compiler_inputs,
        BTreeSet::from([
            "agent_connection_grants".to_owned(),
            "capability_slice".to_owned(),
            "catalog_renderer_revision".to_owned(),
            "credential_references".to_owned(),
            "free_offer_slice".to_owned(),
            "gateway_authority".to_owned(),
            "local_inventory".to_owned(),
            "materialized_bindings".to_owned(),
            "ordering_price_slice".to_owned(),
            "rating_slice".to_owned(),
            "registered_protocol_endpoints".to_owned(),
        ])
    );
    assert_eq!(
        fixture.runtime_forbidden_inputs,
        BTreeSet::from([
            "control_database".to_owned(),
            "price_service".to_owned(),
            "rating_service".to_owned(),
            "routing_template".to_owned(),
        ])
    );
    assert!(!fixture.contains_secret_literal);
    let encoded = serde_json::to_vec(&fixture).unwrap();
    for forbidden in ["api_key", "bearer", "credential_value", "endpoint_url"] {
        assert!(
            !encoded
                .windows(forbidden.len())
                .any(|window| window.eq_ignore_ascii_case(forbidden.as_bytes()))
        );
    }
}
