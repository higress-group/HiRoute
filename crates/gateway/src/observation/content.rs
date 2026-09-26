use bytes::Bytes;
use std::sync::Arc;

use crate::content_ref::ContentRef;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1, ModelUsage};
use hiroute_gateway_core::transport::GatewayResponseHead;

use super::contracts::CONVERSATION_CONTENT_PORT_DIGEST;
use super::crypto::{WorkspaceHmac, stable_id};
use super::otel::emit_content_ref;
use super::request::AttemptObservation;
use super::schema::{CONVERSATION_CONTENT_SCHEMA, ContentRefV1, ConversationContentEnvelopeV1};
use super::{RequestObservation, unix_nanos};
use hiroute_diagnostics::event::{
    CaptureAbortReason, CaptureOutcome, ContentCaptureEnd, ContentRole, DiagnosticEvent,
};

const CONTENT_CHUNK_BYTES: usize = 64 * 1024;
const CANONICALIZATION_VERSION: &str = "hiroute.model-request-ir/v1";
const CANONICAL_RESPONSE_EVENT_MEDIA_TYPE: &str =
    "application/vnd.hiroute.model-stream-event+json;version=1";

#[path = "content/request.rs"]
mod request;

#[derive(Default)]
pub(super) struct CanonicalFrameDelivery {
    pub(super) events: Vec<ModelStreamEventV1>,
    pub(super) usage: Option<ModelUsage>,
}

impl CanonicalFrameDelivery {
    pub(super) fn merge_usage(&mut self, usage: &ModelUsage) {
        self.usage
            .get_or_insert_with(ModelUsage::default)
            .merge_from(usage);
    }

    pub(super) fn merge(&mut self, mut other: Self) {
        self.events.append(&mut other.events);
        if let Some(usage) = other.usage {
            self.merge_usage(&usage);
        }
    }
}

impl RequestObservation {
    pub(super) fn begin_content(
        &self,
        direction: &'static str,
        attempt: Option<&AttemptObservation>,
    ) {
        if !self.captures_content() {
            return;
        }
        let (should_emit, fork_id, parent_transcript_root) = {
            let mut state = self.lock_state();
            let started = match direction {
                "request_input" => &mut state.request_content_started,
                _ => &mut state.response_content_started,
            };
            if *started {
                return;
            }
            *started = true;
            let fork_id = self.fork_id(direction, attempt);
            let parent_transcript_root = (direction == "response_delivered")
                .then(|| state.request_result_transcript_root.clone())
                .flatten();
            let mut transcript = WorkspaceHmac::new(&self.inner.key, b"transcript-root");
            transcript.update(CANONICALIZATION_VERSION.as_bytes());
            transcript.update(
                parent_transcript_root
                    .as_deref()
                    .unwrap_or("no-parent-root")
                    .as_bytes(),
            );
            transcript.update(b"append");
            transcript.update(self.metadata().request_id.as_bytes());
            transcript.update(direction.as_bytes());
            transcript.update(fork_id.as_bytes());
            match direction {
                "request_input" => state.request_transcript = Some(transcript),
                _ => state.response_transcript = Some(transcript),
            }
            (true, fork_id, parent_transcript_root)
        };
        if should_emit {
            self.emit_content(ContentEvent {
                direction,
                phase: "begin",
                attempt_id: attempt.map(|value| value.attempt_id.clone()),
                fork_id,
                parent_transcript_root,
                result_transcript_root: None,
                message_instance_id: None,
                message_role: None,
                content_kind: None,
                content_id: None,
                content_blob_digest: None,
                message_ordinal: None,
                part_ordinal: None,
                chunk_ordinal: None,
                transport_frame_id: None,
                canonical_media_type: None,
                canonical_bytes_base64: None,
                content_ref: None,
                downstream_delivery: None,
                abort_reason: None,
                completeness_delta: None,
            });
        }
    }

