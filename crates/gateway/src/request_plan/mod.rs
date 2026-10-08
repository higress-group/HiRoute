//! Request-owned authority established before any Provider side effect.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hiroute_domain::{CanonicalDigest, PriceBillingContextV1, UsageSemanticsV1};
use hiroute_gateway_core::core::execution_plan::{
    CompiledAcceptedResponsePlanHandle, CompiledLogicalRequestPlanHandle,
    CompiledRequestPlanHandle, RequestExecutionBinding, TransportTarget,
};
use serde::{Deserialize, Serialize};

use crate::server::core_runtime::profiles::CompiledPlannerPolicyV1;

/// Public ingress protocols supported by the shared AccessPoint.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressProtocol {
    Responses,
    ChatCompletions,
    Messages,
}

impl IngressProtocol {
    pub fn from_path(path: &str) -> Option<Self> {
        if let Some(resource) = path.strip_prefix(hiroute_domain::QODER_MODEL_BASE_PATH) {
            return match resource {
                "/responses" => Some(Self::Responses),
                "/messages" => Some(Self::Messages),
                _ => None,
            };
        }
        match path {
            "/v1/responses" => Some(Self::Responses),
            "/v1/chat/completions" => Some(Self::ChatCompletions),
            "/v1/messages" => Some(Self::Messages),
            _ => None,
        }
    }

    pub const fn path(self) -> &'static str {
        match self {
            Self::Responses => "/v1/responses",
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Messages => "/v1/messages",
        }
    }
}

/// Stable, body-free identity copied into receipts and observations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RequestAuthorityReceipt {
    pub schema_version: &'static str,
    pub workspace_id: Arc<str>,
    pub authority_id: Arc<str>,
    pub authority_epoch: u64,
    pub publication_revision: u64,
    pub publication_digest: Arc<str>,
    pub route: hiroute_domain::ModelRequestRouteV2,
    pub plan_display_name: Option<Arc<str>>,
    pub served_model_id: Arc<str>,
    pub grant_id: Arc<str>,
    pub grant_generation: u64,
    pub ingress_protocol: IngressProtocol,
    pub max_attempts: u32,
    pub overall_timeout_ms: u64,
}

/// Non-secret, alias-owned candidate facts handed to Provider dispatch. The
/// endpoint has not been DNS-resolved and the CredentialRef has not been
/// materialized when this object is created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCandidateAuthority {
    pub binding_local_id: u32,
    pub endpoint: Arc<str>,
    pub credential_refs: Arc<[Arc<str>]>,
}

/// Exact, non-secret pricing identity projected from the request's pinned
/// publication. The mutable price generation is deliberately not part of V.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestPriceBindingV1 {
    pub stable_binding_id: Arc<str>,
    pub model_configuration_id: Arc<str>,
    pub profile_digest: Arc<str>,
    pub source_id: Arc<str>,
    pub source_identity_digest: CanonicalDigest,
    pub actual_offer_ref: Arc<str>,
    pub usage_semantics: UsageSemanticsV1,
    pub billing_context: PriceBillingContextV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClassifierAuthenticationAuthorityV1 {
    None,
    Header {
        name: Arc<str>,
        value_secret_ref: Arc<str>,
    },
}

/// Exact, publication-pinned authority for one external classification call.
/// It contains no secret material and cannot authorize an ordinary model route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RestBranchClassifierAuthorityV1 {
    pub(crate) endpoint: Arc<str>,
    pub(crate) http_authority: Arc<str>,
    pub(crate) request_path: Arc<str>,
    pub(crate) timeout: Duration,
    pub(crate) transport_target: TransportTarget,
    pub(crate) authentication: ClassifierAuthenticationAuthorityV1,
    pub(crate) decision: hiroute_domain::DecisionDefinitionV1,
    pub(crate) smart_judgment: Option<hiroute_domain::JudgmentSettingsV1>,
    pub(crate) system_one_model: Option<String>,
    pub(crate) branch_policies: Vec<hiroute_domain::MaterializedBranchV1>,
}

