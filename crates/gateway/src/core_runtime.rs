//! Product composition root around publication, authority and catalog.

#[path = "adapters/mod.rs"]
pub mod adapters;
#[path = "core_runtime/classification.rs"]
mod classification;
pub use classification::{
    CLASSIFIER_DIAGNOSTIC_LATEST_USER, ClassifierDiagnosticError, ClassifierDiagnosticOutcome,
};
#[path = "model_ir/mod.rs"]
pub mod model_ir;
#[path = "observation.rs"]
pub mod observation;
#[path = "profiles/mod.rs"]
pub mod profiles;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use bytes::Bytes;
use hiroute_gateway_core::runtime::body::{
    BodyDirection, BodyError, BodyPlanExecutor, ChargedBytes, MemoryRole, StreamBudget,
};
use hiroute_gateway_core::runtime::driver::{
    BoundRequestAdmission, DecisionCandidateAuthority, GatewayCoreLifecycle,
    GatewayCoreLifecycleLimits, NoopGatewayFilterManager, ObservationLabel,
};
use hiroute_gateway_core::transport::pingora::PingoraConnectorAdapter;
use hiroute_gateway_core::transport::{
    GatewayLifecycle, GatewayRequestHead, GatewayResponseHead, GatewaySession, SessionReuse,
    TransportError,
};
use http::header::{AUTHORIZATION, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use http::{HeaderMap, HeaderValue, Method, StatusCode};

use crate::agent_turn_history::{
    AcceptedExecution, AgentTurnBegin, AgentTurnHistoryError, AgentTurnHistoryStore,
    AgentTurnStatus, AgentTurnTicket, PlanSnapshot,
};
use crate::content_ref::{
    ContentRef, compact_ingress_document, externalize_model_request, model_content_refs,
    scan_ingress_document,
};
use crate::context_hold::{ContextHoldStore, ContextRequest, HoldCompletion, begin_context};
use crate::provider_state::ProviderStateScopeV1;
use crate::replay::{ReplayError, ReplayManager, ReplayReader, ReplayStore};
use crate::runtime::{
    ProductionProvider, ProductionReplaySeed, ProductionReplaySeedEnvelope, ProductionRouteContext,
};
use crate::server::composition::{
    PlannerInputAuthority, ProductionPorts, PublicationPlannerInputAuthority,
    RuntimePublicationFeed,
};
use crate::server::dispatch::{
    DispatchError, GatewayRequestAuthority, ModelSelector, RunRequestAuthorityPort,
};
use crate::server::publication::{
    CatalogError, GatewayCatalog, SEALED_CANDIDATE_EXECUTION_SCHEMA_V1, SealedCandidateExecutionV1,
};
use crate::server::request_plan::{AuthorizedRequestPlan, IngressProtocol};

use self::observation::{
    AcceptedResponseCapture, GatewayObservation, ObservedCredentialResolver,
    ObservedProductionProvider, ObservedProductionSelection, ObservedRuntimeStateStore,
    RequestObservation, with_active_request,
};

#[derive(Clone)]
pub struct ProductionGatewayRuntime {
    publications: Arc<dyn RuntimePublicationFeed>,
    authority: GatewayRequestAuthority,
    catalog: GatewayCatalog,
    ports: Arc<ProductionPorts>,
    planner_inputs: Arc<dyn PlannerInputAuthority>,
    execution: Arc<ProductionExecution>,
    classifier_connector: PingoraConnectorAdapter,
    observation: Arc<GatewayObservation>,
    context_holds: Arc<ContextHoldStore>,
    agent_turn_history: Arc<AgentTurnHistoryStore>,
    replay: Option<ReplayManager>,
    provider_states: Arc<crate::provider_state::ProviderStateStore>,
    executable_sha256: Option<Arc<str>>,
    capture_observation_content: bool,
}

type ProductionExecution = GatewayCoreLifecycle<
    ObservedProductionSelection,
    ObservedProductionProvider,
    NoopGatewayFilterManager,
    PingoraConnectorAdapter,
>;

impl ProductionGatewayRuntime {
    pub fn compose(ports: ProductionPorts) -> Self {
        Self::compose_with_planner(ports, Arc::new(PublicationPlannerInputAuthority))
    }

    pub fn compose_with_planner(
        ports: ProductionPorts,
        planner_inputs: Arc<dyn PlannerInputAuthority>,
    ) -> Self {
        Self::compose_with_planner_and_observation(
            ports,
            planner_inputs,
            Arc::new(GatewayObservation::from_environment()),
        )
    }
    pub fn compose_with_planner_and_observation(
        mut ports: ProductionPorts,
        planner_inputs: Arc<dyn PlannerInputAuthority>,
        observation: Arc<GatewayObservation>,
    ) -> Self {
        ports.credentials = Arc::new(ObservedCredentialResolver::new(Arc::clone(
            &ports.credentials,
        )));
        ports.runtime_state = Arc::new(ObservedRuntimeStateStore::new(Arc::clone(
            &ports.runtime_state,
        )));
        let ports = Arc::new(ports);
        let publications = Arc::clone(&ports.publications);
        let connector = PingoraConnectorAdapter::new();
        let execution = Arc::new(
            ProductionExecution::new_bound(
                ObservedProductionSelection::default(),
                ObservedProductionProvider::new(ProductionProvider::new(&ports)),
                NoopGatewayFilterManager,
                connector.clone(),
                GatewayCoreLifecycleLimits {
                    // Replay and retained state use the shared memory owner;
                    // the model proxy adds no cumulative request byte quota.
                    max_request_body_bytes: usize::MAX,
                    ..GatewayCoreLifecycleLimits::default()
                },
            )
            .expect("static production lifecycle limits are valid"),
        );
        Self {
            authority: GatewayRequestAuthority::from_feed(Arc::clone(&publications)),
            catalog: GatewayCatalog::from_feed(Arc::clone(&publications)),
            publications,
            ports,
            planner_inputs,
            execution,
            classifier_connector: connector,
            observation,
            context_holds: Arc::new(ContextHoldStore::default()),
            agent_turn_history: Arc::new(AgentTurnHistoryStore::default()),
            replay: ReplayManager::from_environment().ok(),
            provider_states: Arc::new(crate::provider_state::ProviderStateStore::default()),
            executable_sha256: None,
            capture_observation_content: true,
        }
    }

    pub fn with_executable_sha256(mut self, digest: impl Into<Arc<str>>) -> Self {
        self.executable_sha256 = Some(digest.into());
        self
    }

    /// Adds the narrow delegated-run branch without changing ordinary publication authority.
    pub fn with_run_request_authority(
        mut self,
        authority: Arc<dyn RunRequestAuthorityPort>,
    ) -> Self {
        self.authority = self.authority.with_run_request_authority(authority);
        self
    }

    pub fn publications(&self) -> &Arc<dyn RuntimePublicationFeed> {
        &self.publications
    }

    pub fn authority(&self) -> &GatewayRequestAuthority {
        &self.authority
    }

    pub fn catalog(&self) -> &GatewayCatalog {
        &self.catalog
    }

    pub fn ports(&self) -> &ProductionPorts {
        &self.ports
    }

    pub fn observation(&self) -> &Arc<GatewayObservation> {
        &self.observation
    }

    pub fn authorize_bytes(
        &self,
        protocol: IngressProtocol,
        authorization: Option<&str>,
        body: &[u8],
        now: Instant,
    ) -> Result<AuthorizedRequestPlan, DispatchError> {
        self.authority
            .authorize_bytes(protocol, authorization, body, now)
    }
}

/// The production listener enters through this implementation. Later protocol
/// work can extend this owner without changing the frozen Oracle adapter.
#[async_trait]
impl GatewayLifecycle for ProductionGatewayRuntime {
    async fn process(
        &self,
        session: &mut dyn GatewaySession,
    ) -> Result<SessionReuse, TransportError> {
        let request = session.request_head()?;
        let path = request
            .path_and_query
            .split_once('?')
            .map_or(request.path_and_query.as_ref(), |(path, _)| path);
        if request.method == Method::GET && path == "/_hiroute/ready" {
            let publication = self.publications().pin();
            let revision = publication
                .as_ref()
                .map(|publication| publication.publication_revision());
            let publication_digest = publication
                .as_ref()
                .map(|publication| publication.payload_digest());
            let body = serialize_body(serde_json::json!({
                "schema_version": "hiroute.gateway.ready/v2",
                "status": "ready",
                "publication_revision": revision,
                "publication_digest": publication_digest,
                "executable_sha256": self.executable_sha256.as_deref(),
            }))?;
            return write_response(session, StatusCode::OK, body, None).await;
        }
        let authorization = inbound_authorization(path, &request.headers);
        let authorization = authorization.as_deref();
        let declared_content_length = request
            .headers
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());
        if request.method == Method::GET && path == "/v1/models" {
            let if_none_match = request
                .headers
                .get(IF_NONE_MATCH)
                .and_then(|value| value.to_str().ok());
            return match self.catalog().get(authorization, if_none_match) {
                Ok(response) => {
                    write_response(
                        session,
                        response.status,
                        response.body,
                        Some(&response.etag),
                    )
                    .await
                }
                Err(error) => write_catalog_error(session, error).await,
            };
        }
        let Some(protocol) = IngressProtocol::from_path(path) else {
            return write_typed_error(
                session,
                StatusCode::NOT_FOUND,
                "INGRESS_PROTOCOL_NOT_AVAILABLE",
            )
            .await;
        };
        if request.method != Method::POST {
            return write_typed_error(
                session,
                StatusCode::METHOD_NOT_ALLOWED,
                "INGRESS_METHOD_NOT_ALLOWED",
            )
            .await;
        }

        // Authentication and protocol grant happen before the first body
        // poll. The returned object pins exactly one aggregate revision.
        // Keep large request-owned values behind pointers before any error arm
        // awaits a reply. Otherwise they inflate this future enough to exceed
        // the production listener task's bounded stack while it is created.
        let authenticated = match self
            .authority()
            .begin(protocol, authorization)
            .map(Box::new)
        {
            Ok(authenticated) => authenticated,
            Err(error) => return write_dispatch_error(session, error).await,
        };
        let budget = match self.execution.allocate_bound_stream_budget() {
            Ok(budget) => budget,
            Err(_) => {
                return write_typed_error_phase(
                    session,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "REPLAY_BUDGET_UNAVAILABLE",
                    "canonical_request",
                )
                .await;
            }
        };
        let selector_deadline = authenticated.selector_deadline();
        let mut selector = ModelSelector::new(self.authority().selector_limit());
        // Retain every opaque chunk already polled while the bounded selector
        // sees only its prefix, then replay those exact Bytes into the
        // canonical decoder before polling the rest of the body.
        let mut ingress_prefix = Vec::new();
        let mut selector_charges = Vec::new();
        let served_model_id = loop {
            match session.read_request_body_before(selector_deadline).await {
                Err(TransportError::DeadlineExceeded) => {
                    return write_dispatch_error(session, DispatchError::RequestDeadlineExceeded)
                        .await;
                }
                Err(error) => return Err(error),
                Ok(Some(chunk)) => {
                    let charge_bytes = chunk.len().checked_mul(3).and_then(|n| n.checked_add(128));
                    let charge = charge_bytes
                        .and_then(|n| budget.reserve(MemoryRole::ModelIrBacking, n).ok());
                    let Some(charge) = charge else {
                        return write_typed_error_phase(
                            session,
                            StatusCode::SERVICE_UNAVAILABLE,
                            "REPLAY_BUDGET_UNAVAILABLE",
                            "model_selector",
                        )
                        .await;
                    };
                    selector_charges.push(charge);
                    let selected = selector.feed(&chunk);
                    let retained =
                        ChargedBytes::copy_from_opaque(&budget, MemoryRole::RawRequest, &chunk)
                            .map_err(|_| {
                                TransportError::Io("selector memory budget unavailable".into())
                            })?;
                    ingress_prefix.push(retained);
                    match selected {
                        Ok(Some(alias)) => break alias,
                        Ok(None) => {}
                        Err(error) => return write_dispatch_error(session, error.into()).await,
                    }
                }
                Ok(None) => match selector.finish() {
                    Ok(alias) => break alias,
                    Err(error) => return write_dispatch_error(session, error.into()).await,
                },
            }
        };
        drop(selector);
        drop(selector_charges);
        let authorized = match authenticated
            .authorize_alias(&served_model_id, Instant::now())
            .map(Box::new)
        {
            Ok(authorized) => authorized,
            Err(error) => return write_dispatch_error(session, error).await,
        };
        drop(authenticated);
        self.process_authorized(
            session,
            Box::new(request),
            protocol,
            declared_content_length,
            ingress_prefix,
            authorized,
            budget,
        )
        .await
    }
}