    fn append_content_chunk(&self, chunk: ContentChunk<'_>) {
        if !self.captures_content() {
            return;
        }
        let message_instance_id =
            self.message_instance_id(chunk.direction, chunk.message_ordinal, chunk.role);
        let content_id =
            self.content_id(&message_instance_id, chunk.part_ordinal, chunk.whole_digest);
        let parent_transcript_root = self.content_parent_root(chunk.direction);
        let (chunk_ordinal, attempt_id, fork_id) = {
            let mut state = self.lock_state();
            let current = if chunk.direction == "request_input" {
                if state.request_content_terminal {
                    return;
                }
                let current = state.request_content_ordinal;
                state.request_content_ordinal = current.saturating_add(1);
                if chunk.starts_content
                    && let Some(transcript) = &mut state.request_transcript
                {
                    update_transcript(transcript, &message_instance_id, &content_id);
                }
                current
            } else {
                if state.response_content_terminal {
                    return;
                }
                let current = state.response_content_ordinal;
                state.response_content_ordinal = current.saturating_add(1);
                if chunk.starts_content
                    && let Some(transcript) = &mut state.response_transcript
                {
                    update_transcript(transcript, &message_instance_id, &content_id);
                }
                current
            };
            let stats = if chunk.direction == "request_input" {
                &mut state.request_content_stats
            } else {
                &mut state.response_content_stats
            };
            stats.note_chunk(chunk.bytes.len());
            (
                current,
                chunk.attempt.map(|attempt| attempt.attempt_id.clone()),
                self.fork_id(chunk.direction, chunk.attempt),
            )
        };
        self.emit_content(ContentEvent {
            direction: chunk.direction,
            phase: "append",
            attempt_id,
            fork_id,
            parent_transcript_root,
            result_transcript_root: None,
            message_instance_id: Some(message_instance_id),
            message_role: Some(chunk.role.into()),
            content_kind: Some(chunk.kind.into()),
            content_id: Some(content_id),
            content_blob_digest: Some(chunk.whole_digest.into()),
            message_ordinal: Some(chunk.message_ordinal),
            part_ordinal: Some(chunk.part_ordinal),
            chunk_ordinal: Some(chunk_ordinal),
            transport_frame_id: chunk.transport_frame_id.map(Into::into),
            canonical_media_type: Some(chunk.media_type.into()),
            canonical_bytes_base64: Some(base64_encode(chunk.bytes)),
            content_ref: chunk.public_ref,
            downstream_delivery: chunk.downstream_delivery.map(Into::into),
            abort_reason: None,
            completeness_delta: None,
        });
    }

    pub(super) fn terminal_content(
        &self,
        direction: &'static str,
        phase: &'static str,
        abort_reason: Option<&str>,
    ) {
        if !self.captures_content() {
            return;
        }
        let (started, already_terminal, transcript, attempt) = {
            let mut state = self.lock_state();
            match direction {
                "request_input" => {
                    let started = state.request_content_started;
                    let terminal = state.request_content_terminal;
                    if started && !terminal {
                        state.request_content_terminal = true;
                    }
                    (started, terminal, state.request_transcript.take(), None)
                }
                _ => {
                    let started = state.response_content_started;
                    let terminal = state.response_content_terminal;
                    if started && !terminal {
                        state.response_content_terminal = true;
                    }
                    (
                        started,
                        terminal,
                        state.response_transcript.take(),
                        state.accepted_attempt.clone(),
                    )
                }
            }
        };
        if !started || already_terminal {
            return;
        }
        let stats = self.take_content_stats(direction);
        self.emit_diagnostic(DiagnosticEvent::ContentCaptureEnd(ContentCaptureEnd {
            role: match direction {
                "request_input" => ContentRole::Request,
                _ => ContentRole::Response,
            },
            outcome: if phase == "finish" {
                CaptureOutcome::Success
            } else {
                CaptureOutcome::Abort
            },
            bytes: stats.bytes,
            chunks: stats.chunks,
            read_calls: stats.read_calls,
            short_reads: stats.short_reads,
            max_chunk_bytes: stats.max_chunk_bytes,
            abort: abort_reason.map(capture_abort_reason),
        }));
        let result_transcript_root = transcript.map(|transcript| {
            format!(
                "transcript-{}",
                transcript.finish().trim_start_matches("sha256:")
            )
        });
        if direction == "request_input" {
            self.lock_state().request_result_transcript_root = result_transcript_root.clone();
        }
        self.emit_content(ContentEvent {
            direction,
            phase,
            attempt_id: attempt.as_ref().map(|value| value.attempt_id.clone()),
            fork_id: self.fork_id(direction, attempt.as_ref()),
            parent_transcript_root: self.content_parent_root(direction),
            result_transcript_root,
            message_instance_id: None,
            message_role: None,
            content_kind: None,
            content_id: None,
            content_blob_digest: None,
            message_ordinal: None,
            part_ordinal: None,
            chunk_ordinal: None,
            transport_frame_id: None,
            canonical_media_type: None,
            canonical_bytes_base64: None,
            content_ref: None,
            downstream_delivery: (direction == "response_delivered")
                .then(|| "full_frame_transport_accepted".into()),
            abort_reason: abort_reason.map(Into::into),
            completeness_delta: Some(if phase == "finish" {
                "complete".into()
            } else {
                "partial".into()
            }),
        });
    }

