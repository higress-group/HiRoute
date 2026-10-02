//! Frozen business choices. Execution profiles and their checksums are rebuilt on admission.
use super::*;
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const STORED_PLAN_SCHEMA: &str = "hiroute.stored-plan/v1";
pub const STABLE_ROUTE_SCHEMA: &str = "hiroute.stable-route/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredProtocolChoiceV1 {
    pub ingress_protocol: UpstreamProtocol,
    /// Frozen model/transport guarantees required by the confirmed plan.
    pub capability: StoredModelCapabilitiesV1,
    pub connector: StoredConnectorFactsV1,
    pub native_target: Option<GatewayNativeProfileTargetV2>,
    /// Credential destinations bind this source profile identity. It is not a decoder revision.
    pub credential_profile_identity: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredConnectorFactsV1 {
    pub provider_id: String,
    pub endpoint_id: String,
    pub entitlement_id: String,
    pub connector_id: String,
    pub connector_revision: String,
    pub upstream_protocol: UpstreamProtocol,
    pub request_path: String,
    pub authentication: GatewayCriticalFactV1<GatewayAuthenticationSemanticsV1>,
    pub content_type: String,
    pub required_headers: Vec<(String, String)>,
}

impl StoredProtocolChoiceV1 {
    fn freeze(profile: &GatewayCandidateProtocolProfileV1) -> Result<Self, CompiledPlanError> {
        let headers = profile
            .connector
            .headers
            .exact()
            .ok_or(CompiledPlanError::InvalidCandidate)?;
        Ok(Self {
            ingress_protocol: profile.ingress_protocol,
            capability: StoredModelCapabilitiesV1::freeze(&profile.capability),
            connector: StoredConnectorFactsV1 {
                provider_id: profile.connector.provider_id.clone(),
                endpoint_id: profile.connector.endpoint_id.clone(),
                entitlement_id: profile.connector.entitlement_id.clone(),
                connector_id: profile.connector.connector_id.clone(),
                connector_revision: profile.connector.connector_revision.clone(),
                upstream_protocol: profile.connector.upstream_protocol,
                request_path: profile.connector.request_path.clone(),
                authentication: profile.connector.authentication.clone(),
                content_type: headers.content_type.clone(),
                required_headers: headers.required_headers.clone(),
            },
            native_target: profile.native_target.clone(),
            credential_profile_identity: profile
                .native_target
                .as_ref()
                .map(|_| profile.adapter_revision.clone()),
        })
    }

    fn build(&self) -> Result<GatewayCandidateProtocolProfileV1, CompiledPlanError> {
        if self.native_target.is_some() != self.credential_profile_identity.is_some() {
            return Err(CompiledPlanError::InvalidCandidate);
        }
        let c = &self.connector;
        let mut forbidden = vec!["authorization".to_owned(), "x-api-key".to_owned()];
        if let GatewayCriticalFactV1::Exact(GatewayAuthenticationSemanticsV1::ApiKeyHeader {
            header,
        }) = &c.authentication
            && !forbidden.contains(header)
        {
            forbidden.push(header.clone());
        }
        Ok(GatewayCandidateProtocolProfileV1 {
            schema_version: "hiroute.candidate-protocol-profile/v1".into(),
            path_id: format!(
                "{}-to-{}",
                protocol_name(self.ingress_protocol),
                protocol_name(c.upstream_protocol)
            ),
            ingress_protocol: self.ingress_protocol,
            adapter_revision: self.credential_profile_identity.clone().unwrap_or_else(|| {
                format!("adapter/native/{}@1", protocol_name(c.upstream_protocol))
            }),
            serializer_revision: "hiroute-target-json/v1".into(),
            decoder_revision: "hiroute-native-response/v1".into(),
            capability: self.capability.compile(c.upstream_protocol),
            connector: GatewayConnectorProfileV1 {
                schema_version: "hiroute.connector-profile/v1".into(),
                provider_id: c.provider_id.clone(),
                endpoint_id: c.endpoint_id.clone(),
                entitlement_id: c.entitlement_id.clone(),
                connector_id: c.connector_id.clone(),
                connector_revision: c.connector_revision.clone(),
                upstream_protocol: c.upstream_protocol,
                request_path: c.request_path.clone(),
                authentication: c.authentication.clone(),
                headers: GatewayCriticalFactV1::Exact(GatewayHeaderSemanticsV1 {
                    content_type: c.content_type.clone(),
                    required_headers: c.required_headers.clone(),
                    forbidden_forward_headers: forbidden,
                }),
                errors: GatewayCriticalFactV1::Exact(GatewayErrorSemanticsV1 {
                    http_status_typed: true,
                    sse_error_typed: true,
                    retry_after_header: Some("retry-after".into()),
                }),
            },
            native_target: self.native_target.clone(),
        })
    }
}