impl ProductionGatewayRuntime {
    #[allow(clippy::too_many_arguments)] // The frozen ingress boundary is intentionally explicit.
    fn process_authorized<'a>(
        &'a self,
        session: &'a mut dyn GatewaySession,
        request: Box<GatewayRequestHead>,
        protocol: IngressProtocol,
        declared_content_length: Option<usize>,
        ingress_prefix: Vec<ChargedBytes>,
        authorized: Box<AuthorizedRequestPlan>,
        budget: StreamBudget,
    ) -> Pin<Box<dyn Future<Output = Result<SessionReuse, TransportError>> + Send + 'a>> {
        Box::pin(async move {
            let continuation_scope = tool_continuation_scope(&authorized);
            let Some(replay_manager) = &self.replay else {
                return write_typed_error_phase(
                    session,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "REPLAY_STORE_UNAVAILABLE",
                    "canonical_request",
                )
                .await;
            };
            let replay = match replay_manager.begin_request(budget.clone()) {
                Ok(replay) => replay,
                Err(_) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::SERVICE_UNAVAILABLE,
                        "REPLAY_STORE_UNAVAILABLE",
                        "canonical_request",
                    )
                    .await;
                }
            };
            let raw_body = match read_replay_body(
                session,
                ingress_prefix,
                declared_content_length,
                &authorized,
                &replay,
            )
            .await
            {
                Ok(body) => body,
                Err(ReplayBodyError::Deadline) => {
                    return write_dispatch_error(session, DispatchError::RequestDeadlineExceeded)
                        .await;
                }
                Err(ReplayBodyError::Transport(error)) => return Err(error),
                Err(ReplayBodyError::Limit) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "REQUEST_BODY_PLAN_LIMIT_EXCEEDED",
                        "canonical_request",
                    )
                    .await;
                }
                Err(ReplayBodyError::InvalidFraming) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::BAD_REQUEST,
                        "REQUEST_BODY_FRAMING_INVALID",
                        "canonical_request",
                    )
                    .await;
                }
                Err(ReplayBodyError::PlanUnavailable) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "CANONICAL_BODY_PLAN_UNAVAILABLE",
                        "canonical_request",
                    )
                    .await;
                }
                Err(ReplayBodyError::Replay) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::BAD_REQUEST,
                        "REPLAY_INTEGRITY_FAILED",
                        "canonical_request",
                    )
                    .await;
                }
            };
            let ingress_stats = match replay.reader(&raw_body).and_then(scan_ingress_document) {
                Ok(stats) => stats,
                Err(ReplayError::StructureLimit) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "REQUEST_STRUCTURE_LIMIT_EXCEEDED",
                        "canonical_request",
                    )
                    .await;
                }
                Err(_) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::BAD_REQUEST,
                        "REPLAY_INTEGRITY_FAILED",
                        "canonical_request",
                    )
                    .await;
                }
            };
            let ingress_workspace =
                match ingress_stats.reserve_workspace(&replay, raw_body.byte_len()) {
                    Ok(reservation) => reservation,
                    Err(_) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::SERVICE_UNAVAILABLE,
                            "REPLAY_BUDGET_UNAVAILABLE",
                            "canonical_request",
                        )
                        .await;
                    }
                };
            let mut document = match replay.reader(&raw_body).and_then(|mut reader| {
                let document =
                    serde_json::from_reader(&mut reader).map_err(|_| ReplayError::Integrity)?;
                reader.verify_terminal()?;
                Ok(document)
            }) {
                Ok(document) => document,
                Err(_) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::BAD_REQUEST,
                        "PROTOCOL_DOCUMENT_INVALID",
                        "canonical_request",
                    )
                    .await;
                }
            };
            // Resolve controls before either ingress or canonical JSON becomes a content reference.
            let fixed_reasoning = match PublicationPlannerInputAuthority::bind_request_reasoning(
                &document,
                protocol,
                &authorized,
            ) {
                Ok(selected) => selected,
                Err(error) => {
                    let (status, code) = match error {
                        crate::server::composition::PortError::InvalidReasoningControl => {
                            (StatusCode::BAD_REQUEST, "FIXED_REASONING_UNSUPPORTED")
                        }
                        _ => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "PLANNER_INPUT_UNAVAILABLE",
                        ),
                    };
                    return write_typed_error_phase(session, status, code, "planner").await;
                }
            };
            // Native Messages state already has an exact single-source binding.
            // Responses ciphertext, including its Messages signature projection,
            // must instead resolve the source actually accepted downstream.
            let native_messages_owner = if protocol == IngressProtocol::Messages {
                match provider_state_owner(&authorized, protocol) {
                    Ok(owner) => {
                        owner.filter(|owner| owner.upstream_protocol == IngressProtocol::Messages)
                    }
                    Err(_) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "PROVIDER_STATE_AUTHORITY_UNAVAILABLE",
                            "continuation_authority",
                        )
                        .await;
                    }
                }
            } else {
                None
            };
            let provider_state_owner = if protocol == IngressProtocol::Responses
                || (protocol == IngressProtocol::Messages && native_messages_owner.is_none())
            {
                match self
                    .provider_states
                    .resolve(&continuation_scope, &document, Instant::now())
                {
                    Ok(owner) => owner,
                    Err(error) => {
                        let code = if error == model_ir::ModelIrError::ProviderStateNotPortable {
                            "PROVIDER_STATE_CONTINUATION_CONFLICT"
                        } else {
                            "PROVIDER_STATE_CONTINUATION_UNAVAILABLE"
                        };
                        return write_typed_error_phase(
                            session,
                            StatusCode::BAD_REQUEST,
                            code,
                            "continuation_authority",
                        )
                        .await;
                    }
                }
            } else {
                match if protocol == IngressProtocol::Messages {
                    Ok(native_messages_owner)
                } else {
                    provider_state_owner(&authorized, protocol)
                } {
                    Ok(owner) => owner,
                    Err(_) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "PROVIDER_STATE_AUTHORITY_UNAVAILABLE",
                            "continuation_authority",
                        )
                        .await;
                    }
                }
            };
            if let Err(error) = compact_ingress_document(protocol, &mut document, &replay) {
                let (status, code) = match error {
                    ReplayError::StructureLimit => (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "REQUEST_STRUCTURE_LIMIT_EXCEEDED",
                    ),
                    _ => (StatusCode::SERVICE_UNAVAILABLE, "CONTENT_REF_UNAVAILABLE"),
                };
                return write_typed_error_phase(session, status, code, "canonical_request").await;
            }
            let parse_started = Instant::now();
            let mut canonical_request = match adapters::decode_ingress_request_with_bindings(
                protocol,
                &document,
                &adapters::IngressRequestBindings {
                    provider_state_owner,
                },
            )
            .map(Box::new)
            {
                Ok(request) => request,
                Err(error) => {
                    let error = adapters::ProtocolAdapterError::from(error);
                    return write_typed_error_phase(
                        session,
                        StatusCode::BAD_REQUEST,
                        error.code(),
                        "canonical_request",
                    )
                    .await;
                }
            };
            let parse_elapsed = parse_started.elapsed();
            let (identity, hold_ticket) = begin_context(
                &self.context_holds,
                &self.observation.correlation_key(),
                &authorized,
                ContextRequest {
                    ingress: protocol,
                    headers: &request.headers,
                    document: &document,
                    request: &canonical_request,
                },
                Instant::now(),
            );
            let hold_ticket = hold_ticket.map(Box::new);
            drop(document);
            if canonical_request.web_search.is_some() && !authorized.web_search_allowed() {
                return write_typed_error_phase(
                    session,
                    StatusCode::FORBIDDEN,
                    "WORKER_NETWORK_DENIED",
                    "run_authority",
                )
                .await;
            }
            if canonical_request.served_model_id != authorized.served_model_id() {
                return write_typed_error_phase(
                    session,
                    StatusCode::BAD_REQUEST,
                    "MODEL_SELECTOR_CANONICAL_MISMATCH",
                    "canonical_request",
                )
                .await;
            }
            if let Some(digest) = fixed_reasoning {
                canonical_request.requested_reasoning.fixed_profile_digest = Some(digest);
                canonical_request.requested_reasoning.disposition =
                    model_ir::RequestedReasoningDisposition::AppliedToFixedBinding;
            }
            // Move large canonical content into the request-owned aggregate pool
            // before candidate facts or Planner input are built. Planner owns and
            // later returns this compact IR; no full request clone is retained.
            let externalize_started = Instant::now();
            if externalize_model_request(&mut canonical_request, &replay, 8 * 1024).is_err() {
                return write_typed_error_phase(
                    session,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "CONTENT_REF_UNAVAILABLE",
                    "canonical_request",
                )
                .await;
            }
            let externalize_elapsed = externalize_started.elapsed();
            let prevalidate_started = Instant::now();
            let mut replay_references = model_content_refs(&canonical_request);
            replay_references.push(raw_body.clone());
            if replay.prevalidate(&replay_references).is_err() {
                return write_typed_error_phase(
                    session,
                    StatusCode::BAD_REQUEST,
                    "REPLAY_INTEGRITY_FAILED",
                    "canonical_request",
                )
                .await;
            }
            let prevalidate_elapsed = prevalidate_started.elapsed();
            let request_observation = self.observation.begin_request_with_content(
                &authorized,
                protocol,
                &identity,
                self.capture_observation_content,
            );
            let _observation_guard = ObservationRequestGuard(request_observation.clone());
            request_observation.record_pipeline_stages(
                parse_elapsed,
                externalize_elapsed,
                prevalidate_elapsed,
            );
            request_observation.capture_request(&canonical_request, &replay);
            let mut agent_turn_guard = None;
            let mut correlated_branch = None;
            let classification_outcome = if let Some(strategy) = authorized
                .planner_policy()
                .complexity_strategy
                .as_ref()
                .cloned()
            {
                // History preparation belongs to the same bounded classifier operation.
                // Starting the clock here prevents Replay-backed projection from receiving
                // an extra, unbounded budget before the REST call.
                let classification_started = Instant::now();
                let classification_cancellation = session.cancellation_token().child_token();
                let profiles::PlannerRouteIdentityV2::Plan { plan_id, revision } =
                    &authorized.planner_policy().identity
                else {
                    return write_typed_error_phase(
                        session,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "AGENT_TURN_PLAN_UNAVAILABLE",
                        "agent_turn",
                    )
                    .await;
                };
                let turn_key = match self.agent_turn_history.scope_key(
                    &request_observation.metadata().workspace_id,
                    &request_observation.metadata().conversation_id,
                ) {
                    Ok(key) => key,
                    Err(_) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "AGENT_TURN_SCOPE_UNAVAILABLE",
                            "agent_turn",
                        )
                        .await;
                    }
                };
                let context_decision = crate::agent_turn_history::ContextDecisionFacts {
                    history_continues: hold_ticket
                        .as_ref()
                        .is_some_and(|ticket| ticket.history_continues),
                    has_hold_preference: hold_ticket
                        .as_ref()
                        .is_some_and(|ticket| ticket.hint.is_some()),
                };
                let turn_begin = self.agent_turn_history.begin_with_context(
                    turn_key,
                    PlanSnapshot {
                        plan_id: plan_id.clone(),
                        plan_revision: *revision,
                    },
                    &canonical_request,
                    &replay,
                    context_decision,
                    Instant::now(),
                );
                let (turn_ticket, turn_history, completed_turn) = match turn_begin {
                    Ok(AgentTurnBegin::NewTurn {
                        ticket,
                        history,
                        completed,
                    }) => (ticket, history, completed),
                    Ok(AgentTurnBegin::Continuation { ticket, decision }) => {
                        let kind = if canonical_request.messages.iter().any(|message| {
                            message.content.iter().any(|part| {
                                matches!(part, model_ir::ContentPart::ToolResult { .. })
                            })
                        }) {
                            profiles::ContinuationKindV1::ToolContinuation
                        } else {
                            profiles::ContinuationKindV1::TaskRoot
                        };
                        correlated_branch = Some(Box::new(profiles::CorrelatedBranchDecisionV1 {
                            kind,
                            decision,
                        }));
                        (
                            ticket,
                            crate::agent_turn_history::AgentTurnHistorySnapshot {
                                visible_conversation: Vec::new(),
                                history_partial: true,
                                assessment_from: None,
                                assessment_target: None,
                                _pin: None,
                            },
                            None,
                        )
                    }
                    Err(AgentTurnHistoryError::TurnContextUnavailable) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::CONFLICT,
                            "TURN_CONTEXT_UNAVAILABLE",
                            "agent_turn",
                        )
                        .await;
                    }
                    Err(AgentTurnHistoryError::SessionTurnConflict) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::CONFLICT,
                            "SESSION_TURN_CONFLICT",
                            "agent_turn",
                        )
                        .await;
                    }
                    Err(AgentTurnHistoryError::Resource) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::SERVICE_UNAVAILABLE,
                            "AGENT_TURN_HISTORY_UNAVAILABLE",
                            "agent_turn",
                        )
                        .await;
                    }
                    Err(AgentTurnHistoryError::Integrity) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::BAD_REQUEST,
                            "REPLAY_INTEGRITY_FAILED",
                            "agent_turn",
                        )
                        .await;
                    }
                };
                if let Some(completed) = completed_turn {
                    request_observation.agent_turn_finished(&completed);
                }
                agent_turn_guard = Some(AgentTurnRequestGuard::new(
                    Arc::clone(&self.agent_turn_history),
                    turn_ticket.clone(),
                ));
                request_observation.bind_agent_turn_output(
                    Arc::clone(&self.agent_turn_history),
                    turn_ticket.clone(),
                );
                let mut classification = Box::pin(self.classify_for_planner(
                    &strategy,
                    classification::ClassificationRequest {
                        request: &canonical_request,
                        replay: &replay,
                        classifier: authorized.classifier(),
                        history: &turn_history,
                        correlated: correlated_branch.as_deref(),
                        publication_revision: authorized.publication_revision(),
                        overall_deadline: authorized.deadline(),
                        cancellation: classification_cancellation.clone(),
                        started_at: classification_started,
                    },
                ));
                let classification_result = tokio::select! {
                    // At the source deadline Pingora's liveness wait and the
                    // classifier can become ready together. Preserve the
                    // typed deadline response instead of misclassifying that
                    // tie as a downstream disconnect.
                    biased;
                    result = &mut classification => result,
                    _ = session.wait_for_disconnect() => {
                        classification_cancellation.cancel();
                        let _ = classification.await;
                        Err(classification::ClassificationError::Cancelled)
                    }
                };
                match classification_result {
                    Ok(result) => {
                        if turn_ticket.new_turn
                            && self
                                .agent_turn_history
                                .commit_decision(&turn_ticket, result.decision.clone())
                                .is_err()
                        {
                            return write_typed_error_phase(
                                session,
                                StatusCode::CONFLICT,
                                "TURN_CONTEXT_UNAVAILABLE",
                                "agent_turn",
                            )
                            .await;
                        }
                        if let Some(assessment) = &result.assessment {
                            request_observation.branch_assessment_recorded(assessment);
                        }
                        Some(result)
                    }
                    Err(classification::ClassificationError::Deadline) => {
                        return write_dispatch_error(
                            session,
                            DispatchError::RequestDeadlineExceeded,
                        )
                        .await;
                    }
                    Err(classification::ClassificationError::Cancelled) => {
                        return Err(TransportError::Io(
                            "request cancelled during classification".into(),
                        ));
                    }
                    Err(classification::ClassificationError::Integrity) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::BAD_REQUEST,
                            "REPLAY_INTEGRITY_FAILED",
                            "classifier",
                        )
                        .await;
                    }
                    Err(classification::ClassificationError::Resource) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::SERVICE_UNAVAILABLE,
                            "REPLAY_BUDGET_UNAVAILABLE",
                            "classifier",
                        )
                        .await;
                    }
                }
            } else {
                None
            };
            let mut planner_input = match self
                .planner_inputs
                .build_input(*canonical_request, &authorized)
                .map(Box::new)
            {
                Ok(input) => input,
                Err(crate::server::composition::PortError::InvalidReasoningControl) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::BAD_REQUEST,
                        "FIXED_REASONING_UNSUPPORTED",
                        "planner",
                    )
                    .await;
                }
                Err(_) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "PLANNER_INPUT_UNAVAILABLE",
                        "planner",
                    )
                    .await;
                }
            };
            planner_input.context_hold =
                hold_ticket.as_ref().and_then(|ticket| ticket.hint.clone());
            planner_input.correlated_branch = correlated_branch.map(|decision| *decision);
            if let Some(outcome) = classification_outcome {
                planner_input.classification_decision = Some(outcome.decision);
                planner_input.classification_facts = Some(outcome.facts);
            }
            // The Planner is invoked exactly once for this request. Everything
            // below consumes its frozen output; no runtime fallback path may
            // reconstruct order from the publication candidate set.
            let plan_started = Instant::now();
            let planner_output = match profiles::Planner.plan(&planner_input).map(Box::new) {
                Ok(output) => output,
                Err(_) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "PLANNER_FAILED",
                        "planner",
                    )
                    .await;
                }
            };
            request_observation.record_plan_stage(plan_started.elapsed());
            request_observation.record_planner(&planner_input, &planner_output);
            if !matches!(planner_output.outcome, profiles::PlannerOutcomeV1::Ready)
                || planner_output.ledger.ordered_candidates.is_empty()
            {
                let (status, code) = no_eligible_response(&planner_output.ledger.evaluations);
                return write_typed_error_phase(session, status, code, "planner").await;
            }
            let fact_by_id = planner_input
                .candidates
                .iter()
                .map(|candidate| (candidate.candidate_id.as_str(), candidate))
                .collect::<std::collections::BTreeMap<_, _>>();
            let mut binding_by_stable = std::collections::BTreeMap::new();
            for binding in authorized
                .core_binding()
                .candidate_bindings()
                .map_err(|_| {
                    TransportError::Io("authorized candidate closure unavailable".into())
                })?
            {
                let attempt = authorized
                    .core_binding()
                    .resolve_attempt(*binding)
                    .map_err(|_| {
                        TransportError::Io("authorized candidate binding unavailable".into())
                    })?;
                binding_by_stable.insert(
                    attempt.plan().stable_target_key.as_str().to_owned(),
                    *binding,
                );
            }
            let reason_ledger_identity =
                ObservationLabel::new(planner_output.output_digest.clone())
                    .map_err(|_| TransportError::Io("planner output identity is invalid".into()))?;
            let mut frozen_candidates =
                Vec::with_capacity(planner_output.ledger.ordered_candidates.len());
            let mut hold_candidates =
                Vec::with_capacity(planner_output.ledger.ordered_candidates.len());
            let mut agent_turn_candidates = std::collections::BTreeMap::new();
            for frozen in &planner_output.ledger.ordered_candidates {
                let facts = fact_by_id
                    .get(frozen.candidate_id.as_str())
                    .copied()
                    .ok_or_else(|| {
                        TransportError::Io("planner candidate facts unavailable".into())
                    })?;
                if facts.stable_binding_id != frozen.stable_binding_id
                    || facts.profile_digest != frozen.profile_digest
                {
                    return Err(TransportError::Io(
                        "planner frozen candidate identity mismatch".into(),
                    ));
                }
                let binding = *binding_by_stable
                    .get(&frozen.stable_binding_id)
                    .ok_or_else(|| {
                        TransportError::Io("planner selected an unauthorized binding".into())
                    })?;
                let attempt = authorized
                    .core_binding()
                    .resolve_attempt(binding)
                    .map_err(|_| TransportError::Io("planner binding resolution failed".into()))?;
                if attempt.plan().config_cell_ids.len() != 1 {
                    return Err(TransportError::Io(
                        "sealed candidate config is unavailable".into(),
                    ));
                }
                let config_id = attempt.plan().config_cell_ids[0];
                let configs = attempt.acquire_attempt_configs().map_err(|_| {
                    TransportError::Io("sealed candidate config acquisition failed".into())
                })?;
                let candidate: crate::server::publication::CandidateBindingV1 =
                    serde_json::from_slice(
                        &configs
                            .value(config_id)
                            .ok_or_else(|| {
                                TransportError::Io("sealed candidate config is unavailable".into())
                            })?
                            .bytes,
                    )
                    .map_err(|_| TransportError::Io("sealed candidate config is invalid".into()))?;
                let selected_profile =
                    serde_json::to_value(&facts.protocol_profile).map_err(|_| {
                        TransportError::Io("planner profile serialization failed".into())
                    })?;
                let profile_is_member = candidate.protocol_profiles.iter().any(|profile| {
                    serde_json::to_value(profile)
                        .ok()
                        .and_then(|value| {
                            serde_json::from_value::<profiles::CandidateProtocolProfile>(value).ok()
                        })
                        .and_then(|profile| {
                            profile.for_request_reasoning(&planner_input.request).ok()
                        })
                        .and_then(|profile| serde_json::to_value(profile).ok())
                        .is_some_and(|profile| profile == selected_profile)
                });
                if candidate.local_id != binding.local_id()
                    || candidate.stable_target_key != frozen.stable_binding_id
                    || !profile_is_member
                {
                    return Err(TransportError::Io(
                        "sealed candidate profile mismatch".into(),
                    ));
                }
                let profile = serde_json::to_vec(&SealedCandidateExecutionV1 {
                    schema_version: SEALED_CANDIDATE_EXECUTION_SCHEMA_V1.into(),
                    stable_target_key: candidate.stable_target_key,
                    credential_destination_ref: candidate.credential_destination_ref,
                    logical_endpoint: candidate.endpoint,
                    upstream_model_id: candidate.upstream_model_id,
                    native_transport_model: candidate.native_transport_model,
                    connector_runtime: candidate.connector_runtime,
                    operational_target: candidate.operational_target,
                    operational_target_digest: candidate.operational_target_digest,
                    protocol_set_digest: candidate.protocol_profile_digest,
                    profile_digest: facts.profile_digest.clone(),
                    protocol_profile: facts.protocol_profile.clone(),
                })
                .map_err(|_| TransportError::Io("planner profile serialization failed".into()))?;
                frozen_candidates.push(DecisionCandidateAuthority {
                    binding,
                    stable_target: ObservationLabel::new(frozen.stable_binding_id.clone())
                        .map_err(|_| {
                            TransportError::Io("stable target identity is invalid".into())
                        })?,
                    credential_refs: Arc::clone(&attempt.plan().credential_refs),
                    candidate_id: Some(
                        ObservationLabel::new(frozen.candidate_id.clone()).map_err(|_| {
                            TransportError::Io("candidate identity is invalid".into())
                        })?,
                    ),
                    profile_digest: Some(
                        ObservationLabel::new(frozen.profile_digest.clone()).map_err(|_| {
                            TransportError::Io("profile identity is invalid".into())
                        })?,
                    ),
                    reason_ledger_identity: Some(reason_ledger_identity.clone()),
                    provider_profile: Some(profile.into()),
                });
                hold_candidates.push((
                    binding,
                    profiles::HoldPreferenceV1 {
                        stable_binding_id: frozen.stable_binding_id.clone(),
                        candidate_id: frozen.candidate_id.clone(),
                        profile_digest: frozen.profile_digest.clone(),
                        reasoning_profile_id: frozen.reasoning_profile_id.clone(),
                        origin_group_id: frozen.group_id.clone(),
                    },
                ));
                let executed_branch_id = match frozen.group_id.as_str() {
                    "economy" => hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_ID,
                    "primary" => hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID,
                    _ => planner_output
                        .complexity
                        .as_ref()
                        .map_or("unknown", |decision| decision.branch_id.as_str()),
                };
                agent_turn_candidates.insert(
                    frozen.candidate_id.clone(),
                    (
                        facts
                            .protocol_profile
                            .capability
                            .model_configuration_id
                            .clone(),
                        facts.profile_digest.clone(),
                        executed_branch_id.to_owned(),
                    ),
                );
            }
            drop(fact_by_id);
            let canonical_request = Box::new(planner_input.request);
            drop(ingress_workspace);
            let mut replay_references = model_content_refs(&canonical_request);
            replay_references.push(raw_body.clone());
            if replay.prevalidate(&replay_references).is_err() {
                return write_typed_error_phase(
                    session,
                    StatusCode::BAD_REQUEST,
                    "REPLAY_INTEGRITY_FAILED",
                    "canonical_request",
                )
                .await;
            }
            let frozen_candidates: Arc<[DecisionCandidateAuthority]> = frozen_candidates.into();
            let route_binding = frozen_candidates[0].binding;
            let active_continuation = adapters::ActiveResponseDelivery::new(
                protocol,
                crate::provider_state::ActiveProviderStates::with_budget(
                    Arc::clone(&self.provider_states),
                    continuation_scope,
                    protocol,
                    budget.clone(),
                ),
            );
            request_observation.bind_tool_id_projection(active_continuation.tool_id_projection());
            let response_delivery = active_continuation.clone();
            let admission = BoundRequestAdmission {
                route_binding,
                accepted_response_body_plan: authorized.accepted_response_plan().body_plan.clone(),
                overall_deadline: authorized.deadline(),
                max_attempts: planner_output.limits.max_attempts,
                frozen_candidates,
                binding: authorized.into_core_binding(),
            };
            let request_head = *request;
            let replay_reader = replay
                .reader(&raw_body)
                .map_err(|_| TransportError::Io("replay integrity failure".into()))?;
            let runtime_state_authority = crate::runtime::RuntimeStateAuthoritySignal::default();
            let route_context = ProductionRouteContext {
                ingress: protocol,
                context_holds: Arc::clone(&self.context_holds),
                hold_completion: hold_ticket.map(|ticket| HoldCompletion {
                    ticket: *ticket,
                    candidates: hold_candidates.into(),
                }),
            };
            let seed = Arc::new(ProductionReplaySeedEnvelope::new(ProductionReplaySeed {
                ingress: protocol,
                replay,
                raw_body,
                ir: *canonical_request,
                runtime_state_authority: runtime_state_authority.clone(),
                route_context,
            }));
            let mut core_session = Box::new(ReplayBodySession {
                inner: session,
                request_head,
                request_body: Some(replay_reader),
                budget: budget.clone(),
                response_capture: AcceptedResponseCapture::new(request_observation.clone()),
                response_started: false,
                runtime_state_authority: runtime_state_authority.clone(),
                continuation_scanner: active_continuation.scanner(),
            });
            let execution = Box::pin(self.execution.process_bound_with_context(
                &mut *core_session,
                admission,
                budget,
                seed,
            ));
            let execution = adapters::with_active_response_delivery(active_continuation, execution);
            let execution = Box::pin(with_active_request(request_observation.clone(), execution));
            let result = execution.await;
            let response_started = core_session.response_started;
            drop(core_session);
            let runtime_state_authority_failed =
                !response_started && runtime_state_authority.failed();
            let observation_outcome =
                match (request_observation.has_accepted_attempt(), result.is_ok()) {
                    (true, true) => "accepted",
                    (true, false) => "postcommit_transport_failed",
                    (false, _) => "failed",
                };
            if let Some(mut guard) = agent_turn_guard.take() {
                request_observation.finish_agent_turn_output().await;
                let executions = request_observation
                    .accepted_attempt_identity()
                    .and_then(|(candidate_id, _, _)| {
                        agent_turn_candidates.get(&candidate_id).map(
                            |(model_configuration_id, profile_digest, executed_branch_id)| {
                                vec![AcceptedExecution {
                                    model_configuration_id: model_configuration_id.clone(),
                                    profile_digest: profile_digest.clone(),
                                    executed_branch_id: executed_branch_id.clone(),
                                    request_id: request_observation.metadata().request_id.clone(),
                                }]
                            },
                        )
                    })
                    .unwrap_or_default();
                let status = match (request_observation.has_accepted_attempt(), result.is_ok()) {
                    (true, true) => AgentTurnStatus::Completed,
                    (true, false) => AgentTurnStatus::Interrupted,
                    (false, _) => AgentTurnStatus::Failed,
                };
                if let Ok(completed) = self.agent_turn_history.finish_request(
                    guard.ticket(),
                    request_observation.metadata().request_id.clone(),
                    status,
                    executions,
                    response_delivery.accepted_count() == 0,
                    Instant::now(),
                ) {
                    if let Some(completed) = completed {
                        request_observation.agent_turn_finished(&completed);
                    }
                    guard.disarm();
                }
            }
            request_observation.finish(observation_outcome);
            if runtime_state_authority_failed {
                write_typed_error_phase(
                    session,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "RUNTIME_STATE_AUTHORITY_UNAVAILABLE",
                    "runtime_state",
                )
                .await
            } else {
                result
            }
        })
    }
}

