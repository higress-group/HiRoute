use serde_json::Value;

use super::RequestObservation;
use super::crypto::stable_id;
use super::request::AttemptObservation;
use super::schema::{
    ContentRefV1, OTEL_GEN_AI_SCHEMA, OtelAttributeV1, OtelEventV1, OtelGenAiRecordV1, OtelSignalV1,
};

pub const OTEL_MAPPER_VERSION: &str = "hiroute.otel-gen-ai-mapper/1";
pub const OTEL_SEMANTIC_CONVENTIONS_VERSION: &str = "1.37.0";
pub const OTEL_MAPPING_CONTRACT: &str = "hiroute.otel.gen-ai-mapping/v1|hiroute.otel-gen-ai-mapper/1|1.37.0|production_exporter:not_installed|default_content:none|explicit_content:hiroute_extension_content_ref_only";
pub const OTEL_MAPPING_DIGEST: &str =
    "sha256:d730badfd13c19d1c464c5a0c871a35a3e72a0f9c662d13d1997f066bf762e33";

/// Content is never part of the standard GenAI attributes. The only P0
/// opt-in form is a HiRoute extension event carrying an already-authorized
/// private ContentRef; no raw bytes can enter this mapper.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OtelContentPolicy {
    #[default]
    Disabled,
    ContentRefOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtelAttempt {
    pub ordinal: u32,
    pub attempt_id: String,
    pub stable_binding_id: String,
    pub provider_name: String,
    pub request_model: String,
}

impl OtelAttempt {
    pub(super) fn from_observation(attempt: &AttemptObservation) -> Self {
        Self {
            ordinal: attempt.ordinal,
            attempt_id: attempt.attempt_id.clone(),
            stable_binding_id: attempt.stable_binding_id.clone(),
            provider_name: attempt.provider_name.clone(),
            request_model: attempt.request_model.clone(),
        }
    }
}

/// Pure, allocation-only mapper used by both the local producer and golden
/// tests. It performs no network I/O and has no exporter configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OtelGenAiMapper {
    content_policy: OtelContentPolicy,
}

impl OtelGenAiMapper {
    pub const fn new(content_policy: OtelContentPolicy) -> Self {
        Self { content_policy }
    }

    pub fn server_span(
        &self,
        request: &RequestObservation,
        outcome: &str,
        error_class: Option<&str>,
    ) -> OtelSignalV1 {
        let metadata = request.metadata();
        let mut attributes = vec![
            attribute("hiroute.workspace.id", metadata.workspace_id.clone()),
            attribute("hiroute.request.id", metadata.request_id.clone()),
            attribute("hiroute.served_model.id", metadata.served_model_id.clone()),
            attribute(
                "hiroute.gateway.publication.revision",
                metadata.publication_revision,
            ),
            attribute("hiroute.observation.outcome", outcome),
        ];
        if let Some(error_class) = error_class {
            attributes.push(attribute("error.type", error_class));
        }
        OtelSignalV1::Span {
            name: "hiroute.gateway.request".into(),
            span_kind: "server".into(),
            status: span_status(outcome).into(),
            attributes,
            events: Vec::new(),
        }
    }

