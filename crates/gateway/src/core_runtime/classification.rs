use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hiroute_gateway_core::core::execution_plan::{AttemptBodyPlans, AttemptTimeouts, PlanRevision};
use hiroute_gateway_core::runtime::attempt::{
    AttemptError, AttemptExchange, AttemptGeneration, AttemptId, Disposition, PrecommitEvent,
    PreparedAttemptBody, PreparedAttemptHttpRequest, PreparedRequestHead, RequestId, WriterGate,
    WriterState,
};
use hiroute_gateway_core::runtime::body::{
    BodyError, BodyPlan, MemoryRole, RequestLeaseBook, Reservation,
};
use http::header::{CONTENT_LENGTH, CONTENT_TYPE, HOST};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use tokio_util::sync::CancellationToken;

#[path = "classification/protocol.rs"]
mod protocol;

use super::ProductionGatewayRuntime;
use super::adapters::{PreparedReplayTemplate, sequential_replay_body};
use super::model_ir::{
    CanonicalMessage, ContentPart, MODEL_REQUEST_IR_SCHEMA, MessageRole, ModelRequestIRV1,
    RequestedReasoningControl, ToolChoice,
};
use super::profiles::{
    BranchDecisionV1, ClassifierFallbackReasonV1, CompiledClassifierKindV1,
    CompiledComplexityStrategyV1, ComplexityDecisionSourceV1, ComplexityReasonCodeV1, ComplexityV1,
    CorrelatedBranchDecisionV1, SanitizedStructuralFactsV1,
};
use crate::agent_turn_history::{AgentTurnHistorySnapshot, AssessmentTarget};
use crate::content_ref::ContentValueExt;
use crate::ports::{ExecutionScope, HeaderSecretLeaseRequest};
use crate::replay::{ReplayError, ReplayStore};
use crate::runtime::{ProductionProvider, resolve_target};
use crate::server::request_plan::{
    ClassifierAuthenticationAuthorityV1, IngressProtocol, RestBranchClassifierAuthorityV1,
};
use protocol::{
    ClassifierAssessment, ClassifierResponse, classifier_request_template,
    parse_classifier_response,
};

const CLASSIFIER_CLEANUP_TIMEOUT: Duration = Duration::from_millis(250);
const CLASSIFIER_RESPONSE_BYTES: usize = 64 * 1024;
const CLASSIFIER_WRITE_QUANTUM: usize = 16 * 1024;
pub const CLASSIFIER_DIAGNOSTIC_LATEST_USER: &str =
    "This is a synthetic connectivity test. Choose the economy branch for this clear, local task.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassifierDiagnosticOutcome {
    pub branch_id: String,
    pub duration_millis: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassifierDiagnosticError {
    InvalidConfig,
    Timeout,
    Unavailable,
    RejectedInput,
    InvalidOutput,
    Cancelled,
    Resource,
    Integrity,
}

impl ClassifierDiagnosticError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidConfig => "CLASSIFIER_CONFIG_INVALID",
            Self::Timeout => "CLASSIFIER_TIMEOUT",
            Self::Unavailable => "CLASSIFIER_UNAVAILABLE",
            Self::RejectedInput => "CLASSIFIER_INPUT_REJECTED",
            Self::InvalidOutput => "CLASSIFIER_OUTPUT_INVALID",
            Self::Cancelled => "CLASSIFIER_TEST_CANCELLED",
            Self::Resource => "CLASSIFIER_TEST_RESOURCE_UNAVAILABLE",
            Self::Integrity => "CLASSIFIER_TEST_INTEGRITY_FAILED",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ClassificationError {
    Cancelled,
    Deadline,
    Integrity,
    Resource,
}

