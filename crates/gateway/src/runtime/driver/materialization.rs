use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hiroute_gateway_core::core::execution_plan::{
    AttemptExecutionBinding, CompiledAttemptPlan, TransportScheme, TransportTarget,
};
use hiroute_gateway_core::runtime::attempt::{
    AttemptTimeoutKind, PreparedAttemptBody, PreparedAttemptHttpRequest, PreparedRequestHead,
};
use hiroute_gateway_core::runtime::body::{ChargedBodyQueue, MemoryRole};
use hiroute_gateway_core::runtime::driver::{
    AttemptFailureClass as CoreAttemptFailureClass, AttemptFailureFacts,
    AttemptMaterializationContext, AttemptMaterializationFailure,
    AttemptMaterializationFailureClass, LogicalRequestBodyFrame, LogicalRequestContext,
    ProviderClassificationFacts, RetryabilityFact,
};
use hiroute_gateway_core::transport::GatewayRequestHead;
use http::header::{CONTENT_LENGTH, CONTENT_TYPE, HOST};
use http::{HeaderMap, HeaderValue, Method};

use crate::attempt_outcome::{AttemptFailure, RawAttemptFailure, classify_failure};
use crate::ports::{
    CredentialLeaseRequest, ExecutionScope, RuntimeStateEntry, RuntimeStateKey, RuntimeStateStore,
};
use crate::server::composition::{
    PortError, ProductionPorts, RuntimeStateStore as ProductionStore,
};
use crate::server::core_runtime::adapters;
use crate::server::core_runtime::profiles::{
    AuthenticationSemantics, CandidateProtocolProfile, ClientProtocolProfile, CostClassV1,
    PlannerCandidateFactsV1,
};

use super::response::{PrecommitDecoderBudget, connector_error_profile, failure_facts};
#[path = "materialization/lease_target.rs"]
mod lease_target;

use super::{
    MATERIALIZATION_AUTHORITY_FAILED, MATERIALIZATION_BINDING_COOLING_PREFIX,
    MATERIALIZATION_BINDING_DISABLED, MATERIALIZATION_BINDING_PROBE_BUSY,
    MATERIALIZATION_BINDING_UNAVAILABLE, MATERIALIZATION_CREDENTIAL_COOLING_PREFIX,
    MATERIALIZATION_CREDENTIAL_DISABLED, MATERIALIZATION_CREDENTIAL_EXHAUSTED,
    MATERIALIZATION_CREDENTIAL_PROBE_BUSY, MATERIALIZATION_DNS_FAILED,
    MATERIALIZATION_PROTOCOL_FAILED, MAX_RENDERED_PRECOMMIT_BYTES, MechanicalFailureSignal,
    PermitAvailability, ProductionAttemptState, ProductionLogicalRequest, ProductionProvider,
    ProductionReplaySeedEnvelope, RUNTIME_STATE_AUTHORITY_FAILED, RuntimeCooldownPolicy,
    StatePermit, acquire_target_permit_status, confirm_success, label, record_failure, safe_error,
    validate_attempt_permits,
};

pub(super) struct ProductionStateAdapter {
    pub(super) inner: Arc<dyn ProductionStore>,
}

#[async_trait]
impl RuntimeStateStore for ProductionStateAdapter {
    async fn read(
        &self,
        key: &RuntimeStateKey,
        scope: &ExecutionScope,
    ) -> Result<RuntimeStateEntry, crate::ports::RuntimeStateError> {
        self.inner
            .read_exact(key, scope)
            .await
            .map_err(map_state_port_error)
    }

    async fn compare_and_swap(
        &self,
        key: &RuntimeStateKey,
        expected_generation: u64,
        next: RuntimeStateEntry,
        scope: &ExecutionScope,
    ) -> Result<crate::ports::CasOutcome, crate::ports::RuntimeStateError> {
        self.inner
            .compare_and_swap_exact(key, expected_generation, next, scope)
            .await
            .map_err(map_state_port_error)
    }

    async fn acquire_probe_lease(
        &self,
        key: &RuntimeStateKey,
        expected_generation: u64,
        now: Instant,
        lease_duration: std::time::Duration,
        scope: &ExecutionScope,
    ) -> Result<crate::ports::ProbeLeaseOutcome, crate::ports::RuntimeStateError> {
        self.inner
            .acquire_probe_lease_exact(key, expected_generation, now, lease_duration, scope)
            .await
            .map_err(map_state_port_error)
    }
}

fn map_state_port_error(_error: PortError) -> crate::ports::RuntimeStateError {
    crate::ports::RuntimeStateError::Unavailable
}

impl ProductionProvider {
    pub fn new(ports: &ProductionPorts) -> Self {
        Self {
            credentials: Arc::clone(&ports.credentials),
            state: Arc::new(ProductionStateAdapter {
                inner: Arc::clone(&ports.runtime_state),
            }),
            cooldowns: RuntimeCooldownPolicy::default(),
            #[cfg(feature = "e2e-test-control")]
            test_dial: crate::server::test_control::E2eDialMap::from_environment(),
        }
    }
}

