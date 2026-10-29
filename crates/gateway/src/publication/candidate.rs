//! Typed, non-secret candidate facts owned by one compiled execution. Current
//! core config bytes must still match before a request can consume them.
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, CompiledPlannerPolicyV1, CostClassV1, PlannerCandidateFactsV1,
};
use crate::server::request_plan::IngressProtocol;

use super::{CandidateBindingV1, PreparedCandidateExecution, PublicationInstallError};

#[derive(Debug)]
pub(crate) struct CompiledCandidate {
    pub(crate) binding: CandidateBindingV1,
    pub(crate) config_bytes: Arc<[u8]>,
    pub(crate) profiles: BTreeMap<IngressProtocol, PlannerCandidateFactsV1>,
    executions: BTreeMap<IngressProtocol, Arc<PreparedCandidateExecution>>,
}

impl CompiledCandidate {
    pub(crate) fn compile(
        candidate: &CandidateBindingV1,
    ) -> Result<Arc<Self>, PublicationInstallError> {
        let mut binding = candidate.clone();
        // Matches the existing core configuration boundary exactly. Pricing
        // enrichment is observational and cannot authorize an execution.
        binding.pricing_identity = None;
        let config_bytes = serde_json::to_vec(&binding)
            .map_err(PublicationInstallError::Json)?
            .into();
        let mut profiles = BTreeMap::new();
        for canonical in &binding.protocol_profiles {
            let profile: CandidateProtocolProfile = serde_json::from_value(
                serde_json::to_value(canonical).map_err(PublicationInstallError::Json)?,
            )
            .map_err(PublicationInstallError::Json)?;
            let ingress = profile.ingress_protocol;
            let facts = PlannerCandidateFactsV1::seal(
                binding.stable_target_key.clone(),
                binding.stable_target_key.clone(),
                profile,
                1,
                None,
                match binding.billing_class {
                    Some(hiroute_domain::BillingClass::Free) => CostClassV1::Free,
                    Some(hiroute_domain::BillingClass::Subscription) => CostClassV1::Subscription,
                    Some(hiroute_domain::BillingClass::Paid) => CostClassV1::Paid,
                    _ => CostClassV1::Unknown,
                },
                None,
            )
            .map_err(|_| {
                PublicationInstallError::Schema(super::PublicationSchemaError::InvalidCandidate(
                    binding.local_id,
                ))
            })?;
            if profiles.insert(ingress, facts).is_some() {
                return Err(PublicationInstallError::Schema(
                    super::PublicationSchemaError::InvalidCandidate(binding.local_id),
                ));
            }
        }
        let executions = profiles
            .iter()
            .filter_map(|(ingress, facts)| {
                // An invalid envelope retains the existing request-time rejection.
                PreparedCandidateExecution::compile(&binding, facts)
                    .ok()
                    .map(|value| (*ingress, value))
            })
            .collect();
        Ok(Arc::new(Self {
            binding,
            config_bytes,
            profiles,
            executions,
        }))
    }

    pub(crate) fn execution_for(
        &self,
        facts: &PlannerCandidateFactsV1,
        request: &crate::server::core_runtime::model_ir::ModelRequestIRV1,
    ) -> Result<Arc<PreparedCandidateExecution>, Arc<str>> {
        let published = self
            .profiles
            .get(&request.ingress_protocol)
            .ok_or_else(|| Arc::from("protocol_failed"))?;
        if facts.stable_binding_id != self.binding.stable_target_key
            || !published
                .protocol_profile
                .for_request_reasoning(request)
                .is_ok_and(|profile| profile == facts.protocol_profile)
        {
            return Err(Arc::from("protocol_failed"));
        }
        if published.protocol_profile == facts.protocol_profile
            && published.profile_digest == facts.profile_digest
            && let Some(prepared) = self.executions.get(&request.ingress_protocol)
        {
            return Ok(prepared.clone());
        }
        PreparedCandidateExecution::compile(&self.binding, facts)
    }
}

/// An immutable projection; Planner receives no config, credential or runtime
/// state port. Only records produced by publication compilation reach it.
pub(crate) struct PublishedPlannerFacts<'a> {
    policy: &'a CompiledPlannerPolicyV1,
    candidates: &'a BTreeMap<u32, Arc<CompiledCandidate>>,
}

impl<'a> PublishedPlannerFacts<'a> {
    pub(crate) fn from_compiled(
        policy: &'a CompiledPlannerPolicyV1,
        candidates: &'a BTreeMap<u32, Arc<CompiledCandidate>>,
    ) -> Self {
        Self { policy, candidates }
    }

    pub(crate) fn policy_matches(&self, policy: &CompiledPlannerPolicyV1) -> bool {
        self.policy == policy
    }