fn no_eligible_response(
    evaluations: &[profiles::CandidateEvaluationV1],
) -> (StatusCode, &'static str) {
    use profiles::ExclusionReasonCodeV1;
    if !evaluations.is_empty()
        && evaluations.iter().all(|item| {
            item.first_exclusion == Some(ExclusionReasonCodeV1::ToolInterfaceUnsupported)
        })
    {
        return (StatusCode::BAD_REQUEST, "TOOL_INTERFACE_UNSUPPORTED");
    }
    if !evaluations.is_empty()
        && evaluations.iter().all(|item| {
            item.first_exclusion == Some(ExclusionReasonCodeV1::ProtocolPathUnavailable)
        })
    {
        return (StatusCode::BAD_REQUEST, "CLIENT_PROTOCOL_UNREPRESENTABLE");
    }
    (StatusCode::BAD_GATEWAY, "NO_ELIGIBLE_CANDIDATE")
}

#[cfg(test)]
mod no_eligible_tests {
    use super::*;

    fn excluded(reason: profiles::ExclusionReasonCodeV1) -> profiles::CandidateEvaluationV1 {
        profiles::CandidateEvaluationV1 {
            candidate_id: "candidate".into(),
            stable_binding_id: "binding".into(),
            group_id: "group".into(),
            declared_order: 0,
            profile_digest: "digest".into(),
            eligible: false,
            first_exclusion: Some(reason),
            reasoning_profile_id: None,
            context: None,
            overall_score_tenths: None,
            effective_cost_micros: None,
            cost_class: profiles::CostClassV1::Unknown,
        }
    }

