//! Read-only explanations accompany saved model identities; they never become execution facts.
use std::collections::BTreeMap;

use hiroute_application::compiler::CandidateCompilationFactV1;
use hiroute_application::compute_management::{
    ComputeManagementCompilationErrorV2, ComputeManagementCompilationFactV2,
    compile_compute_management_source,
};
use hiroute_application_api::{
    PlanCandidateUnavailableReasonV1 as Reason, PlanUnavailableCandidateV1,
};
use hiroute_domain::{
    ComputeInventorySnapshotV1, ComputeManagedModelV2, ComputeManagementSourceV2, ComputeSourceV1,
    MaterializationState, SourceBindingV1, valid_upstream_model_id,
};
use hiroute_integrations::TrustedReleaseCatalog;

pub(super) struct CandidateDiagnostics(BTreeMap<String, PlanUnavailableCandidateV1>);

impl CandidateDiagnostics {
    pub(super) fn new(
        sources: &[ComputeManagementSourceV2],
        rows: &[(ComputeSourceV1, SourceBindingV1, ComputeInventorySnapshotV1)],
        catalog: &TrustedReleaseCatalog,
    ) -> Self {
        let mut result = Self(BTreeMap::new());
        for (source, binding, _) in rows {
            let name = catalog
                .model_data()
                .model(&binding.model_configuration_id)
                .map_or(binding.upstream_model_id.as_str(), |model| {
                    model.display_name.as_str()
                });
            result.insert(
                &binding.binding_id,
                name,
                source_reason(source.state).unwrap_or(Reason::InvalidConfiguration),
            );
        }
        for source in sources {
            for model in &source.models {
                result.model(
                    model,
                    model_reason(source, model).unwrap_or(Reason::InvalidConfiguration),
                );
            }
        }
        result
    }

    fn insert(&mut self, id: &str, name: &str, reason: Reason) {
        self.0.insert(
            id.to_owned(),
            PlanUnavailableCandidateV1 {
                binding_id: id.to_owned(),
                display_name: name.chars().filter(|c| !c.is_control()).take(128).collect(),
                reason,
            },
        );
    }

    fn model(&mut self, model: &ComputeManagedModelV2, reason: Reason) {
        self.insert(&model.binding_id, &model.display_name, reason);
    }

    pub(super) fn legacy_reason(&mut self, binding_id: &str, reason: Reason) {
        if let Some(candidate) = self.0.get_mut(binding_id) {
            candidate.reason = reason;
        }
    }

    pub(super) fn catalog_mismatch(&mut self, source: &ComputeManagementSourceV2) {
        for model in &source.models {
            self.model(
                model,
                model_reason(source, model).unwrap_or(Reason::CatalogMismatch),
            );
        }
    }

    pub(super) fn compile_models(
        &mut self,
        source: &ComputeManagementSourceV2,
    ) -> Vec<ComputeManagementCompilationFactV2> {
        let mut facts = Vec::new();
        // Validate each model independently: a retained invalid/unsupported row cannot poison
        // another model's eligibility. Source authorization and credential checks still run.
        let mut selected = source.clone();
        for model in &source.models {
            if let Some(reason) = model_reason(source, model) {
                self.model(model, reason);
                continue;
            }
            selected.models = vec![model.clone()];
            match compile_compute_management_source(&selected) {
                Ok(models) => facts.extend(models),
                Err(error) => self.model(
                    model,
                    match error {
                        ComputeManagementCompilationErrorV2::InvalidSource => {
                            Reason::InvalidConfiguration
                        }
                        ComputeManagementCompilationErrorV2::NotReady => Reason::SourceNotReady,
                        ComputeManagementCompilationErrorV2::UnknownCapability => {
                            Reason::CapabilityUnavailable
                        }
                        ComputeManagementCompilationErrorV2::EligibilityMismatch => {
                            Reason::ModelNotEligible
                        }
                        ComputeManagementCompilationErrorV2::CredentialMissing
                        | ComputeManagementCompilationErrorV2::AuthorizationMissing => {
                            Reason::CredentialUnavailable
                        }
                    },
                ),
            }
        }
        facts
    }

    pub(super) fn materialization_failed(
        &mut self,
        fact: &ComputeManagementCompilationFactV2,
        reason: Reason,
    ) {
        // The caller supplies a closed source-specific diagnosis; other preparation failures
        // do not establish a provider/network/credential failure.
        self.insert(&fact.binding_id, &fact.display_name, reason);
    }

    pub(super) fn finish(
        mut self,
        candidates: &[CandidateCompilationFactV1],
    ) -> Vec<PlanUnavailableCandidateV1> {
        for fact in candidates {
            if fact.is_routable() {
                self.0.remove(&fact.binding.binding_id);
            } else {
                self.insert(
                    &fact.binding.binding_id,
                    &fact.model.display_name,
                    source_reason(fact.source_state).unwrap_or(Reason::ModelNotEligible),
                );
            }
        }
        self.0.into_values().collect()
    }
}