    pub fn attempt_span(
        &self,
        request: &RequestObservation,
        attempt: &OtelAttempt,
        outcome: &str,
        error_class: Option<&str>,
    ) -> OtelSignalV1 {
        let mut attributes = vec![
            attribute("gen_ai.operation.name", "chat"),
            attribute("gen_ai.provider.name", attempt.provider_name.clone()),
            attribute("gen_ai.request.model", attempt.request_model.clone()),
            attribute("hiroute.attempt.id", attempt.attempt_id.clone()),
            attribute("hiroute.attempt.ordinal", attempt.ordinal),
            attribute(
                "hiroute.route.stable_binding_id",
                attempt.stable_binding_id.clone(),
            ),
            attribute(
                "hiroute.served_model.id",
                request.metadata().served_model_id.clone(),
            ),
        ];
        if let Some(error_class) = error_class {
            attributes.push(attribute("error.type", error_class));
        }
        let mut detail_attributes = vec![
            attribute("gen_ai.operation.name", "chat"),
            attribute("gen_ai.provider.name", attempt.provider_name.clone()),
            attribute("gen_ai.request.model", attempt.request_model.clone()),
            attribute("hiroute.observation.outcome", outcome),
        ];
        if let Some(error_class) = error_class {
            detail_attributes.push(attribute("error.type", error_class));
        }
        OtelSignalV1::Span {
            name: format!("chat {}", attempt.request_model),
            span_kind: "client".into(),
            status: span_status(outcome).into(),
            attributes,
            events: vec![OtelEventV1 {
                name: "gen_ai.client.inference.operation.details".into(),
                attributes: detail_attributes,
            }],
        }
    }

    pub fn content_ref(&self, direction: &str, reference: ContentRefV1) -> Option<OtelSignalV1> {
        (self.content_policy == OtelContentPolicy::ContentRefOnly).then(|| {
            let name = match direction {
                "request_input" => "hiroute.gen_ai.request.content",
                _ => "hiroute.gen_ai.response.content",
            };
            OtelSignalV1::ExtensionEvent {
                name: name.into(),
                attributes: vec![
                    attribute("hiroute.content.delivery", "content_ref"),
                    attribute("hiroute.content.direction", direction),
                ],
                content_ref: reference,
            }
        })
    }
}

pub(super) fn emit_server_span(
    request: &RequestObservation,
    outcome: &str,
    error_class: Option<&str>,
) {
    emit_signal(
        request,
        "server-span",
        OtelGenAiMapper::new(request.inner.content_policy).server_span(
            request,
            outcome,
            error_class,
        ),
    );
}

pub(super) fn emit_attempt_span(
    request: &RequestObservation,
    attempt: OtelAttempt,
    outcome: &str,
    error_class: Option<&str>,
) {
    emit_signal(
        request,
        "attempt-span",
        OtelGenAiMapper::new(request.inner.content_policy).attempt_span(
            request,
            &attempt,
            outcome,
            error_class,
        ),
    );
}

pub(super) fn emit_content_ref(
    request: &RequestObservation,
    direction: &str,
    reference: ContentRefV1,
) {
    if let Some(signal) =
        OtelGenAiMapper::new(request.inner.content_policy).content_ref(direction, reference)
    {
        emit_signal(request, "content-ref", signal);
    }
}

fn emit_signal(request: &RequestObservation, kind: &'static str, signal: OtelSignalV1) {
    if !request.is_enabled() {
        return;
    }
    let correlation = request.correlation();
    let request_id = request.metadata().request_id.clone();
    let key = request.inner.key;
    request.inner.channels.otel.publish_timed(
        request.inner.latency.clone(),
        move |producer, sequence, loss| {
            let sequence_bytes = sequence.to_be_bytes();
            OtelGenAiRecordV1 {
                schema_version: OTEL_GEN_AI_SCHEMA.into(),
                schema_digest: OTEL_MAPPING_DIGEST.into(),
                mapper_version: OTEL_MAPPER_VERSION.into(),
                semantic_conventions_version: OTEL_SEMANTIC_CONVENTIONS_VERSION.into(),
                production_exporter: "not_installed".into(),
                producer,
                sequence,
                event_id: stable_id(
                    "otel-event",
                    &key,
                    b"otel-event",
                    &[request_id.as_bytes(), kind.as_bytes(), &sequence_bytes],
                ),
                correlation,
                signal,
                loss_watermark: loss,
            }
        },
    );
}

fn span_status(outcome: &str) -> &'static str {
    match outcome {
        "accepted" | "completed" | "ok" => "ok",
        _ => "error",
    }
}

fn attribute(key: &str, value: impl Into<Value>) -> OtelAttributeV1 {
    OtelAttributeV1 {
        key: key.into(),
        value: value.into(),
    }
}
