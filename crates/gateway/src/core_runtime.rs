//! Product composition root around publication, authority and catalog.

#[path = "adapters/mod.rs"]
pub mod adapters;
#[path = "core_runtime/classification.rs"]
mod classification;
pub use classification::{
    CClassifierDiagnosticOutcome, CLASSIFIER_DIAGNOSTIC_LATEST_USER, ClassifierDiagnosticError,
    ClassifierDiagnosticOutcome,
};
#[path = "core_runtime/decision_history.rs"]
mod decision_history;
#[path = "model_ir/mod.rs"]
pub mod model_ir;
#[path = "core_runtime/native_routing_view.rs"]
mod native_routing_view;
#[path = "observation.rs"]
pub mod observation;
#[path = "profiles/mod.rs"]
pub mod profiles;
#[path = "core_runtime/timing.rs"]
mod timing;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use bytes::Bytes;
use hiroute_gateway_core::runtime::body::{
    BodyDirection, BodyError, BodyPlanExecutor, MemoryRole, StreamBudget,
};
use hiroute_gateway_core::runtime::driver::{
    BoundRequestAdmission, DecisionCandidateAuthority, GatewayCoreLifecycle,
    GatewayCoreLifecycleLimits, NoopGatewayFilterManager, ObservationLabel,
};
use hiroute_gateway_core::runtime::timing::{LocalPhase, WaitKind, set_phase, wait};
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
    ContentRef, compact_ingress_document_with_markers, externalize_model_request,
    model_content_refs, parse_ingress_document, scan_ingress_document,
};
use crate::context_hold::{ContextHoldStore, ContextRequest, HoldCompletion, prepare_context};
use crate::replay::{ReplayError, ReplayManager, ReplayReader, ReplayStore, ReplayWriter};
#[path = "core_runtime/credentials.rs"]
mod credentials;
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
use crate::server::publication::{CatalogError, GatewayCatalog};
use crate::server::request_plan::{AuthorizedRequestPlan, IngressProtocol};

use self::observation::{
    AcceptedResponseCapture, GatewayObservation, ObservedCredentialResolver,
    ObservedProductionProvider, ObservedProductionSelection, ObservedRuntimeStateStore,
    RequestObservation, with_active_request,
};

#[derive(Clone)]
pub struct ProductionGatewayRuntime {
    upgrade: Arc<hiroute_host_runtime::UpgradeDrain>,
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
                ObservedProductionProvider::new(ProductionProvider::for_gateway_model_execution(
                    &ports,
                )),
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
            upgrade: Arc::new(hiroute_host_runtime::UpgradeDrain::default()),
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
            executable_sha256: None,
            capture_observation_content: true,
        }
    }

    pub fn with_executable_sha256(mut self, digest: impl Into<Arc<str>>) -> Self {
        self.executable_sha256 = Some(digest.into());
        self
    }

    pub fn with_upgrade_drain(mut self, upgrade: Arc<hiroute_host_runtime::UpgradeDrain>) -> Self {
        self.upgrade = upgrade;
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
        timing::process(self, session).await
    }
}

