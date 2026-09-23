use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use hiroute_gateway_core::core::execution_plan::{
    AcceptedResponseExecutionBinding, AttemptExecutionBinding, ConfigEventSnapshot, TransportTarget,
};
use hiroute_gateway_core::core::filter::LocalReply;
use hiroute_gateway_core::runtime::attempt::{
    AttemptTimeoutKind, PrecommitEvent, PreparedAttemptHttpRequest, PublishedDisposition,
};
use hiroute_gateway_core::runtime::body::{
    ChargedBodyQueue, ChargedBytes, Reservation, StreamBudget,
};
use hiroute_gateway_core::runtime::driver::{
    AcceptedBodyFrame, AttemptFailureClass as CoreAttemptFailureClass, AttemptFailureFacts,
    AttemptMaterializationContext, AttemptMaterializationFailure, DecisionCandidateAuthority,
    LogicalRequestBodyFrame, LogicalRequestContext, NormalizedAttemptLocalReply, ObservationLabel,
    PinnedConfigContext, PrecommitClassification, ProviderAcceptedEvent, ProviderAttemptCompletion,
    ProviderClassificationFacts, ProviderRuntimePort, UpstreamSideEffectSnapshot,
};
use hiroute_gateway_core::transport::{GatewayRequestHead, GatewayResponseHead};

use crate::attempt_outcome::AttemptFailure;
use crate::content_ref::ContentRef;
use crate::context_hold::{ContextHoldStore, HoldCompletion};
use crate::replay::ReplayStore;
use crate::server::composition::CredentialResolver as ProductionCredentialResolver;
use crate::server::core_runtime::adapters;
use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use crate::server::core_runtime::profiles::CandidateProtocolProfile;
use crate::server::request_plan::IngressProtocol;

use super::state::{
    OwnedAttemptStatePermits, PermitAvailability, RuntimeCooldownPolicy, StatePermit,
    acquire_target_permit_status, confirm_success, record_failure, validate_attempt_permits,
};

mod materialization;
mod response;
mod selection;

pub(crate) use materialization::resolve_target;
pub use selection::{ProductionDecisionSession, ProductionSelection};

const MATERIALIZATION_BINDING_UNAVAILABLE: &str = "binding_unavailable";
const MATERIALIZATION_CREDENTIAL_EXHAUSTED: &str = "credential_exhausted";
const MATERIALIZATION_BINDING_COOLING_PREFIX: &str = "binding_cooling_down:";
const MATERIALIZATION_CREDENTIAL_COOLING_PREFIX: &str = "credential_cooling_down:";
const MATERIALIZATION_BINDING_PROBE_BUSY: &str = "binding_probe_busy";
const MATERIALIZATION_CREDENTIAL_PROBE_BUSY: &str = "credential_probe_busy";
const MATERIALIZATION_BINDING_DISABLED: &str = "binding_disabled";
const MATERIALIZATION_CREDENTIAL_DISABLED: &str = "credential_disabled";
const MATERIALIZATION_AUTHORITY_FAILED: &str = "authority_failed";
const RUNTIME_STATE_AUTHORITY_FAILED: &str = "runtime_state_authority_failed";
const MATERIALIZATION_DNS_FAILED: &str = "dns_failed";
const MATERIALIZATION_PROTOCOL_FAILED: &str = "protocol_failed";

pub struct ProductionProvider {
    credentials: Arc<dyn ProductionCredentialResolver>,
    state: Arc<materialization::ProductionStateAdapter>,
    cooldowns: RuntimeCooldownPolicy,
    #[cfg(feature = "e2e-test-control")]
    test_dial: Result<Option<crate::server::test_control::E2eDialMap>, ()>,
}

pub struct ProductionLogicalRequest {
    ingress: IngressProtocol,
    replay: ReplayStore,
    raw_body: ContentRef,
    observed_body_bytes: u64,
    body_eos: bool,
    ir: Option<ModelRequestIRV1>,
    frozen_candidates: Arc<[DecisionCandidateAuthority]>,
    runtime_state_authority: RuntimeStateAuthoritySignal,
    route_context: Option<ProductionRouteContext>,
}

#[derive(Clone)]
pub struct ProductionRouteContext {
    pub(crate) ingress: IngressProtocol,
    pub(crate) context_holds: Arc<ContextHoldStore>,
    pub(crate) hold_completion: Option<HoldCompletion>,
}