    #[test]
    fn reports_unrepresentable_tools_and_protocols_as_client_errors() {
        use profiles::ExclusionReasonCodeV1 as Reason;
        assert_eq!(
            no_eligible_response(&[excluded(Reason::ToolInterfaceUnsupported)]),
            (StatusCode::BAD_REQUEST, "TOOL_INTERFACE_UNSUPPORTED")
        );
        assert_eq!(
            no_eligible_response(&[excluded(Reason::ProtocolPathUnavailable)]),
            (StatusCode::BAD_REQUEST, "CLIENT_PROTOCOL_UNREPRESENTABLE")
        );
        assert_eq!(
            no_eligible_response(&[
                excluded(Reason::ToolInterfaceUnsupported),
                excluded(Reason::ContextTooLarge),
            ]),
            (StatusCode::BAD_GATEWAY, "NO_ELIGIBLE_CANDIDATE")
        );
    }
}

fn inbound_authorization<'a>(
    path: &str,
    headers: &'a HeaderMap,
) -> Option<std::borrow::Cow<'a, str>> {
    if headers.contains_key("x-hiroute-token") {
        if !matches!(path, "/v1/responses" | "/v1/models")
            || headers.get_all("x-hiroute-token").iter().count() != 1
        {
            return None;
        }
        let token = headers.get("x-hiroute-token")?.to_str().ok()?;
        if token.is_empty()
            || token.starts_with("hr_run_")
            || token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte == b',')
        {
            return None;
        }
        return Some(std::borrow::Cow::Owned(format!("Bearer {token}")));
    }
    if headers.get_all(AUTHORIZATION).iter().count() != 1 {
        return None;
    }
    let authorization = headers.get(AUTHORIZATION)?.to_str().ok()?;
    if path == "/v1/responses" && !authorization.starts_with("Bearer hr_run_model_") {
        return None;
    }
    Some(std::borrow::Cow::Borrowed(authorization))
}