fn protocol_name(p: UpstreamProtocol) -> &'static str {
    match p {
        UpstreamProtocol::Responses => "responses",
        UpstreamProtocol::Messages => "messages",
        UpstreamProtocol::ChatCompletions => "chat-completions",
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredCandidateV1 {
    pub binding_id: String,
    pub binding_revision: u64,
    pub binding_digest: CanonicalDigest,
    pub source_id: String,
    pub source_revision: u64,
    pub source_identity_digest: CanonicalDigest,
    pub connection_option_id: String,
    pub offer_ref: String,
    pub offer_revision: u64,
    pub offer_evidence_digest: CanonicalDigest,
    pub billing_class: BillingClass,
    pub model_configuration_id: String,
    pub model_configuration_revision: u64,
    pub upstream_model_id: String,
    pub native_transport_model: String,
    pub capability_id: String,
    pub capability_revision: u64,
    pub capability_evidence_digest: CanonicalDigest,
    pub connector_id: String,
    pub connector_revision: u64,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    pub protocol_endpoint_id: String,
    pub endpoint: String,
    pub connector_runtime: ConnectorRuntimeKind,
    pub operational_target: GatewayOperationalTargetV1,
    pub upstream_protocol: UpstreamProtocol,
    pub credential_refs: Vec<String>,
    pub credential_destination_ref: Option<String>,
    pub credential_pool_id: Option<String>,
    pub free_offer: Option<MaterializedFreeOfferRefV1>,
    pub exact_reasoning: ExactNativeReasoningV1,
    pub protocols: Vec<StoredProtocolChoiceV1>,
}

impl StoredCandidateV1 {
    pub fn freeze(candidate: &AttemptOwnedCandidateV1) -> Result<Self, CompiledPlanError> {
        candidate.validate()?;
        Self::freeze_validated(candidate)
    }

    /// Projection for an already-validated immutable Plan, or for calculating a
    /// provisional digest which the enclosing Plan must still authenticate.
    pub(super) fn freeze_validated(
        candidate: &AttemptOwnedCandidateV1,
    ) -> Result<Self, CompiledPlanError> {
        // Exhaustive destructuring makes a new runtime field an explicit storage decision.
        let AttemptOwnedCandidateV1 {
            binding_id,
            binding_revision,
            binding_digest,
            source_id,
            source_revision,
            source_identity_digest,
            connection_option_id,
            offer_ref,
            offer_revision,
            offer_evidence_digest,
            billing_class,
            model_configuration_id,
            model_configuration_revision,
            upstream_model_id,
            native_transport_model,
            capability_id,
            capability_revision,
            capability_evidence_digest,
            connector_id,
            connector_revision,
            endpoint_profile_id,
            endpoint_profile_revision,
            protocol_endpoint_id,
            endpoint,
            connector_runtime,
            operational_target,
            operational_target_digest,
            protocol_profiles,
            protocol_profile_digest,
            upstream_protocol,
            adapter_ref,
            adapter_revision,
            credential_refs,
            credential_destination_ref,
            credential_pool_id,
            free_offer,
            exact_reasoning,
        } = candidate;
        let _ = (
            operational_target_digest,
            protocol_profile_digest,
            adapter_ref,
            adapter_revision,
        );
        Ok(Self {
            binding_id: binding_id.clone(),
            binding_revision: *binding_revision,
            binding_digest: binding_digest.clone(),
            source_id: source_id.clone(),
            source_revision: *source_revision,
            source_identity_digest: source_identity_digest.clone(),
            connection_option_id: connection_option_id.clone(),
            offer_ref: offer_ref.clone(),
            offer_revision: *offer_revision,
            offer_evidence_digest: offer_evidence_digest.clone(),
            billing_class: *billing_class,
            model_configuration_id: model_configuration_id.clone(),
            model_configuration_revision: *model_configuration_revision,
            upstream_model_id: upstream_model_id.clone(),
            native_transport_model: native_transport_model.clone(),
            capability_id: capability_id.clone(),
            capability_revision: *capability_revision,
            capability_evidence_digest: capability_evidence_digest.clone(),
            connector_id: connector_id.clone(),
            connector_revision: *connector_revision,
            endpoint_profile_id: endpoint_profile_id.clone(),
            endpoint_profile_revision: *endpoint_profile_revision,
            protocol_endpoint_id: protocol_endpoint_id.clone(),
            endpoint: endpoint.clone(),
            connector_runtime: *connector_runtime,
            operational_target: operational_target.clone(),
            upstream_protocol: *upstream_protocol,
            credential_refs: credential_refs.clone(),
            credential_destination_ref: credential_destination_ref.clone(),
            credential_pool_id: credential_pool_id.clone(),
            free_offer: free_offer.clone(),
            exact_reasoning: exact_reasoning.clone(),
            protocols: protocol_profiles
                .iter()
                .map(StoredProtocolChoiceV1::freeze)
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn build(&self) -> Result<AttemptOwnedCandidateV1, CompiledPlanError> {
        let candidate = self.build_for_plan()?;
        candidate.validate()?;
        Ok(candidate)
    }

    // Only StoredPlan calls this assembly path; seal_current validates the whole
    // materialized Plan before an executable value can escape.
    fn build_for_plan(&self) -> Result<AttemptOwnedCandidateV1, CompiledPlanError> {
        let protocol_profiles = self
            .protocols
            .iter()
            .map(StoredProtocolChoiceV1::build)
            .collect::<Result<Vec<_>, _>>()?;
        let candidate = AttemptOwnedCandidateV1 {
            binding_id: self.binding_id.clone(),
            binding_revision: self.binding_revision,
            binding_digest: self.binding_digest.clone(),
            source_id: self.source_id.clone(),
            source_revision: self.source_revision,
            source_identity_digest: self.source_identity_digest.clone(),
            connection_option_id: self.connection_option_id.clone(),
            offer_ref: self.offer_ref.clone(),
            offer_revision: self.offer_revision,
            offer_evidence_digest: self.offer_evidence_digest.clone(),
            billing_class: self.billing_class,
            model_configuration_id: self.model_configuration_id.clone(),
            model_configuration_revision: self.model_configuration_revision,
            upstream_model_id: self.upstream_model_id.clone(),
            native_transport_model: self.native_transport_model.clone(),
            capability_id: self.capability_id.clone(),
            capability_revision: self.capability_revision,
            capability_evidence_digest: self.capability_evidence_digest.clone(),
            connector_id: self.connector_id.clone(),
            connector_revision: self.connector_revision,
            endpoint_profile_id: self.endpoint_profile_id.clone(),
            endpoint_profile_revision: self.endpoint_profile_revision,
            protocol_endpoint_id: self.protocol_endpoint_id.clone(),
            endpoint: self.endpoint.clone(),
            connector_runtime: self.connector_runtime,
            operational_target: self.operational_target.clone(),
            upstream_protocol: self.upstream_protocol,
            credential_refs: self.credential_refs.clone(),
            credential_destination_ref: self.credential_destination_ref.clone(),
            credential_pool_id: self.credential_pool_id.clone(),
            free_offer: self.free_offer.clone(),
            exact_reasoning: self.exact_reasoning.clone(),
            operational_target_digest: CanonicalDigest::of(&self.operational_target)
                .map_err(|_| CompiledPlanError::Encoding)?,
            protocol_profile_digest: CanonicalDigest::of(&protocol_profiles)
                .map_err(|_| CompiledPlanError::Encoding)?,
            protocol_profiles,
            adapter_ref: format!("adapter/native/{}", protocol_name(self.upstream_protocol)),
            adapter_revision: 1,
        };
        Ok(candidate)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredGroupV1 {
    pub group_id: MaterializedGroupId,
    pub ordering_evidence: MaterializedOrderingV1,
    pub pinned_ratings: BTreeMap<String, MaterializedRatingFactV1>,
    pub candidates: Vec<StoredCandidateV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredPlanV1 {
    pub schema: String,
    pub identity: AgentPlanIdentityV1,
    pub revision: u64,
    pub fact_refs: AgentPlanFactRefsV1,
    pub request: RequestOwnedRouteV1,
    pub limits: RoutingLimitsV1,
    pub groups: Vec<StoredGroupV1>,
}

impl StoredPlanV1 {
    pub fn freeze(plan: &CompiledAgentPlanV1) -> Result<Self, CompiledPlanError> {
        plan.validate()?;
        let m = &plan.body.materialized;
        Ok(Self {
            schema: STORED_PLAN_SCHEMA.into(),
            identity: plan.body.identity.clone(),
            revision: plan.body.agent_plan_revision,
            fact_refs: m.fact_refs.clone(),
            request: m.request_owned.clone(),
            limits: m.attempt_owned.limits.clone(),
            groups: m
                .attempt_owned
                .groups
                .iter()
                .map(|g| {
                    Ok(StoredGroupV1 {
                        group_id: g.group_id,
                        ordering_evidence: g.ordering_evidence.clone(),
                        pinned_ratings: g.pinned_ratings.clone(),
                        candidates: g
                            .candidates
                            .iter()
                            .map(StoredCandidateV1::freeze_validated)
                            .collect::<Result<_, _>>()?,
                    })
                })
                .collect::<Result<_, CompiledPlanError>>()?,
        })
    }

    pub fn build(&self) -> Result<CompiledAgentPlanV1, CompiledPlanError> {
        if self.schema != STORED_PLAN_SCHEMA {
            return Err(CompiledPlanError::UnsupportedSchema);
        }
        let m = MaterializedAgentPlanV1 {
            fact_refs: self.fact_refs.clone(),
            request_owned: self.request.clone(),
            attempt_owned: AttemptOwnedRouteV1 {
                limits: self.limits.clone(),
                groups: self
                    .groups
                    .iter()
                    .map(|g| {
                        Ok(MaterializedModelGroupV1 {
                            group_id: g.group_id,
                            ordering_evidence: g.ordering_evidence.clone(),
                            pinned_ratings: g.pinned_ratings.clone(),
                            candidates: g
                                .candidates
                                .iter()
                                .map(StoredCandidateV1::build_for_plan)
                                .collect::<Result<_, _>>()?,
                        })
                    })
                    .collect::<Result<_, CompiledPlanError>>()?,
            },
        };
        CompiledAgentPlanV1::seal_current(CompiledAgentPlanBodyV1 {
            schema: AGENT_PLAN_COMPILED_SCHEMA_V3.into(),
            compiler_revision: AGENT_PLAN_COMPILER_REVISION_V3.into(),
            identity: self.identity.clone(),
            agent_plan_revision: self.revision,
            materialized_route_digest: m.route_digest_for_sealing()?,
            materialized: m,
        })
    }
}

pub mod plan_codec {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        plan: &CompiledAgentPlanV1,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        StoredPlanV1::freeze(plan)
            .map_err(serde::ser::Error::custom)?
            .serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<CompiledAgentPlanV1, D::Error> {
        StoredPlanV1::deserialize(d)?
            .build()
            .map_err(serde::de::Error::custom)
    }
}

/// Fixed-model grant binding codec; runtime candidate layout is never written to a grant.
pub mod binding_codec {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        binding: &AttemptOwnedCandidateV1,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        StoredCandidateV1::freeze(binding)
            .map_err(serde::ser::Error::custom)?
            .serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<Box<AttemptOwnedCandidateV1>, D::Error> {
        Ok(Box::new(
            StoredCandidateV1::deserialize(d)?
                .build()
                .map_err(serde::de::Error::custom)?,
        ))
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    fn fixture() -> CompiledAgentPlanV1 {
        let input: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
        ))
        .unwrap();
        serde_json::from_value::<crate::CompiledAgentPlanV1>(input["plans"][0].clone()).unwrap()
    }
    #[test]
    fn runtime_revision_changes_do_not_change_stable_plan_or_route() {
        let plan = fixture();
        let facts = StoredPlanV1::freeze(&plan).unwrap();
        let mut body = (*plan.body).clone();
        for g in &mut body.materialized.attempt_owned.groups {
            for c in &mut g.candidates {
                c.adapter_ref = "another-internal-adapter".into();
                c.adapter_revision += 1;
                for p in &mut c.protocol_profiles {
                    p.path_id = "internal-path-revision".into();
                    p.serializer_revision = "serializer/revision2".into();
                    p.decoder_revision = "decoder/revision2".into();
                    p.capability.request.text = GatewayFidelityV1::Normalized;
                    p.capability.request.function_tools = GatewayFidelityV1::Unsupported;
                    p.capability.request.image_url = GatewayFidelityV1::Unsupported;
                    p.capability.response.text = GatewayFidelityV1::Normalized;
                    p.capability.response.stream_refusal =
                        GatewayStreamingRefusalSemanticsV1::Unsupported;
                    p.capability.native_provider_state =
                        GatewayNativeProviderStateEmissionV1::Never;
                    p.capability.context.estimator =
                        GatewayCriticalFactV1::Exact(GatewayTokenEstimatorProfileV1 {
                            revision: "estimator/revision2".into(),
                            bytes_per_token: 4,
                            fixed_overhead_tokens: 31,
                        });
                    for reasoning in &mut p.capability.reasoning_profiles {
                        reasoning.accounting = GatewayReasoningAccountingV1::Additive;
                        reasoning.additional_reservation_tokens = 13;
                    }
                }
                c.protocol_profile_digest = CanonicalDigest::of(&c.protocol_profiles).unwrap();
            }
        }
        body.materialized_route_digest = body.materialized.route_digest().unwrap();
        let changed = CompiledAgentPlanV1::seal_current(body).unwrap();
        assert_ne!(plan.digest, changed.digest);
        assert_eq!(facts, StoredPlanV1::freeze(&changed).unwrap());
        assert_eq!(
            plan.body.materialized_route_digest,
            changed.body.materialized_route_digest
        );
        let restored: StoredPlanV1 =
            serde_json::from_slice(&serde_json::to_vec(&facts).unwrap()).unwrap();
        let rebuilt = restored.build().unwrap();
        for g in &rebuilt.body.materialized.attempt_owned.groups {
            for c in &g.candidates {
                for p in &c.protocol_profiles {
                    assert_eq!(p.capability.request.text, GatewayFidelityV1::Exact);
                    assert_eq!(
                        p.capability.context.estimator.exact().unwrap().revision,
                        "byte-upper-bound/v1"
                    );
                    assert_eq!(
                        p.capability,
                        StoredModelCapabilitiesV1::freeze(&p.capability)
                            .compile(p.connector.upstream_protocol)
                    );
                    assert_ne!(
                        p.capability.response.stream_refusal,
                        GatewayStreamingRefusalSemanticsV1::Unsupported
                    );
                    assert!(p.capability.reasoning_profiles.iter().all(|r| r.accounting
                        == GatewayReasoningAccountingV1::WithinOutputCap
                        && r.additional_reservation_tokens == 0));
                }
            }
        }
        let encoded = serde_json::to_value(&facts).unwrap();
        let capability = &encoded["groups"][0]["candidates"][0]["protocols"][0]["capability"];
        for key in ["request", "response", "estimator", "native_provider_state"] {
            assert!(
                capability.get(key).is_none(),
                "derived field {key} was stored"
            );
        }
        assert_eq!(StoredPlanV1::freeze(&rebuilt).unwrap(), facts);
    }
    #[test]
    fn stable_plan_detects_changed_choices_and_rejects_unknown_fields() {
        let facts = StoredPlanV1::freeze(&fixture()).unwrap();
        let mut changed = facts.clone();
        changed.groups[0].candidates[0].credential_refs.reverse();
        changed.groups[0].candidates[0].native_transport_model = "different-model".into();
        assert_ne!(
            CanonicalDigest::of(&facts).unwrap(),
            CanonicalDigest::of(&changed).unwrap()
        );
        assert!(changed.build().is_err());
        let mut raw = serde_json::to_value(&facts).unwrap();
        raw["compiler_revision"] = serde_json::json!("injected");
        assert!(serde_json::from_value::<StoredPlanV1>(raw).is_err());
    }
}