pub(super) fn begin_request(
    head: GatewayRequestHead,
    context: LogicalRequestContext<'_>,
) -> Result<ProductionLogicalRequest, Arc<str>> {
    let ingress = crate::server::request_plan::IngressProtocol::from_path(
        head.path_and_query
            .split_once('?')
            .map_or(head.path_and_query.as_ref(), |(path, _)| path),
    )
    .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    let seed = context
        .provider_context
        .as_ref()
        .and_then(|context| context.downcast_ref::<ProductionReplaySeedEnvelope>())
        .and_then(ProductionReplaySeedEnvelope::take)
        .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    let frozen_candidates = context
        .frozen_candidates
        .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    if frozen_candidates.is_empty()
        || frozen_candidates.iter().any(|candidate| {
            candidate.candidate_id.is_none()
                || candidate.profile_digest.is_none()
                || candidate.reason_ledger_identity.is_none()
                || candidate.provider_profile.is_none()
        })
    {
        return Err(Arc::from(MATERIALIZATION_PROTOCOL_FAILED));
    }
    if seed.ingress != ingress || seed.ir.ingress_protocol != ingress {
        return Err(Arc::from(MATERIALIZATION_PROTOCOL_FAILED));
    }
    Ok(ProductionLogicalRequest {
        ingress,
        replay: seed.replay,
        raw_body: seed.raw_body,
        observed_body_bytes: 0,
        body_eos: false,
        ir: Some(seed.ir),
        frozen_candidates,
        runtime_state_authority: seed.runtime_state_authority,
        route_context: Some(seed.route_context),
    })
}

pub(super) fn consume_request_body(
    logical: &mut ProductionLogicalRequest,
    frame: LogicalRequestBodyFrame,
) -> Result<(), Arc<str>> {
    if logical.body_eos {
        return Err(Arc::from(MATERIALIZATION_PROTOCOL_FAILED));
    }
    if let Some(bytes) = frame.bytes {
        logical.observed_body_bytes = logical
            .observed_body_bytes
            .checked_add(bytes.bytes().len() as u64)
            .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    }
    if frame.end_stream {
        if logical.observed_body_bytes != logical.raw_body.byte_len() {
            return Err(Arc::from(MATERIALIZATION_PROTOCOL_FAILED));
        }
        logical
            .replay
            .release_stream(&logical.raw_body)
            .map_err(safe_error)?;
        logical.body_eos = true;
    }
    Ok(())
}

pub(super) fn finalize_route_request_context(
    logical: &mut ProductionLogicalRequest,
) -> Result<super::ProductionRouteContext, Arc<str>> {
    if !logical.body_eos || logical.ir.is_none() {
        return Err(Arc::from(MATERIALIZATION_PROTOCOL_FAILED));
    }
    logical
        .route_context
        .take()
        .filter(|context| context.ingress == logical.ingress)
        .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))
}