struct ObservationRequestGuard(RequestObservation);

impl Drop for ObservationRequestGuard {
    fn drop(&mut self) {
        self.0.finish("failed");
    }
}

struct AgentTurnRequestGuard {
    store: Arc<AgentTurnHistoryStore>,
    ticket: Option<AgentTurnTicket>,
}

impl AgentTurnRequestGuard {
    fn new(store: Arc<AgentTurnHistoryStore>, ticket: AgentTurnTicket) -> Self {
        Self {
            store,
            ticket: Some(ticket),
        }
    }

    fn ticket(&self) -> &AgentTurnTicket {
        self.ticket.as_ref().expect("agent turn guard is armed")
    }

    fn disarm(&mut self) {
        self.ticket = None;
    }
}

impl Drop for AgentTurnRequestGuard {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.store.abort(&ticket);
        }
    }
}

struct ReplayBodySession<'a> {
    inner: &'a mut dyn GatewaySession,
    request_head: GatewayRequestHead,
    request_body: Option<ReplayReader>,
    budget: StreamBudget,
    response_capture: AcceptedResponseCapture,
    response_started: bool,
    runtime_state_authority: crate::runtime::RuntimeStateAuthoritySignal,
    continuation_scanner: adapters::AcceptedResponseDeliveryScanner,
}