    pub(crate) fn profile_matches(
        &self,
        candidate: &PlannerCandidateFactsV1,
        ingress: IngressProtocol,
    ) -> bool {
        self.candidates
            .values()
            .find(|compiled| compiled.binding.stable_target_key == candidate.stable_binding_id)
            .and_then(|compiled| compiled.profiles.get(&ingress))
            .is_some_and(|published| {
                published.protocol_profile == candidate.protocol_profile
                    && published.profile_digest == candidate.profile_digest
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::composition::{PlannerInputAuthority, PublicationPlannerInputAuthority};
    use crate::server::core_runtime::{
        adapters,
        profiles::{Planner, PlannerError},
    };
    use serde_json::json;

    #[test]
    fn duplicate_ingress_profiles_cannot_become_a_last_wins_compiled_binding() {
        let authorized = super::super::authorized_native_consumer_fixture(None);
        let binding = authorized.core_binding().candidate_bindings().unwrap()[0];
        let attempt = authorized.core_binding().resolve_attempt(binding).unwrap();
        let configs = attempt.acquire_attempt_configs().unwrap();
        let config = configs.value(attempt.plan().config_cell_ids[0]).unwrap();
        let mut candidate = authorized
            .candidate_config(binding.local_id(), config)
            .unwrap()
            .binding
            .clone();
        candidate
            .protocol_profiles
            .push(candidate.protocol_profiles[0].clone());
        assert!(CompiledCandidate::compile(&candidate).is_err());
    }

    #[test]
    fn an_active_request_does_not_pin_unrelated_compiled_candidate_generations() {
        use super::super::{
            GatewayPublicationInstaller,
            tests::{TestDirectory, publish, snapshot},
        };
        let directory = TestDirectory::new();
        let installer = Arc::new(
            GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap(),
        );
        publish(&installer, snapshot(1, "first"));
        let original = installer.active().unwrap();
        let beta = Arc::downgrade(
            original.aliases["beta"]
                .execution
                .candidate_configs
                .get(&2)
                .unwrap(),
        );
        let authorized = crate::server::dispatch::GatewayRequestAuthority::new(installer.clone())
            .authorize_bytes(
                IngressProtocol::Responses,
                Some("Bearer token-alpha"),
                br#"{"model":"alpha"}"#,
                std::time::Instant::now(),
            )
            .unwrap();
        publish(&installer, snapshot(2, "second"));
        drop(original);
        assert!(
            beta.upgrade().is_none(),
            "an alpha request retained the old beta facts"
        );
        assert_eq!(authorized.publication_revision(), 1);
    }

    #[test]
    fn published_planning_matches_full_validation_and_rejects_changed_profiles_and_policy() {
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
        let published = authorized.planner_facts();
        assert!(published.policy_matches(&input.policy));
        assert!(
            input
                .candidates
                .iter()
                .all(|candidate| published
                    .profile_matches(candidate, input.request.ingress_protocol))
        );
        let expected = Planner.plan(&input).unwrap();
        let actual = Planner.plan_published(&input, &published).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        let mut changed = input.clone();
        changed.policy.limits.max_attempts += 1;
        assert!(matches!(
            Planner.plan_published(&changed, &published),
            Err(PlannerError::PolicyDigestMismatch)
        ));
        let mut changed = input;
        changed.candidates[0]
            .protocol_profile
            .capability
            .native_model = "different-model".into();
        assert!(matches!(
            Planner.plan_published(&changed, &published),
            Err(PlannerError::ProfileDigestMismatch(_))
        ));
    }

    #[test]
    fn compiled_candidate_requires_the_exact_current_core_config_and_generation() {
        use hiroute_gateway_core::core::execution_plan::{ConfigGeneration, ImmutableConfig};
        let authorized = super::super::authorized_native_consumer_fixture(None);
        for binding in authorized.core_binding().candidate_bindings().unwrap() {
            let attempt = authorized.core_binding().resolve_attempt(*binding).unwrap();
            let configs = attempt.acquire_attempt_configs().unwrap();
            let config = configs.value(attempt.plan().config_cell_ids[0]).unwrap();
            let compiled = authorized
                .candidate_config(binding.local_id(), config)
                .unwrap();
            let oracle: CandidateBindingV1 = serde_json::from_slice(&config.bytes).unwrap();
            assert_eq!(
                serde_json::to_value(&compiled.binding).unwrap(),
                serde_json::to_value(oracle).unwrap()
            );
            let mut changed = ImmutableConfig {
                generation: ConfigGeneration(config.generation.0 + 1),
                compatibility_hash: config.compatibility_hash,
                bytes: config.bytes.clone(),
            };
            assert!(
                authorized
                    .candidate_config(binding.local_id(), &changed)
                    .is_none()
            );
            changed.generation = config.generation;
            let mut bytes = config.bytes.to_vec();
            bytes.push(b' ');
            changed.bytes = bytes.into();
            assert!(
                authorized
                    .candidate_config(binding.local_id(), &changed)
                    .is_none()
            );
        }
    }
}