pub(super) async fn materialize_attempt(
    provider: &ProductionProvider,
    logical: &mut ProductionLogicalRequest,
    context: AttemptMaterializationContext<'_>,
) -> Result<(PreparedAttemptHttpRequest, ProductionAttemptState), Arc<str>> {
    let runtime_state_authority = logical.runtime_state_authority.clone();
    let scope = ExecutionScope::new(
        context.attempt_deadline(),
        context.cancellation_token().clone(),
    );
    scope
        .ensure_active()
        .map_err(|_| Arc::from(MATERIALIZATION_AUTHORITY_FAILED))?;
    let plan = context.plan();
    let stable_target = plan.stable_target_key.as_str();
    let candidate = logical
        .frozen_candidates
        .iter()
        .find(|candidate| candidate.binding == context.binding_id())
        .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    if candidate.stable_target.as_str() != stable_target {
        return Err(Arc::from(MATERIALIZATION_PROTOCOL_FAILED));
    }
    let execution: crate::server::publication::SealedCandidateExecutionV1 = serde_json::from_slice(
        candidate
            .provider_profile
            .as_deref()
            .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?,
    )
    .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    let profile = &execution.protocol_profile;
    let expected_profile_digest = candidate
        .profile_digest
        .as_ref()
        .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?
        .as_str();
    if execution.schema_version != crate::server::publication::SEALED_CANDIDATE_EXECUTION_SCHEMA_V1
        || execution.stable_target_key != stable_target
        || execution.credential_destination_ref.trim().is_empty()
        || execution.logical_endpoint.trim().is_empty()
        || execution.upstream_model_id.trim().is_empty()
        || execution.native_transport_model.trim().is_empty()
        || profile.capability.native_model != execution.native_transport_model
        || (execution.connector_runtime == hiroute_domain::ConnectorRuntimeKind::BuiltinNative
            && execution.upstream_model_id != execution.native_transport_model)
        || profile_digest(profile)? != expected_profile_digest
        || execution.profile_digest != expected_profile_digest
        || profile.ingress_protocol != logical.ingress
        || (execution.connector_runtime != hiroute_domain::ConnectorRuntimeKind::CpaBridge
            && execution
                .operational_target
                .for_protocol_path(&profile.connector.request_path)
                .is_none())
        || !execution
            .operational_target
            .validate_for(execution.connector_runtime, &execution.logical_endpoint)
        || !matches!(
            hiroute_domain::CanonicalDigest::of(&execution.operational_target),
            Ok(digest) if digest == execution.operational_target_digest
        )
        || hiroute_domain::CanonicalDigest::parse(execution.protocol_set_digest.as_str()).is_err()
        || !operational_target_matches_plan(&execution.operational_target, plan)
        || profile.connector.authentication.exact().is_none()
    {
        return Err(Arc::from(MATERIALIZATION_PROTOCOL_FAILED));
    }
    let ir = logical
        .ir
        .as_ref()
        .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    let mut prepared = adapters::project_candidate_request_template(ir, profile)
        .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    let streaming = ir.stream;
    let served_model_alias = ir.served_model_id.clone();
    let client_profile = client_profile_for_candidate(profile)?;
    let native_output = profile.ingress_protocol == profile.capability.upstream_protocol;
    let renderer = (streaming && !native_output)
        .then(|| {
            adapters::IncrementalClientSseRenderer::new(
                client_profile.clone(),
                served_model_alias.clone(),
            )
        })
        .transpose()
        .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    let prefix = ChargedBodyQueue::new(
        context.budget,
        MemoryRole::ResponsePrefix,
        &plan.body_plans.attempt_response_precommit,
        MAX_RENDERED_PRECOMMIT_BYTES,
        plan.precommit_event_capacity,
    )
    .map_err(safe_error)?;
    let decoder_budget = PrecommitDecoderBudget::new(context.budget)?;
    let binding_key = RuntimeStateKey::binding(stable_target);
    let binding_permit =
        acquire_target_permit_status(provider.state.as_ref(), &binding_key, &scope)
            .await
            .map_err(|_| {
                runtime_state_authority.fail();
                Arc::from(RUNTIME_STATE_AUTHORITY_FAILED)
            })?;
    let binding_permit = permit_or_materialization_error(binding_permit, true)?;

    let authentication = profile
        .connector
        .authentication
        .exact()
        .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    let no_credential = matches!(authentication, AuthenticationSemantics::None);
    if no_credential
        != context
            .credential_ref()
            .as_str()
            .starts_with("credential/none/")
    {
        return Err(Arc::from(MATERIALIZATION_PROTOCOL_FAILED));
    }
    let lease_target = lease_target::for_request(&execution, &profile.connector.request_path)?;
    let lease_target_digest = hiroute_domain::CanonicalDigest::of(&lease_target)
        .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    let lease_logical_endpoint =
        if execution.connector_runtime == hiroute_domain::ConnectorRuntimeKind::BuiltinNative {
            lease_target.uri()
        } else {
            &execution.logical_endpoint
        };
    let credential = if no_credential {
        None
    } else {
        Some(
            scope
                .run(provider.credentials.lease_exact(
                    CredentialLeaseRequest {
                        stable_binding_id: stable_target,
                        credential_ref: context.credential_ref().as_str(),
                        credential_destination_ref: &execution.credential_destination_ref,
                        excluded_key_ids: &[],
                        connector_runtime: execution.connector_runtime,
                        connector_id: &profile.connector.connector_id,
                        upstream_protocol: profile.capability.upstream_protocol,
                        upstream_model_id: &execution.upstream_model_id,
                        native_transport_model: &execution.native_transport_model,
                        logical_endpoint: lease_logical_endpoint,
                        operational_target: lease_target.uri(),
                        operational_target_digest: lease_target_digest.as_str(),
                        runtime_epoch: execution.operational_target.runtime_epoch(),
                        target_epoch: execution.operational_target.target_epoch(),
                        protocol_profile_digest: expected_profile_digest,
                        request_path: &profile.connector.request_path,
                        authentication,
                    },
                    &scope,
                ))
                .await
                .map_err(|_| Arc::from(MATERIALIZATION_AUTHORITY_FAILED))?
                .map_err(|_| Arc::from(MATERIALIZATION_AUTHORITY_FAILED))?
                .ok_or_else(|| Arc::from(MATERIALIZATION_CREDENTIAL_EXHAUSTED))?,
        )
    };
    let permits = if let Some(credential) = credential.as_ref() {
        if credential.credential_ref() != context.credential_ref().as_str() {
            return Err(Arc::from(MATERIALIZATION_AUTHORITY_FAILED));
        }
        let credential_key = RuntimeStateKey::credential(
            stable_target,
            credential.credential_ref(),
            credential.key_id(),
            credential.generation(),
        );
        let credential_permit =
            acquire_target_permit_status(provider.state.as_ref(), &credential_key, &scope)
                .await
                .map_err(|_| {
                    runtime_state_authority.fail();
                    Arc::from(RUNTIME_STATE_AUTHORITY_FAILED)
                })?;
        let credential_permit = permit_or_materialization_error(credential_permit, false)?;
        super::OwnedAttemptStatePermits::new(
            binding_key,
            binding_permit,
            credential_key,
            credential_permit,
        )
    } else {
        super::OwnedAttemptStatePermits::without_credential(binding_key, binding_permit)
    };

    let resolved_target = match credential
        .as_ref()
        .and_then(|credential| credential.transport_override())
    {
        Some(target) => {
            if execution.connector_runtime != hiroute_domain::ConnectorRuntimeKind::CpaBridge
                || execution.operational_target.runtime_epoch().is_none()
                || target.request_path() != profile.connector.request_path
            {
                return Err(Arc::from(MATERIALIZATION_AUTHORITY_FAILED));
            }
            let mut resolved = plan.transport_target.clone();
            resolved.scheme = TransportScheme::Http;
            resolved.authority = target.address().to_string().into();
            resolved.addresses = Arc::from([target.address()]);
            resolved.sni = None;
            resolved.connection_fingerprint = resolved.derive_connection_fingerprint();
            resolved
                .validate()
                .map_err(|_| Arc::from(MATERIALIZATION_AUTHORITY_FAILED))?;
            resolved
        }
        None => {
            if execution.connector_runtime == hiroute_domain::ConnectorRuntimeKind::CpaBridge {
                return Err(Arc::from(MATERIALIZATION_AUTHORITY_FAILED));
            }
            resolve_target(provider, plan.transport_target.clone(), &scope).await?
        }
    };
    validate_attempt_permits(provider.state.as_ref(), permits.borrowed(), &scope)
        .await
        .map_err(|_| {
            runtime_state_authority.fail();
            Arc::from(RUNTIME_STATE_AUTHORITY_FAILED)
        })?;

    let quantum = context
        .write_quantum
        .min(plan.body_plans.attempt_request.max_chunk_bytes());
    let chat_tool_projection_bytes = prepared
        .chat_tool_projection
        .as_ref()
        .map(|projection| {
            projection
                .retained_bytes()
                .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))
        })
        .transpose()?;
    let chat_tool_projection_budget = match chat_tool_projection_bytes {
        Some(bytes) if bytes != 0 => Some(
            context
                .budget
                .reserve(MemoryRole::AttemptWire, bytes)
                .map_err(safe_error)?,
        ),
        Some(_) | None => None,
    };
    let chat_tool_projection = prepared.chat_tool_projection.take();
    let body = adapters::sequential_attempt_body(
        prepared.clone(),
        logical.replay.clone(),
        context.budget,
        quantum,
    )
    .map_err(safe_error)?;
    let lease = context.leases.acquire().map_err(safe_error)?;
    let mut headers = HeaderMap::new();
    let authority = resolved_target.authority.as_ref();
    headers.insert(
        HOST,
        HeaderValue::from_str(authority).map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?,
    );
    let connector_headers = profile
        .connector
        .headers
        .exact()
        .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&connector_headers.content_type)
            .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?,
    );
    for (name, value) in &connector_headers.required_headers {
        let name = http::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
        let value =
            HeaderValue::from_str(value).map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
        headers.insert(name, value);
    }
    headers.insert(CONTENT_LENGTH, HeaderValue::from(prepared.wire_len as u64));
    if let Some(credential) = credential.as_ref() {
        credential
            .apply_authorization(&mut headers)
            .map_err(|_| Arc::from(MATERIALIZATION_AUTHORITY_FAILED))?;
    }
    headers.insert(
        http::header::HeaderName::from_static("x-hiroute-profile-digest"),
        HeaderValue::from_str(expected_profile_digest)
            .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?,
    );
    headers.insert(
        http::header::HeaderName::from_static("x-hiroute-reason-ledger-id"),
        HeaderValue::from_str(
            candidate
                .reason_ledger_identity
                .as_ref()
                .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?
                .as_str(),
        )
        .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?,
    );
    Ok((
        PreparedAttemptHttpRequest {
            head: PreparedRequestHead {
                method: Method::POST,
                path_and_query: prepared.path.into(),
                headers,
            },
            body: PreparedAttemptBody::from_reader(body, lease).map_err(safe_error)?,
        },
        ProductionAttemptState {
            response_status: None,
            resolved_target,
            permits,
            classified_failure: None,
            profile: profile.clone(),
            client_profile,
            served_model_alias,
            streaming,
            native_output,
            chat_tool_projection,
            chat_tool_projection_budget,
            decoder: None,
            renderer,
            projector: None,
            prefix: Some(prefix),
            prefix_terminal_chunks: None,
            decoder_budget: Some(decoder_budget),
            budget: context.budget.clone(),
            semantic_seen: false,
            terminal_seen: false,
            semantic_terminal: None,
            retry_after: None,
            runtime_state_authority,
        },
    ))
}