#[async_trait]
impl GatewaySession for ReplayBodySession<'_> {
    fn request_head(&self) -> Result<GatewayRequestHead, TransportError> {
        Ok(self.request_head.clone())
    }

    fn cancellation_token(&self) -> tokio_util::sync::CancellationToken {
        self.inner.cancellation_token()
    }

    async fn wait_for_disconnect(&mut self) {
        self.inner.wait_for_disconnect().await;
    }

    async fn read_request_body(&mut self) -> Result<Option<Bytes>, TransportError> {
        let Some(reader) = self.request_body.as_mut() else {
            return Ok(None);
        };
        let chunk = reader
            .next_charged(&self.budget, MemoryRole::RawRequest, 16 * 1024)
            .map_err(|_| TransportError::Io("replay integrity failure".into()))?;
        match chunk {
            Some(chunk) => Ok(Some(chunk.into_tracked_bytes())),
            None => {
                self.request_body.take();
                Ok(None)
            }
        }
    }

    async fn write_response_head(
        &mut self,
        head: GatewayResponseHead,
    ) -> Result<(), TransportError> {
        // Core owns generic candidate-exhaustion replies. Refuse its first
        // write while the fence is clear so the outer lifecycle can render the
        // stable RuntimeState error without risking a double response.
        if !self.response_started && self.runtime_state_authority.failed() {
            return Err(TransportError::Io(
                "runtime state authority response required".into(),
            ));
        }
        let captured = head.clone();
        self.response_started = true;
        self.inner.write_response_head(head).await?;
        self.response_capture.accepted_head(&captured);
        Ok(())
    }

    async fn write_response_body(
        &mut self,
        body: Bytes,
        end_stream: bool,
    ) -> Result<(), TransportError> {
        let captured = body.clone();
        self.inner.write_response_body(body, end_stream).await?;
        self.continuation_scanner
            .accept_bytes(&captured, Instant::now());
        self.response_capture.accepted_frame(&captured, end_stream);
        Ok(())
    }
}

