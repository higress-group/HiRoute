use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use hiroute_gateway_core::core::execution_plan::{
    AcceptedResponseExecutionBinding, AttemptExecutionBinding, ConfigEventSnapshot, TransportTarget,
};
use hiroute_gateway_core::core::filter::LocalReply;
use hiroute_gateway_core::runtime::attempt::{
    PrecommitEvent, PreparedAttemptHttpRequest, PublishedDisposition,
};
use hiroute_gateway_core::runtime::driver::{
    AcceptedBodyFrame, AttemptFailureFacts, AttemptMaterializationContext,
    AttemptMaterializationFailure, ClassifiedAttemptResult, DecodedSseToken,
    LogicalRequestBodyFrame, LogicalRequestContext, NormalizedAttemptLocalReply,
    PinnedConfigContext, PrecommitClassification, ProviderAcceptedEvent, ProviderAttemptCompletion,
    ProviderClassificationFacts, ProviderRuntimePort, UpstreamSideEffectSnapshot,
};
use hiroute_gateway_core::transport::{GatewayRequestHead, GatewayResponseHead};
use tokio_util::sync::CancellationToken;

use crate::runtime::{
    ProductionAttemptState, ProductionDecodedSse, ProductionLogicalRequest, ProductionProvider,
    ProductionReadiness, ProductionRouteContext,
};

use super::{RequestObservation, active_request};

#[path = "provider/capture.rs"]
mod capture;
#[path = "provider/wire_diagnostic.rs"]
mod wire_diagnostic;

pub(super) use capture::{CanonicalCaptureHandle, CanonicalCaptureProducer};

/// Observation-only decorator around the production provider. The inner
/// provider remains the sole execution owner. Only after authoritative
/// classification succeeds does this decorator enqueue a byte-budgeted copy
/// for off-path canonical capture; every observation error degrades capture
/// and is deliberately excluded from the provider result.
pub struct ObservedProductionProvider {
    inner: ProductionProvider,
}

pub struct ObservedAttemptState {
    inner: ProductionAttemptState,
    tracker: Option<CanonicalCaptureHandle>,
    observation: Option<RequestObservation>,
}

pub struct ObservedReadiness {
    inner: ProductionReadiness,
    tracker: Option<CanonicalCaptureHandle>,
    observation: Option<RequestObservation>,
}

pub struct ObservedDecodedSse {
    inner: ProductionDecodedSse,
}