    fn content_blob_digest(&self, media_type: &str, bytes: &[u8]) -> String {
        let mut digest = WorkspaceHmac::new(&self.inner.key, b"conversation-content-blob");
        digest.update(CANONICALIZATION_VERSION.as_bytes());
        digest.update(media_type.as_bytes());
        digest.update_stream(bytes);
        format!("blob-{}", digest.finish().trim_start_matches("sha256:"))
    }

    fn message_instance_id(&self, direction: &str, message_ordinal: u32, role: &str) -> String {
        let parent_transcript_root = self.content_parent_root(direction);
        stable_id(
            "message",
            &self.inner.key,
            b"conversation-message-instance",
            &[
                self.metadata().conversation_id.as_bytes(),
                parent_transcript_root
                    .as_deref()
                    .unwrap_or("no-parent-root")
                    .as_bytes(),
                self.metadata().request_id.as_bytes(),
                direction.as_bytes(),
                &message_ordinal.to_be_bytes(),
                role.as_bytes(),
            ],
        )
    }

    fn content_parent_root(&self, direction: &str) -> Option<String> {
        (direction == "response_delivered")
            .then(|| self.lock_state().request_result_transcript_root.clone())
            .flatten()
    }

    fn content_id(
        &self,
        message_instance_id: &str,
        part_ordinal: u32,
        content_blob_digest: &str,
    ) -> String {
        stable_id(
            "content",
            &self.inner.key,
            b"conversation-content-id",
            &[
                message_instance_id.as_bytes(),
                &part_ordinal.to_be_bytes(),
                content_blob_digest.as_bytes(),
            ],
        )
    }

    fn public_content_ref(
        &self,
        reference: &ContentRef,
        media_type: &str,
        content_id: String,
        digest: String,
    ) -> ContentRefV1 {
        ContentRefV1 {
            content_id,
            digest,
            byte_count: reference.byte_len(),
            media_type: media_type.into(),
        }
    }

    fn fork_id(&self, direction: &str, attempt: Option<&AttemptObservation>) -> String {
        stable_id(
            "fork",
            &self.inner.key,
            b"conversation-fork",
            &[
                self.metadata().request_id.as_bytes(),
                direction.as_bytes(),
                attempt.map_or(b"request".as_slice(), |value| value.attempt_id.as_bytes()),
            ],
        )
    }