pub(super) struct ClassificationRequest<'a> {
    pub(super) request: &'a ModelRequestIRV1,
    pub(super) replay: &'a ReplayStore,
    pub(super) classifier: Option<&'a Arc<RestBranchClassifierAuthorityV1>>,
    pub(super) history: &'a AgentTurnHistorySnapshot,
    pub(super) correlated: Option<&'a CorrelatedBranchDecisionV1>,
    pub(super) publication_revision: u64,
    pub(super) overall_deadline: Instant,
    pub(super) cancellation: CancellationToken,
    pub(super) started_at: Instant,
}

pub(super) struct ClassificationOutcome {
    pub(super) decision: BranchDecisionV1,
    pub(super) facts: SanitizedStructuralFactsV1,
    pub(super) assessment: Option<BoundAssessment>,
}

pub(crate) struct BoundAssessment {
    pub(crate) target: AssessmentTarget,
    pub(crate) score: f64,
    pub(crate) partial: bool,
    pub(crate) reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CallFailure {
    Timeout,
    Unavailable,
    RejectedInput,
    InvalidOutput,
}

struct ResolvedLatestUser {
    text: String,
    _reservation: Reservation,
}

impl ProductionGatewayRuntime {
    /// Runs the exact production REST classifier transport and parser against a fixed,
    /// non-user synthetic first turn. It never invokes the Planner or a business model and
    /// deliberately reports local-rules fallback as diagnostic failure.
    pub async fn test_classifier_decision(
        &self,
        mode: &hiroute_domain::ComplexityClassifierModeV1,
    ) -> Result<ClassifierDiagnosticOutcome, ClassifierDiagnosticError> {
        if !matches!(
            mode,
            hiroute_domain::ComplexityClassifierModeV1::Rest { .. }
        ) {
            return Err(ClassifierDiagnosticError::InvalidConfig);
        }
        mode.validate()
            .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
        let classifier = crate::server::publication::compile_classifier_mode_authority(mode, 1)
            .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
        let config_digest = hiroute_domain::CanonicalDigest::of(mode)
            .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
        let strategy = ComplexityV1::compile_with_classifier(
            Vec::<(String, String)>::new(),
            CompiledClassifierKindV1::Rest,
            Some(config_digest.as_str().to_owned()),
        )
        .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
        let budget = self
            .execution
            .allocate_bound_stream_budget()
            .map_err(|_| ClassifierDiagnosticError::Resource)?;
        let replay = self
            .replay
            .as_ref()
            .ok_or(ClassifierDiagnosticError::Resource)?
            .begin_request(budget)
            .map_err(|_| ClassifierDiagnosticError::Resource)?;
        replay
            .prevalidate(&[])
            .map_err(|_| ClassifierDiagnosticError::Integrity)?;
        let request = diagnostic_request();
        let history = AgentTurnHistorySnapshot {
            visible_conversation: Vec::new(),
            history_partial: false,
            assessment_from: None,
            assessment_target: None,
            _pin: None,
        };
        let started = Instant::now();
        let overall_deadline = started
            .checked_add(classifier.timeout)
            .ok_or(ClassifierDiagnosticError::Resource)?;
        let result = self
            .classify_for_planner(
                &strategy,
                ClassificationRequest {
                    request: &request,
                    replay: &replay,
                    classifier: Some(&classifier),
                    history: &history,
                    correlated: None,
                    publication_revision: 1,
                    overall_deadline,
                    cancellation: CancellationToken::new(),
                    started_at: started,
                },
            )
            .await
            .map_err(|error| match error {
                ClassificationError::Cancelled => ClassifierDiagnosticError::Cancelled,
                ClassificationError::Deadline => ClassifierDiagnosticError::Timeout,
                ClassificationError::Integrity => ClassifierDiagnosticError::Integrity,
                ClassificationError::Resource => ClassifierDiagnosticError::Resource,
            })?;
        if result.decision.fallback_used {
            return Err(match result.decision.fallback_reason {
                Some(ClassifierFallbackReasonV1::Timeout) => ClassifierDiagnosticError::Timeout,
                Some(ClassifierFallbackReasonV1::Unavailable) | None => {
                    ClassifierDiagnosticError::Unavailable
                }
                Some(ClassifierFallbackReasonV1::RejectedInput) => {
                    ClassifierDiagnosticError::RejectedInput
                }
                Some(ClassifierFallbackReasonV1::InvalidOutput) => {
                    ClassifierDiagnosticError::InvalidOutput
                }
            });
        }
        if result.decision.decision_source != ComplexityDecisionSourceV1::ExternalClassifier {
            return Err(ClassifierDiagnosticError::Integrity);
        }
        Ok(ClassifierDiagnosticOutcome {
            branch_id: result.decision.branch_id,
            duration_millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    pub(super) async fn classify_for_planner(
        &self,
        strategy: &CompiledComplexityStrategyV1,
        request: ClassificationRequest<'_>,
    ) -> Result<ClassificationOutcome, ClassificationError> {
        ensure_source_active(request.overall_deadline, &request.cancellation)?;
        let started = request.started_at;
        let fixed_deadline = request
            .classifier
            .and_then(|classifier| started.checked_add(classifier.timeout))
            .unwrap_or(request.overall_deadline);
        let call_deadline = fixed_deadline.min(request.overall_deadline);
        let fixed_timeout_wins = fixed_deadline < request.overall_deadline;

        let latest_user = resolve_latest_user(request.request, request.replay)?;
        ensure_source_active(request.overall_deadline, &request.cancellation)?;
        let (local_decision, facts) = ComplexityV1::decide(
            latest_user.as_ref().map(|value| value.text.as_str()),
            request.correlated,
            strategy,
        )
        .map_err(|_| ClassificationError::Integrity)?;

        if local_decision.decision_source == ComplexityDecisionSourceV1::Inherited
            || strategy.classifier_kind == CompiledClassifierKindV1::LocalRules
        {
            return Ok(ClassificationOutcome {
                decision: local_decision,
                facts,
                assessment: None,
            });
        }

        let result = match request.classifier {
            Some(classifier) => {
                let template = classifier_request_template(
                    request.request,
                    request.history,
                    &classifier.branches,
                )
                .map_err(|_| ClassificationError::Integrity)?;
                match ensure_call_active(
                    call_deadline,
                    request.overall_deadline,
                    &request.cancellation,
                ) {
                    Ok(()) => {
                        self.call_classifier(
                            classifier,
                            template,
                            request.replay,
                            request.publication_revision,
                            call_deadline,
                            request.overall_deadline,
                            fixed_timeout_wins,
                            request.cancellation.clone(),
                            request.history.assessment_target.is_some(),
                        )
                        .await?
                    }
                    Err(ClassificationError::Deadline) if fixed_timeout_wins => {
                        Err(CallFailure::Timeout)
                    }
                    Err(error) => return Err(error),
                }
            }
            None => Err(CallFailure::Unavailable),
        };
        ensure_source_active(request.overall_deadline, &request.cancellation)?;
        let classification_duration_micros =
            u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        match result {
            Ok(response) => {
                let reason_codes = if response.invalid_assessment {
                    vec![ComplexityReasonCodeV1::ClassifierAssessmentInvalid]
                } else {
                    Vec::new()
                };
                let assessment = response.assessment.and_then(|assessment| {
                    request
                        .history
                        .assessment_target
                        .clone()
                        .map(|target| bind_assessment(target, assessment))
                });
                Ok(ClassificationOutcome {
                    decision: BranchDecisionV1 {
                        strategy_id: strategy.strategy_id.clone(),
                        schema_version: strategy.schema_version.clone(),
                        payload_digest: strategy.payload_digest.clone(),
                        branch_id: response.branch_id,
                        complexity_score: None,
                        threshold: None,
                        decision_source: ComplexityDecisionSourceV1::ExternalClassifier,
                        reason_codes,
                        matched_user_phrase_ids: Vec::new(),
                        fallback_used: false,
                        classification_duration_micros: Some(classification_duration_micros),
                        fallback_reason: None,
                    },
                    facts,
                    assessment,
                })
            }
            Err(failure) => {
                let mut fallback = local_decision;
                fallback.fallback_used = true;
                fallback.classification_duration_micros = Some(classification_duration_micros);
                fallback.fallback_reason = Some(match failure {
                    CallFailure::Timeout => ClassifierFallbackReasonV1::Timeout,
                    CallFailure::Unavailable => ClassifierFallbackReasonV1::Unavailable,
                    CallFailure::RejectedInput => ClassifierFallbackReasonV1::RejectedInput,
                    CallFailure::InvalidOutput => ClassifierFallbackReasonV1::InvalidOutput,
                });
                Ok(ClassificationOutcome {
                    decision: fallback,
                    facts,
                    assessment: None,
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn call_classifier(
        &self,
        classifier: &RestBranchClassifierAuthorityV1,
        template: PreparedReplayTemplate,
        replay: &ReplayStore,
        publication_revision: u64,
        call_deadline: Instant,
        overall_deadline: Instant,
        fixed_timeout_wins: bool,
        cancellation: CancellationToken,
        has_assessment_target: bool,
    ) -> Result<Result<ClassifierResponse, CallFailure>, ClassificationError> {
        let scope = ExecutionScope::new(call_deadline, cancellation.clone());
        ensure_call_active(call_deadline, overall_deadline, &cancellation)?;

        let mut headers = HeaderMap::new();
        headers.insert(
            HOST,
            HeaderValue::from_str(&classifier.http_authority)
                .map_err(|_| ClassificationError::Integrity)?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            CONTENT_LENGTH,
            HeaderValue::from(
                u64::try_from(template.wire_len).map_err(|_| ClassificationError::Integrity)?,
            ),
        );
        if let ClassifierAuthenticationAuthorityV1::Header {
            name,
            value_secret_ref,
        } = &classifier.authentication
        {
            let lease = self
                .ports
                .credentials
                .lease_header_secret(
                    HeaderSecretLeaseRequest {
                        secret_ref: value_secret_ref,
                        header_name: name,
                    },
                    &scope,
                )
                .await;
            let lease = match lease {
                Ok(Some(lease)) => lease,
                Ok(None) | Err(_) => {
                    return active_failure_or_abort(
                        CallFailure::Unavailable,
                        call_deadline,
                        overall_deadline,
                        fixed_timeout_wins,
                        &cancellation,
                    );
                }
            };
            if lease.credential_ref() != value_secret_ref.as_ref()
                || lease.apply_authorization(&mut headers).is_err()
            {
                return Ok(Err(CallFailure::Unavailable));
            }
        }

        let provider = ProductionProvider::new(&self.ports);
        let target =
            match resolve_target(&provider, classifier.transport_target.clone(), &scope).await {
                Ok(target) => target,
                Err(_) => {
                    return active_failure_or_abort(
                        CallFailure::Unavailable,
                        call_deadline,
                        overall_deadline,
                        fixed_timeout_wins,
                        &cancellation,
                    );
                }
            };
        let body = sequential_replay_body(
            template.clone(),
            replay.clone(),
            replay.budget(),
            CLASSIFIER_WRITE_QUANTUM,
        )
        .map_err(map_preparation_error)?;
        let leases = RequestLeaseBook::new();
        let request = PreparedAttemptHttpRequest {
            head: PreparedRequestHead {
                method: Method::POST,
                path_and_query: Arc::clone(&classifier.request_path),
                headers,
            },
            body: PreparedAttemptBody::from_reader(
                body,
                leases
                    .acquire()
                    .map_err(|_| ClassificationError::Resource)?,
            )
            .map_err(map_preparation_error)?,
        };
        let body_plans = AttemptBodyPlans {
            attempt_request: BodyPlan::StreamingReplay {
                max_chunk_bytes: CLASSIFIER_WRITE_QUANTUM,
                max_replay_bytes: template.wire_len,
            },
            attempt_response_precommit: BodyPlan::BufferedTransform {
                max_body_bytes: CLASSIFIER_RESPONSE_BYTES,
            },
        };
        let timeouts = AttemptTimeouts {
            request_write: classifier.timeout,
            first_byte: classifier.timeout,
            stream_idle: classifier.timeout,
        };
        let mut exchange = AttemptExchange::new_with_cancellation(
            RequestId(1),
            AttemptId(1),
            AttemptGeneration(1),
            PlanRevision(publication_revision),
            target,
            self.classifier_connector.attempt_transport(),
            request,
            body_plans,
            timeouts,
            call_deadline,
            4,
            replay.budget().clone(),
            CLASSIFIER_WRITE_QUANTUM,
            cancellation.clone(),
        )
        .map_err(map_preparation_error)?;

        let response_reservation = replay
            .budget()
            .reserve(MemoryRole::ResponsePrefix, CLASSIFIER_RESPONSE_BYTES)
            .map_err(|_| ClassificationError::Resource)?;
        let exchange_result = collect_response(&mut exchange, call_deadline).await;
        let (status, body) = match exchange_result {
            Ok(response) => response,
            Err(CollectResponseError::Attempt(error)) => {
                let _ = exchange
                    .finish_or_abort_bounded(classifier_cleanup_timeout(call_deadline))
                    .await;
                drop(response_reservation);
                return map_call_error(
                    error,
                    call_deadline,
                    overall_deadline,
                    fixed_timeout_wins,
                    &cancellation,
                );
            }
            Err(CollectResponseError::InvalidOutput) => {
                let _ = exchange
                    .finish_or_abort_bounded(classifier_cleanup_timeout(call_deadline))
                    .await;
                drop(response_reservation);
                return active_failure_or_abort(
                    CallFailure::InvalidOutput,
                    call_deadline,
                    overall_deadline,
                    fixed_timeout_wins,
                    &cancellation,
                );
            }
        };
        exchange
            .submit_disposition_candidate(Disposition::Accept)
            .map_err(|_| ClassificationError::Integrity)?;
        let permit = match exchange.wait_writer_gate(call_deadline).await {
            Ok(WriterGate::ReadyToPublishAccept { permit, .. }) => permit,
            Ok(_) | Err(_) => {
                let _ = exchange
                    .finish_or_abort_bounded(classifier_cleanup_timeout(call_deadline))
                    .await;
                drop(response_reservation);
                return active_failure_or_abort(
                    CallFailure::Unavailable,
                    call_deadline,
                    overall_deadline,
                    fixed_timeout_wins,
                    &cancellation,
                );
            }
        };
        if exchange
            .publish_disposition(Disposition::Accept, permit)
            .is_err()
            || exchange.finish_accepted_response(true).await.is_err()
        {
            let _ = exchange
                .finish_or_abort_bounded(classifier_cleanup_timeout(call_deadline))
                .await;
            drop(response_reservation);
            return active_failure_or_abort(
                CallFailure::Unavailable,
                call_deadline,
                overall_deadline,
                fixed_timeout_wins,
                &cancellation,
            );
        }
        drop(response_reservation);
        if let Err(error) = ensure_call_active(call_deadline, overall_deadline, &cancellation) {
            return match error {
                ClassificationError::Deadline if fixed_timeout_wins => {
                    Ok(Err(CallFailure::Timeout))
                }
                error => Err(error),
            };
        }

        if status == StatusCode::OK {
            let allowed = classifier
                .branches
                .iter()
                .map(|branch| branch.id.as_ref())
                .collect::<Vec<_>>();
            return Ok(parse_classifier_response(
                &body,
                &allowed,
                has_assessment_target,
            ));
        }
        Ok(Err(classifier_status_failure(status)))
    }
}

fn diagnostic_request() -> ModelRequestIRV1 {
    ModelRequestIRV1 {
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: IngressProtocol::Responses,
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
        served_model_id: "hiroute-classifier-diagnostic".into(),
        stream: false,
        instructions: Vec::new(),
        messages: vec![CanonicalMessage {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: CLASSIFIER_DIAGNOSTIC_LATEST_USER.into(),
            }],
            name: None,
        }],
        tools: Vec::new(),
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
        tool_choice: ToolChoice::None,
        parallel_tool_calls: false,
        requested_reasoning: RequestedReasoningControl::absent(),
        requested_max_output_tokens: None,
        provider_state: Vec::new(),
    }
}

fn classifier_cleanup_timeout(call_deadline: Instant) -> Duration {
    CLASSIFIER_CLEANUP_TIMEOUT.min(call_deadline.saturating_duration_since(Instant::now()))
}

enum CollectResponseError {
    Attempt(AttemptError),
    InvalidOutput,
}

fn bind_assessment(target: AssessmentTarget, assessment: ClassifierAssessment) -> BoundAssessment {
    BoundAssessment {
        partial: target.target_partial || assessment.partial,
        target,
        score: assessment.score,
        reason: assessment.reason,
    }
}

async fn collect_response<T: hiroute_gateway_core::runtime::attempt::AttemptTransport>(
    exchange: &mut AttemptExchange<T>,
    deadline: Instant,
) -> Result<(StatusCode, Vec<u8>), CollectResponseError> {
    let mut status = None;
    let mut body = Vec::new();
    let mut ended = false;
    while exchange.snapshot().writer_state != WriterState::QuiescedNormalEos {
        exchange
            .drive_writer_once()
            .await
            .map_err(CollectResponseError::Attempt)?;
        while let Some(event) = exchange.next_precommit_event() {
            consume_response_event(event, &mut status, &mut body, &mut ended)?;
        }
    }
    while !ended {
        let Some(event) = exchange
            .wait_precommit_event(deadline)
            .await
            .map_err(CollectResponseError::Attempt)?
        else {
            return Err(CollectResponseError::InvalidOutput);
        };
        consume_response_event(event, &mut status, &mut body, &mut ended)?;
    }
    let status = status.ok_or(CollectResponseError::InvalidOutput)?;
    Ok((status, body))
}

fn consume_response_event(
    event: PrecommitEvent,
    status: &mut Option<StatusCode>,
    body: &mut Vec<u8>,
    ended: &mut bool,
) -> Result<(), CollectResponseError> {
    if *ended {
        return Err(CollectResponseError::InvalidOutput);
    }
    match event {
        PrecommitEvent::ResponseHead(head) if status.is_none() => {
            *status = Some(head.status());
        }
        PrecommitEvent::ResponseHead(_) => {
            return Err(CollectResponseError::InvalidOutput);
        }
        PrecommitEvent::Body(bytes) if status.is_some() => {
            if body.len().saturating_add(bytes.bytes().len()) > CLASSIFIER_RESPONSE_BYTES {
                return Err(CollectResponseError::InvalidOutput);
            }
            body.extend_from_slice(bytes.bytes());
        }
        PrecommitEvent::Body(_) | PrecommitEvent::SseEvent { .. } => {
            return Err(CollectResponseError::InvalidOutput);
        }
        PrecommitEvent::EndStream if status.is_some() => *ended = true,
        PrecommitEvent::EndStream => {
            return Err(CollectResponseError::InvalidOutput);
        }
    }
    Ok(())
}

fn map_preparation_error(error: AttemptError) -> ClassificationError {
    match error {
        AttemptError::Body(_) => ClassificationError::Resource,
        AttemptError::DeadlineExceeded => ClassificationError::Deadline,
        AttemptError::Cancelled => ClassificationError::Cancelled,
        _ => ClassificationError::Integrity,
    }
}

fn classifier_status_failure(status: StatusCode) -> CallFailure {
    if status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.as_u16() == 529
        || status.is_server_error()
    {
        CallFailure::Unavailable
    } else if status.is_client_error() {
        CallFailure::RejectedInput
    } else {
        CallFailure::InvalidOutput
    }
}

fn map_call_error(
    error: AttemptError,
    call_deadline: Instant,
    overall_deadline: Instant,
    fixed_timeout_wins: bool,
    cancellation: &CancellationToken,
) -> Result<Result<ClassifierResponse, CallFailure>, ClassificationError> {
    if error == AttemptError::Cancelled {
        return Err(ClassificationError::Cancelled);
    }
    if error == AttemptError::SequentialBodyContractViolation {
        return Err(ClassificationError::Integrity);
    }
    active_failure_or_abort(
        classify_attempt_failure(&error),
        call_deadline,
        overall_deadline,
        fixed_timeout_wins,
        cancellation,
    )
}

fn classify_attempt_failure(error: &AttemptError) -> CallFailure {
    match error {
        AttemptError::DeadlineExceeded
        | AttemptError::RequestWriteTimeout
        | AttemptError::FirstByteTimeout
        | AttemptError::StreamIdleTimeout => CallFailure::Timeout,
        AttemptError::Body(BodyError::BodyLimitExceeded) => CallFailure::InvalidOutput,
        _ => CallFailure::Unavailable,
    }
}

fn active_failure_or_abort(
    failure: CallFailure,
    call_deadline: Instant,
    overall_deadline: Instant,
    fixed_timeout_wins: bool,
    cancellation: &CancellationToken,
) -> Result<Result<ClassifierResponse, CallFailure>, ClassificationError> {
    match ensure_call_active(call_deadline, overall_deadline, cancellation) {
        Ok(()) => Ok(Err(failure)),
        Err(ClassificationError::Deadline) if fixed_timeout_wins => Ok(Err(CallFailure::Timeout)),
        Err(error) => Err(error),
    }
}

fn ensure_source_active(
    overall_deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), ClassificationError> {
    if cancellation.is_cancelled() {
        Err(ClassificationError::Cancelled)
    } else if Instant::now() >= overall_deadline {
        Err(ClassificationError::Deadline)
    } else {
        Ok(())
    }
}

fn ensure_call_active(
    call_deadline: Instant,
    overall_deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), ClassificationError> {
    ensure_source_active(overall_deadline, cancellation)?;
    if Instant::now() >= call_deadline {
        Err(ClassificationError::Deadline)
    } else {
        Ok(())
    }
}

fn resolve_latest_user(
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
) -> Result<Option<ResolvedLatestUser>, ClassificationError> {
    let Some(parts) = latest_user_parts(request) else {
        return Ok(None);
    };
    let byte_len = joined_text_len(&parts).map_err(|_| ClassificationError::Integrity)?;
    let reservation = replay
        .budget()
        .reserve(MemoryRole::SemanticState, byte_len)
        .map_err(|_| ClassificationError::Resource)?;
    let mut text = String::with_capacity(byte_len);
    for (index, part) in parts.into_iter().enumerate() {
        if index != 0 {
            text.push('\n');
        }
        if let Some(reference) = part.content_ref() {
            let mut reader = replay
                .reader(&reference)
                .map_err(map_replay_classification_error)?;
            reader
                .read_to_string(&mut text)
                .map_err(|_| ClassificationError::Integrity)?;
            reader
                .verify_terminal()
                .map_err(map_replay_classification_error)?;
        } else {
            text.push_str(part);
        }
    }
    if text.len() != byte_len {
        return Err(ClassificationError::Integrity);
    }
    Ok(Some(ResolvedLatestUser {
        text,
        _reservation: reservation,
    }))
}

fn latest_user_parts(request: &ModelRequestIRV1) -> Option<Vec<&str>> {
    request.messages.iter().rev().find_map(|message| {
        if message.role != MessageRole::User {
            return None;
        }
        let parts = message
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        (!parts.is_empty() && joined_text_len(&parts).ok().is_some_and(|len| len != 0))
            .then_some(parts)
    })
}

fn joined_text_len(parts: &[&str]) -> Result<usize, ReplayError> {
    parts
        .iter()
        .enumerate()
        .try_fold(0_usize, |length, (index, value)| {
            let value_len = match value.content_ref() {
                Some(reference) => reference.byte_len(),
                None => u64::try_from(value.len()).map_err(|_| ReplayError::LengthOverflow)?,
            };
            length
                .checked_add(usize::from(index != 0))
                .and_then(|length| usize::try_from(value_len).ok()?.checked_add(length))
                .ok_or(ReplayError::LengthOverflow)
        })
}

fn map_replay_classification_error(_error: ReplayError) -> ClassificationError {
    ClassificationError::Integrity
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    use super::*;
    use crate::server::composition::ProductionPorts;
    use crate::server::publication::GatewayPublicationInstaller;

    #[test]
    fn classifier_statuses_keep_request_rejection_distinct_from_unavailability() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::PAYLOAD_TOO_LARGE,
            StatusCode::UNPROCESSABLE_ENTITY,
        ] {
            assert_eq!(
                classifier_status_failure(status),
                CallFailure::RejectedInput
            );
        }
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::from_u16(529).unwrap(),
        ] {
            assert_eq!(classifier_status_failure(status), CallFailure::Unavailable);
        }
        assert_eq!(
            classifier_status_failure(StatusCode::TEMPORARY_REDIRECT),
            CallFailure::InvalidOutput
        );
        assert_eq!(
            classify_attempt_failure(&AttemptError::Body(BodyError::BodyLimitExceeded)),
            CallFailure::InvalidOutput
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn classifier_diagnostic_uses_the_production_transport_and_exact_protocol() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            let (header_end, content_length) = loop {
                let count = stream.read(&mut chunk).unwrap();
                assert_ne!(count, 0, "classifier request ended before its body");
                request.extend_from_slice(&chunk[..count]);
                let Some(header_end) = request.windows(4).position(|value| value == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = std::str::from_utf8(&request[..header_end]).unwrap();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                break (header_end + 4, content_length);
            };
            while request.len() < header_end + content_length {
                let count = stream.read(&mut chunk).unwrap();
                assert_ne!(count, 0, "classifier request body was truncated");
                request.extend_from_slice(&chunk[..count]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&request[header_end..header_end + content_length]).unwrap();
            assert_eq!(
                body.as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<std::collections::BTreeSet<_>>(),
                [
                    "assessment_from",
                    "branches",
                    "history_partial",
                    "latest_user",
                    "visible_conversation",
                ]
                .into_iter()
                .collect()
            );
            let response = br#"{"branch_id":"smart_saving_simple"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .unwrap();
            stream.write_all(response).unwrap();
        });

        let root = std::env::temp_dir().join(format!(
            "hiroute-classifier-diagnostic-{}-{}",
            std::process::id(),
            address.port()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let publications =
            Arc::new(GatewayPublicationInstaller::open(root.join("publication.json")).unwrap());
        let runtime = ProductionGatewayRuntime::compose(ProductionPorts::fail_closed(publications));
        let mode = hiroute_domain::ComplexityClassifierModeV1::Rest {
            endpoint: format!("http://{address}/v1/decisions"),
            timeout_ms: hiroute_domain::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS,
            auth_header: None,
        };

        let outcome = runtime.test_classifier_decision(&mode).await.unwrap();
        assert_eq!(outcome.branch_id, "smart_saving_simple");
        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