fn source_reason(state: MaterializationState) -> Option<Reason> {
    match state {
        MaterializationState::Ready => None,
        MaterializationState::Disabled => Some(Reason::SourceNotReady),
        MaterializationState::NeedsCredential | MaterializationState::NeedsAuthorization => {
            Some(Reason::CredentialUnavailable)
        }
    }
}

fn model_reason(
    source: &ComputeManagementSourceV2,
    model: &ComputeManagedModelV2,
) -> Option<Reason> {
    if !valid_upstream_model_id(&model.upstream_model_id) {
        Some(Reason::InvalidModelId)
    } else if let Some(reason) = source_reason(source.state) {
        Some(reason)
    } else if !model.execution_eligible {
        Some(Reason::ModelNotEligible)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::{COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2, CanonicalDigest};
    use serde_json::json;

    fn source(ids: &[&str]) -> ComputeManagementSourceV2 {
        let evidence = CanonicalDigest::of_bytes(b"retained-source-evidence");
        serde_json::from_value(json!({
            "schema": COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2,
            "source_id": "source/retained",
            "revision": 1,
            "lineage_digest": evidence,
            "display_name": "Retained user source",
            "provenance": {"kind": "user_configured", "configuration_revision": 1, "evidence_digest": evidence},
            "target": {"scheme":"http", "authority":"127.0.0.1", "port":8123,
                "request_path":"/v1/responses", "upstream_protocol":"responses",
                "protocol_profile_id":"profile/custom/responses", "protocol_profile_revision":1},
            "authentication": {"kind":"none"},
            "state": "ready",
            "models": ids.iter().enumerate().map(|(i, id)| json!({
                "model_ref":format!("model/retained-{i}"), "binding_id":format!("binding/retained-{i}"),
                "revision":1, "upstream_model_id":id, "display_name":format!("Retained model {i}"),
                "membership":"user_declared", "execution_eligible":true,
                "capabilities":{
                    "tool":{"value":true,"basis":"user_declared"},
                    "vision":{"value":false,"basis":"user_declared"},
                    "streaming":{"value":true,"basis":"user_declared"},
                    "context_tokens":{"value":32768,"basis":"user_declared"},
                    "max_output_tokens":{"value":4096,"basis":"user_declared"},
                    "native_reasoning":{"value":{"kind":"fixed","profile":"provider-default"},"basis":"user_declared"}
                },
                "capability_evidence_digest": evidence
            })).collect::<Vec<_>>(),
            "last_candidate_ref":"candidate/retained", "last_candidate_revision":1
        })).unwrap()
    }

    #[test]
    fn retained_invalid_model_is_explained_without_poisoning_a_valid_sibling() {
        let catalog = crate::release_catalog::current_fixture_catalog();
        let source = source(&["bad\tmodel", "内网模型"]);
        // The historical management reader must preserve old bytes for repair.
        source.validate().unwrap();
        let mut diagnostics =
            CandidateDiagnostics::new(std::slice::from_ref(&source), &[], &catalog);
        let facts = diagnostics.compile_models(&source);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].upstream_model_id, "内网模型");
        let candidate =
            crate::control::runtime::candidate_execution::materialize_management_candidate(
                &facts[0], None,
            )
            .unwrap();
        let unavailable = diagnostics.finish(&[candidate]);
        assert_eq!(unavailable.len(), 1);
        assert_eq!(unavailable[0].binding_id, "binding/retained-0");
        assert_eq!(unavailable[0].reason, Reason::InvalidModelId);
    }

    #[test]
    fn unavailable_model_count_does_not_limit_the_execution_snapshot() {
        let catalog = crate::release_catalog::current_fixture_catalog();
        let ids = (0..257)
            .map(|i| format!("unavailable-{i}"))
            .collect::<Vec<_>>();
        let mut disabled = source(&ids.iter().map(String::as_str).collect::<Vec<_>>());
        disabled.state = MaterializationState::Disabled;
        disabled.validate().unwrap();
        let mut valid = source(&["内网模型"]);
        valid.source_id = "source/live".into();
        valid.models[0].binding_id = "binding/live".into();
        valid.models[0].model_ref = "model/live".into();
        let mut diagnostics =
            CandidateDiagnostics::new(&[disabled.clone(), valid.clone()], &[], &catalog);
        assert!(diagnostics.compile_models(&disabled).is_empty());
        let facts = diagnostics.compile_models(&valid);
        let candidate =
            crate::control::runtime::candidate_execution::materialize_management_candidate(
                &facts[0], None,
            )
            .unwrap();
        let unavailable = diagnostics.finish(&[candidate]);
        assert_eq!(unavailable.len(), 257);
        assert!(
            unavailable
                .iter()
                .all(|row| row.reason == Reason::SourceNotReady)
        );
    }
}