fn client_profile_for_candidate(
    candidate: &CandidateProtocolProfile,
) -> Result<ClientProtocolProfile, Arc<str>> {
    ClientProtocolProfile::for_candidate(candidate)
        .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))
}

pub(super) fn resolved_transport_target(
    state: &ProductionAttemptState,
    binding: &AttemptExecutionBinding,
) -> Option<TransportTarget> {
    (state.resolved_target != binding.plan().transport_target)
        .then(|| state.resolved_target.clone())
}

pub(super) fn classify_materialization_failure(error: &Arc<str>) -> AttemptMaterializationFailure {
    let binding_cooldown = parse_cooldown(error, MATERIALIZATION_BINDING_COOLING_PREFIX);
    let credential_cooldown = parse_cooldown(error, MATERIALIZATION_CREDENTIAL_COOLING_PREFIX);
    let cooldown = binding_cooldown.or(credential_cooldown);
    let probe_busy = matches!(
        error.as_ref(),
        MATERIALIZATION_BINDING_PROBE_BUSY | MATERIALIZATION_CREDENTIAL_PROBE_BUSY
    );
    let disabled = matches!(
        error.as_ref(),
        MATERIALIZATION_BINDING_DISABLED | MATERIALIZATION_CREDENTIAL_DISABLED
    );
    let retryable = cooldown.is_some()
        || probe_busy
        || disabled
        || matches!(
            error.as_ref(),
            MATERIALIZATION_BINDING_UNAVAILABLE
                | MATERIALIZATION_CREDENTIAL_EXHAUSTED
                | MATERIALIZATION_DNS_FAILED
        );
    let class = match error.as_ref() {
        MATERIALIZATION_DNS_FAILED => AttemptMaterializationFailureClass::Dns,
        MATERIALIZATION_CREDENTIAL_EXHAUSTED
        | MATERIALIZATION_CREDENTIAL_PROBE_BUSY
        | MATERIALIZATION_CREDENTIAL_DISABLED => AttemptMaterializationFailureClass::Credential,
        _ if credential_cooldown.is_some() => AttemptMaterializationFailureClass::Credential,
        _ => AttemptMaterializationFailureClass::Materialization,
    };
    AttemptMaterializationFailure {
        class,
        provider: ProviderClassificationFacts {
            error_class: Some(label(if cooldown.is_some() {
                "runtime_state_cooling_down"
            } else if probe_busy {
                "runtime_state_probe_busy"
            } else if disabled {
                "runtime_state_disabled"
            } else if retryable {
                "materialization_retryable"
            } else {
                "materialization_fail_closed"
            })),
            retryability: if retryable {
                RetryabilityFact::Retryable
            } else {
                RetryabilityFact::NonRetryable
            },
            retry_after: cooldown.or_else(|| probe_busy.then(|| Duration::from_secs(1))),
            readiness: label("materialization_failed"),
            ..ProviderClassificationFacts::default()
        },
        termination_reason: label(match class {
            AttemptMaterializationFailureClass::Credential => "credential",
            AttemptMaterializationFailureClass::Dns => "dns",
            AttemptMaterializationFailureClass::Materialization => "materialization",
        }),
    }
}