impl ObservedProductionProvider {
    pub fn new(inner: ProductionProvider) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ProviderRuntimePort for ObservedProductionProvider {
    type LogicalRequest = ProductionLogicalRequest;
    type RouteRequestContext = ProductionRouteContext;
    type AttemptState = ObservedAttemptState;
    type Readiness = ObservedReadiness;
    type DecodedSseEvent = ObservedDecodedSse;

    async fn begin_request(
        &self,
        head: GatewayRequestHead,
        context: LogicalRequestContext<'_>,
    ) -> Result<Self::LogicalRequest, Arc<str>> {
        self.inner.begin_request(head, context).await
    }

    async fn consume_request_body(
        &self,
        logical: &mut Self::LogicalRequest,
        frame: LogicalRequestBodyFrame,
    ) -> Result<(), Arc<str>> {
        self.inner.consume_request_body(logical, frame).await
    }

    fn finalize_route_request_context(
        &self,
        logical: &mut Self::LogicalRequest,
    ) -> Result<Self::RouteRequestContext, Arc<str>> {
        self.inner.finalize_route_request_context(logical)
    }

    async fn materialize_attempt(
        &self,
        logical: &mut Self::LogicalRequest,
        context: AttemptMaterializationContext<'_>,
    ) -> Result<(PreparedAttemptHttpRequest, Self::AttemptState), Arc<str>> {
        let stable_binding_id = context.plan().stable_target_key.as_str().to_owned();
        let no_credential_ref = context
            .credential_ref()
            .as_str()
            .starts_with("credential/none/")
            .then(|| context.credential_ref().as_str().to_owned());
        let (request, inner) = self.inner.materialize_attempt(logical, context).await?;
        let observation = active_request().filter(RequestObservation::is_enabled);
        if let Some(observation) = &observation {
            observation.wire_diagnostic(wire_diagnostic::request(&request.head.headers));
        }
        if let (Some(observation), Some(credential_ref)) = (&observation, no_credential_ref) {
            observation.no_credential_materialized(&stable_binding_id, &credential_ref);
        }
        let tracker = observation.as_ref().and_then(|observation| {
            observation.begin_response_capture(
                &stable_binding_id,
                inner.chat_tool_projection_for_observation(),
            )
        });
        Ok((
            request,
            ObservedAttemptState {
                inner,
                tracker,
                observation,
            },
        ))
    }

    fn resolved_transport_target(
        &self,
        state: &Self::AttemptState,
        binding: &AttemptExecutionBinding,
    ) -> Result<Option<TransportTarget>, Arc<str>> {
        self.inner.resolved_transport_target(&state.inner, binding)
    }

    fn classify_materialization_failure(&self, error: &Arc<str>) -> AttemptMaterializationFailure {
        self.inner.classify_materialization_failure(error)
    }

    fn classify_precommit(
        &self,
        state: &mut Self::AttemptState,
        event: PrecommitEvent,
        pinned_configs: &PinnedConfigContext<'_>,
        event_configs: &ConfigEventSnapshot,
    ) -> Result<PrecommitClassification<Self::Readiness, Self::DecodedSseEvent>, Arc<str>> {
        if let (Some(observation), PrecommitEvent::ResponseHead(head)) =
            (&state.observation, &event)
        {
            observation.wire_diagnostic(wire_diagnostic::response(
                head.headers(),
                head.status().as_u16(),
            ));
        }
        let prepared = state
            .tracker
            .as_ref()
            .and_then(|tracker| tracker.prepare_native(&event));
        let result =
            self.inner
                .classify_precommit(&mut state.inner, event, pinned_configs, event_configs);
        let result = match result {
            Ok(result) => {
                if let Some(prepared) = prepared {
                    prepared.commit();
                }
                result
            }
            Err(error) => {
                if let Some(prepared) = prepared {
                    prepared.cancel("canonical_capture_authoritative_classification_failed");
                } else if let Some(tracker) = &state.tracker {
                    tracker.cancel_unless_terminal(
                        "canonical_capture_authoritative_classification_failed",
                    );
                }
                return Err(error);
            }
        };
        Ok(PrecommitClassification {
            classified: result.classified.map(|classified| ClassifiedAttemptResult {
                facts: classified.facts,
                readiness: ObservedReadiness {
                    inner: classified.readiness,
                    tracker: state.tracker.take(),
                    observation: state.observation.clone(),
                },
            }),
            decoded_sse: result.decoded_sse.map(|decoded| DecodedSseToken {
                sequence: decoded.sequence,
                decoded: ObservedDecodedSse {
                    inner: decoded.decoded,
                },
            }),
        })
    }

    async fn confirm_precommit(
        &self,
        state: &mut Self::AttemptState,
        facts: &ProviderClassificationFacts,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<(), Arc<str>> {
        self.inner
            .confirm_precommit(&mut state.inner, facts, deadline, cancellation)
            .await
    }

    async fn confirm_attempt_failure(
        &self,
        state: &mut Self::AttemptState,
        failure: &AttemptFailureFacts,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Option<ProviderClassificationFacts>, Arc<str>> {
        self.inner
            .confirm_attempt_failure(&mut state.inner, failure, deadline, cancellation)
            .await
    }

    fn normalize_attempt_local_reply(
        &self,
        state: &mut Self::AttemptState,
        reply: LocalReply,
        upstream_side_effects: UpstreamSideEffectSnapshot,
    ) -> Result<NormalizedAttemptLocalReply<Self::Readiness>, Arc<str>> {
        let normalized = self.inner.normalize_attempt_local_reply(
            &mut state.inner,
            reply,
            upstream_side_effects,
        )?;
        Ok(NormalizedAttemptLocalReply {
            classified: ClassifiedAttemptResult {
                facts: normalized.classified.facts,
                readiness: ObservedReadiness {
                    inner: normalized.classified.readiness,
                    tracker: state.tracker.take(),
                    observation: state.observation.clone(),
                },
            },
            reply: normalized.reply,
            upstream_side_effects: normalized.upstream_side_effects,
        })
    }

    fn finalize_attempt_facts(
        &self,
        state: &mut Self::AttemptState,
        readiness: Option<&mut Self::Readiness>,
        published_facts: Option<&ProviderClassificationFacts>,
        completion: &ProviderAttemptCompletion,
    ) -> Result<ProviderClassificationFacts, Arc<str>> {
        self.inner.finalize_attempt_facts(
            &mut state.inner,
            readiness.map(|readiness| &mut readiness.inner),
            published_facts,
            completion,
        )
    }

    fn accepted_response_head(
        &self,
        readiness: &mut Self::Readiness,
        published: &PublishedDisposition,
        accepted: &AcceptedResponseExecutionBinding,
        configs: &PinnedConfigContext<'_>,
    ) -> Result<GatewayResponseHead, Arc<str>> {
        let head = self.inner.accepted_response_head(
            &mut readiness.inner,
            published,
            accepted,
            configs,
        )?;
        if let (Some(observation), Some(tracker)) = (&readiness.observation, &readiness.tracker) {
            observation.bind_response_capture(tracker.clone());
        }
        Ok(head)
    }

    fn take_accepted_prefix(
        &self,
        readiness: &mut Self::Readiness,
    ) -> Result<Option<ProviderAcceptedEvent<Self::DecodedSseEvent>>, Arc<str>> {
        self.inner
            .take_accepted_prefix(&mut readiness.inner)
            .map(|event| event.map(wrap_accepted_event))
    }

    fn encode_accepted_event(
        &self,
        readiness: &mut Self::Readiness,
        event: ProviderAcceptedEvent<Self::DecodedSseEvent>,
        published: &PublishedDisposition,
        accepted: &AcceptedResponseExecutionBinding,
        pinned_configs: &PinnedConfigContext<'_>,
        event_configs: &ConfigEventSnapshot,
    ) -> Result<Option<AcceptedBodyFrame>, Arc<str>> {
        let prepared = match (&readiness.tracker, &event) {
            (Some(tracker), ProviderAcceptedEvent::Raw(raw)) => tracker.prepare_native(raw),
            _ => None,
        };
        let frame = self.inner.encode_accepted_event(
            &mut readiness.inner,
            unwrap_accepted_event(event),
            published,
            accepted,
            pinned_configs,
            event_configs,
        );
        match frame {
            Ok(frame) => {
                if let Some(prepared) = prepared {
                    prepared.commit();
                }
                Ok(frame)
            }
            Err(error) => {
                if let Some(prepared) = prepared {
                    prepared.cancel("canonical_capture_authoritative_encoding_failed");
                } else if let Some(tracker) = &readiness.tracker {
                    tracker
                        .cancel_unless_terminal("canonical_capture_authoritative_encoding_failed");
                }
                Err(error)
            }
        }
    }

    fn release_terminal_request(
        &self,
        logical: Self::LogicalRequest,
        published: &PublishedDisposition,
    ) -> Result<(), Arc<str>> {
        self.inner.release_terminal_request(logical, published)
    }
}

impl Drop for ObservedAttemptState {
    fn drop(&mut self) {
        if let Some(tracker) = &self.tracker {
            tracker.cancel_unless_terminal("canonical_capture_attempt_released");
        }
    }
}

impl Drop for ObservedReadiness {
    fn drop(&mut self) {
        if let Some(tracker) = &self.tracker {
            tracker.cancel_unless_terminal("canonical_capture_readiness_released");
        }
    }
}

fn wrap_accepted_event(
    event: ProviderAcceptedEvent<ProductionDecodedSse>,
) -> ProviderAcceptedEvent<ObservedDecodedSse> {
    match event {
        ProviderAcceptedEvent::Raw(event) => ProviderAcceptedEvent::Raw(event),
        ProviderAcceptedEvent::DecodedSse {
            sequence,
            decoded,
            provenance,
        } => ProviderAcceptedEvent::DecodedSse {
            sequence,
            decoded: ObservedDecodedSse { inner: decoded },
            provenance,
        },
        ProviderAcceptedEvent::Terminal {
            body,
            provenance,
            upstream_side_effects,
        } => ProviderAcceptedEvent::Terminal {
            body,
            provenance,
            upstream_side_effects,
        },
    }
}

fn unwrap_accepted_event(
    event: ProviderAcceptedEvent<ObservedDecodedSse>,
) -> ProviderAcceptedEvent<ProductionDecodedSse> {
    match event {
        ProviderAcceptedEvent::Raw(event) => ProviderAcceptedEvent::Raw(event),
        ProviderAcceptedEvent::DecodedSse {
            sequence,
            decoded,
            provenance,
        } => ProviderAcceptedEvent::DecodedSse {
            sequence,
            decoded: decoded.inner,
            provenance,
        },
        ProviderAcceptedEvent::Terminal {
            body,
            provenance,
            upstream_side_effects,
        } => ProviderAcceptedEvent::Terminal {
            body,
            provenance,
            upstream_side_effects,
        },
    }
}