    fn emit_content(&self, event: ContentEvent) {
        let correlation = self.correlation();
        let request_id = self.metadata().request_id.clone();
        let key = self.inner.key;
        let event_kind = event.phase;
        self.inner
            .channels
            .content
            .publish(move |producer, sequence, loss| {
                let sequence_bytes = sequence.to_be_bytes();
                ConversationContentEnvelopeV1 {
                    schema_version: CONVERSATION_CONTENT_SCHEMA.into(),
                    schema_digest: CONVERSATION_CONTENT_PORT_DIGEST.into(),
                    channel: "conversation_content".into(),
                    producer,
                    sequence,
                    event_id: stable_id(
                        "content-event",
                        &key,
                        b"conversation-content-event",
                        &[
                            request_id.as_bytes(),
                            event.direction.as_bytes(),
                            event_kind.as_bytes(),
                            &sequence_bytes,
                        ],
                    ),
                    correlation,
                    direction: event.direction.into(),
                    phase: event.phase.into(),
                    attempt_id: event.attempt_id,
                    fork_id: event.fork_id,
                    parent_transcript_root: event.parent_transcript_root,
                    result_transcript_root: event.result_transcript_root,
                    message_instance_id: event.message_instance_id,
                    message_role: event.message_role,
                    content_kind: event.content_kind,
                    content_id: event.content_id,
                    content_blob_digest: event.content_blob_digest,
                    message_ordinal: event.message_ordinal,
                    part_ordinal: event.part_ordinal,
                    chunk_ordinal: event.chunk_ordinal,
                    transport_frame_id: event.transport_frame_id,
                    canonical_media_type: event.canonical_media_type,
                    canonical_bytes_base64: event.canonical_bytes_base64,
                    content_ref: event.content_ref,
                    downstream_delivery: event.downstream_delivery,
                    abort_reason: event.abort_reason,
                    occurred_at_unix_nanos: unix_nanos(),
                    completeness_delta: loss
                        .as_ref()
                        .map(|_| "partial".into())
                        .or(event.completeness_delta),
                    loss_watermark: loss,
                }
            });
    }
}

pub struct AcceptedResponseCapture {
    request: RequestObservation,
    attempt: Option<Arc<AttemptObservation>>,
    ended: bool,
}

impl AcceptedResponseCapture {
    pub fn new(request: RequestObservation) -> Self {
        Self {
            request,
            attempt: None,
            ended: false,
        }
    }

    pub fn accepted_head(&mut self, _head: &GatewayResponseHead) {}

    /// Must be called only after the inner transport accepted the complete
    /// body frame. The caller retains ownership of this sequencing rule.
    pub fn accepted_frame(&mut self, body: &Bytes, end_stream: bool) {
        if !self.request.is_enabled() {
            return;
        }
        if !body.is_empty() || end_stream {
            let frame_ordinal = self.request.lock_state().response_frame_ordinal;
            let frame_id = stable_id(
                "frame",
                &self.request.inner.key,
                b"downstream-accepted-frame",
                &[
                    self.request.metadata().request_id.as_bytes(),
                    &frame_ordinal.to_be_bytes(),
                    body,
                ],
            );
            if let Some(attempt) = self.request.accept_current(&frame_id, body.len()) {
                let capture_attempt = self
                    .attempt
                    .get_or_insert_with(|| Arc::new(attempt.clone()))
                    .clone();
                let result = self
                    .request
                    .response_capture()
                    .ok_or("canonical_capture_not_bound")
                    .and_then(|capture| {
                        capture.accepted_frame(
                            self.request.clone(),
                            capture_attempt,
                            frame_id,
                            body,
                            end_stream,
                        )
                    });
                if let Err(reason) = result {
                    self.request
                        .begin_content("response_delivered", Some(&attempt));
                    self.request
                        .terminal_content("response_delivered", "abort", Some(reason));
                }
            }
        }
        if end_stream {
            self.ended = true;
            self.request.clear_response_capture();
        }
    }
}

