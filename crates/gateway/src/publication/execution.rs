//! Immutable execution facts. The only untrusted entry validates the complete
//! sealed envelope; publication builds the same envelope once per base profile.
use std::sync::Arc;

use crate::server::core_runtime::profiles::{CostClassV1, PlannerCandidateFactsV1};

use super::{CandidateBindingV1, SealedCandidateExecutionV1, lease_target};

#[derive(Debug)]
pub(crate) struct PreparedCandidateExecution {
    execution: SealedCandidateExecutionV1,
    bytes: Arc<[u8]>,
    lease_target: hiroute_domain::GatewayOperationalTargetV1,
    lease_target_digest: hiroute_domain::CanonicalDigest,
    retained_bytes: Option<usize>,
}

impl PreparedCandidateExecution {
    pub(super) fn compile(
        binding: &CandidateBindingV1,
        facts: &PlannerCandidateFactsV1,
    ) -> Result<Arc<Self>, Arc<str>> {
        let execution = SealedCandidateExecutionV1 {
            schema_version: super::SEALED_CANDIDATE_EXECUTION_SCHEMA_V1.into(),
            stable_target_key: binding.stable_target_key.clone(),
            source_binding_id: binding.source_binding_id.clone(),
            credential_destination_ref: binding.credential_destination_ref.clone(),
            logical_endpoint: binding.endpoint.clone(),
            upstream_model_id: binding.upstream_model_id.clone(),
            native_transport_model: binding.native_transport_model.clone(),
            connector_runtime: binding.connector_runtime,
            operational_target: binding.operational_target.clone(),
            operational_target_digest: binding.operational_target_digest.clone(),
            protocol_set_digest: binding.protocol_profile_digest.clone(),
            profile_digest: facts.profile_digest.clone(),
            protocol_profile: facts.protocol_profile.clone(),
        };
        let bytes = serde_json::to_vec(&execution)
            .map_err(|_| Arc::from("protocol_failed"))?
            .into();
        Self::validated(execution, bytes)
    }

    pub(crate) fn from_untrusted_bytes(bytes: &Arc<[u8]>) -> Result<Arc<Self>, Arc<str>> {
        let execution = serde_json::from_slice(bytes).map_err(|_| Arc::from("protocol_failed"))?;
        Self::validated(execution, bytes.clone())
    }

    fn validated(
        execution: SealedCandidateExecutionV1,
        bytes: Arc<[u8]>,
    ) -> Result<Arc<Self>, Arc<str>> {
        let profile = &execution.protocol_profile;
        let digest = PlannerCandidateFactsV1::seal(
            "digest-check",
            "digest-check",
            profile.clone(),
            1,
            None,
            CostClassV1::Free,
            None,
        )
        .map_err(|_| Arc::from("protocol_failed"))?
        .profile_digest;
        if execution.schema_version != super::SEALED_CANDIDATE_EXECUTION_SCHEMA_V1
            || execution.credential_destination_ref.trim().is_empty()
            || execution.logical_endpoint.trim().is_empty()
            || execution.upstream_model_id.trim().is_empty()
            || execution.native_transport_model.trim().is_empty()
            || profile.capability.native_model != execution.native_transport_model
            || (execution.connector_runtime == hiroute_domain::ConnectorRuntimeKind::BuiltinNative
                && execution.upstream_model_id != execution.native_transport_model)
            || digest != execution.profile_digest
            || (execution.connector_runtime != hiroute_domain::ConnectorRuntimeKind::CpaBridge
                && profile.native_target.is_none()
                && execution
                    .operational_target
                    .for_protocol_path(&profile.connector.request_path)
                    .is_none())
            || !execution
                .operational_target
                .validate_for(execution.connector_runtime, &execution.logical_endpoint)
            || !matches!(hiroute_domain::CanonicalDigest::of(&execution.operational_target),
                Ok(digest) if digest == execution.operational_target_digest)
            || hiroute_domain::CanonicalDigest::parse(execution.protocol_set_digest.as_str())
                .is_err()
            || profile.connector.authentication.exact().is_none()
        {
            return Err(Arc::from("protocol_failed"));
        }
        let lease_target = lease_target::for_request(&execution, profile)?;
        let lease_target_digest = hiroute_domain::CanonicalDigest::of(&lease_target)
            .map_err(|_| Arc::from("protocol_failed"))?;
        // Reuse ingress's structural bound, plus encoded/string storage and
        // fixed DTO/map overhead. A sizing refusal only disables reuse.
        let retained_bytes = crate::content_ref::inline_document_retention_bytes(&bytes);
        Ok(Arc::new(Self {
            execution,
            bytes,
            lease_target,
            lease_target_digest,
            retained_bytes,
        }))
    }

    pub(crate) fn execution(&self) -> &SealedCandidateExecutionV1 {
        &self.execution
    }

    pub(crate) fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    pub(crate) fn lease_target(&self) -> &hiroute_domain::GatewayOperationalTargetV1 {
        &self.lease_target
    }

    pub(crate) fn lease_target_digest(&self) -> &hiroute_domain::CanonicalDigest {
        &self.lease_target_digest
    }

