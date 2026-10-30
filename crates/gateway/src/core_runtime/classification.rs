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

#[path = "classification/diagnostic.rs"]
mod diagnostic;
#[cfg(test)]
use diagnostic::classifier_status_failure;
use diagnostic::{CallDiagnostic, classifier_response_failure, diagnostic_request};

#[path = "classification/c_capability.rs"]
mod c_capability;
#[path = "classification/c_request.rs"]
mod c_request;
pub use c_capability::CClassifierDiagnosticOutcome;
#[path = "classification/extension.rs"]
mod extension;
#[path = "classification/group_policy.rs"]
mod group_policy;
#[path = "classification/openai_decisions.rs"]
mod openai_decisions;
#[path = "classification/protocol.rs"]
mod protocol;
#[cfg(test)]
#[path = "classification/resource_tests.rs"]
mod resource_tests;
#[path = "classification/system_one.rs"]
mod system_one;
pub(super) use group_policy::{apply as apply_group_policy, execution_position};

use super::ProductionGatewayRuntime;
use super::adapters::{PreparedReplayTemplate, sequential_replay_body};
use super::model_ir::{ContentPart, ModelRequestIRV1};
use super::profiles::{
    BranchDecisionV1, ClassifierFallbackReasonV1, CompiledClassifierKindV1,
    CompiledComplexityStrategyV1, ComplexityDecisionSourceV1, ComplexityReasonCodeV1, ComplexityV1,
    CorrelatedBranchDecisionV1, SanitizedStructuralFactsV1,
};
use crate::agent_turn_history::{AgentTurnHistorySnapshot, AssessmentTarget};
use crate::content_ref::ContentValueExt;
use crate::ports::{CredentialLease, ExecutionScope, HeaderSecretLeaseRequest, RuntimeStateKey};
use crate::replay::{ReplayError, ReplayStore};
use crate::runtime::TargetResolver;
use crate::server::request_plan::{
    ClassifierAuthenticationAuthorityV1, RestBranchClassifierAuthorityV1,
};
use protocol::{ClassifierAssessment, ClassifierResponse};

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
    SourceDisabled,
    NoUsableCredentials,
    Timeout,
    Unavailable,
    RejectedInput,
    AuthenticationRejected,
    RateLimited,
    EndpointRejected,
    InvalidOutput,
    Cancelled,
    Resource,
    Integrity,
}

impl ClassifierDiagnosticError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidConfig => "CLASSIFIER_CONFIG_INVALID",
            Self::SourceDisabled => "DECISION_SOURCE_DISABLED",
            Self::NoUsableCredentials => "DECISION_NO_USABLE_CREDENTIALS",
            Self::Timeout => "CLASSIFIER_TIMEOUT",
            Self::Unavailable => "CLASSIFIER_UNAVAILABLE",
            Self::RejectedInput => "CLASSIFIER_INPUT_REJECTED",
            Self::AuthenticationRejected => "CLASSIFIER_AUTH_REJECTED",
            Self::RateLimited => "CLASSIFIER_RATE_LIMITED",
            Self::EndpointRejected => "CLASSIFIER_ENDPOINT_REJECTED",
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
    pub(super) current_input: Option<&'a [ContentPart]>,
    /// A recognized auxiliary call or continuation has no independent current input.
    pub(super) native_user_unavailable: bool,
    /// Missing source evidence is disclosed to judgment; usable input stays enabled.
    pub(super) current_input_origin_unverified: bool,
    pub(super) replay: &'a ReplayStore,
    pub(super) classifier: Option<&'a Arc<RestBranchClassifierAuthorityV1>>,
    pub(super) history: &'a AgentTurnHistorySnapshot,
    pub(super) correlated: Option<&'a CorrelatedBranchDecisionV1>,
    pub(super) publication_revision: u64,
    pub(super) overall_deadline: Instant,
    pub(super) cancellation: CancellationToken,
    pub(super) started_at: Instant,
    /// Present only for an explicit diagnostic that must report admitted state, even on failure.
    pub(super) admitted_credential_state: Option<&'a mut Option<hiroute_domain::CanonicalDigest>>,
}

/// Current input is an independent descriptor; selecting it never rewrites the
/// original transcript or demotes other User envelopes to instructions.
#[derive(Clone, Copy)]
pub(super) struct DecisionInput<'a> {
    request: &'a ModelRequestIRV1,
    current: Option<&'a [ContentPart]>,
}

impl<'a> From<&'a ModelRequestIRV1> for DecisionInput<'a> {
    fn from(request: &'a ModelRequestIRV1) -> Self {
        Self {
            request,
            current: None,
        }
    }
}