impl RequestObservation {
    pub(super) fn emit_canonical_response_delivery(
        &self,
        attempt: &AttemptObservation,
        frame_id: &str,
        delivery: CanonicalFrameDelivery,
    ) {
        if let Some((store, ticket)) = self.inner.agent_turn_output.get() {
            store.accept_output(ticket, &delivery.events);
        }
        if self.captures_content() {
            for event in delivery.events {
                let Some(kind) = canonical_response_content_kind(&event.event) else {
                    continue;
                };
                let Ok(bytes) = serialize_observed_response_event(&event) else {
                    self.terminal_content(
                        "response_delivered",
                        "abort",
                        Some("canonical_response_serialization_failed"),
                    );
                    return;
                };
                let part_ordinal = {
                    let mut state = self.lock_state();
                    let ordinal = state.response_part_ordinal;
                    state.response_part_ordinal = ordinal.saturating_add(1);
                    ordinal
                };
                let whole_digest =
                    self.content_blob_digest(CANONICAL_RESPONSE_EVENT_MEDIA_TYPE, &bytes);
                let message_instance_id =
                    self.message_instance_id("response_delivered", 0, "assistant");
                let content_ref = ContentRefV1 {
                    content_id: self.content_id(&message_instance_id, part_ordinal, &whole_digest),
                    digest: whole_digest.clone(),
                    byte_count: bytes.len() as u64,
                    media_type: CANONICAL_RESPONSE_EVENT_MEDIA_TYPE.into(),
                };
                emit_content_ref(self, "response_delivered", content_ref.clone());
                for (index, chunk) in bytes.chunks(CONTENT_CHUNK_BYTES).enumerate() {
                    self.append_content_chunk(ContentChunk {
                        direction: "response_delivered",
                        attempt: Some(attempt),
                        message_ordinal: 0,
                        part_ordinal,
                        role: "assistant",
                        kind,
                        media_type: CANONICAL_RESPONSE_EVENT_MEDIA_TYPE,
                        bytes: chunk,
                        public_ref: Some(content_ref.clone()),
                        whole_digest: &whole_digest,
                        starts_content: index == 0,
                        transport_frame_id: Some(frame_id),
                        downstream_delivery: Some("full_frame_transport_accepted"),
                    });
                }
            }
        }
        if let Some(usage) = delivery.usage {
            self.usage_model(&usage);
        }
    }
}

/// Projects execution-internal Tool identity onto the product-visible
/// canonical event. Native IDs and physical ownership remain available to the
/// runtime ledger but never cross the conversation-content port.
fn serialize_observed_response_event(
    event: &ModelStreamEventV1,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut projected = serde_json::to_value(event)?;
    if matches!(
        event.event,
        ModelEvent::ToolCallStarted { .. } | ModelEvent::WebSearch { .. }
    ) && let Some(fields) = projected
        .get_mut("event")
        .and_then(serde_json::Value::as_object_mut)
    {
        fields.remove("native_id");
        fields.remove("owner");
        fields.remove("item_id");
    }
    serde_json::to_vec(&projected)
}

fn canonical_response_content_kind(event: &ModelEvent) -> Option<&'static str> {
    match event {
        ModelEvent::TextAnnotation { .. } => Some("text_annotation"),
        ModelEvent::TextFinished { .. } => Some("text_finished"),
        ModelEvent::ReasoningFinished { .. } => Some("reasoning_finished"),
        ModelEvent::RefusalFinished { .. } => Some("refusal_finished"),
        ModelEvent::WebSearch { .. } => Some("web_search"),
        ModelEvent::ContentBlockStarted { .. } => Some("content_block_started"),
        ModelEvent::TextDelta { .. } => Some("text_delta"),
        ModelEvent::ReasoningDelta { .. } => Some("reasoning_delta"),
        ModelEvent::RefusalDelta { .. } => Some("refusal_delta"),
        ModelEvent::ToolCallStarted { .. } => Some("tool_call_started"),
        ModelEvent::ToolArgumentsDelta { .. } => Some("tool_arguments_delta"),
        ModelEvent::ToolCallFinished { .. } => Some("tool_call_finished"),
        ModelEvent::ResponseStarted { .. }
        | ModelEvent::UsageUpdated { .. }
        | ModelEvent::FinishReason { .. }
        | ModelEvent::ResponseCompleted { .. }
        | ModelEvent::ResponseFailed { .. }
        | ModelEvent::ProviderState { .. }
        | ModelEvent::ResponsesMetadata { .. } => None,
    }
}