/// Gateway-internal, non-secret provenance copied only from an authenticated
/// delegated-run authority. Request bodies, headers other than the run bearer,
/// and ambient process state cannot construct this value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedRunObservationContext {
    pub(crate) task_id: String,
    pub(crate) run_id: String,
    pub(crate) plan_id: String,
    pub(crate) plan_revision: u64,
    pub(crate) publication_ref: String,
    pub(crate) harness_id: String,
    pub(crate) native_session_id: Option<String>,
    pub(crate) parent_context_ref: Option<String>,
    pub(crate) continued_from_run_id: Option<String>,
}

/// Exact alias-owned plan frozen for one logical request.
///
/// It intentionally does not retain the aggregate publication root. The core
/// binding has already narrowed itself to the selected route's candidate,
/// logical-request and accepted-response closure, so long streams cannot pin
/// unrelated aliases or grants.
#[derive(Debug)]
pub struct AuthorizedRequestPlan {
    web_search_allowed: bool,
    verified_run_observation: Option<VerifiedRunObservationContext>,
    receipt: RequestAuthorityReceipt,
    deadline: Instant,
    compiled: CompiledRequestPlanHandle,
    planner_policy: Arc<CompiledPlannerPolicyV1>,
    classifier: Option<Arc<RestBranchClassifierAuthorityV1>>,
    candidates: Arc<[ProviderCandidateAuthority]>,
    pricing_bindings: Arc<[RequestPriceBindingV1]>,
    core: RequestExecutionBinding,
}

impl AuthorizedRequestPlan {
    #[allow(clippy::too_many_arguments)] // Exact authority components are intentionally explicit.
    pub(crate) fn new(
        receipt: RequestAuthorityReceipt,
        verified_run_observation: Option<VerifiedRunObservationContext>,
        deadline: Instant,
        compiled: CompiledRequestPlanHandle,
        planner_policy: Arc<CompiledPlannerPolicyV1>,
        classifier: Option<Arc<RestBranchClassifierAuthorityV1>>,
        candidates: Arc<[ProviderCandidateAuthority]>,
        pricing_bindings: Arc<[RequestPriceBindingV1]>,
        core: RequestExecutionBinding,
    ) -> Self {
        Self {
            web_search_allowed: false,
            verified_run_observation,
            receipt,
            deadline,
            compiled,
            planner_policy,
            classifier,
            candidates,
            pricing_bindings,
            core,
        }
    }

    pub fn receipt(&self) -> &RequestAuthorityReceipt {
        &self.receipt
    }

    pub(crate) fn with_web_search_allowed(mut self, allowed: bool) -> Self {
        self.web_search_allowed = allowed;
        self
    }

    pub(crate) fn web_search_allowed(&self) -> bool {
        self.web_search_allowed
    }

    pub(crate) fn verified_run_observation(&self) -> Option<&VerifiedRunObservationContext> {
        self.verified_run_observation.as_ref()
    }

    pub fn publication_revision(&self) -> u64 {
        self.receipt.publication_revision
    }

    pub fn agent_plan_revision(&self) -> Option<u64> {
        self.receipt.route.plan_revision()
    }

    pub fn served_model_id(&self) -> &str {
        &self.receipt.served_model_id
    }

    pub fn max_attempts(&self) -> u32 {
        self.receipt.max_attempts
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    pub fn logical_request_plan(&self) -> CompiledLogicalRequestPlanHandle {
        Arc::clone(&self.compiled.logical_request)
    }

    pub fn accepted_response_plan(&self) -> CompiledAcceptedResponsePlanHandle {
        Arc::clone(&self.compiled.accepted_response)
    }

    pub fn planner_policy(&self) -> &Arc<CompiledPlannerPolicyV1> {
        &self.planner_policy
    }

    pub(crate) fn classifier(&self) -> Option<&Arc<RestBranchClassifierAuthorityV1>> {
        self.classifier.as_ref()
    }

    pub fn candidates(&self) -> &[ProviderCandidateAuthority] {
        &self.candidates
    }

    pub fn pricing_bindings(&self) -> &[RequestPriceBindingV1] {
        &self.pricing_bindings
    }

    pub fn core_binding(&self) -> &RequestExecutionBinding {
        &self.core
    }

    pub fn core_binding_mut(&mut self) -> &mut RequestExecutionBinding {
        &mut self.core
    }

    pub fn into_core_binding(self) -> RequestExecutionBinding {
        self.core
    }
}