impl<'a> DecisionInput<'a> {
    fn parts(self) -> Option<&'a [ContentPart]> {
        self.current.or_else(|| {
            super::native_routing_view::observed_input_index(self.request)
                .and_then(|index| self.request.messages.get(index))
                .map(|message| message.content.as_slice())
        })
    }
}

struct PreparedDecisionRequest {
    routing_observation: Option<hiroute_domain::RoutingObservationV1>,
    template: PreparedReplayTemplate,
    target_partial: bool,
    target_category: Option<String>,
    bindings: Option<system_one::Bindings>,
}

pub(super) struct ClassificationOutcome {
    diagnostic_failure: Option<ClassifierDiagnosticError>,
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
    SourceDisabled,
    NoUsableCredentials,
    RejectedInput,
    AuthenticationRejected,
    RateLimited,
    EndpointRejected,
    InvalidOutput,
}

impl CallFailure {
    fn applicability_reason(self) -> hiroute_domain::ApplicabilityReasonV1 {
        use hiroute_domain::ApplicabilityReasonV1 as R;
        match self {
            Self::Timeout => R::Timeout,
            Self::Unavailable | Self::SourceDisabled | Self::NoUsableCredentials => R::Unavailable,
            Self::RejectedInput => R::RejectedInput,
            Self::AuthenticationRejected => R::AuthenticationRejected,
            Self::RateLimited => R::RateLimited,
            Self::EndpointRejected => R::EndpointRejected,
            Self::InvalidOutput => R::InvalidOutput,
        }
    }
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
        self.test_classifier_decision_inner(mode, &mut None).await
    }

    pub async fn test_classifier_decision_with_admitted_state(
        &self,
        mode: &hiroute_domain::ComplexityClassifierModeV1,
    ) -> (
        Result<ClassifierDiagnosticOutcome, ClassifierDiagnosticError>,
        Option<hiroute_domain::CanonicalDigest>,
    ) {
        let mut admitted = None;
        let result = self
            .test_classifier_decision_inner(mode, &mut admitted)
            .await;
        (result, admitted)
    }

    async fn test_classifier_decision_inner(
        &self,
        mode: &hiroute_domain::ComplexityClassifierModeV1,
        admitted: &mut Option<hiroute_domain::CanonicalDigest>,
    ) -> Result<ClassifierDiagnosticOutcome, ClassifierDiagnosticError> {
        if matches!(mode, hiroute_domain::ComplexityClassifierModeV1::LocalRules) {
            return Err(ClassifierDiagnosticError::InvalidConfig);
        }
        mode.validate()
            .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
        let mut classifier = crate::server::publication::compile_classifier_mode_authority(mode, 1)
            .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
        Arc::make_mut(&mut classifier).default_category = Some("general".into());
        // Exercise category and degree in one synthetic first turn, without business tokens.
        Arc::make_mut(&mut classifier).decision =
            hiroute_domain::DecisionDefinitionV1::Categorical {
                instructions:
                    "Choose the task category by its main intent; use general if nothing matches."
                        .into(),
                options: [
                    ("local", "A clear local coding or editing task"),
                    ("general", "All other tasks"),
                ]
                .into_iter()
                .map(|(id, criterion)| hiroute_domain::CategoryOptionV1 {
                    id: id.into(),
                    criterion: criterion.into(),
                    refinement: Some(hiroute_domain::DegreePolicyV1::default().definition()),
                })
                .collect(),
            };
        if hiroute_domain::native_mode(mode) {
            let protocol = hiroute_domain::FrozenNativeProtocolV1::freeze(
                &hiroute_domain::NativeProtocolV1::default(),
                classifier.decision.clone(),
                ["local", "general"]
                    .map(|id| (id.to_owned(), hiroute_domain::JudgmentSettingsV1::default())),
            )
            .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
            Arc::make_mut(&mut classifier).native_protocol = Some(protocol);
        }
        let config_digest = hiroute_domain::CanonicalDigest::of(mode)
            .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
        let strategy = ComplexityV1::compile_with_classifier(
            Vec::<(String, String)>::new(),
            CompiledClassifierKindV1::Rest,
            Some(config_digest.as_str().to_owned()),
        )
        .and_then(|strategy| {
            ComplexityV1::with_branches(
                strategy,
                vec!["local".into(), "general".into()],
                "general".into(),
            )
        })
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
            previous_decision: None,
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
                    current_input: None,
                    native_user_unavailable: false,
                    current_input_origin_unverified: false,
                    replay: &replay,
                    classifier: Some(&classifier),
                    history: &history,
                    correlated: None,
                    publication_revision: 1,
                    overall_deadline,
                    cancellation: CancellationToken::new(),
                    started_at: started,
                    admitted_credential_state: Some(admitted),
                },
            )
            .await
            .map_err(|error| match error {
                ClassificationError::Cancelled => ClassifierDiagnosticError::Cancelled,
                ClassificationError::Deadline => ClassifierDiagnosticError::Timeout,
                ClassificationError::Integrity => ClassifierDiagnosticError::Integrity,
                ClassificationError::Resource => ClassifierDiagnosticError::Resource,
            })?;
        if let Some(failure) = result.diagnostic_failure {
            return Err(failure);
        }
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
        if result.decision.simple_probability.is_none()
            && !matches!(
                result.decision.selection_reason,
                hiroute_domain::ModelGroupReasonV1::ExtensionDecision
            )
        {
            return Err(ClassifierDiagnosticError::InvalidOutput);
        }
        Ok(ClassifierDiagnosticOutcome {
            branch_id: result.decision.branch_id,
            duration_millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    pub(super) async fn classify_for_planner(
        &self,
        strategy: &CompiledComplexityStrategyV1,
        mut request: ClassificationRequest<'_>,
    ) -> Result<ClassificationOutcome, ClassificationError> {
        ensure_source_active(request.overall_deadline, &request.cancellation)?;
        let started = request.started_at;
        let fixed_deadline = request
            .classifier
            .and_then(|classifier| started.checked_add(classifier.timeout))
            .unwrap_or(request.overall_deadline);
        let call_deadline = fixed_deadline.min(request.overall_deadline);
        let fixed_timeout_wins = fixed_deadline < request.overall_deadline;

        let (latest_user, local_input_unavailable) = if request.native_user_unavailable {
            (None, true)
        } else {
            match resolve_latest_user(
                DecisionInput {
                    request: request.request,
                    current: request.current_input,
                },
                request.replay,
            ) {
                Ok(user) => (user, false),
                Err(ClassificationError::Resource) => (None, true),
                Err(error) => return Err(error),
            }
        };
        ensure_source_active(request.overall_deadline, &request.cancellation)?;
        let (local_decision, facts) = ComplexityV1::decide(
            latest_user.as_ref().map(|value| value.text.as_str()),
            request.correlated,
            strategy,
        )
        .map_err(|_| ClassificationError::Integrity)?;
        drop(latest_user);

        if local_decision.decision_source == ComplexityDecisionSourceV1::Inherited
            || strategy.classifier_kind == CompiledClassifierKindV1::LocalRules
        {
            return Ok(ClassificationOutcome {
                diagnostic_failure: None,
                decision: local_decision,
                facts,
                assessment: None,
            });
        }

        let mut routing_observation = None;
        let result = if local_input_unavailable {
            // Optional judgment projection must not consume the model request's
            // remaining memory or reject content that Replay can still forward.
            Err(CallFailure::Unavailable)
        } else {
            match request.classifier {
                Some(classifier) => {
                    hiroute_gateway_core::runtime::timing::set_phase(
                        hiroute_gateway_core::runtime::timing::LocalPhase::History,
                    );
                    let prepared = if classifier.system_one_model.is_some() {
                        system_one::request_template(
                            classifier,
                            DecisionInput {
                                request: request.request,
                                current: request.current_input,
                            },
                            request.history,
                            request.current_input_origin_unverified,
                        )
                    } else {
                        extension::request_template(
                            DecisionInput {
                                request: request.request,
                                current: request.current_input,
                            },
                            request.history,
                            classifier,
                            request.current_input_origin_unverified,
                        )
                        .map_err(|_| CallFailure::RejectedInput)
                    };
                    match prepared {
                        Err(error) => Err(error),
                        Ok(prepared) => {
                            routing_observation = prepared.routing_observation.clone();
                            if let Some(o) = routing_observation.as_mut()
                                && o.applicability_requested
                            {
                                o.applicability_requested = false;
                                o.applicability_state =
                                    hiroute_domain::ApplicabilityStateV1::NotRequested;
                                o.applicability_reason =
                                    hiroute_domain::ApplicabilityReasonV1::DispatchNotStarted;
                            }
                            let target_trimmed = prepared.target_partial;
                            match ensure_call_active(
                                call_deadline,
                                request.overall_deadline,
                                &request.cancellation,
                            ) {
                                Ok(()) => {
                                    let result = match self
                                        .call_classifier(
                                            classifier,
                                            prepared,
                                            &mut routing_observation,
                                            request.replay,
                                            request.publication_revision,
                                            call_deadline,
                                            request.overall_deadline,
                                            fixed_timeout_wins,
                                            request.cancellation.clone(),
                                            request.admitted_credential_state.as_deref_mut(),
                                        )
                                        .await
                                    {
                                        Ok(result) => result,
                                        Err(ClassificationError::Resource) => {
                                            Err(CallFailure::Unavailable)
                                        }
                                        Err(error) => return Err(error),
                                    };
                                    result.map(|mut response| {
                                        if let Some(assessment) = response.assessment.as_mut() {
                                            assessment.partial |= target_trimmed;
                                        }
                                        response
                                    })
                                }
                                Err(ClassificationError::Deadline) if fixed_timeout_wins => {
                                    Err(CallFailure::Timeout)
                                }
                                Err(error) => return Err(error),
                            }
                        }
                    }
                }
                None => Err(CallFailure::Unavailable),
            }
        };
        ensure_source_active(request.overall_deadline, &request.cancellation)?;
        let classification_duration_micros =
            u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        match result {
            Ok(response) => {
                if let Some(o) = routing_observation.as_mut()
                    && o.applicability_requested
                {
                    o.applicability_probability = response
                        .applicability
                        .and_then(serde_json::Number::from_f64);
                    o.applicability_state = if o.applicability_probability.is_some() {
                        hiroute_domain::ApplicabilityStateV1::Valid
                    } else {
                        hiroute_domain::ApplicabilityStateV1::Unavailable
                    };
                    o.applicability_reason = if o.applicability_probability.is_some() {
                        hiroute_domain::ApplicabilityReasonV1::Answered
                    } else {
                        hiroute_domain::ApplicabilityReasonV1::InvalidOutput
                    };
                }
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
                    diagnostic_failure: None,
                    decision: BranchDecisionV1 {
                        extension_reason: response.extension_reason,
                        decision_protocol: Some(hiroute_domain::DecisionProtocolV1::LocalRules),
                        routing_observation,
                        policy: None,
                        competence_trigger: None,
                        execution_group: response
                            .extension_group
                            .unwrap_or(hiroute_domain::ExecutionGroupV1::Primary),
                        simple_probability: response
                            .probabilities
                            .as_ref()
                            .and_then(|p| p.get("simple"))
                            .and_then(|p| serde_json::Number::from_f64(*p)),
                        simple_threshold_millis: None,
                        selection_reason: if response.extension_decision {
                            if response.degree_failed {
                                hiroute_domain::ModelGroupReasonV1::DecisionUnavailable
                            } else {
                                hiroute_domain::ModelGroupReasonV1::ExtensionDecision
                            }
                        } else if response.degree_failed {
                            hiroute_domain::ModelGroupReasonV1::DegreeUnavailable
                        } else {
                            hiroute_domain::ModelGroupReasonV1::SimpleTask
                        },
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
                if let Some(o) = routing_observation.as_mut()
                    && o.applicability_requested
                {
                    o.applicability_reason = failure.applicability_reason();
                }
                fallback.routing_observation = routing_observation;
                fallback.fallback_used = true;
                fallback.classification_duration_micros = Some(classification_duration_micros);
                fallback.fallback_reason = Some(match failure {
                    CallFailure::Timeout => ClassifierFallbackReasonV1::Timeout,
                    CallFailure::Unavailable
                    | CallFailure::SourceDisabled
                    | CallFailure::NoUsableCredentials
                    | CallFailure::AuthenticationRejected
                    | CallFailure::RateLimited => ClassifierFallbackReasonV1::Unavailable,
                    CallFailure::RejectedInput | CallFailure::EndpointRejected => {
                        ClassifierFallbackReasonV1::RejectedInput
                    }
                    CallFailure::InvalidOutput => ClassifierFallbackReasonV1::InvalidOutput,
                });
                Ok(ClassificationOutcome {
                    diagnostic_failure: Some(failure.diagnostic()),
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
        prepared: PreparedDecisionRequest,
        routing_observation: &mut Option<hiroute_domain::RoutingObservationV1>,
        replay: &ReplayStore,
        publication_revision: u64,
        call_deadline: Instant,
        overall_deadline: Instant,
        fixed_timeout_wins: bool,
        cancellation: CancellationToken,
        admitted_state: Option<&mut Option<hiroute_domain::CanonicalDigest>>,
    ) -> Result<Result<ClassifierResponse, CallFailure>, ClassificationError> {
        let ClassifierAuthenticationAuthorityV1::SharedDecision { binding } =
            &classifier.authentication
        else {
            return self
                .call_classifier_attempt(
                    classifier,
                    &prepared,
                    routing_observation,
                    replay,
                    publication_revision,
                    call_deadline,
                    overall_deadline,
                    fixed_timeout_wins,
                    cancellation,
                    None,
                    admitted_state,
                )
                .await;
        };
        let scope = ExecutionScope::new(call_deadline, cancellation.clone());
        ensure_call_active(call_deadline, overall_deadline, &cancellation)?;
        let admission = match self.ports.credentials.admit_decision_credentials(binding) {
            Ok(admission) => admission,
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
        if let Some(state) = admitted_state {
            *state = Some(admission.credential_state_revision.clone());
        }
        if admission.state == hiroute_domain::MaterializationState::Disabled {
            return Ok(Err(CallFailure::SourceDisabled));
        }
        if admission.keys.is_empty() {
            return Ok(Err(CallFailure::NoUsableCredentials));
        }
        let store = crate::runtime::ProductionStateAdapter {
            inner: Arc::clone(&self.ports.runtime_state),
        };
        let policy = crate::runtime::RuntimeCooldownPolicy::for_gateway_model_execution();
        let binding_key = RuntimeStateKey::binding(binding.consumer_binding_id.clone());
        let mut excluded = Vec::new();
        let mut last_failure = CallFailure::NoUsableCredentials;
        for key in &admission.keys {
            ensure_call_active(call_deadline, overall_deadline, &cancellation)?;
            let binding_permit =
                match crate::runtime::acquire_target_permit_status(&store, &binding_key, &scope)
                    .await
                {
                    Ok(crate::runtime::PermitAvailability::Permit(permit)) => permit,
                    _ => {
                        return active_failure_or_abort(
                            CallFailure::Unavailable,
                            call_deadline,
                            overall_deadline,
                            fixed_timeout_wins,
                            &cancellation,
                        );
                    }
                };
            let key_state = RuntimeStateKey::credential(
                binding.consumer_binding_id.clone(),
                key.key_id.clone(),
                key.key_id.clone(),
                key.generation,
            );
            let key_permit = match crate::runtime::acquire_target_permit_status(
                &store, &key_state, &scope,
            )
            .await
            {
                Ok(crate::runtime::PermitAvailability::Permit(permit)) => permit,
                _ => {
                    let permits = crate::runtime::OwnedAttemptStatePermits::without_credential(
                        binding_key.clone(),
                        binding_permit,
                    );
                    let failure = crate::attempt_outcome::AttemptFailure {
                        class: crate::attempt_outcome::AttemptFailureClass::Unclassified,
                        status: None,
                        retry_after: None,
                    };
                    let _ = crate::runtime::record_failure(
                        &store,
                        permits.borrowed(),
                        &failure,
                        &policy,
                        &scope,
                    )
                    .await;
                    excluded.push(key.key_id.clone());
                    continue;
                }
            };
            let permits = crate::runtime::OwnedAttemptStatePermits::new(
                binding_key.clone(),
                binding_permit,
                key_state,
                key_permit,
            );
            let request = hiroute_domain::DecisionCredentialLeaseRequestV1 {
                binding: binding.clone(),
                admitted_keys: admission.keys.clone(),
                excluded_key_ids: excluded.clone(),
                key_id: key.key_id.clone(),
            };
            let lease = self
                .ports
                .credentials
                .lease_decision_credential(&request, &scope)
                .await;
            let lease = if let Ok(Some(lease)) = lease {
                lease
            } else {
                let failure = crate::attempt_outcome::AttemptFailure {
                    class: crate::attempt_outcome::AttemptFailureClass::Unclassified,
                    status: None,
                    retry_after: None,
                };
                let _ = crate::runtime::record_failure(
                    &store,
                    permits.borrowed(),
                    &failure,
                    &policy,
                    &scope,
                )
                .await;
                match lease {
                    Err(crate::server::composition::PortError::CredentialDenied(
                        hiroute_domain::NativeCredentialAccessDeniedV1::SourceDisabled,
                    )) => {
                        return active_failure_or_abort(
                            CallFailure::SourceDisabled,
                            call_deadline,
                            overall_deadline,
                            fixed_timeout_wins,
                            &cancellation,
                        );
                    }
                    Ok(None)
                    | Err(crate::server::composition::PortError::CredentialDenied(
                        hiroute_domain::NativeCredentialAccessDeniedV1::CredentialDisabled,
                    )) => {
                        excluded.push(key.key_id.clone());
                        continue;
                    }
                    Err(_) => {
                        return active_failure_or_abort(
                            CallFailure::Unavailable,
                            call_deadline,
                            overall_deadline,
                            fixed_timeout_wins,
                            &cancellation,
                        );
                    }
                    Ok(Some(_)) => unreachable!(),
                }
            };
            let result = self
                .call_classifier_attempt(
                    classifier,
                    &prepared,
                    routing_observation,
                    replay,
                    publication_revision,
                    call_deadline,
                    overall_deadline,
                    fixed_timeout_wins,
                    cancellation.clone(),
                    Some(&lease),
                    None,
                )
                .await;
            let class = match &result {
                Ok(Ok(_)) => {
                    if crate::runtime::confirm_success(&store, permits.borrowed(), &scope)
                        .await
                        .is_err()
                    {
                        return active_failure_or_abort(
                            CallFailure::Unavailable,
                            call_deadline,
                            overall_deadline,
                            fixed_timeout_wins,
                            &cancellation,
                        );
                    }
                    return result;
                }
                Ok(Err(CallFailure::AuthenticationRejected)) => {
                    crate::attempt_outcome::AttemptFailureClass::Credential
                }
                Ok(Err(CallFailure::RateLimited)) => {
                    crate::attempt_outcome::AttemptFailureClass::Quota
                }
                _ => crate::attempt_outcome::AttemptFailureClass::Unclassified,
            };
            let failure = crate::attempt_outcome::AttemptFailure {
                class,
                status: None,
                retry_after: None,
            };
            if crate::runtime::record_failure(&store, permits.borrowed(), &failure, &policy, &scope)
                .await
                .is_err()
            {
                return active_failure_or_abort(
                    CallFailure::Unavailable,
                    call_deadline,
                    overall_deadline,
                    fixed_timeout_wins,
                    &cancellation,
                );
            }
            match result {
                Ok(Err(
                    failure @ (CallFailure::AuthenticationRejected | CallFailure::RateLimited),
                )) => {
                    last_failure = failure;
                    excluded.push(key.key_id.clone());
                }
                other => return other,
            }
        }
        active_failure_or_abort(
            last_failure,
            call_deadline,
            overall_deadline,
            fixed_timeout_wins,
            &cancellation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn call_classifier_attempt(
        &self,
        classifier: &RestBranchClassifierAuthorityV1,
        prepared: &PreparedDecisionRequest,
        routing_observation: &mut Option<hiroute_domain::RoutingObservationV1>,
        replay: &ReplayStore,
        publication_revision: u64,
        call_deadline: Instant,
        overall_deadline: Instant,
        fixed_timeout_wins: bool,
        cancellation: CancellationToken,
        shared_lease: Option<&CredentialLease>,
        admitted_state: Option<&mut Option<hiroute_domain::CanonicalDigest>>,
    ) -> Result<Result<ClassifierResponse, CallFailure>, ClassificationError> {
        let template = &prepared.template;
        let mut diagnostic = CallDiagnostic::new(self.observation.diagnostic_context());
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
            if let Some(state) = admitted_state {
                *state = hiroute_domain::decision_header_state_revision(
                    lease.credential_ref(),
                    lease.generation(),
                )
                .ok();
            }
        } else if let ClassifierAuthenticationAuthorityV1::SharedDecision { .. } =
            &classifier.authentication
        {
            let Some(lease) = shared_lease else {
                return Ok(Err(CallFailure::Unavailable));
            };
            if lease.apply_authorization(&mut headers).is_err() {
                return Ok(Err(CallFailure::Unavailable));
            }
        }

        diagnostic.prepared(&headers, &template.bytes);
        let resolver = TargetResolver::new();
        let target = match resolver
            .resolve(classifier.transport_target.clone(), &scope)
            .await
        {
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
        if prepared
            .bindings
            .as_ref()
            .is_some_and(|b| b.applicability.is_some())
            && let Some(o) = routing_observation.as_mut()
        {
            o.applicability_requested = true;
            o.applicability_state = hiroute_domain::ApplicabilityStateV1::Unavailable;
            o.applicability_reason = hiroute_domain::ApplicabilityReasonV1::Pending;
        }
        hiroute_gateway_core::runtime::timing::set_phase(
            hiroute_gateway_core::runtime::timing::LocalPhase::Decision,
        );
        let exchange_result = collect_response(&mut exchange, call_deadline, &mut diagnostic).await;
        diagnostic.protocol(exchange.transport_facts().upstream_protocol);
        let (status, body) = match exchange_result {
            Ok(response) => response,
            Err(CollectResponseError::Attempt(error)) => {
                diagnostic.failure(match error {
                    AttemptError::DeadlineExceeded
                    | AttemptError::ConnectTimeout
                    | AttemptError::FirstByteTimeout
                    | AttemptError::StreamIdleTimeout
                    | AttemptError::RequestWriteTimeout => CallFailure::Timeout,
                    _ => CallFailure::Unavailable,
                });
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
                diagnostic.failure(CallFailure::InvalidOutput);
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
            let result = match &prepared.bindings {
                Some(bindings) => match classifier.wire_protocol {
                    hiroute_domain::DecisionWireProtocolV2::SystemOne => {
                        system_one::parse_response(&body, bindings)
                    }
                    hiroute_domain::DecisionWireProtocolV2::OpenaiDecisions => {
                        openai_decisions::parse_response(&body, bindings)
                    }
                },
                None => extension::parse_response(
                    &body,
                    &classifier.decision,
                    prepared.target_category.as_deref(),
                ),
            };
            diagnostic.finish(result.as_ref().err().copied());
            return Ok(result);
        }
        let failure = classifier_response_failure(status, &body);
        diagnostic.finish(Some(failure));
        Ok(Err(failure))
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
    diagnostic: &mut CallDiagnostic,
) -> Result<(StatusCode, Vec<u8>), CollectResponseError> {
    let mut status = None;
    let mut body = Vec::new();
    let mut ended = false;
    while exchange.snapshot().writer_state != WriterState::QuiescedNormalEos {
        exchange
            .drive_writer_once()
            .await
            .map_err(CollectResponseError::Attempt)?;
        diagnostic.protocol(exchange.transport_facts().upstream_protocol);
        while let Some(event) = exchange.next_precommit_event() {
            consume_response_event(event, &mut status, &mut body, &mut ended, diagnostic)?;
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
        consume_response_event(event, &mut status, &mut body, &mut ended, diagnostic)?;
    }
    let status = status.ok_or(CollectResponseError::InvalidOutput)?;
    Ok((status, body))
}

fn consume_response_event(
    event: PrecommitEvent,
    status: &mut Option<StatusCode>,
    body: &mut Vec<u8>,
    ended: &mut bool,
    diagnostic: &mut CallDiagnostic,
) -> Result<(), CollectResponseError> {
    if *ended {
        return Err(CollectResponseError::InvalidOutput);
    }
    match event {
        PrecommitEvent::ResponseHead(head) if status.is_none() => {
            diagnostic.response(head.headers(), head.status());
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

fn resolve_latest_user<'a>(
    request: impl Into<DecisionInput<'a>>,
    replay: &ReplayStore,
) -> Result<Option<ResolvedLatestUser>, ClassificationError> {
    let Some(parts) = latest_user_parts(request.into()) else {
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

fn latest_user_parts(request: DecisionInput<'_>) -> Option<Vec<&str>> {
    let parts = request
        .parts()?
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    (!parts.is_empty() && joined_text_len(&parts).ok().is_some_and(|len| len != 0)).then_some(parts)
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

    struct RevokedDecisionSource {
        error: u8,
        leases: std::sync::atomic::AtomicUsize,
        revision: hiroute_domain::CanonicalDigest,
    }

    #[async_trait::async_trait]
    impl crate::server::composition::CredentialResolver for RevokedDecisionSource {
        fn acquire(
            &self,
            _: &str,
        ) -> Result<
            crate::server::composition::CredentialLease,
            crate::server::composition::PortError,
        > {
            Err(crate::server::composition::PortError::Rejected)
        }

        fn admit_decision_credentials(
            &self,
            _: &hiroute_domain::DecisionCredentialBindingV1,
        ) -> Result<
            hiroute_domain::DecisionCredentialAdmissionV1,
            crate::server::composition::PortError,
        > {
            Ok(hiroute_domain::DecisionCredentialAdmissionV1 {
                state: hiroute_domain::MaterializationState::Ready,
                keys: ["key/one", "key/two"]
                    .into_iter()
                    .map(|key| hiroute_domain::DecisionCredentialKeyV1 {
                        key_id: key.into(),
                        generation: 1,
                    })
                    .collect(),
                credential_state_revision: self.revision.clone(),
            })
        }

        async fn lease_decision_credential(
            &self,
            _: &hiroute_domain::DecisionCredentialLeaseRequestV1,
            _: &ExecutionScope,
        ) -> Result<Option<CredentialLease>, crate::server::composition::PortError> {
            self.leases
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(match self.error {
                0 => crate::server::composition::PortError::CredentialDenied(
                    hiroute_domain::NativeCredentialAccessDeniedV1::SourceDisabled,
                ),
                1 => crate::server::composition::PortError::Unavailable("fixture"),
                _ => crate::server::composition::PortError::CredentialDenied(
                    hiroute_domain::NativeCredentialAccessDeniedV1::CredentialDisabled,
                ),
            })
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shared_decision_preserves_revocation_and_authority_failures_after_admission() {
        let root = std::env::temp_dir().join(format!(
            "hiroute-decision-revocation-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let service: hiroute_domain::DecisionServiceV2 = serde_json::from_value(serde_json::json!({
            "schema": "hiroute.decision-service/v2", "id": "decision/shared", "revision": 1, "name": "fixture",
            "connection": {"kind": "native", "provider": "openai", "wire_protocol": "openai_decisions", "model": "gpt-6-luna", "endpoint": "http://127.0.0.1:1/v1/decisions", "timeout_ms": 3000,
                "authentication": {"kind": "shared_source", "source_id": "source/one", "source_lineage_digest": format!("sha256:{}", "1".repeat(64))}}
        })).unwrap();
        let mode = hiroute_domain::ComplexityClassifierModeV1::DecisionServiceV2 {
            service: Box::new(service),
        };
        for (error, expected, attempts) in [
            (0, ClassifierDiagnosticError::SourceDisabled, 1),
            (1, ClassifierDiagnosticError::Unavailable, 1),
            (2, ClassifierDiagnosticError::NoUsableCredentials, 2),
        ] {
            let publications = Arc::new(
                GatewayPublicationInstaller::open(root.join(format!("{error}.json"))).unwrap(),
            );
            let revision = hiroute_domain::CanonicalDigest::of(&"admitted").unwrap();
            let credentials = Arc::new(RevokedDecisionSource {
                error,
                leases: std::sync::atomic::AtomicUsize::new(0),
                revision: revision.clone(),
            });
            let mut ports = ProductionPorts::fail_closed(publications);
            ports.credentials = credentials.clone();
            ports.runtime_state = Arc::new(crate::ports::InMemoryRuntimeStateStore::default());
            let runtime = ProductionGatewayRuntime::compose(ports);
            let (result, admitted) = runtime
                .test_classifier_decision_with_admitted_state(&mode)
                .await;
            assert_eq!(result.unwrap_err(), expected);
            assert_eq!(admitted, Some(revision));
            assert_eq!(
                credentials.leases.load(std::sync::atomic::Ordering::SeqCst),
                attempts
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

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
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            assert_eq!(
                classifier_response_failure(status, b"private token"),
                CallFailure::AuthenticationRejected
            );
        }
        assert_eq!(
            classifier_status_failure(StatusCode::TOO_MANY_REQUESTS),
            CallFailure::RateLimited
        );
        assert_eq!(
            classifier_response_failure(
                StatusCode::BAD_REQUEST,
                br#"{"error":{"message":"Workspace endpoint is invalid."}}"#
            ),
            CallFailure::EndpointRejected
        );
        let failure = classifier_response_failure(
            StatusCode::BAD_REQUEST,
            br#"{"error":{"message":"private token details"}}"#,
        );
        assert_eq!(failure, CallFailure::RejectedInput);
        assert_eq!(failure.diagnostic().code(), "CLASSIFIER_INPUT_REJECTED");
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
                    "preceding_stage",
                    "route",
                    "history_partial",
                    "latest_user",
                    "visible_conversation",
                ]
                .into_iter()
                .collect()
            );
            let response = br#"{"category":"local","group":"regular"}"#;
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let publications =
            Arc::new(GatewayPublicationInstaller::open(root.join("publication.json")).unwrap());
        let diagnostics = hiroute_diagnostics::runtime::DiagnosticRuntime::start(
            hiroute_diagnostics::runtime::RuntimeConfig {
                root: root.join("diagnostics"),
                role: hiroute_diagnostics::event::ProcessRole::Daemon,
                component: hiroute_diagnostics::record::Component::Gateway,
                parent_session_id: None,
                level_override: Some(hiroute_diagnostics::DiagnosticLevel::Debug),
            },
        );
        assert!(
            diagnostics.owns_role(),
            "diagnostic fixture must own its writer"
        );
        let observation = Arc::new(
            super::super::observation::GatewayObservation::from_environment()
                .with_diagnostics(diagnostics.port()),
        );
        let runtime = ProductionGatewayRuntime::compose_with_planner_and_observation(
            ProductionPorts::fail_closed(publications),
            Arc::new(super::super::PublicationPlannerInputAuthority),
            observation,
        );
        let mode = hiroute_domain::ComplexityClassifierModeV1::DecisionService {
            service: Box::new(hiroute_domain::DecisionServiceV1 {
                id: "decision-fixture".into(),
                revision: 1,
                name: "Fixture extension".into(),
                connection: hiroute_domain::DecisionConnectionV1::Custom {
                    endpoint: format!("http://{address}/v1/decisions"),
                    timeout_ms: hiroute_domain::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS,
                    auth_header: None,
                },
            }),
        };

        let outcome = runtime.test_classifier_decision(&mode).await.unwrap();
        assert_eq!(outcome.branch_id, "local");
        server.join().unwrap();
        diagnostics.shutdown();
        let log = std::fs::read_to_string(root.join("diagnostics/daemon/current.jsonl")).unwrap();
        assert!(
            log.contains("\"request_kind\":\"decision_service\""),
            "{log}"
        );
        assert!(log.contains("\"http_protocol\":\"http1\""), "{log}");
        assert!(log.contains("\"phase\":\"completed\""), "{log}");
        assert!(!log.contains(CLASSIFIER_DIAGNOSTIC_LATEST_USER), "{log}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