impl ProductionGatewayRuntime {
    async fn process_request(
        &self,
        session: &mut dyn GatewaySession,
        request: GatewayRequestHead,
    ) -> Result<SessionReuse, TransportError> {
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
        // Existing delegated runs still need model calls to finish. The ordinary run bearer
        // verifier below remains the authority; this prefix never authorizes an aggregate call.
        let Some(_upgrade_call) = self
            .upgrade
            .enter(authorization.is_some_and(|a| a.starts_with("Bearer hr_run_model_")))
        else {
            return write_typed_error(session, StatusCode::SERVICE_UNAVAILABLE, "UPGRADE_WAITING")
                .await;
        };
        let declared_content_length = request
            .headers
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());
        if request.method == Method::GET
            && (path == "/v1/models"
                || path.strip_prefix(hiroute_domain::QODER_MODEL_BASE_PATH) == Some("/models"))
        {
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
        let Some(replay_manager) = &self.replay else {
            return write_typed_error_phase(
                session,
                StatusCode::SERVICE_UNAVAILABLE,
                "REPLAY_STORE_UNAVAILABLE",
                "canonical_request",
            )
            .await;
        };
        let mut ingress = match IngressReplay::begin(replay_manager, budget.clone()) {
            Ok(ingress) => ingress,
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
        let selector_charge =
            match budget.reserve(MemoryRole::ModelIrBacking, ModelSelector::MEMORY_BYTES) {
                Ok(charge) => charge,
                Err(_) => {
                    return write_typed_error_phase(
                        session,
                        StatusCode::SERVICE_UNAVAILABLE,
                        "REPLAY_BUDGET_UNAVAILABLE",
                        "model_selector",
                    )
                    .await;
                }
            };
        let selector_deadline = authenticated.selector_deadline();
        let mut selector = Box::new(ModelSelector::new(self.authority().selector_limit()));
        let served_model_id = loop {
            match wait(
                WaitKind::ClientRead,
                session.read_request_body_before(selector_deadline),
            )
            .await
            {
                Err(TransportError::DeadlineExceeded) => {
                    return write_dispatch_error(session, DispatchError::RequestDeadlineExceeded)
                        .await;
                }
                Err(error) => return Err(error),
                Ok(Some(chunk)) => {
                    // One raw owner from the first authenticated byte, including
                    // the remainder of the chunk containing the selected alias.
                    if let Err(error) = ingress.append_prefix(&chunk) {
                        let (status, code, phase) = match error {
                            ReplayError::Body(BodyError::BudgetExceeded) => (
                                StatusCode::SERVICE_UNAVAILABLE,
                                "REPLAY_BUDGET_UNAVAILABLE",
                                "model_selector",
                            ),
                            _ => (
                                StatusCode::BAD_REQUEST,
                                "REPLAY_INTEGRITY_FAILED",
                                "canonical_request",
                            ),
                        };
                        return write_typed_error_phase(session, status, code, phase).await;
                    }
                    match selector.feed(&chunk) {
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
        drop(selector_charge);
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
            ingress,
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
        ingress: IngressReplay,
        mut authorized: Box<AuthorizedRequestPlan>,
        budget: StreamBudget,
    ) -> Pin<Box<dyn Future<Output = Result<SessionReuse, TransportError>> + Send + 'a>> {
        Box::pin(async move {
            set_phase(LocalPhase::Replay);
            let (replay, raw_body) = match read_replay_body(
                session,
                ingress,
                declared_content_length,
                &authorized,
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
            set_phase(LocalPhase::Parse);
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
            let (mut document, mut ingress_workspace) =
                match parse_ingress_document(protocol, &replay, &raw_body, ingress_stats) {
                    Ok(parsed) => parsed,
                    Err(ReplayError::InvalidJson) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::BAD_REQUEST,
                            "PROTOCOL_DOCUMENT_INVALID",
                            "canonical_request",
                        )
                        .await;
                    }
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
            if let Err(error) = compact_ingress_document_with_markers(
                protocol,
                &mut document,
                &replay,
                ingress_workspace.generated_markers(),
            ) {
                let (status, code) = match error {
                    ReplayError::StructureLimit => (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "REQUEST_STRUCTURE_LIMIT_EXCEEDED",
                    ),
                    ReplayError::InvalidJson => {
                        (StatusCode::BAD_REQUEST, "PROTOCOL_DOCUMENT_INVALID")
                    }
                    _ => (StatusCode::SERVICE_UNAVAILABLE, "CONTENT_REF_UNAVAILABLE"),
                };
                return write_typed_error_phase(session, status, code, "canonical_request").await;
            }
            set_phase(LocalPhase::Parse);
            let parse_started = Instant::now();
            let identity_document = crate::context_hold::context_document(&document);
            let mut canonical_request = match adapters::decode_owned_ingress_request_with_replay(
                protocol, document, &replay,
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
            if let Some(profile) = fixed_reasoning {
                let digest = hiroute_domain::CanonicalDigest::of(&profile)
                    .expect("resolved reasoning profile is serializable");
                canonical_request.requested_reasoning.fixed_profile_digest = Some(digest);
                canonical_request.requested_reasoning.fixed_profile = Some(profile);
                canonical_request.requested_reasoning.disposition =
                    model_ir::RequestedReasoningDisposition::AppliedToFixedBinding;
            }
            // Move large canonical content into the request-owned aggregate pool
            // before candidate facts or Planner input are built. Planner owns and
            // later returns this compact IR; no full request clone is retained.
            set_phase(LocalPhase::Externalize);
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
            set_phase(LocalPhase::History);
            let routing_view = native_routing_view::NativeRoutingView::prepare(
                &request.headers,
                &canonical_request,
                &replay,
                &budget,
            );
            let routing_request = routing_view.request(&canonical_request);
            let (identity, prepared_hold, native_user_unavailable) = prepare_context(
                &self.context_holds,
                &self.observation.correlation_key(),
                &authorized,
                ContextRequest {
                    ingress: protocol,
                    headers: &request.headers,
                    document: &identity_document,
                    request: &canonical_request,
                    semantic_request: Some(routing_request),
                    user_origin_unverified: routing_view.origin_unverified()
                        || request
                            .path_and_query
                            .starts_with(hiroute_domain::QODER_MODEL_BASE_PATH),
                    replay: &replay,
                },
                Instant::now(),
            );
            let current_input_origin_unverified = routing_view.origin_unverified()
                || identity.input_origin_unverified(prepared_hold.as_ref());
            drop(identity_document);
            drop(ingress_workspace);
            set_phase(LocalPhase::Prevalidate);
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
            set_phase(LocalPhase::Prepare);
            let request_observation = self.observation.begin_request_with_content(
                &authorized,
                protocol,
                &identity,
                self.capture_observation_content,
                Some((&request.headers, routing_view.projection())),
            );
            let _observation_guard = ObservationRequestGuard(request_observation.clone());
            request_observation.record_pipeline_stages(
                parse_elapsed,
                externalize_elapsed,
                prevalidate_elapsed,
            );
            request_observation.record_input_selection(
                routing_view.excerpt_selection(&canonical_request, current_input_origin_unverified),
            );
            request_observation.capture_request_async(&canonical_request, &replay);
            let mut hold_ticket;
            let mut agent_turn_guard = None;
            let mut correlated_branch = None;
            let mut classification_context = None;
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
                let capture_scope = self.context_holds.digest_key().and_then(|key| {
                    decision_history::CaptureScope::authorized(
                        &authorized,
                        protocol,
                        &identity,
                        &key,
                        prepared_hold.as_ref(),
                    )
                });
                set_phase(LocalPhase::History);
                let tracking = match self.begin_authorized_decision_history(
                    &request_observation,
                    PlanSnapshot {
                        plan_id: plan_id.clone(),
                        plan_revision: *revision,
                    },
                    capture_scope,
                    prepared_hold,
                    decision_history::HistoryInput {
                        request: &canonical_request,
                        replay: &replay,
                        current_input: routing_view.current_input(),
                        current_input_position: routing_view.current_input_position(),
                        current_input_unavailable: routing_view.current_input_unavailable(),
                    },
                ) {
                    Ok(tracking) => tracking,
                    Err(_) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::BAD_REQUEST,
                            "REPLAY_INTEGRITY_FAILED",
                            "agent_turn",
                        )
                        .await;
                    }
                };
                agent_turn_guard = tracking.guard;
                correlated_branch = tracking.correlated;
                hold_ticket = tracking.hold_ticket;
                let turn_history = tracking.history;
                set_phase(LocalPhase::Decision);
                let mut classification = Box::pin(self.classify_for_planner(
                    &strategy,
                    classification::ClassificationRequest {
                        request: routing_request,
                        current_input: routing_view.current_input(),
                        native_user_unavailable: native_user_unavailable
                            || routing_view.current_input_unavailable(),
                        current_input_origin_unverified,
                        replay: &replay,
                        classifier: authorized.classifier(),
                        history: &turn_history,
                        correlated: correlated_branch.as_deref(),
                        publication_revision: authorized.publication_revision(),
                        overall_deadline: authorized.deadline(),
                        cancellation: classification_cancellation.clone(),
                        started_at: classification_started,
                        admitted_credential_state: None,
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
                        let _ = (&mut classification).await;
                        Err(classification::ClassificationError::Cancelled)
                    }
                };
                drop(classification);
                match classification_result {
                    Ok(mut result) => {
                        use hiroute_domain::DecisionProtocolV1 as P;
                        result.decision.decision_protocol = Some(match authorized.classifier() {
                            Some(c) if c.native_protocol.is_some() => P::SystemOneC,
                            Some(c) if c.system_one_model.is_some() => P::SystemOneLegacy,
                            Some(_) => P::CustomExtensionV1,
                            None => P::LocalRules,
                        });
                        if agent_turn_guard.is_none()
                            && !result
                                .decision
                                .reason_codes
                                .contains(&profiles::ComplexityReasonCodeV1::TaskContextUnresolved)
                        {
                            result
                                .decision
                                .reason_codes
                                .push(profiles::ComplexityReasonCodeV1::TaskContextUnresolved);
                        }
                        classification_context = Some(turn_history);
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
                        request_observation.finish("cancelled");
                        return Err(TransportError::Cancelled);
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
                hold_ticket = prepared_hold
                    .and_then(|prepared| self.context_holds.commit(prepared, Instant::now()))
                    .map(Box::new);
                None
            };
            drop(routing_view);
            set_phase(LocalPhase::Prepare);
            let prepared_planner_input =
                match with_active_request(request_observation.clone(), async {
                    self.planner_inputs
                        .build_prepared_input(*canonical_request, &authorized, &budget)
                        .map(Box::new)
                })
                .await
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
            let mut planner_input = Box::new(prepared_planner_input.input);
            let mut prepared_templates = prepared_planner_input.templates;
            planner_input.context_hold =
                hold_ticket.as_ref().and_then(|ticket| ticket.hint.clone());
            if let Some(previous) = hold_ticket
                .as_ref()
                .and_then(|ticket| ticket.previous_success.as_ref())
            {
                let mut matches = planner_input.candidates.iter().filter(|candidate| {
                    candidate.candidate_id == previous.candidate_id
                        && candidate.stable_binding_id == previous.stable_binding_id
                        && candidate.profile_digest == previous.profile_digest
                });
                if let Some(candidate) = matches.next()
                    && matches.next().is_none()
                {
                    planner_input.previous_success_candidate_id =
                        Some(candidate.candidate_id.clone());
                }
            }
            planner_input.correlated_branch = correlated_branch.map(|decision| *decision);
            let mut assessment = None;
            if let Some(mut outcome) = classification_outcome {
                if let Some(history) = classification_context {
                    classification::apply_group_policy(&planner_input, &history, &mut outcome);
                    if let Some(guard) = agent_turn_guard.as_mut()
                        && guard.ticket().new_turn
                        && guard.commit_decision_or_defer(&outcome.decision).is_err()
                    {
                        // History writes are optional. Keep this request's validated
                        // decision, but relinquish all shared history write authority.
                        agent_turn_guard = None;
                        hold_ticket = None;
                        planner_input.context_hold = None;
                        planner_input.previous_success_candidate_id = None;
                        planner_input.correlated_branch = None;
                        if !outcome
                            .decision
                            .reason_codes
                            .contains(&profiles::ComplexityReasonCodeV1::TaskContextUnresolved)
                        {
                            outcome
                                .decision
                                .reason_codes
                                .push(profiles::ComplexityReasonCodeV1::TaskContextUnresolved);
                        }
                    }
                }
                if let Some(guard) = agent_turn_guard.as_ref() {
                    guard.withhold_pending_affinity(&mut hold_ticket, &mut planner_input);
                    request_observation.bind_agent_turn_output(
                        Arc::clone(&self.agent_turn_history),
                        guard.ticket().clone(),
                    );
                }
                planner_input.classification_decision = Some(outcome.decision);
                planner_input.classification_facts = Some(outcome.facts);
                assessment = outcome.assessment;
            }
            // The Planner is invoked exactly once for this request. Everything
            // below consumes its frozen output; no runtime fallback path may
            // reconstruct order from the publication candidate set.
            set_phase(LocalPhase::Plan);
            let plan_started = Instant::now();
            let planner_output = match profiles::Planner
                .plan_published(&planner_input, &authorized.planner_facts())
                .map(Box::new)
            {
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
            set_phase(LocalPhase::Prepare);
            request_observation.record_planner(
                &planner_input,
                &planner_output,
                identity.native_auxiliary(),
            );
            // The planner establishes the trusted Plan identity used by every execution fact.
            if let Some(assessment) = &assessment {
                request_observation.branch_assessment_recorded(assessment);
            }
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
            let mut admission_preparation_failure: Option<crate::runtime::PreparationFailure> =
                None;
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
                let config = configs.value(config_id).ok_or_else(|| {
                    TransportError::Io("sealed candidate config is unavailable".into())
                })?;
                let compiled = authorized
                    .candidate_config(binding.local_id(), config)
                    .ok_or_else(|| {
                        TransportError::Io("sealed candidate config is invalid".into())
                    })?;
                if compiled.binding.local_id != binding.local_id()
                    || compiled.binding.stable_target_key != frozen.stable_binding_id
                {
                    return Err(TransportError::Io(
                        "sealed candidate profile mismatch".into(),
                    ));
                }
                let execution = compiled
                    .execution_for(facts, &planner_input.request)
                    .map_err(|_| TransportError::Io("sealed candidate profile mismatch".into()))?;
                let pool = with_active_request(request_observation.clone(), async {
                    credentials::bind_current(&self.ports, &mut authorized, binding, &execution)
                })
                .await;
                let excluded = match pool {
                    Ok(false) => Some(crate::runtime::PreparationFailure::CredentialUnavailable),
                    Err(crate::server::composition::PortError::CredentialDenied(reason)) => {
                        Some(match reason {
                            hiroute_domain::NativeCredentialAccessDeniedV1::SourceDisabled => {
                                crate::runtime::PreparationFailure::SourceDisabled
                            }
                            hiroute_domain::NativeCredentialAccessDeniedV1::CredentialDisabled => {
                                crate::runtime::PreparationFailure::CredentialDisabled
                            }
                        })
                    }
                    Ok(true) => None,
                    Err(_) => {
                        return write_typed_error_phase(
                            session,
                            StatusCode::BAD_GATEWAY,
                            "CREDENTIAL_AUTHORITY_FAILED",
                            "credential",
                        )
                        .await;
                    }
                };
                if let Some(cause) = excluded {
                    admission_preparation_failure = Some(
                        admission_preparation_failure
                            .map_or(cause, |previous| previous.merge(cause)),
                    );
                    continue;
                }
                let attempt = authorized
                    .core_binding()
                    .resolve_attempt(binding)
                    .map_err(|_| {
                        TransportError::Io("current candidate binding unavailable".into())
                    })?;
                let profile = execution.bytes().clone();
                prepared_templates.insert_execution(binding.local_id(), execution);
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
                    provider_profile: Some(profile),
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
                let (executed_branch_id, branch_execution) =
                    classification::execution_position(&planner_input, frozen);
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
                        branch_execution,
                    ),
                );
            }
            drop(fact_by_id);
            if frozen_candidates.is_empty() {
                let (code, phase) = admission_preparation_failure
                    .unwrap_or(crate::runtime::PreparationFailure::Other)
                    .code_phase();
                return write_typed_error_phase(session, StatusCode::BAD_GATEWAY, code, phase)
                    .await;
            }
            let switch_fallback = planner_input
                .previous_success_candidate_id
                .as_deref()
                .filter(|id| {
                    planner_output
                        .ledger
                        .ordered_candidates
                        .first()
                        .is_some_and(|first| first.candidate_id != *id)
                        && planner_output
                            .ledger
                            .ordered_candidates
                            .get(1)
                            .is_some_and(|second| second.candidate_id == *id)
                })
                .and_then(|id| {
                    frozen_candidates
                        .iter()
                        .find(|candidate| {
                            candidate
                                .candidate_id
                                .as_ref()
                                .is_some_and(|candidate_id| candidate_id.as_str() == id)
                        })
                        .map(|candidate| candidate.binding)
                })
                .filter(|binding| {
                    frozen_candidates
                        .first()
                        .is_some_and(|first| first.binding != *binding)
                });
            let canonical_request = Box::new(planner_input.request);
            let reused_cleanup = hold_ticket.as_ref().and_then(|ticket| {
                let prefix = ticket.cleaned_prefix_len?;
                let previous = ticket.previous_success.as_ref()?;
                hold_candidates
                    .iter()
                    .find(|(_, candidate)| *candidate == *previous)
                    .map(|(binding, _)| (*binding, prefix))
            });
            let cleanup_eligible = hold_ticket
                .as_ref()
                .filter(|ticket| {
                    ticket.history_continuity != crate::context_hold::HistoryContinuity::Continuing
                })
                .and(planner_input.previous_success_candidate_id.as_ref())
                .and_then(|previous| {
                    let first = frozen_candidates.first()?;
                    let first_id = first.candidate_id.as_ref()?.as_str();
                    if first_id == previous {
                        return None;
                    }
                    let profile = &planner_input
                        .candidates
                        .iter()
                        .find(|candidate| candidate.candidate_id == first_id)?
                        .protocol_profile;
                    let normal =
                        adapters::project_candidate_request_template(&canonical_request, profile)
                            .ok()?;
                    let cleaned = adapters::project_candidate_request_template_with_cleanup(
                        &canonical_request,
                        profile,
                        Some(canonical_request.messages.len()),
                    )
                    .ok()?;
                    (normal.bytes != cleaned.bytes)
                        .then_some((first.binding, canonical_request.messages.len()))
                });
            let reasoning_cleanup = Arc::new(crate::runtime::ReasoningCleanup::new(
                cleanup_eligible,
                reused_cleanup,
            ));
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
            let active_continuation =
                adapters::ActiveResponseDelivery::new(protocol, budget.clone());
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
                switch_fallback,
                reasoning_cleanup,
                admission_preparation_failure,
                hold_completion: hold_ticket.map(|ticket| HoldCompletion {
                    ticket: *ticket,
                    candidates: hold_candidates.into(),
                }),
            };
            let seed = Arc::new(ProductionReplaySeedEnvelope::new(ProductionReplaySeed {
                templates: prepared_templates,
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
            set_phase(LocalPhase::Stream);
            let execution = Box::pin(self.execution.process_bound_with_context(
                &mut *core_session,
                admission,
                budget,
                seed,
            ));
            let execution = adapters::with_active_response_delivery(active_continuation, execution);
            let execution = Box::pin(with_active_request(request_observation.clone(), execution));
            let result = execution.await;
            #[cfg(feature = "e2e-test-control")]
            if let Err(error) = &result {
                eprintln!("Gateway fixture execution failed: {error:?}");
            }
            let response_started = core_session.response_started;
            drop(core_session);
            let runtime_state_authority_failed =
                !response_started && runtime_state_authority.failed();
            let request_cancelled = request_observation.accepted_attempt_cancelled()
                || matches!(&result, Err(TransportError::Cancelled));
            let model_failed = result.is_ok() && request_observation.accepted_attempt_failed();
            let observation_outcome = if model_failed {
                "failed"
            } else {
                classify_request_observation_outcome(
                    request_observation.has_accepted_attempt(),
                    result.is_ok(),
                    request_cancelled,
                )
            };
            set_phase(LocalPhase::History);
            if let Some(mut guard) = agent_turn_guard.take() {
                wait(
                    WaitKind::LocalWork,
                    request_observation.finish_agent_turn_output(),
                )
                .await;
                let executions = request_observation
                    .accepted_attempt_identity()
                    .and_then(|(candidate_id, _, _)| {
                        agent_turn_candidates.get(&candidate_id).map(
                            |(
                                model_configuration_id,
                                profile_digest,
                                executed_branch_id,
                                branch_execution,
                            )| {
                                vec![AcceptedExecution {
                                    branch_execution: branch_execution.clone(),
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
                    (_, _) if request_cancelled => AgentTurnStatus::Interrupted,
                    (true, true) if model_failed => AgentTurnStatus::Failed,
                    (true, true) if request_observation.accepted_attempt_unknown() => {
                        AgentTurnStatus::Unknown
                    }
                    (true, true) => AgentTurnStatus::Completed,
                    (true, false) => AgentTurnStatus::Interrupted,
                    (false, _) => AgentTurnStatus::Failed,
                };
                if guard
                    .finish_deferred_decision(planner_input.classification_decision.as_ref())
                    .is_ok()
                    && let Ok(completed) = self.agent_turn_history.finish_request(
                        guard.ticket(),
                        request_observation.metadata().request_id.clone(),
                        status,
                        executions,
                        // Delivered tools keep a successful stage open for the
                        // client's continuation. They cannot keep a cancelled
                        // or failed request alive indefinitely.
                        matches!(
                            status,
                            AgentTurnStatus::Interrupted | AgentTurnStatus::Failed
                        ) || response_delivery.accepted_count() == 0,
                        Instant::now(),
                    )
                {
                    if let Some(completed) = completed {
                        request_observation.agent_turn_finished(&completed);
                    }
                    guard.disarm();
                }
            }
            request_observation.finish(observation_outcome);
            if runtime_state_authority_failed && !request_cancelled {
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

fn classify_request_observation_outcome(
    accepted: bool,
    completed: bool,
    request_cancelled: bool,
) -> &'static str {
    match (accepted, completed, request_cancelled) {
        (_, _, true) => "cancelled",
        (true, true, false) => "accepted",
        (true, false, false) => "postcommit_transport_failed",
        (false, _, _) => "failed",
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
                excluded(Reason::ContextLimitUnknown),
            ]),
            (StatusCode::BAD_GATEWAY, "NO_ELIGIBLE_CANDIDATE")
        );
    }
}

fn inbound_authorization<'a>(
    path: &str,
    headers: &'a HeaderMap,
) -> Option<std::borrow::Cow<'a, str>> {
    if path
        .strip_prefix(hiroute_domain::QODER_MODEL_BASE_PATH)
        .is_some_and(|resource| matches!(resource, "/responses" | "/messages" | "/models"))
    {
        // This explicit native entry consumes only local model-grant credentials.
        // Normal publication authentication still rejects unissued/native account
        // tokens; delegated run authority remains on its separate existing entry.
        if headers.contains_key("x-hiroute-token")
            || headers.get_all(AUTHORIZATION).iter().count() != 1
        {
            return None;
        }
        let authorization = headers.get(AUTHORIZATION)?.to_str().ok()?;
        let token = authorization.strip_prefix("Bearer ")?;
        if token.is_empty()
            || token.starts_with("hr_run_")
            || token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte == b',')
        {
            return None;
        }
        return Some(std::borrow::Cow::Borrowed(authorization));
    }
    if headers.contains_key("x-hiroute-token") {
        if !matches!(
            path,
            "/v1/responses" | "/v1/chat/completions" | "/v1/messages" | "/v1/models"
        ) || headers.get_all("x-hiroute-token").iter().count() != 1
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
    // Native Anthropic clients also use x-api-key for model-grant credentials,
    // including auxiliary calls that do not load an Agent's request middleware.
    // Higher-priority carriers remain authoritative, even when they are invalid.
    if path == "/v1/messages" && !headers.contains_key(AUTHORIZATION) {
        if headers.get_all("x-api-key").iter().count() != 1 {
            return None;
        }
        let token = headers.get("x-api-key")?.to_str().ok()?;
        if token.is_empty()
            || (token.starts_with("hr_run_") && !token.starts_with("hr_run_model_"))
            || token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte == b',')
        {
            return None;
        }
        // Extraction grants no authority: current publication/run verification
        // still resolves the exact issued token, protocol and model aliases.
        return Some(std::borrow::Cow::Owned(format!("Bearer {token}")));
    }
    if headers.get_all(AUTHORIZATION).iter().count() != 1 {
        return None;
    }
    let authorization = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = authorization.strip_prefix("Bearer ")?;
    if token.is_empty()
        || token
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte == b',')
    {
        return None;
    }
    // The carrier does not identify authority. Publication/run verification below
    // must still resolve an issued current token and its exact protocol/aliases.
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
    decision_deferred: bool,
}

impl AgentTurnRequestGuard {
    fn new(store: Arc<AgentTurnHistoryStore>, ticket: AgentTurnTicket) -> Self {
        Self {
            store,
            ticket: Some(ticket),
            decision_deferred: false,
        }
    }

    fn ticket(&self) -> &AgentTurnTicket {
        self.ticket.as_ref().expect("agent turn guard is armed")
    }

    fn disarm(&mut self) {
        self.ticket = None;
    }
}

#[path = "core_runtime/decision_history/deferred_decision.rs"]
mod deferred_decision;

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
        timing::write_head(self.inner, head).await?;
        self.response_capture.accepted_head(&captured);
        Ok(())
    }

    async fn write_response_body(
        &mut self,
        body: Bytes,
        end_stream: bool,
    ) -> Result<(), TransportError> {
        let captured = body.clone();
        wait(
            WaitKind::DownstreamWrite,
            self.inner.write_response_body(body, end_stream),
        )
        .await?;
        self.continuation_scanner
            .accept_bytes(&captured, Instant::now());
        self.response_capture.accepted_frame(&captured, end_stream);
        Ok(())
    }
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

// The prefix needs only two counters to enforce the eventual alias body plan.
// ReplayWriter owns its bytes before and after authorization; no prefix copy or
// per-frame metadata survives model selection.
struct IngressReplay {
    replay: ReplayStore,
    body: ReplayWriter,
    prefix_bytes: usize,
    largest_prefix_chunk: usize,
}

impl IngressReplay {
    fn begin(manager: &ReplayManager, budget: StreamBudget) -> Result<Self, ReplayError> {
        let replay = manager.begin_request(budget)?;
        let body = replay.begin_raw()?;
        Ok(Self {
            replay,
            body,
            prefix_bytes: 0,
            largest_prefix_chunk: 0,
        })
    }

    fn append_prefix(&mut self, chunk: &[u8]) -> Result<(), ReplayError> {
        self.body.append(chunk)?;
        self.prefix_bytes = self
            .prefix_bytes
            .checked_add(chunk.len())
            .ok_or(ReplayError::Integrity)?;
        self.largest_prefix_chunk = self.largest_prefix_chunk.max(chunk.len());
        Ok(())
    }
}

async fn read_replay_body(
    session: &mut dyn GatewaySession,
    ingress: IngressReplay,
    declared_content_length: Option<usize>,
    authorized: &AuthorizedRequestPlan,
) -> Result<(ReplayStore, ContentRef), ReplayBodyError> {
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
    owner
        .admit_prefix(ingress.prefix_bytes, ingress.largest_prefix_chunk)
        .map_err(map_body_error)?;
    let IngressReplay {
        replay, mut body, ..
    } = ingress;
    loop {
        let chunk = wait(
            WaitKind::ClientRead,
            session.read_request_body_before(authorized.deadline()),
        )
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
    let reference = body.seal().map_err(|_| ReplayBodyError::Replay)?;
    Ok((replay, reference))
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
    timing::write_head(session, GatewayResponseHead { status, headers }).await?;
    wait(
        WaitKind::DownstreamWrite,
        session.write_response_body(body, true),
    )
    .await?;
    Ok(SessionReuse::Close)
}

#[cfg(test)]
#[path = "core_runtime/inbound_auth_tests.rs"]
mod inbound_auth_tests;

#[cfg(test)]
#[path = "core_runtime/acceptance_tests.rs"]
mod acceptance_tests;

#[cfg(all(test, unix, debug_assertions, feature = "e2e-test-control"))]
#[path = "core_runtime/response_diagnostics_tests.rs"]
mod response_diagnostics_tests;
