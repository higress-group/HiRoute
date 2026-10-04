//! Aggregate Application → Gateway publication, durable LKG and catalog view.

mod catalog;
mod compiler;
mod installer;
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

pub(crate) const SEALED_CANDIDATE_EXECUTION_SCHEMA_V1: &str =
    "hiroute.gateway.sealed-candidate-execution/v1";

#[cfg(test)]
mod tests;

#[cfg(test)]
mod no_new_calls_tests;
