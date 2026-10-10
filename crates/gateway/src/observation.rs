//! Gateway-owned observation producers.
//!
//! Lifecycle, immutable execution facts and conversation content use separate
//! byte-bounded, non-blocking queues. This module deliberately contains no
//! database writer, query service, retention policy, Hub client or production
//! OTLP exporter.

#[path = "observation/content.rs"]
mod content;
#[path = "observation/contracts.rs"]
mod contracts;
#[path = "observation/crypto.rs"]
mod crypto;
#[path = "observation/otel.rs"]
mod otel;
#[path = "observation/ports.rs"]
mod ports;
#[path = "observation/pricing.rs"]
mod pricing;
#[path = "observation/producer.rs"]
mod producer;
#[path = "observation/provider.rs"]
mod provider;
pub(super) use provider::wire_diagnostic;
#[path = "observation/request.rs"]
mod request;
#[path = "observation/schema.rs"]
mod schema;
#[path = "observation/selection.rs"]
mod selection;

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_diagnostics::event::ObservationChannel;
use hiroute_diagnostics::runtime::DiagnosticsPort;
use hiroute_domain::{
    FrozenExecutionTrustV1, LogicalRequestId, ModelRequestRouteV2, RunObservationLink, SessionId,
    WorkspaceId,
};
use serde::Serialize;
use thiserror::Error;

use crate::context_hold::ContextIdentityFacts;
use crate::server::request_plan::{AuthorizedRequestPlan, IngressProtocol};

pub use content::AcceptedResponseCapture;
pub use contracts::*;
pub use otel::{
    OTEL_MAPPER_VERSION, OTEL_MAPPING_CONTRACT, OTEL_MAPPING_DIGEST,
    OTEL_SEMANTIC_CONVENTIONS_VERSION, OtelContentPolicy, OtelGenAiMapper,
};
pub use ports::{ObservedCredentialResolver, ObservedRuntimeStateStore};
pub use pricing::{RequestPriceSnapshot, RequestPriceSource};
pub use producer::{
    GatewayObservationSinks, ObservationAck, ObservationNack, ObservationRecord,
    ObservationRecordSink, accounted_acknowledgement,
};
pub use provider::ObservedProductionProvider;
pub use request::{RequestObservation, RequestObservationMetadata};
pub use schema::*;
pub use selection::ObservedProductionSelection;

use crypto::{random_key, stable_id};
use producer::{ChannelProducer, EnvironmentObservation, ObservationProducerIdentity};
use provider::CanonicalCaptureProducer;

const PRODUCER_REVISION: &str = "hiroute-gateway-observation/1";

#[derive(Clone)]
struct ObservationChannels {
    lifecycle: ChannelProducer,
    execution: ChannelProducer,
    content: ChannelProducer,
    run_relation: ChannelProducer,
    otel: ChannelProducer,
    response_capture: CanonicalCaptureProducer,
}

pub struct GatewayObservation {
    price_source: Option<Arc<dyn RequestPriceSource>>,
    enabled: bool,
    key: [u8; 32],
    instance_nonce: [u8; 32],
    channels: ObservationChannels,
    next_request: AtomicU64,
    content_policy: OtelContentPolicy,
    diagnostics: Arc<Mutex<DiagnosticsPort>>,
}

