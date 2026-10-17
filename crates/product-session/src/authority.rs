//! Host effects needed by a product session. Neither transport nor UI supplies grants.
use hiroute_application_api::*;
use std::{
    any::Any,
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

pub enum ApplyPurpose {
    AgentPlan,
    SourcePrice,
}

pub trait HostAuthority: Send + Sync + Any {
    fn as_any(&self) -> &dyn Any;
    fn has_authority(&self) -> bool;
    fn register_operation(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        operation: &'static str,
    ) -> Result<Zeroizing<String>, String>;
    fn register_input(
        &self,
        candidate: &ComputeCandidateRefV2,
        secret: Zeroizing<String>,
    ) -> Result<(), String>;
    fn release_model_input(&self, candidate: &ComputeCandidateRefV2) -> Result<(), String>;
    fn remove_resident_login_item(&self) -> Result<AgentLoginItemDeclarationV2, String> {
        Err("SERVICE_UNAVAILABLE".into())
    }
    fn compensate_resident_login_item_removal(&self) {}
    fn register(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "ApplyAgentPlanChange")
    }
    fn register_for(
        &self,
        purpose: ApplyPurpose,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(
            digest,
            revisions,
            match purpose {
                ApplyPurpose::AgentPlan => "ApplyAgentPlanChange",
                ApplyPurpose::SourcePrice => "ApplyPriceOverrideChange",
            },
        )
    }
    fn register_observation(
        &self,
        request: &ObservationReadRequestV2,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        let digest = CanonicalDigest::of(request).map_err(|_| "OBSERVATION_REQUEST_INVALID")?;
        self.register_operation(&digest, revisions, request.protected_operation())
    }
    fn register_classifier_diagnostic(
        &self,
        request: &ClassifierDecisionTestRequestV1,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        let digest = CanonicalDigest::of(request).map_err(|_| "CLASSIFIER_TEST_INVALID")?;
        self.register_operation(&digest, revisions, "TestClassifierDecision")
    }
    fn register_retention(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        apply: bool,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(
            digest,
            revisions,
            if apply {
                "ApplySessionDeletionV2"
            } else {
                "PreviewSessionDeletionV2"
            },
        )
    }
    fn register_agent_settings(
        &self,
        restore: bool,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(
            digest,
            revisions,
            if restore {
                "ApplyAgentConnectionRestore"
            } else {
                "ApplyAgentConnectionChange"
            },
        )
    }
    fn register_model_input(
        &self,
        secret: Zeroizing<String>,
    ) -> Result<ComputeCandidateRefV2, String> {
        self.create_input("candidate/native/", secret)
    }
    fn register_agent_token_input(
        &self,
        secret: Zeroizing<String>,
    ) -> Result<ComputeCandidateRefV2, String> {
        self.create_input("candidate/native/agent-token-", secret)
    }
    fn create_input(
        &self,
        prefix: &str,
        secret: Zeroizing<String>,
    ) -> Result<ComputeCandidateRefV2, String> {
        if secret.is_empty() || secret.len() > 32 * 1024 || !self.has_authority() {
            return Err("PROTECTED_INPUT_INVALID".into());
        }
        let candidate = ComputeCandidateRefV2 {
            candidate_ref: format!("{prefix}{}", crate::random_id()?),
            candidate_revision: 1,
        };
        self.register_input(&candidate, secret)?;
        Ok(candidate)
    }
    fn register_subscription_callback_input(
        &self,
        candidate: &ComputeCandidateRefV2,
        secret: Zeroizing<String>,
    ) -> Result<(), String> {
        if candidate.validate_shape().is_err()
            || candidate.candidate_revision != 1
            || !candidate
                .candidate_ref
                .starts_with("candidate/subscription-login/")
            || secret.is_empty()
            || secret.len() > 32 * 1024
            || !self.has_authority()
        {
            return Err("PROTECTED_INPUT_INVALID".into());
        }
        self.register_input(candidate, secret)
    }
    fn register_agent_check(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CheckAgentConnection")
    }

    fn register_model_check(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CheckNativeModelConnection")
    }

    fn register_registered_model_check(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CheckRegisteredModelConnection")
    }

    fn register_discovered_model_prepare(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "PrepareDiscoveredModelConnection")
    }

    fn register_saved_model_check(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CheckSavedModelConnection")
    }

    fn register_model_save(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "ApplyComputeSave")
    }

    fn register_subscription_check(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "ApplySubscriptionCheck")
    }

    fn register_subscription_release(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "ReleaseSubscriptionCheck")
    }

    fn register_operation_cancel(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CancelOperation")
    }
}

/// Owns every protected input registered through this session, including a pending prompt.
/// Random references retain the released wire format and cannot collide across sessions.
pub struct SessionAuthority {
    host: Arc<dyn HostAuthority>,
    inputs: Mutex<BTreeMap<String, ComputeCandidateRefV2>>,
}
impl SessionAuthority {
    pub fn new(host: Arc<dyn HostAuthority>) -> Self {
        Self {
            host,
            inputs: Mutex::default(),
        }
    }
}
impl HostAuthority for SessionAuthority {
    fn as_any(&self) -> &dyn Any {
        self.host.as_any()
    }
    fn has_authority(&self) -> bool {
        self.host.has_authority()
    }
    fn register_operation(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        operation: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        self.host.register_operation(digest, revisions, operation)
    }
    fn register_input(
        &self,
        candidate: &ComputeCandidateRefV2,
        secret: Zeroizing<String>,
    ) -> Result<(), String> {
        let mut inputs = self
            .inputs
            .lock()
            .map_err(|_| "PROTECTED_INPUT_UNAVAILABLE")?;
        self.host.register_input(candidate, secret)?;
        inputs.insert(candidate.candidate_ref.clone(), candidate.clone());
        Ok(())
    }
    fn release_model_input(&self, candidate: &ComputeCandidateRefV2) -> Result<(), String> {
        candidate
            .validate_shape()
            .map_err(|_| "PROTECTED_INPUT_INVALID")?;
        let mut inputs = self
            .inputs
            .lock()
            .map_err(|_| "PROTECTED_INPUT_UNAVAILABLE")?;
        if let Some(owned) = inputs.get(&candidate.candidate_ref) {
            // Preparation/checking advances the candidate revision without changing
            // ownership of the protected input. Release the registered input by its
            // session-owned identity, just as the host's release protocol does.
            self.host.release_model_input(owned)?;
            inputs.remove(&candidate.candidate_ref);
        }
        Ok(())
    }
    fn remove_resident_login_item(&self) -> Result<AgentLoginItemDeclarationV2, String> {
        self.host.remove_resident_login_item()
    }
    fn compensate_resident_login_item_removal(&self) {
        self.host.compensate_resident_login_item_removal()
    }
}
impl Drop for SessionAuthority {
    fn drop(&mut self) {
        let inputs = self
            .inputs
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for candidate in inputs.values() {
            let _ = self.host.release_model_input(candidate);
        }
    }
}
