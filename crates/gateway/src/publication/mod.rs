//! Aggregate Application → Gateway publication, durable LKG and catalog view.

mod candidate;
mod catalog;
pub(crate) use candidate::{CompiledCandidate, PublishedPlannerFacts};
mod compiler;
mod execution;
pub(crate) use execution::PreparedCandidateExecution;
mod installer;
pub(crate) mod lease_target;
mod run;
mod schema;
mod trust;

pub use catalog::{CatalogError, CatalogResponse, GatewayCatalog};
pub(crate) use compiler::{CompiledGrant, CompiledGrantRoute, compile_classifier_mode_authority};
pub use installer::{
    GatewayPrepareOutcome, GatewayPublicationInstaller, PreparedGatewayPublication,
    PublicationFailpoint, PublicationInstallError, PublishedGatewayPublication,
};
pub use run::{RunPublicationError, RunPublicationHandle};
pub(crate) use schema::constant_time_digest_eq;
pub use schema::{
    AliasComplexityClassifierV1, AliasCostPolicyV1, AliasGroupIdV1, AliasModelGroupV1, AliasPlanV1,
    AliasRequestOwnedRouteV1, AliasRoutingV1, CandidateBindingV1, GATEWAY_PUBLICATION_SCHEMA,
    GatewayPublicationSnapshotV3, GrantV1, MAX_LOGICAL_REQUEST_DURATION_MS, MAX_REQUEST_ATTEMPTS,
    ModelRouteV2, PublicationSchemaError, token_sha256,
};
#[cfg(test)]
pub(crate) use schema::{exact_cpa_test_candidate, exact_test_candidate, test_plan_route};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SealedCandidateExecutionV1 {
    pub schema_version: String,
    pub stable_target_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_binding_id: Option<String>,
    pub credential_destination_ref: String,
    pub logical_endpoint: String,
    pub upstream_model_id: String,
    pub native_transport_model: String,
    pub connector_runtime: hiroute_domain::ConnectorRuntimeKind,
    pub operational_target: hiroute_domain::GatewayOperationalTargetV1,
    pub operational_target_digest: hiroute_domain::CanonicalDigest,
    pub protocol_set_digest: hiroute_domain::CanonicalDigest,
    pub profile_digest: String,
    pub protocol_profile: crate::server::core_runtime::profiles::CandidateProtocolProfile,
}

impl SealedCandidateExecutionV1 {
    pub(crate) fn credential_binding_id(&self) -> &str {
        self.source_binding_id
            .as_deref()
            .unwrap_or(&self.stable_target_key)
    }
}

pub(crate) const SEALED_CANDIDATE_EXECUTION_SCHEMA_V1: &str =
    "hiroute.gateway.sealed-candidate-execution/v1";

#[cfg(test)]
mod tests;

#[cfg(test)]
mod no_new_calls_tests;

// Shared native-consumer fixture uses the production publication/authentication path.
#[cfg(test)]
pub(crate) fn authorized_native_consumer_fixture(
    consumer: Option<hiroute_domain::AgentModelConsumerV1>,
) -> crate::server::request_plan::AuthorizedRequestPlan {
    let directory = tests::TestDirectory::new();
    let installer = std::sync::Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap(),
    );
    let mut publication = tests::snapshot(1, "native-consumer-test");
    publication.grants[0].consumer = consumer;
    publication.payload_digest = publication.canonical_digest().unwrap();
    tests::publish(&installer, publication);
    crate::server::dispatch::GatewayRequestAuthority::new(installer)
        .authorize_bytes(
            crate::server::request_plan::IngressProtocol::Responses,
            Some("Bearer token-alpha"),
            br#"{"model":"alpha"}"#,
            std::time::Instant::now(),
        )
        .unwrap()
}