impl Drop for AcceptedResponseCapture {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        if let Some(capture) = self.request.response_capture() {
            capture.cancel_unless_terminal("downstream_transport_incomplete");
        }
        if let Some(attempt) = &self.attempt {
            self.request
                .begin_content("response_delivered", Some(attempt));
        }
        self.request.terminal_content(
            "response_delivered",
            "abort",
            Some("downstream_transport_incomplete"),
        );
    }
}

struct ContentChunk<'a> {
    direction: &'static str,
    attempt: Option<&'a AttemptObservation>,
    message_ordinal: u32,
    part_ordinal: u32,
    role: &'a str,
    kind: &'a str,
    media_type: &'a str,
    bytes: &'a [u8],
    public_ref: Option<ContentRefV1>,
    whole_digest: &'a str,
    starts_content: bool,
    transport_frame_id: Option<&'a str>,
    downstream_delivery: Option<&'a str>,
}

struct ContentEvent {
    direction: &'static str,
    phase: &'static str,
    attempt_id: Option<String>,
    fork_id: String,
    parent_transcript_root: Option<String>,
    result_transcript_root: Option<String>,
    message_instance_id: Option<String>,
    message_role: Option<String>,
    content_kind: Option<String>,
    content_id: Option<String>,
    content_blob_digest: Option<String>,
    message_ordinal: Option<u32>,
    part_ordinal: Option<u32>,
    chunk_ordinal: Option<u32>,
    transport_frame_id: Option<String>,
    canonical_media_type: Option<String>,
    canonical_bytes_base64: Option<String>,
    content_ref: Option<ContentRefV1>,
    downstream_delivery: Option<String>,
    abort_reason: Option<String>,
    completeness_delta: Option<String>,
}

fn update_transcript(transcript: &mut WorkspaceHmac, message_instance_id: &str, content_id: &str) {
    transcript.update(message_instance_id.as_bytes());
    transcript.update(content_id.as_bytes());
}

fn capture_abort_reason(reason: &str) -> CaptureAbortReason {
    match reason {
        "canonical_content_unavailable" => CaptureAbortReason::CanonicalContentUnavailable,
        "canonical_capture_not_bound" => CaptureAbortReason::CanonicalCaptureNotBound,
        "canonical_capture_state_unavailable" => {
            CaptureAbortReason::CanonicalCaptureStateUnavailable
        }
        "canonical_response_serialization_failed" => {
            CaptureAbortReason::ResponseSerializationFailed
        }
        "canonical_response_wire_mismatch"
        | "canonical_response_wire_truncated"
        | "canonical_response_unexpected_wire_bytes"
        | "canonical_response_decode_failed" => CaptureAbortReason::IntegrityFailure,
        _ => CaptureAbortReason::Other,
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(ALPHABET[(first >> 2) as usize] as char);
        output.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::server::core_runtime::model_ir::{ExactProviderPathV1, OpaqueProviderState};
    use crate::server::request_plan::IngressProtocol;

    use super::*;

    #[test]
    fn provider_hidden_state_is_not_conversation_response_content() {
        let event = ModelEvent::ProviderState {
            state: Box::new(OpaqueProviderState {
                owner: ExactProviderPathV1 {
                    provider_id: "provider".into(),
                    endpoint_id: "endpoint".into(),
                    entitlement_id: "entitlement".into(),
                    connector_id: "connector".into(),
                    connector_revision: "1".into(),
                    capability_id: "capability".into(),
                    capability_revision: "1".into(),
                    model_configuration_id: "model-config".into(),
                    native_model: "native".into(),
                    upstream_protocol: IngressProtocol::Responses,
                    adapter_revision: "adapter/v1".into(),
                    serializer_revision: "serializer/v1".into(),
                    decoder_revision: "decoder/v1".into(),
                },
                block_index: Some(0),
                kind: "encrypted_content".into(),
                value: json!("provider-hidden-secret"),
                messages_thinking: None,
            }),
        };
        assert_eq!(canonical_response_content_kind(&event), None);
    }
}