    // Sharing a publication allocation does not waive request-local accounting.
    pub(crate) fn retained_bytes(&self) -> Option<usize> {
        self.retained_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::composition::{
        PlannerInputAuthority, PublicationPlannerInputAuthority, RequestTemplates,
    };
    use crate::server::core_runtime::{adapters, profiles::PlannerInputV1};
    use crate::server::request_plan::IngressProtocol;
    use hiroute_gateway_core::runtime::body::BudgetTree;
    use serde_json::json;

    fn fixture() -> (Arc<super::super::CompiledCandidate>, PlannerInputV1) {
        let authorized = super::super::authorized_native_consumer_fixture(None);
        let input = PublicationPlannerInputAuthority
            .build_input(
                adapters::decode_ingress_request(
                    IngressProtocol::Responses,
                    &json!({"model":"alpha","input":"hello"}),
                )
                .unwrap(),
                &authorized,
            )
            .unwrap();
        let binding = authorized.core_binding().candidate_bindings().unwrap()[0];
        let attempt = authorized.core_binding().resolve_attempt(binding).unwrap();
        let configs = attempt.acquire_attempt_configs().unwrap();
        let config = configs.value(attempt.plan().config_cell_ids[0]).unwrap();
        (
            super::super::CompiledCandidate::compile(
                &authorized
                    .candidate_config(binding.local_id(), config)
                    .unwrap()
                    .binding,
            )
            .unwrap(),
            input,
        )
    }

    #[test]
    fn compiled_execution_and_untrusted_envelope_have_identical_facts() {
        let (compiled, input) = fixture();
        let facts = &input.candidates[0];
        let first = compiled.execution_for(facts, &input.request).unwrap();
        let second = compiled.execution_for(facts, &input.request).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "base profile is compiled once"
        );
        let checked = PreparedCandidateExecution::from_untrusted_bytes(first.bytes()).unwrap();
        assert_eq!(
            serde_json::to_value(first.execution()).unwrap(),
            serde_json::to_value(checked.execution()).unwrap()
        );
        assert_eq!(first.lease_target(), checked.lease_target());
        assert_eq!(first.lease_target_digest(), checked.lease_target_digest());
        let binding = &compiled.binding;
        let body: serde_json::Value = serde_json::from_slice(first.bytes()).unwrap();
        assert_eq!(
            body["credential_destination_ref"],
            binding.credential_destination_ref
        );
        assert_eq!(body["logical_endpoint"], binding.endpoint);
        assert_eq!(
            body["native_transport_model"],
            binding.native_transport_model
        );
        assert_eq!(body["profile_digest"], facts.profile_digest);
        assert_eq!(
            body["protocol_profile"],
            serde_json::to_value(&facts.protocol_profile).unwrap()
        );
    }

    #[test]
    fn untrusted_profile_and_target_changes_still_require_complete_validation() {
        let (compiled, input) = fixture();
        let first = compiled
            .execution_for(&input.candidates[0], &input.request)
            .unwrap();
        let original: serde_json::Value = serde_json::from_slice(first.bytes()).unwrap();
        for field in [
            "schema_version",
            "credential_destination_ref",
            "logical_endpoint",
            "upstream_model_id",
            "native_transport_model",
            "operational_target_digest",
            "protocol_set_digest",
            "profile_digest",
        ] {
            let mut changed = original.clone();
            changed[field] = json!("");
            let bytes: Arc<[u8]> = serde_json::to_vec(&changed).unwrap().into();
            assert!(
                PreparedCandidateExecution::from_untrusted_bytes(&bytes).is_err(),
                "{field}"
            );
        }
        let mut changed = original;
        changed["protocol_profile"]["capability"]["native_model"] = json!("other-model");
        let bytes: Arc<[u8]> = serde_json::to_vec(&changed).unwrap().into();
        assert!(PreparedCandidateExecution::from_untrusted_bytes(&bytes).is_err());
        let mut facts = input.candidates[0].clone();
        facts.protocol_profile.capability.native_model = "other-model".into();
        assert!(compiled.execution_for(&facts, &input.request).is_err());
        let mut facts = input.candidates[0].clone();
        facts.profile_digest = "changed".into();
        assert!(compiled.execution_for(&facts, &input.request).is_err());
    }

    #[test]
    fn request_execution_reuse_requires_the_exact_sealed_allocation_and_releases_its_charge() {
        let (compiled, input) = fixture();
        let prepared = compiled
            .execution_for(&input.candidates[0], &input.request)
            .unwrap();
        let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
            .unwrap()
            .stream(1024 * 1024)
            .unwrap();
        let initial = budget.snapshot().unwrap().live;
        let mut templates = RequestTemplates::new(budget.clone());
        templates.insert_execution(compiled.binding.local_id, prepared.clone());
        assert!(budget.snapshot().unwrap().live > initial);
        assert!(
            templates
                .execution(compiled.binding.local_id, prepared.bytes())
                .is_some()
        );
        assert!(
            templates
                .execution(compiled.binding.local_id + 1, prepared.bytes())
                .is_none()
        );
        let copied: Arc<[u8]> = prepared.bytes().to_vec().into();
        assert!(
            templates
                .execution(compiled.binding.local_id, &copied)
                .is_none()
        );
        assert!(PreparedCandidateExecution::from_untrusted_bytes(&copied).is_ok());
        drop(templates);
        assert_eq!(budget.snapshot().unwrap().live, initial);
        let small = BudgetTree::new(4096, 4096).unwrap().stream(4096).unwrap();
        let mut declined = RequestTemplates::new(small.clone());
        declined.insert_execution(compiled.binding.local_id, prepared.clone());
        assert!(
            declined
                .execution(compiled.binding.local_id, prepared.bytes())
                .is_none()
        );
        assert_eq!(small.snapshot().unwrap().live, 0);
    }
}