/// Native identity fields that the Gateway itself accepts as reliable Agent correlation. Live
/// verification uses this only after the native client returns its own session/thread identity;
/// the resulting database session remains the producer's HMAC-derived identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NativeAgentObservationIdentityV1 {
    ClaudeMetadataSession {
        session_id: String,
    },
    CodexThread {
        thread_id: String,
    },
    CodexSession {
        session_id: String,
    },
    /// Qoder native session UUID from the supported API client `session-id` header.
    QoderSession {
        session_id: String,
    },
    CodexThreadSession {
        thread_id: String,
        session_id: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum NativeAgentObservationSessionError {
    #[error("the observation workspace key is invalid")]
    InvalidWorkspaceKey,
    #[error("the frozen execution trust is invalid")]
    InvalidTrust,
    #[error("the frozen execution route is invalid")]
    InvalidRoute,
    #[error("the ingress protocol cannot identify a native Agent observation")]
    UnsupportedIngress,
    #[error("the native Agent observation identity is invalid")]
    InvalidIdentity,
    #[error("the derived observation session identity is invalid")]
    InvalidSession,
}

/// Derives the exact observation session used by the Gateway producer for a reliable native
/// Agent identity. Keeping this mapping here prevents consumers from copying the private HMAC
/// formula or mistaking a Claude/Codex native UUID for the observation database session ID.
pub fn derive_native_agent_observation_session_id(
    workspace: &WorkspaceId,
    workspace_key: &[u8; 32],
    trust: &FrozenExecutionTrustV1,
    identity: &NativeAgentObservationIdentityV1,
) -> Result<SessionId, NativeAgentObservationSessionError> {
    if workspace_key.iter().all(|byte| *byte == 0) {
        return Err(NativeAgentObservationSessionError::InvalidWorkspaceKey);
    }
    trust
        .validate()
        .map_err(|_| NativeAgentObservationSessionError::InvalidTrust)?;
    let route_identity = match (&trust.route, &trust.agent_plan_id) {
        (ModelRequestRouteV2::Plan { .. }, Some(plan_id)) => plan_id.as_str(),
        (ModelRequestRouteV2::Fixed { binding_digest }, None) => binding_digest.as_str(),
        _ => return Err(NativeAgentObservationSessionError::InvalidRoute),
    };
    let ingress = match trust.ingress_protocol {
        hiroute_domain::IngressProtocolV1::Responses => "responses",
        hiroute_domain::IngressProtocolV1::ChatCompletions => "chat_completions",
        hiroute_domain::IngressProtocolV1::Messages => "messages",
        hiroute_domain::IngressProtocolV1::Unknown => {
            return Err(NativeAgentObservationSessionError::UnsupportedIngress);
        }
    };
    let (kind, parts) = match identity {
        NativeAgentObservationIdentityV1::ClaudeMetadataSession { session_id } => {
            ("claude_metadata_session", vec![session_id.as_str()])
        }
        NativeAgentObservationIdentityV1::CodexThread { thread_id } => {
            ("codex_thread", vec![thread_id.as_str()])
        }
        NativeAgentObservationIdentityV1::CodexSession { session_id } => {
            ("codex_session", vec![session_id.as_str()])
        }
        NativeAgentObservationIdentityV1::QoderSession { session_id } => {
            if !matches!(
                trust.ingress_protocol,
                hiroute_domain::IngressProtocolV1::Responses
                    | hiroute_domain::IngressProtocolV1::Messages
            ) {
                return Err(NativeAgentObservationSessionError::UnsupportedIngress);
            }
            // The existing wire producer already accepts this header independently of client
            // branding. Retain its historical HMAC namespace; do not create another parser or
            // relabel the native client as Codex merely to locate its receipt.
            ("codex_session", vec![session_id.as_str()])
        }
        NativeAgentObservationIdentityV1::CodexThreadSession {
            thread_id,
            session_id,
        } => (
            "codex_thread_session",
            vec![thread_id.as_str(), session_id.as_str()],
        ),
    };
    if parts
        .iter()
        .any(|part| part.is_empty() || part.len() > 128 || part.chars().any(char::is_control))
    {
        return Err(NativeAgentObservationSessionError::InvalidIdentity);
    }
    SessionId::parse(reliable_observation_session_id(
        workspace_key,
        workspace.as_str(),
        &trust.authority_id,
        &trust.grant_id,
        route_identity,
        &trust.served_model_id,
        ingress,
        kind,
        &parts,
    ))
    .map_err(|_| NativeAgentObservationSessionError::InvalidSession)
}

#[allow(clippy::too_many_arguments)]
fn reliable_observation_session_id(
    key: &[u8; 32],
    workspace_id: &str,
    authority_id: &str,
    grant_id: &str,
    route_identity: &str,
    served_model_id: &str,
    ingress_protocol: &str,
    kind: &str,
    identity_parts: &[&str],
) -> String {
    let part_count = (identity_parts.len() as u64).to_be_bytes();
    let mut parts = vec![
        workspace_id.as_bytes(),
        authority_id.as_bytes(),
        grant_id.as_bytes(),
        route_identity.as_bytes(),
        served_model_id.as_bytes(),
        ingress_protocol.as_bytes(),
        kind.as_bytes(),
        part_count.as_slice(),
    ];
    parts.extend(identity_parts.iter().map(|part| part.as_bytes()));
    stable_id(
        "observation-session",
        key,
        b"observation-session/v1",
        &parts,
    )
}

impl GatewayObservation {
    pub(super) fn diagnostic_context(&self) -> hiroute_diagnostics::context::DiagnosticContext {
        self.diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .handle()
            .context()
    }
    pub fn with_price_source(mut self, source: Arc<dyn RequestPriceSource>) -> Self {
        self.price_source = Some(source);
        self
    }

    /// Attach the local diagnostic port. Diagnostics are an independent sink: a
    /// disabled or degraded port never changes which lifecycle/execution facts
    /// are produced, and channel loss/NACK projection stays inert until a port
    /// exists.
    pub fn with_diagnostics(self, diagnostics: DiagnosticsPort) -> Self {
        self.set_diagnostics(diagnostics);
        self
    }

    pub(crate) fn set_diagnostics(&self, diagnostics: DiagnosticsPort) {
        *self
            .diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = diagnostics;
    }
    pub fn from_environment() -> Self {
        let environment = EnvironmentObservation::load();
        Self::with_sinks_and_policy(
            environment.enabled,
            environment.queue_bytes,
            environment.sinks,
            OtelContentPolicy::Disabled,
        )
    }

    pub fn with_sinks(queue_bytes: usize, sinks: GatewayObservationSinks) -> Self {
        Self::with_sinks_and_policy(true, queue_bytes, sinks, OtelContentPolicy::Disabled)
    }

    pub fn with_sinks_and_policy(
        enabled: bool,
        queue_bytes: usize,
        sinks: GatewayObservationSinks,
        content_policy: OtelContentPolicy,
    ) -> Self {
        Self::build(enabled, queue_bytes, sinks, content_policy, random_key())
    }

    /// Binds producer identity and transcript digests to caller-owned,
    /// workspace-stable, nonzero key material. The key must come from a
    /// protected local or enterprise adapter and is never serialized into an
    /// observation. An all-zero key fails closed by disabling observation.
    pub fn with_sinks_and_policy_and_workspace_key(
        enabled: bool,
        queue_bytes: usize,
        sinks: GatewayObservationSinks,
        content_policy: OtelContentPolicy,
        workspace_hmac_key: [u8; 32],
    ) -> Self {
        Self::build(
            enabled,
            queue_bytes,
            sinks,
            content_policy,
            Some(workspace_hmac_key),
        )
    }

    fn build(
        enabled: bool,
        queue_bytes: usize,
        sinks: GatewayObservationSinks,
        content_policy: OtelContentPolicy,
        key: Option<[u8; 32]>,
    ) -> Self {
        let key_is_valid = key
            .as_ref()
            .is_some_and(|key| key.iter().any(|byte| *byte != 0));
        let instance_nonce = random_key();
        let enabled = enabled && key_is_valid && instance_nonce.is_some();
        let key = key.unwrap_or([0_u8; 32]);
        let instance_nonce = instance_nonce.unwrap_or([0_u8; 32]);
        let started_at = unix_nanos().to_be_bytes();
        let diagnostics = Arc::new(Mutex::new(DiagnosticsPort::default()));
        let channel = |name: &'static str,
                       component: &'static str,
                       channel_kind: ObservationChannel,
                       sink: Arc<dyn ObservationRecordSink>| {
            if !enabled {
                return ChannelProducer::disabled(component, PRODUCER_REVISION);
            }
            ChannelProducer::new(
                ObservationProducerIdentity {
                    channel: Arc::from(name),
                    component: Arc::from(component),
                    revision: Arc::from(PRODUCER_REVISION),
                    producer_id: Arc::from(stable_id(
                        &format!("{name}-producer"),
                        &key,
                        b"producer-id",
                        &[name.as_bytes()],
                    )),
                    producer_epoch: Arc::from(stable_id(
                        &format!("{name}-epoch"),
                        &key,
                        b"producer-epoch",
                        &[name.as_bytes(), &instance_nonce, &started_at],
                    )),
                    stream_id: Arc::from(stable_id(
                        &format!("{name}-stream"),
                        &key,
                        b"producer-stream",
                        &[name.as_bytes(), &instance_nonce, &started_at],
                    )),
                },
                queue_bytes,
                sink,
                Arc::clone(&diagnostics),
                channel_kind,
            )
        };
        Self {
            price_source: None,
            enabled,
            key,
            instance_nonce,
            channels: ObservationChannels {
                lifecycle: channel(
                    "lifecycle",
                    "gateway-lifecycle",
                    ObservationChannel::Facts,
                    sinks.lifecycle,
                ),
                execution: channel(
                    "execution_fact",
                    "gateway-execution",
                    ObservationChannel::Facts,
                    sinks.execution_fact,
                ),
                content: channel(
                    "conversation_content",
                    "gateway-content",
                    ObservationChannel::Content,
                    sinks.conversation_content,
                ),
                run_relation: channel(
                    "run_relation",
                    "gateway-run-relation",
                    ObservationChannel::Facts,
                    sinks.run_relation,
                ),
                otel: channel(
                    "otel",
                    "gateway-otel-mapper",
                    ObservationChannel::Projection,
                    sinks.otel,
                ),
                // In-memory agent history also uses this bounded canonical decoder;
                // disabling persistent observation must not disable turn history.
                response_capture: CanonicalCaptureProducer::new(true, queue_bytes),
            },
            next_request: AtomicU64::new(1),
            content_policy,
            diagnostics,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn correlation_key(&self) -> [u8; 32] {
        self.key
    }

    pub(crate) fn begin_request_with_content(
        &self,
        authorized: &AuthorizedRequestPlan,
        ingress: IngressProtocol,
        identity: &ContextIdentityFacts,
        capture_content: bool,
    ) -> RequestObservation {
        let ordinal = self.next_request.fetch_add(1, Ordering::Relaxed);
        let ordinal_bytes = ordinal.to_be_bytes();
        let receipt = authorized.receipt();
        let ingress_protocol = match ingress {
            IngressProtocol::Responses => "responses",
            IngressProtocol::ChatCompletions => "chat_completions",
            IngressProtocol::Messages => "messages",
        };
        let workspace_id = receipt.workspace_id.to_string();
        let conversation_id = identity.observation_identity().map_or_else(
            || {
                stable_id(
                    "conversation",
                    &self.key,
                    b"observation-request/v1",
                    &[
                        &self.instance_nonce,
                        &ordinal_bytes,
                        receipt.served_model_id.as_bytes(),
                    ],
                )
            },
            |(kind, identity_parts)| {
                let route_identity = match &authorized.planner_policy().identity {
                    crate::server::core_runtime::profiles::PlannerRouteIdentityV2::Plan {
                        plan_id,
                        ..
                    } => plan_id.as_str(),
                    crate::server::core_runtime::profiles::PlannerRouteIdentityV2::Fixed {
                        binding_digest,
                    } => binding_digest.as_str(),
                };
                let parts = identity_parts
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                reliable_observation_session_id(
                    &self.key,
                    receipt.workspace_id.as_ref(),
                    receipt.authority_id.as_ref(),
                    receipt.grant_id.as_ref(),
                    route_identity,
                    receipt.served_model_id.as_ref(),
                    ingress_protocol,
                    kind,
                    &parts,
                )
            },
        );
        let request_id = stable_id(
            "request",
            &self.key,
            b"logical-request",
            &[
                &self.instance_nonce,
                &ordinal_bytes,
                receipt.authority_id.as_bytes(),
                &receipt.publication_revision.to_be_bytes(),
            ],
        );
        let turn_id = stable_id(
            "turn",
            &self.key,
            b"request-scoped-turn",
            &[request_id.as_bytes()],
        );
        if let Some(run) = authorized.verified_run_observation().cloned()
            && let (Ok(workspace_id), Ok(request_identity)) = (
                WorkspaceId::parse(workspace_id.as_str()),
                LogicalRequestId::parse(request_id.as_str()),
            )
        {
            let key = self.key;
            let relation_request_id = request_id.clone();
            self.channels
                .run_relation
                .publish(move |producer, _sequence, _loss| RunObservationLink {
                    workspace_id,
                    request_id: request_identity,
                    task_id: run.task_id,
                    run_id: run.run_id.clone(),
                    producer_epoch: producer.producer_epoch,
                    source_event_id: stable_id(
                        "relation-event",
                        &key,
                        b"verified-run-relation",
                        &[relation_request_id.as_bytes(), run.run_id.as_bytes()],
                    ),
                    plan_id: run.plan_id,
                    plan_revision: run.plan_revision.to_string(),
                    publication_ref: run.publication_ref,
                    harness_id: run.harness_id,
                    protocol_kind: ingress_protocol.into(),
                    native_session_id: run.native_session_id,
                    native_turn_id: None,
                    parent_context_ref: run.parent_context_ref,
                    continued_from_run_id: run.continued_from_run_id,
                });
        }
        RequestObservation::new(
            request::RequestObservationCapture {
                enabled: self.enabled,
                content: capture_content,
            },
            self.key,
            self.channels.clone(),
            self.content_policy,
            pricing::capture(
                self.price_source.as_ref(),
                &workspace_id,
                authorized.pricing_bindings(),
                (unix_nanos() / 1_000_000) as i64,
            ),
            RequestObservationMetadata {
                workspace_id,
                conversation_id,
                session_scope: identity.session_scope().into(),
                correlation_provenance: identity.correlation_provenance().into(),
                turn_id,
                request_id,
                authority_id: receipt.authority_id.to_string(),
                authority_epoch: receipt.authority_epoch,
                publication_revision: receipt.publication_revision,
                publication_digest: receipt.publication_digest.to_string(),
                route: receipt.route.clone(),
                plan_display_name: receipt.plan_display_name.as_ref().map(ToString::to_string),
                served_model_id: receipt.served_model_id.to_string(),
                grant_id: receipt.grant_id.to_string(),
                grant_generation: receipt.grant_generation,
                ingress_protocol: ingress_protocol.into(),
            },
            self.diagnostics
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        )
    }
}

tokio::task_local! {
    static ACTIVE_REQUEST_OBSERVATION: RequestObservation;
}

pub(super) async fn with_active_request<F>(request: RequestObservation, future: F) -> F::Output
where
    F: Future,
{
    ACTIVE_REQUEST_OBSERVATION.scope(request, future).await
}

pub(crate) fn active_request() -> Option<RequestObservation> {
    ACTIVE_REQUEST_OBSERVATION.try_with(Clone::clone).ok()
}

pub(super) fn unix_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_nanos().min(u128::from(u64::MAX)) as u64
        })
}