pub(crate) struct ProductionReplaySeed {
    pub(crate) ingress: IngressProtocol,
    pub(crate) replay: ReplayStore,
    pub(crate) raw_body: ContentRef,
    pub(crate) ir: ModelRequestIRV1,
    pub(crate) runtime_state_authority: RuntimeStateAuthoritySignal,
    pub(crate) route_context: ProductionRouteContext,
}

/// Request-owned marker that lets the outer lifecycle replace only a still-
/// uncommitted generic core failure; it is never shared across requests.
#[derive(Clone, Default)]
pub(crate) struct RuntimeStateAuthoritySignal(Arc<AtomicBool>);

impl RuntimeStateAuthoritySignal {
    pub(crate) fn fail(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn failed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub(crate) struct ProductionReplaySeedEnvelope {
    seed: Mutex<Option<ProductionReplaySeed>>,
}

impl ProductionReplaySeedEnvelope {
    pub(crate) fn new(seed: ProductionReplaySeed) -> Self {
        Self {
            seed: Mutex::new(Some(seed)),
        }
    }

    fn take(&self) -> Option<ProductionReplaySeed> {
        self.seed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

pub struct ProductionAttemptState {
    response_status: Option<http::StatusCode>,
    resolved_target: TransportTarget,
    permits: OwnedAttemptStatePermits,
    classified_failure: Option<AttemptFailure>,
    profile: CandidateProtocolProfile,
    client_profile: crate::server::core_runtime::profiles::ClientProtocolProfile,
    served_model_alias: String,
    streaming: bool,
    native_output: bool,
    chat_tool_projection: Option<adapters::ChatToolProjection>,
    chat_tool_projection_budget: Option<Reservation>,
    decoder: Option<adapters::NativeResponseDecoder>,
    renderer: Option<adapters::IncrementalClientSseRenderer>,
    projector: Option<Box<adapters::NativeResponseProjector>>,
    prefix: Option<ChargedBodyQueue>,
    prefix_terminal_chunks: Option<usize>,
    decoder_budget: Option<response::PrecommitDecoderBudget>,
    budget: StreamBudget,
    semantic_seen: bool,
    terminal_seen: bool,
    semantic_terminal: Option<SemanticTerminalOutcome>,
    retry_after: Option<std::time::Duration>,
    runtime_state_authority: RuntimeStateAuthoritySignal,
}

impl ProductionAttemptState {
    pub(crate) fn chat_tool_projection_for_observation(
        &self,
    ) -> Option<&adapters::ChatToolProjection> {
        self.chat_tool_projection.as_ref()
    }
}

pub struct ProductionReadiness {
    response_status: http::StatusCode,
    content_type: &'static str,
    prefix: ChargedBodyQueue,
    terminal_body: Option<ChargedBytes>,
    decoder: Option<adapters::NativeResponseDecoder>,
    renderer: Option<adapters::IncrementalClientSseRenderer>,
    projector: Option<Box<adapters::NativeResponseProjector>>,
    _decoder_budget: Option<response::PrecommitDecoderBudget>,
    _chat_tool_projection_budget: Option<Reservation>,
    budget: StreamBudget,
    streaming: bool,
    prefix_eos_pending: bool,
    prefix_terminal_chunks: Option<usize>,
    semantic_terminal: Option<SemanticTerminalOutcome>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SemanticTerminalOutcome {
    Complete,
    Incomplete,
    Failed,
    Unknown,
}

pub struct ProductionDecodedSse {
    bytes: ChargedBytes,
    end_stream: bool,
}

#[derive(Clone)]
struct MechanicalFailureSignal {
    class: CoreAttemptFailureClass,
    timeout: Option<AttemptTimeoutKind>,
    runtime_state_authority: RuntimeStateAuthoritySignal,
}

#[async_trait]
impl ProviderRuntimePort for ProductionProvider {
    type LogicalRequest = ProductionLogicalRequest;
    type RouteRequestContext = ProductionRouteContext;
    type AttemptState = ProductionAttemptState;
    type Readiness = ProductionReadiness;
    type DecodedSseEvent = ProductionDecodedSse;

    async fn begin_request(
        &self,
        head: GatewayRequestHead,
        context: LogicalRequestContext<'_>,
    ) -> Result<Self::LogicalRequest, Arc<str>> {
        materialization::begin_request(head, context)
    }

    async fn consume_request_body(
        &self,
        logical: &mut Self::LogicalRequest,
        frame: LogicalRequestBodyFrame,
    ) -> Result<(), Arc<str>> {
        materialization::consume_request_body(logical, frame)
    }

    fn finalize_route_request_context(
        &self,
        logical: &mut Self::LogicalRequest,
    ) -> Result<Self::RouteRequestContext, Arc<str>> {
        materialization::finalize_route_request_context(logical)
    }

    async fn materialize_attempt(
        &self,
        logical: &mut Self::LogicalRequest,
        context: AttemptMaterializationContext<'_>,
    ) -> Result<(PreparedAttemptHttpRequest, Self::AttemptState), Arc<str>> {
        materialization::materialize_attempt(self, logical, context).await
    }

    fn resolved_transport_target(
        &self,
        state: &Self::AttemptState,
        binding: &AttemptExecutionBinding,
    ) -> Result<Option<TransportTarget>, Arc<str>> {
        Ok(materialization::resolved_transport_target(state, binding))
    }

    fn classify_materialization_failure(&self, error: &Arc<str>) -> AttemptMaterializationFailure {
        materialization::classify_materialization_failure(error)
    }

    fn classify_precommit(
        &self,
        state: &mut Self::AttemptState,
        event: PrecommitEvent,
        _pinned_configs: &PinnedConfigContext<'_>,
        _event_configs: &ConfigEventSnapshot,
    ) -> Result<PrecommitClassification<Self::Readiness, Self::DecodedSseEvent>, Arc<str>> {
        response::classify_precommit(state, event)
    }

    async fn confirm_precommit(
        &self,
        state: &mut Self::AttemptState,
        _facts: &ProviderClassificationFacts,
        deadline: Instant,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<(), Arc<str>> {
        materialization::confirm_precommit(self, state, deadline, cancellation).await
    }

    async fn confirm_attempt_failure(
        &self,
        state: &mut Self::AttemptState,
        failure: &AttemptFailureFacts,
        deadline: Instant,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<Option<ProviderClassificationFacts>, Arc<str>> {
        materialization::confirm_attempt_failure(self, state, failure, deadline, cancellation).await
    }

    fn normalize_attempt_local_reply(
        &self,
        state: &mut Self::AttemptState,
        reply: LocalReply,
        upstream_side_effects: UpstreamSideEffectSnapshot,
    ) -> Result<NormalizedAttemptLocalReply<Self::Readiness>, Arc<str>> {
        response::normalize_attempt_local_reply(state, reply, upstream_side_effects)
    }

    fn finalize_attempt_facts(
        &self,
        state: &mut Self::AttemptState,
        readiness: Option<&mut Self::Readiness>,
        published_facts: Option<&ProviderClassificationFacts>,
        completion: &ProviderAttemptCompletion,
    ) -> Result<ProviderClassificationFacts, Arc<str>> {
        Ok(response::finalize_attempt_facts(
            state,
            readiness.as_deref(),
            published_facts,
            completion,
        ))
    }

    fn accepted_response_head(
        &self,
        readiness: &mut Self::Readiness,
        published: &PublishedDisposition,
        _accepted: &AcceptedResponseExecutionBinding,
        _configs: &PinnedConfigContext<'_>,
    ) -> Result<GatewayResponseHead, Arc<str>> {
        response::accepted_response_head(readiness, published)
    }

    fn encode_accepted_event(
        &self,
        readiness: &mut Self::Readiness,
        event: ProviderAcceptedEvent<Self::DecodedSseEvent>,
        _published: &PublishedDisposition,
        _accepted: &AcceptedResponseExecutionBinding,
        _pinned_configs: &PinnedConfigContext<'_>,
        _event_configs: &ConfigEventSnapshot,
    ) -> Result<Option<AcceptedBodyFrame>, Arc<str>> {
        response::encode_accepted_event(readiness, event)
    }

    fn take_accepted_prefix(
        &self,
        readiness: &mut Self::Readiness,
    ) -> Result<Option<ProviderAcceptedEvent<Self::DecodedSseEvent>>, Arc<str>> {
        Ok(response::take_accepted_prefix(readiness))
    }

    fn release_terminal_request(
        &self,
        logical: Self::LogicalRequest,
        _published: &PublishedDisposition,
    ) -> Result<(), Arc<str>> {
        materialization::release_terminal_request(logical);
        Ok(())
    }
}

fn label(value: &'static str) -> ObservationLabel {
    ObservationLabel::new(value).expect("static observation label is safe")
}

fn safe_error(error: impl ToString) -> Arc<str> {
    Arc::from(error.to_string())
}