fn permit_or_materialization_error(
    availability: PermitAvailability,
    binding: bool,
) -> Result<StatePermit, Arc<str>> {
    match availability {
        PermitAvailability::Permit(permit) => Ok(permit),
        PermitAvailability::CoolingDown { until } => {
            let remaining = until.saturating_duration_since(Instant::now());
            let rounded_millis = remaining
                .as_millis()
                .saturating_add(u128::from(remaining.subsec_nanos() % 1_000_000 != 0));
            let millis = u64::try_from(rounded_millis).unwrap_or(u64::MAX).max(1);
            let prefix = if binding {
                MATERIALIZATION_BINDING_COOLING_PREFIX
            } else {
                MATERIALIZATION_CREDENTIAL_COOLING_PREFIX
            };
            Err(Arc::from(format!("{prefix}{millis}")))
        }
        PermitAvailability::ProbeBusy => Err(Arc::from(if binding {
            MATERIALIZATION_BINDING_PROBE_BUSY
        } else {
            MATERIALIZATION_CREDENTIAL_PROBE_BUSY
        })),
        PermitAvailability::Disabled => Err(Arc::from(if binding {
            MATERIALIZATION_BINDING_DISABLED
        } else {
            MATERIALIZATION_CREDENTIAL_DISABLED
        })),
    }
}

fn parse_cooldown(error: &str, prefix: &str) -> Option<Duration> {
    error
        .strip_prefix(prefix)?
        .parse::<u64>()
        .ok()
        .filter(|millis| *millis != 0)
        .map(Duration::from_millis)
}