#[cfg(test)]
pub(super) fn accepted_request_for_runtime_test() -> RequestObservation {
    let gateway = GatewayObservation::with_sinks_and_policy(
        true,
        64 * 1024,
        GatewayObservationSinks::discard(),
        OtelContentPolicy::Disabled,
    );
    let request = RequestObservation::new(
        request::RequestObservationCapture {
            enabled: true,
            content: true,
        },
        gateway.key,
        gateway.channels.clone(),
        OtelContentPolicy::Disabled,
        pricing::capture(None, "workspace:runtime-test", &[], 0),
        RequestObservationMetadata {
            workspace_id: "workspace:runtime-test".into(),
            conversation_id: "conversation:runtime-test".into(),
            session_scope: "request_scoped".into(),
            correlation_provenance: "test".into(),
            turn_id: "turn:runtime-test".into(),
            request_id: "request:runtime-test".into(),
            authority_id: "authority:runtime-test".into(),
            authority_epoch: 1,
            publication_revision: 1,
            publication_digest: "sha256:runtime-test-publication".into(),
            route: hiroute_domain::ModelRequestRouteV2::Plan {
                revision: 1,
                semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"runtime-test-plan"),
            },
            plan_display_name: Some("Runtime test".into()),
            served_model_id: "runtime-test".into(),
            grant_id: "grant:runtime-test".into(),
            grant_generation: 1,
            ingress_protocol: "responses".into(),
        },
        gateway
            .diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone(),
    );
    request.no_credential_materialized("binding:runtime-test", "credential/none/runtime-test");
    request.disposition_published(
        &hiroute_gateway_core::runtime::attempt::PublishedDisposition {
            request_id: hiroute_gateway_core::runtime::attempt::RequestId(1),
            attempt_id: hiroute_gateway_core::runtime::attempt::AttemptId(1),
            generation: hiroute_gateway_core::runtime::attempt::AttemptGeneration(1),
            disposition: hiroute_gateway_core::runtime::attempt::Disposition::Accept,
        },
    );
    request
}

#[cfg(test)]
pub(super) fn accepted_frame_count_for_runtime_test(request: &RequestObservation) -> u32 {
    request.lock_state().response_frame_ordinal
}

#[cfg(test)]
#[path = "observation/tests.rs"]
mod tests;
