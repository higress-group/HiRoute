use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::WorkspaceId;

use super::{
    AttemptId, ContentBlobDigest, ContentId, CorrelationProvenance, EventId,
    ExecutionFactEnvelopeV1, LogicalRequestId, LossNoticeV1, MessageInstanceId, ObservationChannel,
    ObservationScopeV1, ObservationStreamV1, SequenceRangeV1, SessionId, SessionScopeV1,
    TranscriptRoot, TurnId,
};

pub const CONVERSATION_CONTENT_SCHEMA_V2: &str =
    "hiroute.observation.conversation-content-envelope/v2";
pub const CONVERSATION_CONTENT_PORT_DIGEST_V2: &str =
    "sha256:6f17cd772a0fb322d25dd31dc95e34325cbabbd68912b32f5bb9bed526adfe44";
pub const OBSERVATION_GAP_HEARTBEAT_SCHEMA_V1: &str = "hiroute.observation.gap-heartbeat/v1";
pub const CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1: &str =
    "application/vnd.hiroute.model-stream-event+json;version=1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationContentChannelV2 {
    ConversationContent,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationContentDirectionV2 {
    RequestInput,
    ResponseDelivered,
}

impl ConversationContentDirectionV2 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RequestInput => "request_input",
            Self::ResponseDelivered => "response_delivered",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationContentPhaseV2 {
    Begin,
    Append,
    Finish,
    Abort,
}

