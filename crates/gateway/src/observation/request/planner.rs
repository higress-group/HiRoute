use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::runtime::native_endpoint_state_key;
use crate::server::core_runtime::profiles::{PlannerInputV1, PlannerOutputV1};

use super::super::schema::{CandidateDecisionFactV1, ExecutionFactV1};
use super::{CandidateObservation, RequestObservation};

impl RequestObservation {
    pub fn record_planner(&self, input: &PlannerInputV1, output: &PlannerOutputV1) {
        if !self.is_enabled() {
            return;
        }
        let plan_id = match &output.identity {
            crate::server::core_runtime::profiles::PlannerRouteIdentityV2::Plan {
                plan_id, ..
            } => Some(plan_id.clone()),
            crate::server::core_runtime::profiles::PlannerRouteIdentityV2::Fixed { .. } => None,
        };
        self.lock_state().agent_plan_id = plan_id.clone();
        let branch = serde_json::to_value(output.branch).unwrap_or(Value::Null);
        let (outcome, outcome_code) = match output.outcome {
            crate::server::core_runtime::profiles::PlannerOutcomeV1::Ready => {
                ("ready".into(), None)
            }
            crate::server::core_runtime::profiles::PlannerOutcomeV1::NoEligibleCandidates {
                code,
            } => ("no_eligible_candidates".into(), Some(schema_name(&code))),
        };
        self.emit_execution(
            ExecutionFactV1::RouteDecision {
                planner_version: output.planner_version.clone(),
                plan_id,
                route: self.inner.metadata.route.clone(),
                input_digest: output.input_digest.clone(),
                policy_digest: output.policy_digest.clone(),
                output_digest: output.output_digest.clone(),
                branch,
                complexity: output
                    .complexity
                    .as_ref()
                    .and_then(|value| serde_json::to_value(value).ok()),
                groups: serde_json::to_value(&output.groups).unwrap_or(Value::Null),
                reason_ledger: serde_json::to_value(&output.reason_ledger).unwrap_or(Value::Null),
                requirements: input.request.requirements(),
                requested_reasoning_disposition: schema_name(
                    &input.request.requested_reasoning.disposition,
                ),
                requested_reasoning_value: input.request.requested_reasoning.native_value.clone(),
                requested_max_output_tokens: input.request.requested_max_output_tokens,
                stream: input.request.stream,
                outcome,
                outcome_code,
                max_attempts: output.limits.max_attempts,
            },
            None,
        );

        let ranked = output
            .ledger
            .ordered_candidates
            .iter()
            .map(|candidate| (candidate.candidate_id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        let facts = input
            .candidates
            .iter()
            .map(|candidate| (candidate.candidate_id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        let mut candidates = BTreeMap::new();
        for evaluation in &output.ledger.evaluations {
            let candidate = facts.get(evaluation.candidate_id.as_str()).copied();
            let profile = candidate.map(|candidate| &candidate.protocol_profile);
            let ranking_reasons = ranked
                .get(evaluation.candidate_id.as_str())
                .map_or_else(Vec::new, |candidate| {
                    candidate.ranking_reasons.iter().map(schema_name).collect()
                });
            self.emit_execution(
                ExecutionFactV1::CandidateDecision(Box::new(CandidateDecisionFactV1 {
                    candidate_id: evaluation.candidate_id.clone(),
                    stable_binding_id: evaluation.stable_binding_id.clone(),
                    group_id: evaluation.group_id.clone(),
                    declared_order: evaluation.declared_order,
                    profile_digest: evaluation.profile_digest.clone(),
                    ingress_protocol: profile.map_or_else(
                        || "unknown".into(),
                        |profile| schema_name(&profile.ingress_protocol),
                    ),
                    upstream_protocol: profile.map_or_else(
                        || "unknown".into(),
                        |profile| schema_name(&profile.capability.upstream_protocol),
                    ),
                    path_id: profile
                        .map_or_else(|| "unknown".into(), |profile| profile.path_id.clone()),
                    provider_id: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.connector.provider_id.clone(),
                    ),
                    endpoint_id: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.connector.endpoint_id.clone(),
                    ),
                    entitlement_id: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.connector.entitlement_id.clone(),
                    ),
                    connector_id: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.connector.connector_id.clone(),
                    ),
                    connector_revision: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.connector.connector_revision.clone(),
                    ),
                    capability_id: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.capability.capability_id.clone(),
                    ),
                    capability_revision: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.capability.capability_revision.clone(),
                    ),
                    model_configuration_id: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.capability.model_configuration_id.clone(),
                    ),
                    native_model: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.capability.native_model.clone(),
                    ),
                    adapter_revision: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.adapter_revision.clone(),
                    ),
                    serializer_revision: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.serializer_revision.clone(),
                    ),
                    decoder_revision: profile.map_or_else(
                        || "unknown".into(),
                        |profile| profile.decoder_revision.clone(),
                    ),
                    target_serialized_bytes: candidate
                        .map_or(0, |candidate| candidate.target_serialized_bytes),
                    eligible: evaluation.eligible,
                    exclusion_reason: evaluation.first_exclusion.as_ref().map(schema_name),
                    reasoning_profile_id: evaluation.reasoning_profile_id.clone(),
                    overall_score_tenths: evaluation.overall_score_tenths,
                    effective_cost_micros: evaluation.effective_cost_micros,
                    api_equivalent_cost_micros: candidate
                        .and_then(|candidate| candidate.api_equivalent_cost_micros),
                    cost_class: schema_name(&evaluation.cost_class),
                    cache_cost: candidate
                        .and_then(|candidate| serde_json::to_value(candidate.cache_cost).ok())
                        .unwrap_or(Value::Null),
                    cache_affinity: candidate.is_some_and(|candidate| candidate.cache_affinity),
                    compute_scope_order: candidate
                        .map_or(0, |candidate| candidate.compute_scope_order),
                    ranking_reasons,
                })),
                None,
            );
            if let Some(candidate) = candidate {
                let observed = CandidateObservation {
                    candidate_id: candidate.candidate_id.clone(),
                    stable_binding_id: evaluation.stable_binding_id.clone(),
                    declared_order: evaluation.declared_order,
                    profile_digest: candidate.profile_digest.clone(),
                    provider_name: candidate.protocol_profile.connector.provider_id.clone(),
                    request_model: candidate.protocol_profile.capability.native_model.clone(),
                    upstream_protocol: schema_name(
                        &candidate.protocol_profile.capability.upstream_protocol,
                    ),
                    model_configuration_id: candidate
                        .protocol_profile
                        .capability
                        .model_configuration_id
                        .clone(),
                    adapter_revision: candidate.protocol_profile.adapter_revision.clone(),
                    effective_cost_micros: evaluation.effective_cost_micros,
                    cost_class: schema_name(&evaluation.cost_class),
                    protocol_profile: Some(std::sync::Arc::new(candidate.protocol_profile.clone())),
                    streaming: input.request.stream,
                };
                if candidate.protocol_profile.native_target.is_some()
                    && let Some(key) = native_endpoint_state_key(
                        &evaluation.stable_binding_id,
                        &candidate.profile_digest,
                    )
                {
                    candidates.insert(key, observed.clone());
                }
                candidates.insert(evaluation.stable_binding_id.clone(), observed);
            }
        }
        self.lock_state().candidates = candidates;
    }
}

fn schema_name(value: &(impl Serialize + std::fmt::Debug)) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| match value {
            Value::String(value) => Some(value),
            Value::Object(object) => object
                .get("kind")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            _ => None,
        })
        .unwrap_or_else(|| format!("{value:?}"))
}