fn tool_continuation_scope(authorized: &AuthorizedRequestPlan) -> ProviderStateScopeV1 {
    let receipt = authorized.receipt();
    ProviderStateScopeV1 {
        authority_id: receipt.authority_id.to_string(),
        authority_epoch: receipt.authority_epoch,
        grant_id: receipt.grant_id.to_string(),
        grant_generation: receipt.grant_generation,
        served_model_id: receipt.served_model_id.to_string(),
        route: receipt.route.clone(),
    }
}

fn provider_state_owner(
    authorized: &AuthorizedRequestPlan,
    ingress: IngressProtocol,
) -> Result<Option<model_ir::ExactProviderPathV1>, ()> {
    use profiles::{Fidelity, NativeProviderStateEmission, StateAffinity};

    let mut owner = None;
    for binding in authorized
        .core_binding()
        .candidate_bindings()
        .map_err(|_| ())?
    {
        let attempt = authorized
            .core_binding()
            .resolve_attempt(*binding)
            .map_err(|_| ())?;
        if attempt.plan().config_cell_ids.len() != 1 {
            return Err(());
        }
        let config_id = attempt.plan().config_cell_ids[0];
        let configs = attempt.acquire_attempt_configs().map_err(|_| ())?;
        let candidate: crate::server::publication::CandidateBindingV1 =
            serde_json::from_slice(&configs.value(config_id).ok_or(())?.bytes).map_err(|_| ())?;
        for product_profile in candidate.protocol_profiles {
            let profile: profiles::CandidateProtocolProfile =
                serde_json::from_value(serde_json::to_value(product_profile).map_err(|_| ())?)
                    .map_err(|_| ())?;
            if profile.ingress_protocol != ingress
                || profile.capability.native_provider_state
                    != NativeProviderStateEmission::ExactOwnerAffine
                || profile.capability.request.provider_state != Fidelity::Exact
                || profile.capability.request.state_affinity != StateAffinity::ExactOwner
            {
                continue;
            }
            let candidate_owner = profile.exact_provider_path().map_err(|_| ())?;
            match owner.as_ref() {
                None => owner = Some(candidate_owner),
                Some(current) if current == &candidate_owner => {}
                Some(_) => return Ok(None),
            }
        }
    }
    Ok(owner)
}