impl ConversationContentPhaseV2 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Begin => "begin",
            Self::Append => "append",
            Self::Finish => "finish",
            Self::Abort => "abort",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentCompletenessDeltaV2 {
    Complete,
    Partial,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentDownstreamDeliveryV2 {
    FullFrameTransportAccepted,
    PartialFrameNotMaterialized,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationProducerV2 {
    pub component: String,
    pub revision: String,
    #[serde(flatten)]
    pub stream: ObservationStreamV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentCorrelationV2 {
    pub workspace_id: WorkspaceId,
    pub conversation_id: SessionId,
    pub session_scope: SessionScopeV1,
    pub correlation_provenance: CorrelationProvenance,
    pub turn_id: TurnId,
    pub request_id: LogicalRequestId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentLossWatermarkV2 {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub reason: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentRefV2 {
    pub content_id: ContentId,
    pub digest: ContentBlobDigest,
    pub byte_count: u64,
    pub media_type: String,
}

/// First Product representation of the frozen Gateway conversation-content/v2 wire contract.
/// Optional fields mirror the sender one-for-one so an adapter never joins events.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationContentEnvelopeV1 {
    pub schema_version: String,
    pub schema_digest: String,
    pub channel: ConversationContentChannelV2,
    pub producer: ObservationProducerV2,
    pub sequence: u64,
    pub event_id: EventId,
    pub correlation: ContentCorrelationV2,
    pub direction: ConversationContentDirectionV2,
    pub phase: ConversationContentPhaseV2,
    pub attempt_id: Option<AttemptId>,
    pub fork_id: String,
    pub parent_transcript_root: Option<TranscriptRoot>,
    pub result_transcript_root: Option<TranscriptRoot>,
    pub message_instance_id: Option<MessageInstanceId>,
    pub message_role: Option<String>,
    pub content_kind: Option<String>,
    pub content_id: Option<ContentId>,
    pub content_blob_digest: Option<ContentBlobDigest>,
    pub message_ordinal: Option<u32>,
    pub part_ordinal: Option<u32>,
    pub chunk_ordinal: Option<u32>,
    pub transport_frame_id: Option<String>,
    pub canonical_media_type: Option<String>,
    pub canonical_bytes_base64: Option<String>,
    pub content_ref: Option<ContentRefV2>,
    pub downstream_delivery: Option<ContentDownstreamDeliveryV2>,
    pub abort_reason: Option<String>,
    pub occurred_at_unix_nanos: u64,
    pub loss_watermark: Option<ContentLossWatermarkV2>,
    pub completeness_delta: Option<ContentCompletenessDeltaV2>,
}

impl ConversationContentEnvelopeV1 {
    pub fn validate(&self) -> Result<(), ObservationEnvelopeError> {
        if self.schema_version != CONVERSATION_CONTENT_SCHEMA_V2
            || self.schema_digest != CONVERSATION_CONTENT_PORT_DIGEST_V2
        {
            return Err(ObservationEnvelopeError::UnsupportedSchema);
        }
        if self.sequence == 0 || self.occurred_at_unix_nanos == 0 {
            return Err(ObservationEnvelopeError::InvalidSequence);
        }
        if !portable_text(&self.producer.component)
            || !portable_text(&self.producer.revision)
            || !portable_text(&self.fork_id)
        {
            return Err(ObservationEnvelopeError::InvalidPayload);
        }
        if let Some(loss) = &self.loss_watermark
            && (loss.first_sequence == 0
                || loss.first_sequence > loss.last_sequence
                || loss.last_sequence >= self.sequence
                || loss.reason.trim().is_empty())
        {
            return Err(ObservationEnvelopeError::InvalidLossWatermark);
        }
        let expected_completeness = match (self.phase, self.loss_watermark.is_some()) {
            (ConversationContentPhaseV2::Begin | ConversationContentPhaseV2::Append, false) => None,
            (ConversationContentPhaseV2::Begin | ConversationContentPhaseV2::Append, true)
            | (ConversationContentPhaseV2::Finish, true)
            | (ConversationContentPhaseV2::Abort, _) => Some(ContentCompletenessDeltaV2::Partial),
            (ConversationContentPhaseV2::Finish, false) => {
                Some(ContentCompletenessDeltaV2::Complete)
            }
        };
        if self.completeness_delta != expected_completeness {
            return Err(ObservationEnvelopeError::InvalidLossWatermark);
        }
        match self.phase {
            ConversationContentPhaseV2::Begin => self.validate_begin(),
            ConversationContentPhaseV2::Append => self.validate_append(),
            ConversationContentPhaseV2::Finish => self.validate_finish(),
            ConversationContentPhaseV2::Abort => self.validate_abort(),
        }
    }

    fn validate_begin(&self) -> Result<(), ObservationEnvelopeError> {
        if self.result_transcript_root.is_some()
            || self.has_content_coordinates()
            || self.canonical_bytes_base64.is_some()
            || self.content_ref.is_some()
            || self.downstream_delivery.is_some()
            || self.abort_reason.is_some()
        {
            return Err(ObservationEnvelopeError::InvalidPayload);
        }
        match self.direction {
            ConversationContentDirectionV2::RequestInput if self.attempt_id.is_none() => Ok(()),
            ConversationContentDirectionV2::ResponseDelivered
                if self.attempt_id.is_some() && self.parent_transcript_root.is_some() =>
            {
                Ok(())
            }
            _ => Err(ObservationEnvelopeError::InvalidPayload),
        }
    }

    fn validate_append(&self) -> Result<(), ObservationEnvelopeError> {
        let required = self.message_instance_id.is_some()
            && nonempty(self.message_role.as_deref())
            && nonempty(self.content_kind.as_deref())
            && self.content_id.is_some()
            && self.content_blob_digest.is_some()
            && self.message_ordinal.is_some()
            && self.part_ordinal.is_some()
            && self.chunk_ordinal.is_some()
            && nonempty(self.canonical_media_type.as_deref())
            && self.canonical_bytes_base64.is_some();
        if !required || self.result_transcript_root.is_some() || self.abort_reason.is_some() {
            return Err(ObservationEnvelopeError::InvalidPayload);
        }
        if let Some(reference) = &self.content_ref
            && (Some(&reference.content_id) != self.content_id.as_ref()
                || Some(&reference.digest) != self.content_blob_digest.as_ref()
                || Some(reference.media_type.as_str()) != self.canonical_media_type.as_deref())
        {
            return Err(ObservationEnvelopeError::InvalidPayload);
        }
        match self.direction {
            ConversationContentDirectionV2::RequestInput
                if self.attempt_id.is_none()
                    && self.transport_frame_id.is_none()
                    && self.downstream_delivery.is_none() =>
            {
                Ok(())
            }
            ConversationContentDirectionV2::ResponseDelivered
                if self.attempt_id.is_some()
                    && nonempty(self.transport_frame_id.as_deref())
                    && self.downstream_delivery
                        == Some(ContentDownstreamDeliveryV2::FullFrameTransportAccepted)
                    && self.content_ref.is_some()
                    && self.canonical_media_type.as_deref()
                        == Some(CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1) =>
            {
                Ok(())
            }
            _ => Err(ObservationEnvelopeError::InvalidPayload),
        }
    }

    fn validate_finish(&self) -> Result<(), ObservationEnvelopeError> {
        if self.result_transcript_root.is_none()
            || self.has_content_coordinates()
            || self.canonical_bytes_base64.is_some()
            || self.content_ref.is_some()
            || self.abort_reason.is_some()
        {
            return Err(ObservationEnvelopeError::InvalidPayload);
        }
        self.validate_terminal_delivery()
    }

    fn validate_abort(&self) -> Result<(), ObservationEnvelopeError> {
        if self.result_transcript_root.is_none()
            || self.has_content_coordinates()
            || self.canonical_bytes_base64.is_some()
            || self.content_ref.is_some()
            || !nonempty(self.abort_reason.as_deref())
        {
            return Err(ObservationEnvelopeError::InvalidPayload);
        }
        self.validate_terminal_delivery()
    }

    fn validate_terminal_delivery(&self) -> Result<(), ObservationEnvelopeError> {
        match self.direction {
            ConversationContentDirectionV2::RequestInput
                if self.attempt_id.is_none() && self.downstream_delivery.is_none() =>
            {
                Ok(())
            }
            ConversationContentDirectionV2::ResponseDelivered
                if self.attempt_id.is_some()
                    && self.downstream_delivery
                        == Some(ContentDownstreamDeliveryV2::FullFrameTransportAccepted) =>
            {
                Ok(())
            }
            _ => Err(ObservationEnvelopeError::InvalidPayload),
        }
    }

    fn has_content_coordinates(&self) -> bool {
        self.message_instance_id.is_some()
            || self.message_role.is_some()
            || self.content_kind.is_some()
            || self.content_id.is_some()
            || self.content_blob_digest.is_some()
            || self.message_ordinal.is_some()
            || self.part_ordinal.is_some()
            || self.chunk_ordinal.is_some()
            || self.transport_frame_id.is_some()
            || self.canonical_media_type.is_some()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationGapHeartbeatV1 {
    pub schema_version: String,
    pub channel: String,
    pub producer: ObservationProducerV2,
    pub sequence: u64,
    pub event_id: EventId,
    pub loss_watermark: ContentLossWatermarkV2,
    pub completeness_delta: ContentCompletenessDeltaV2,
}

impl ObservationGapHeartbeatV1 {
    pub fn validate(&self) -> Result<(), ObservationEnvelopeError> {
        if self.schema_version != OBSERVATION_GAP_HEARTBEAT_SCHEMA_V1 {
            return Err(ObservationEnvelopeError::UnsupportedSchema);
        }
        if self.sequence == 0
            || self.channel.trim().is_empty()
            || self.loss_watermark.first_sequence == 0
            || self.loss_watermark.first_sequence > self.loss_watermark.last_sequence
            || self.loss_watermark.last_sequence != self.sequence
            || self.loss_watermark.reason.trim().is_empty()
            || self.completeness_delta != ContentCompletenessDeltaV2::Partial
            || !portable_text(&self.producer.component)
            || !portable_text(&self.producer.revision)
        {
            return Err(ObservationEnvelopeError::InvalidLossWatermark);
        }
        Ok(())
    }
}

pub trait SequencedObservationEnvelope: Clone + Serialize + Send + 'static {
    fn channel(&self) -> ObservationChannel;
    fn stream(&self) -> &ObservationStreamV1;
    fn sequence(&self) -> u64;
    fn event_id(&self) -> &EventId;
    fn scope(&self) -> ObservationScopeV1;
    fn reported_losses(&self) -> Vec<LossNoticeV1>;
}

impl SequencedObservationEnvelope for ExecutionFactEnvelopeV1 {
    fn channel(&self) -> ObservationChannel {
        ObservationChannel::Fact
    }
    fn stream(&self) -> &ObservationStreamV1 {
        &self.producer.stream
    }
    fn sequence(&self) -> u64 {
        self.sequence
    }
    fn event_id(&self) -> &EventId {
        &self.event_id
    }
    fn scope(&self) -> ObservationScopeV1 {
        ObservationScopeV1 {
            workspace_id: self.correlation.workspace_id.clone(),
            session_id: Some(self.correlation.conversation_id.clone()),
        }
    }
    fn reported_losses(&self) -> Vec<LossNoticeV1> {
        self.loss_watermark
            .as_ref()
            .map(|loss| LossNoticeV1 {
                range: SequenceRangeV1 {
                    first: loss.first_sequence,
                    last: loss.last_sequence,
                },
                scope: self.scope(),
            })
            .into_iter()
            .collect()
    }
}

impl SequencedObservationEnvelope for ConversationContentEnvelopeV1 {
    fn channel(&self) -> ObservationChannel {
        ObservationChannel::Content
    }
    fn stream(&self) -> &ObservationStreamV1 {
        &self.producer.stream
    }
    fn sequence(&self) -> u64 {
        self.sequence
    }
    fn event_id(&self) -> &EventId {
        &self.event_id
    }
    fn scope(&self) -> ObservationScopeV1 {
        ObservationScopeV1 {
            workspace_id: self.correlation.workspace_id.clone(),
            session_id: Some(self.correlation.conversation_id.clone()),
        }
    }
    fn reported_losses(&self) -> Vec<LossNoticeV1> {
        self.loss_watermark
            .as_ref()
            .map(|loss| LossNoticeV1 {
                range: SequenceRangeV1 {
                    first: loss.first_sequence,
                    last: loss.last_sequence,
                },
                scope: self.scope(),
            })
            .into_iter()
            .collect()
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ObservationEnvelopeError {
    #[error("observation input schema or digest is unsupported")]
    UnsupportedSchema,
    #[error("observation envelope payload is invalid")]
    InvalidPayload,
    #[error("observation sequence or timestamp is invalid")]
    InvalidSequence,
    #[error("observation loss watermark is invalid")]
    InvalidLossWatermark,
}

fn nonempty(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn portable_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(phase: ConversationContentPhaseV2) -> ConversationContentEnvelopeV1 {
        ConversationContentEnvelopeV1 {
            schema_version: CONVERSATION_CONTENT_SCHEMA_V2.into(),
            schema_digest: CONVERSATION_CONTENT_PORT_DIGEST_V2.into(),
            channel: ConversationContentChannelV2::ConversationContent,
            producer: ObservationProducerV2 {
                component: "gateway-content".into(),
                revision: "g-star".into(),
                stream: ObservationStreamV1 {
                    producer_id: super::super::ProducerId::parse("producer").unwrap(),
                    producer_epoch: super::super::ProducerEpoch::parse("epoch").unwrap(),
                    stream_id: super::super::StreamId::parse("stream").unwrap(),
                },
            },
            sequence: 1,
            event_id: EventId::parse("event-1").unwrap(),
            correlation: ContentCorrelationV2 {
                workspace_id: WorkspaceId::default(),
                conversation_id: SessionId::parse("conversation").unwrap(),
                session_scope: SessionScopeV1::Conversation,
                correlation_provenance: CorrelationProvenance::AgentSupplied,
                turn_id: TurnId::parse("turn").unwrap(),
                request_id: LogicalRequestId::parse("request").unwrap(),
            },
            direction: ConversationContentDirectionV2::RequestInput,
            phase,
            attempt_id: None,
            fork_id: "fork-request".into(),
            parent_transcript_root: None,
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
            occurred_at_unix_nanos: 1,
            loss_watermark: None,
            completeness_delta: None,
        }
    }

    #[test]
    fn empty_append_chunk_is_valid_but_wrong_digest_and_unknown_fields_fail_closed() {
        let mut value = envelope(ConversationContentPhaseV2::Append);
        value.message_instance_id = Some(MessageInstanceId::parse("message-1").unwrap());
        value.message_role = Some("user".into());
        value.content_kind = Some("text".into());
        value.content_id = Some(ContentId::parse("content-1").unwrap());
        value.content_blob_digest =
            Some(ContentBlobDigest::parse(format!("blob-{}", "11".repeat(32))).unwrap());
        value.message_ordinal = Some(0);
        value.part_ordinal = Some(0);
        value.chunk_ordinal = Some(0);
        value.canonical_media_type = Some("text/plain".into());
        value.canonical_bytes_base64 = Some(String::new());
        assert_eq!(value.validate(), Ok(()));
        let mut json = serde_json::to_value(&value).unwrap();
        json["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ConversationContentEnvelopeV1>(json).is_err());
        value.schema_digest = format!("sha256:{}", "00".repeat(32));
        assert_eq!(
            value.validate(),
            Err(ObservationEnvelopeError::UnsupportedSchema)
        );
    }
}