pub(super) async fn confirm_precommit(
    provider: &ProductionProvider,
    state: &mut ProductionAttemptState,
    deadline: Instant,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<(), Arc<str>> {
    let scope = ExecutionScope::new(deadline, cancellation.clone());
    let authority = state.runtime_state_authority.clone();
    match &state.classified_failure {
        Some(failure) => record_failure(
            provider.state.as_ref(),
            state.permits.borrowed(),
            failure,
            &provider.cooldowns,
            &scope,
        )
        .await
        .map_err(|_| {
            authority.fail();
            Arc::from(RUNTIME_STATE_AUTHORITY_FAILED)
        }),
        None => confirm_success(provider.state.as_ref(), state.permits.borrowed(), &scope)
            .await
            .map_err(|_| {
                authority.fail();
                Arc::from(RUNTIME_STATE_AUTHORITY_FAILED)
            }),
    }
}

pub(super) async fn confirm_attempt_failure(
    provider: &ProductionProvider,
    state: &mut ProductionAttemptState,
    failure: &AttemptFailureFacts,
    deadline: Instant,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<Option<ProviderClassificationFacts>, Arc<str>> {
    let Some((classified, facts)) = confirm_mechanical_state(
        provider.state.as_ref(),
        &state.permits,
        &state.profile,
        &provider.cooldowns,
        MechanicalFailureSignal {
            class: failure.class,
            timeout: failure.transport.timeout,
            runtime_state_authority: state.runtime_state_authority.clone(),
        },
        deadline,
        cancellation,
    )
    .await?
    else {
        return Ok(failure.provider.clone());
    };
    state.classified_failure = Some(classified);
    Ok(Some(facts))
}

pub(super) fn release_terminal_request(mut logical: ProductionLogicalRequest) {
    let _ = logical.replay.release_stream(&logical.raw_body);
    logical.replay.mark_terminal();
    logical.ir.take();
}

async fn confirm_mechanical_state(
    store: &dyn RuntimeStateStore,
    permits: &super::OwnedAttemptStatePermits,
    profile: &CandidateProtocolProfile,
    cooldowns: &RuntimeCooldownPolicy,
    failure: MechanicalFailureSignal,
    deadline: Instant,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<Option<(AttemptFailure, ProviderClassificationFacts)>, Arc<str>> {
    let raw = match failure.class {
        CoreAttemptFailureClass::Connect
            if failure.timeout == Some(AttemptTimeoutKind::Connect) =>
        {
            RawAttemptFailure::Timeout {
                phase: crate::attempt_outcome::AttemptTimeoutPhase::Connect,
            }
        }
        CoreAttemptFailureClass::Connect => RawAttemptFailure::Connect,
        CoreAttemptFailureClass::RequestWrite => RawAttemptFailure::Timeout {
            phase: crate::attempt_outcome::AttemptTimeoutPhase::RequestWrite,
        },
        CoreAttemptFailureClass::FirstByte => RawAttemptFailure::Timeout {
            phase: crate::attempt_outcome::AttemptTimeoutPhase::FirstByte,
        },
        CoreAttemptFailureClass::StreamIdle => RawAttemptFailure::Timeout {
            phase: crate::attempt_outcome::AttemptTimeoutPhase::StreamIdle,
        },
        CoreAttemptFailureClass::AttemptDeadline => RawAttemptFailure::Timeout {
            phase: crate::attempt_outcome::AttemptTimeoutPhase::Overall,
        },
        CoreAttemptFailureClass::Transport => RawAttemptFailure::Disconnect,
        CoreAttemptFailureClass::Credential
        | CoreAttemptFailureClass::Dns
        | CoreAttemptFailureClass::Materialization
        | CoreAttemptFailureClass::AttemptRequestFilter => return Ok(None),
    };
    let classified = classify_failure(&raw, connector_error_profile(profile)?);
    let facts = failure_facts(&classified);
    let scope = ExecutionScope::new(deadline, cancellation.clone());
    record_failure(store, permits.borrowed(), &classified, cooldowns, &scope)
        .await
        .map_err(|_| {
            failure.runtime_state_authority.fail();
            Arc::from(RUNTIME_STATE_AUTHORITY_FAILED)
        })?;
    Ok(Some((classified, facts)))
}

pub(crate) async fn resolve_target(
    provider: &ProductionProvider,
    target: TransportTarget,
    scope: &ExecutionScope,
) -> Result<TransportTarget, Arc<str>> {
    #[cfg(feature = "e2e-test-control")]
    {
        let test_dial = provider
            .test_dial
            .as_ref()
            .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
        if test_dial
            .as_ref()
            .is_some_and(|dial| dial.fails_dns(&target))
        {
            return Err(Arc::from(MATERIALIZATION_DNS_FAILED));
        }
        if let Some(mapped) = test_dial
            .as_ref()
            .map(|dial| dial.apply(target.clone()))
            .transpose()
            .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?
            .flatten()
        {
            return Ok(mapped);
        }
    }
    #[cfg(not(feature = "e2e-test-control"))]
    let _ = provider;
    let Some(authority) = target.unresolved_authority().map(ToOwned::to_owned) else {
        return Ok(target);
    };
    let (host, port) = dns_lookup_target(&authority, target.scheme)?;
    let addresses = scope
        .run(tokio::net::lookup_host((host.as_str(), port)))
        .await
        .map_err(|_| Arc::from(MATERIALIZATION_DNS_FAILED))?
        .map_err(|_| Arc::from(MATERIALIZATION_DNS_FAILED))?
        .collect::<Vec<SocketAddr>>();
    if addresses.is_empty() {
        return Err(Arc::from(MATERIALIZATION_DNS_FAILED));
    }
    target
        .with_resolved_addresses(addresses.into())
        .map_err(|_| Arc::from(MATERIALIZATION_DNS_FAILED))
}

fn dns_lookup_target(authority: &str, scheme: TransportScheme) -> Result<(String, u16), Arc<str>> {
    let authority = authority
        .parse::<http::uri::Authority>()
        .map_err(|_| Arc::from(MATERIALIZATION_DNS_FAILED))?;
    let port = authority.port_u16().unwrap_or(match scheme {
        TransportScheme::Http => 80,
        TransportScheme::Https => 443,
    });
    Ok((authority.host().to_owned(), port))
}

fn operational_target_matches_plan(
    operational: &hiroute_domain::GatewayOperationalTargetV1,
    plan: &CompiledAttemptPlan,
) -> bool {
    let Ok(uri) = operational.uri().parse::<http::Uri>() else {
        return false;
    };
    let Some(authority) = uri.authority() else {
        return false;
    };
    let scheme_matches = matches!(
        (uri.scheme_str(), plan.transport_target.scheme),
        (Some("http"), TransportScheme::Http) | (Some("https"), TransportScheme::Https)
    );
    let plan_authority = plan
        .transport_target
        .unresolved_authority()
        .unwrap_or(&plan.transport_target.authority);
    if !scheme_matches || authority.as_str() != plan_authority {
        return false;
    }
    match operational {
        hiroute_domain::GatewayOperationalTargetV1::RegisteredHttps { .. } => {
            plan.transport_target.requires_resolution()
        }
        hiroute_domain::GatewayOperationalTargetV1::UserConfiguredNative { .. } => {
            let numeric_host = authority
                .host()
                .strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .unwrap_or_else(|| authority.host());
            match numeric_host.parse::<std::net::IpAddr>() {
                Ok(ip) => {
                    !plan.transport_target.requires_resolution()
                        && plan.transport_target.addresses.len() == 1
                        && plan.transport_target.addresses[0]
                            == SocketAddr::new(
                                ip,
                                authority.port_u16().unwrap_or(
                                    if uri.scheme_str() == Some("https") {
                                        443
                                    } else {
                                        80
                                    },
                                ),
                            )
                }
                Err(_) => plan.transport_target.requires_resolution(),
            }
        }
        hiroute_domain::GatewayOperationalTargetV1::ManagedCpaLoopback { .. } => {
            let numeric_host = authority
                .host()
                .strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .unwrap_or_else(|| authority.host());
            !plan.transport_target.requires_resolution()
                && plan.transport_target.addresses.len() == 1
                && numeric_host
                    .parse::<std::net::IpAddr>()
                    .ok()
                    .zip(authority.port_u16())
                    .is_some_and(|(ip, port)| {
                        plan.transport_target.addresses[0] == SocketAddr::new(ip, port)
                    })
        }
    }
}

fn profile_digest(profile: &CandidateProtocolProfile) -> Result<String, Arc<str>> {
    PlannerCandidateFactsV1::seal(
        "digest-check",
        "digest-check",
        profile.clone(),
        1,
        None,
        CostClassV1::Free,
        None,
    )
    .map(|candidate| candidate.profile_digest)
    .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::attempt_outcome::AttemptFailureClass;
    use crate::ports::{CasOutcome, InMemoryRuntimeStateStore, ProbeLeaseOutcome, RuntimeHealth};
    use crate::runtime::state::acquire_target_permit;
    use crate::server::core_runtime::profiles::fixed_reasoning;

    use super::*;

    #[test]
    fn dns_lookup_uses_scheme_default_and_preserves_explicit_port() {
        assert_eq!(
            dns_lookup_target("provider.example", TransportScheme::Https).unwrap(),
            ("provider.example".to_owned(), 443)
        );
        assert_eq!(
            dns_lookup_target("provider.example:8443", TransportScheme::Https).unwrap(),
            ("provider.example".to_owned(), 8443)
        );
        assert_eq!(
            dns_lookup_target("provider.example", TransportScheme::Http).unwrap(),
            ("provider.example".to_owned(), 80)
        );
    }

    async fn active_permits(
        store: &dyn RuntimeStateStore,
    ) -> super::super::OwnedAttemptStatePermits {
        let scope = ExecutionScope::new(
            Instant::now() + Duration::from_secs(1),
            tokio_util::sync::CancellationToken::new(),
        );
        let binding_key = RuntimeStateKey::binding("mechanical-binding");
        let binding_permit = acquire_target_permit(store, &binding_key, &scope)
            .await
            .unwrap()
            .unwrap();
        let credential_key =
            RuntimeStateKey::credential("mechanical-binding", "credential", "key", 1);
        let credential_permit = acquire_target_permit(store, &credential_key, &scope)
            .await
            .unwrap()
            .unwrap();
        super::super::OwnedAttemptStatePermits::new(
            binding_key,
            binding_permit,
            credential_key,
            credential_permit,
        )
    }

    fn production_profile() -> CandidateProtocolProfile {
        let mut profile = CandidateProtocolProfile::exact_portable_path(
            crate::server::request_plan::IngressProtocol::Responses,
            crate::server::request_plan::IngressProtocol::Responses,
            "native-model",
            fixed_reasoning("fixed"),
        );
        profile.connector.connector_id = "builtin-openai".into();
        profile
    }

    #[tokio::test]
    async fn connect_first_byte_and_idle_failures_cool_exact_binding() {
        for (class, timeout, expected_class) in [
            (
                CoreAttemptFailureClass::Connect,
                None,
                AttemptFailureClass::Transient,
            ),
            (
                CoreAttemptFailureClass::Connect,
                Some(AttemptTimeoutKind::Connect),
                AttemptFailureClass::Timeout(crate::attempt_outcome::AttemptTimeoutPhase::Connect),
            ),
            (
                CoreAttemptFailureClass::FirstByte,
                Some(AttemptTimeoutKind::FirstByte),
                AttemptFailureClass::Timeout(
                    crate::attempt_outcome::AttemptTimeoutPhase::FirstByte,
                ),
            ),
            (
                CoreAttemptFailureClass::StreamIdle,
                Some(AttemptTimeoutKind::StreamIdle),
                AttemptFailureClass::Timeout(
                    crate::attempt_outcome::AttemptTimeoutPhase::StreamIdle,
                ),
            ),
        ] {
            let store = InMemoryRuntimeStateStore::default();
            let permits = active_permits(&store).await;
            let cancellation = tokio_util::sync::CancellationToken::new();
            let authority = super::super::RuntimeStateAuthoritySignal::default();
            let confirmed = confirm_mechanical_state(
                &store,
                &permits,
                &production_profile(),
                &RuntimeCooldownPolicy::default(),
                MechanicalFailureSignal {
                    class,
                    timeout,
                    runtime_state_authority: authority,
                },
                Instant::now() + Duration::from_secs(1),
                &cancellation,
            )
            .await
            .unwrap()
            .expect("mechanical failure is classified");

            assert_eq!(confirmed.1.retryability, RetryabilityFact::Retryable);
            assert_eq!(confirmed.0.class, expected_class);
            let binding = store.entry(&RuntimeStateKey::binding("mechanical-binding"));
            assert_eq!(binding.generation, 1);
            assert!(matches!(binding.health, RuntimeHealth::CoolingDown { .. }));
            assert_eq!(
                store
                    .entry(&RuntimeStateKey::credential(
                        "mechanical-binding",
                        "credential",
                        "key",
                        1,
                    ))
                    .generation,
                0,
                "mechanical failures must not cool the credential"
            );
        }
    }

    struct FailingCasStore;

    #[async_trait]
    impl ProductionStore for FailingCasStore {
        async fn read_exact(
            &self,
            _key: &RuntimeStateKey,
            _scope: &ExecutionScope,
        ) -> Result<RuntimeStateEntry, PortError> {
            Ok(RuntimeStateEntry::default())
        }

        async fn compare_and_swap_exact(
            &self,
            _key: &RuntimeStateKey,
            _expected_generation: u64,
            _next: RuntimeStateEntry,
            _scope: &ExecutionScope,
        ) -> Result<CasOutcome, PortError> {
            Err(PortError::Unavailable("external RuntimeStateStore"))
        }

        async fn acquire_probe_lease_exact(
            &self,
            _key: &RuntimeStateKey,
            _expected_generation: u64,
            _now: Instant,
            _lease_duration: Duration,
            _scope: &ExecutionScope,
        ) -> Result<ProbeLeaseOutcome, PortError> {
            Err(PortError::Unavailable("external RuntimeStateStore"))
        }
    }

    #[tokio::test]
    async fn mechanical_failure_state_write_error_is_fail_closed() {
        let store = ProductionStateAdapter {
            inner: Arc::new(FailingCasStore),
        };
        let permits = active_permits(&store).await;
        let cancellation = tokio_util::sync::CancellationToken::new();
        let authority = super::super::RuntimeStateAuthoritySignal::default();

        let error = confirm_mechanical_state(
            &store,
            &permits,
            &production_profile(),
            &RuntimeCooldownPolicy::default(),
            MechanicalFailureSignal {
                class: CoreAttemptFailureClass::Connect,
                timeout: None,
                runtime_state_authority: authority.clone(),
            },
            Instant::now() + Duration::from_secs(1),
            &cancellation,
        )
        .await
        .unwrap_err();

        assert_eq!(error.as_ref(), RUNTIME_STATE_AUTHORITY_FAILED);
        assert!(authority.failed());
    }
}