#[derive(Debug)]
enum ReplayBodyError {
    Deadline,
    Transport(TransportError),
    Limit,
    InvalidFraming,
    PlanUnavailable,
    Replay,
}

async fn read_replay_body(
    session: &mut dyn GatewaySession,
    ingress_prefix: Vec<ChargedBytes>,
    declared_content_length: Option<usize>,
    authorized: &AuthorizedRequestPlan,
    replay: &ReplayStore,
) -> Result<ContentRef, ReplayBodyError> {
    let plan = authorized.logical_request_plan();
    let Some(hard_limit) = plan.body_plan.max_retained_bytes() else {
        return Err(ReplayBodyError::PlanUnavailable);
    };
    let mut owner = BodyPlanExecutor::new(
        BodyDirection::LogicalRequest,
        plan.body_plan.clone(),
        hard_limit,
    )
    .map_err(map_body_error)?;
    if let Some(content_length) = declared_content_length {
        owner
            .preflight_content_length(content_length)
            .map_err(map_body_error)?;
    }
    let mut body = replay.begin_raw().map_err(|_| ReplayBodyError::Replay)?;
    for chunk in ingress_prefix {
        owner
            .admit_chunk(chunk.bytes().len())
            .map_err(map_body_error)?;
        body.append(chunk.bytes())
            .map_err(|_| ReplayBodyError::Replay)?;
    }
    loop {
        let chunk = session
            .read_request_body_before(authorized.deadline())
            .await
            .map_err(|error| match error {
                TransportError::DeadlineExceeded => ReplayBodyError::Deadline,
                error => ReplayBodyError::Transport(error),
            })?;
        let Some(chunk) = chunk else {
            break;
        };
        owner.admit_chunk(chunk.len()).map_err(map_body_error)?;
        body.append(&chunk).map_err(|_| ReplayBodyError::Replay)?;
    }
    owner.finish().map_err(map_body_error)?;
    body.seal().map_err(|_| ReplayBodyError::Replay)
}

fn map_body_error(error: BodyError) -> ReplayBodyError {
    match error {
        BodyError::BodyLimitExceeded | BodyError::BudgetExceeded => ReplayBodyError::Limit,
        BodyError::ContentLengthMismatch
        | BodyError::BodyAfterEos
        | BodyError::DuplicateBodyEos => ReplayBodyError::InvalidFraming,
        _ => ReplayBodyError::PlanUnavailable,
    }
}

fn serialize_body(value: serde_json::Value) -> Result<Bytes, TransportError> {
    serde_json::to_vec(&value)
        .map(Bytes::from)
        .map_err(|error| TransportError::Io(error.to_string().into()))
}

async fn write_catalog_error(
    session: &mut dyn GatewaySession,
    error: CatalogError,
) -> Result<SessionReuse, TransportError> {
    let (status, code) = match error {
        CatalogError::PublicationUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "GATEWAY_PUBLICATION_UNAVAILABLE",
        ),
        CatalogError::Unauthorized => (StatusCode::UNAUTHORIZED, "GATEWAY_GRANT_UNAUTHORIZED"),
        CatalogError::Json(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "MODEL_CATALOG_RENDER_FAILED",
        ),
    };
    write_typed_error(session, status, code).await
}

async fn write_dispatch_error(
    session: &mut dyn GatewaySession,
    error: DispatchError,
) -> Result<SessionReuse, TransportError> {
    write_typed_error(session, error.status(), error.code()).await
}

async fn write_typed_error(
    session: &mut dyn GatewaySession,
    status: StatusCode,
    code: &'static str,
) -> Result<SessionReuse, TransportError> {
    write_typed_error_phase(session, status, code, "request_authority").await
}

async fn write_typed_error_phase(
    session: &mut dyn GatewaySession,
    status: StatusCode,
    code: &'static str,
    phase: &'static str,
) -> Result<SessionReuse, TransportError> {
    let body = serialize_body(serde_json::json!({
        "schema_version": "hiroute.gateway.error/v1",
        "code": code,
        "phase": phase,
    }))?;
    write_response(session, status, body, None).await
}

async fn write_response(
    session: &mut dyn GatewaySession,
    status: StatusCode,
    body: Bytes,
    etag: Option<&str>,
) -> Result<SessionReuse, TransportError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len() as u64));
    headers.insert(CONNECTION, HeaderValue::from_static("close"));
    if let Some(etag) = etag {
        headers.insert(
            ETAG,
            HeaderValue::from_str(etag)
                .map_err(|error| TransportError::Io(error.to_string().into()))?,
        );
    }
    session
        .write_response_head(GatewayResponseHead { status, headers })
        .await?;
    session.write_response_body(body, true).await?;
    Ok(SessionReuse::Close)
}

#[cfg(test)]
#[path = "core_runtime/inbound_auth_tests.rs"]
mod inbound_auth_tests;

#[cfg(test)]
#[path = "core_runtime/acceptance_tests.rs"]
mod acceptance_tests;
